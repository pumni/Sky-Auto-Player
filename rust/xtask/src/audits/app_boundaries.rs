use crate::Result;
use serde_json::Value;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const REQUIRED_FILES: &[&str] = &[
    "apps/desktop/package.json",
    "apps/desktop/bun.lock",
    "apps/desktop/src-tauri/Cargo.toml",
    "apps/desktop/src-tauri/tauri.conf.json",
    "apps/website/package.json",
    "apps/website/bun.lock",
    "apps/website/astro.config.mjs",
    "rust/Cargo.toml",
    "rust/Cargo.lock",
    ".cargo/config.toml",
];

const FORBIDDEN_DUPLICATES: &[&str] = &[
    "desktop",
    "site",
    "package.json",
    "bun.lock",
    "bun.lockb",
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "apps/package.json",
    "apps/bun.lock",
    "apps/bun.lockb",
    "apps/desktop/bun.lockb",
    "apps/desktop/package-lock.json",
    "apps/desktop/yarn.lock",
    "apps/desktop/pnpm-lock.yaml",
    "apps/website/bun.lockb",
    "apps/website/package-lock.json",
    "apps/website/yarn.lock",
    "apps/website/pnpm-lock.yaml",
];

#[derive(Clone, Copy, Debug)]
enum App {
    Desktop,
    Website,
}

impl App {
    fn name(self) -> &'static str {
        match self {
            Self::Desktop => "apps/desktop",
            Self::Website => "apps/website",
        }
    }

    fn forbidden_dependency(self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        match self {
            Self::Desktop => name == "astro" || name.starts_with("@astrojs/"),
            Self::Website => name == "tauri" || name.starts_with("@tauri-apps/"),
        }
    }

    fn forbidden_build_command(self, command: &str) -> bool {
        let command = command.to_ascii_lowercase();
        match self {
            Self::Desktop => command.contains("astro"),
            Self::Website => command.contains("cargo") || command.contains("tauri build"),
        }
    }

    fn references_sibling_app(self, value: &str) -> bool {
        let value = value.to_ascii_lowercase().replace('\\', "/");
        let sibling = match self {
            Self::Desktop => "website",
            Self::Website => "desktop",
        };
        [
            format!("apps/{sibling}/"),
            format!("../{sibling}/"),
            format!("file:../{sibling}"),
            format!("link:../{sibling}"),
        ]
        .iter()
        .any(|marker| value.contains(marker))
    }

    fn forbidden_import(self, specifier: &str) -> bool {
        let specifier = specifier.to_ascii_lowercase().replace('\\', "/");
        match self {
            Self::Desktop => {
                specifier == "astro"
                    || specifier.starts_with("astro/")
                    || specifier.starts_with("@astrojs/")
                    || specifier.contains("apps/website/")
                    || specifier.starts_with("../website/")
                    || specifier.starts_with("../../website/")
            }
            Self::Website => {
                specifier == "tauri"
                    || specifier.starts_with("@tauri-apps/")
                    || specifier.contains("src-tauri/")
                    || specifier.contains("apps/desktop/")
                    || specifier.starts_with("../desktop/")
                    || specifier.starts_with("../../desktop/")
            }
        }
    }
}

fn validate_layout(root: &Path) -> std::result::Result<(), String> {
    for path in REQUIRED_FILES {
        if !root.join(path).is_file() {
            return Err(format!(
                "required application/workspace file is missing: {path}"
            ));
        }
    }
    for path in FORBIDDEN_DUPLICATES {
        if root.join(path).exists() {
            return Err(format!(
                "duplicate or retired application/toolchain path must not exist: {path}"
            ));
        }
    }
    Ok(())
}

fn validate_package_manifest(app: App, source: &str) -> std::result::Result<(), String> {
    let manifest: Value = serde_json::from_str(source)
        .map_err(|error| format!("{} package.json is invalid JSON: {error}", app.name()))?;
    if manifest.get("workspaces").is_some() {
        return Err(format!(
            "{} must keep its independent Bun package and lockfile",
            app.name()
        ));
    }

    for section in ["dependencies", "devDependencies", "optionalDependencies"] {
        if let Some(dependencies) = manifest.get(section).and_then(Value::as_object) {
            if let Some(name) = dependencies
                .keys()
                .find(|name| app.forbidden_dependency(name))
            {
                return Err(format!(
                    "{} {section} must not depend on {name}",
                    app.name()
                ));
            }
            if let Some(name) = dependencies.iter().find_map(|(name, version)| {
                version
                    .as_str()
                    .is_some_and(|version| app.references_sibling_app(version))
                    .then_some(name)
            }) {
                return Err(format!(
                    "{} {section} dependency {name} points into the sibling application",
                    app.name()
                ));
            }
        }
    }

    if let Some(scripts) = manifest.get("scripts").and_then(Value::as_object) {
        if let Some((name, _)) = scripts.iter().find(|(_, command)| {
            command.as_str().is_some_and(|command| {
                app.forbidden_build_command(command) || app.references_sibling_app(command)
            })
        }) {
            return Err(format!(
                "{} script {name} crosses the other application's build boundary",
                app.name()
            ));
        }
    }
    Ok(())
}

fn imported_specifier(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") || trimmed.starts_with("*") {
        return None;
    }
    let marker = if let Some(rest) = trimmed.strip_prefix("import ") {
        if rest.starts_with('\'') || rest.starts_with('"') {
            Some("import ")
        } else {
            trimmed.find(" from ").map(|_| "from ")
        }
    } else if trimmed.starts_with("export ") {
        trimmed.find(" from ").map(|_| "from ")
    } else if trimmed.contains(" from ") {
        Some("from ")
    } else if let Some(position) = trimmed.find("import(") {
        Some(&trimmed[position..])
    } else if let Some(position) = trimmed.find("require(") {
        Some(&trimmed[position..])
    } else {
        None
    }?;

    let rest = trimmed
        .find(marker)
        .map_or(marker, |position| &trimmed[position + marker.len()..])
        .trim_start();
    let quote = rest.chars().next()?;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let specifier = &rest[quote.len_utf8()..];
    let end = specifier.find(quote)?;
    Some(&specifier[..end])
}

fn source_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !matches!(
                entry.file_name().to_str(),
                Some("node_modules" | "dist" | ".astro" | "test-results" | "playwright-report")
            )
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        if matches!(
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("astro" | "js" | "jsx" | "mjs" | "ts" | "tsx")
        ) {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

fn validate_source_imports(app: App, app_root: &Path) -> Result<()> {
    for path in source_files(app_root)? {
        let source = fs::read_to_string(&path)?;
        for (line_number, line) in source.lines().enumerate() {
            if imported_specifier(line).is_some_and(|specifier| app.forbidden_import(specifier)) {
                let relative = path.strip_prefix(app_root).unwrap_or(&path).display();
                return Err(format!(
                    "{} source import crosses the application boundary at {relative}:{}",
                    app.name(),
                    line_number + 1
                )
                .into());
            }
        }
    }
    Ok(())
}

fn contains_word(source: &str, word: &str) -> bool {
    source
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|token| token.eq_ignore_ascii_case(word))
}

fn validate_website_build_source(source: &str) -> std::result::Result<(), String> {
    let source = source.to_ascii_lowercase().replace('\\', "/");
    if contains_word(&source, "cargo")
        || [
            "tauri build",
            "tauri dev",
            "tauri signer",
            "@tauri-apps/",
            "src-tauri",
            "apps/desktop",
            "../desktop/",
        ]
        .iter()
        .any(|marker| source.contains(marker))
    {
        return Err(
            "Astro build configuration and website scripts must not invoke or import the desktop toolchain"
                .to_owned(),
        );
    }
    Ok(())
}

fn validate_website_build_sources(app_root: &Path) -> Result<()> {
    let mut files = vec![app_root.join("astro.config.mjs")];
    files.extend(source_files(&app_root.join("scripts"))?);
    for path in files {
        let source = fs::read_to_string(&path)?;
        validate_website_build_source(&source).map_err(|error| {
            format!(
                "{}: {error}",
                path.strip_prefix(app_root).unwrap_or(&path).display()
            )
        })?;
    }
    Ok(())
}

fn normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(name) => normalized.push(name),
        }
    }
    normalized
}

fn canonical(path: PathBuf, label: &str) -> std::result::Result<PathBuf, String> {
    path.canonicalize().map_err(|error| {
        format!(
            "{label} path does not resolve: {} ({error})",
            path.display()
        )
    })
}

fn validate_relative_packaging_paths(root: &Path, source: &str) -> std::result::Result<(), String> {
    let config: Value = serde_json::from_str(source)
        .map_err(|error| format!("Tauri config is invalid JSON: {error}"))?;
    let repo_root = canonical(root.to_path_buf(), "repository root")?;
    let desktop_root = canonical(repo_root.join("apps/desktop"), "desktop root")?;
    let config_root = canonical(desktop_root.join("src-tauri"), "Tauri config root")?;
    let build = config
        .get("build")
        .and_then(Value::as_object)
        .ok_or_else(|| "Tauri config is missing its build object".to_owned())?;
    if build.get("beforeBuildCommand").and_then(Value::as_str)
        != Some("bun run build && cd ..\\.. && cargo xtask calibration build")
    {
        return Err(
            "Tauri beforeBuildCommand must return from apps/desktop to the repository root".into(),
        );
    }
    if normalize(&desktop_root.join("..\\..")) != repo_root {
        return Err(
            "Tauri beforeBuildCommand relative root does not resolve to the repository".into(),
        );
    }
    if build.get("frontendDist").and_then(Value::as_str) != Some("../dist")
        || normalize(&config_root.join("../dist")) != normalize(&desktop_root.join("dist"))
    {
        return Err("Tauri frontendDist must resolve to apps/desktop/dist".into());
    }

    let resources = config
        .get("bundle")
        .and_then(|bundle| bundle.get("resources"))
        .and_then(Value::as_object)
        .ok_or_else(|| "Tauri config is missing bundle.resources".to_owned())?;
    for (source_path, expected) in [
        ("../../../songs/*", repo_root.join("songs")),
        (
            "../../../builtin-songs/manifest.json",
            repo_root.join("builtin-songs/manifest.json"),
        ),
    ] {
        if !resources.contains_key(source_path) {
            return Err(format!(
                "Tauri resource source path is stale: {source_path}"
            ));
        }
        let resolved = canonical(
            config_root.join(source_path.trim_end_matches("/*")),
            source_path,
        )?;
        if resolved != canonical(expected, source_path)? {
            return Err(format!(
                "Tauri resource path resolves outside its canonical source: {source_path}"
            ));
        }
    }
    let calibration_source = config_root.join("resources/native_calibration.exe");
    if !resources.contains_key("resources/native_calibration.exe")
        || canonical(
            calibration_source
                .parent()
                .unwrap_or(&config_root)
                .to_path_buf(),
            "calibration resource directory",
        )? != canonical(
            config_root.join("resources"),
            "calibration resource directory",
        )?
        || resources
            .get("resources/native_calibration.exe")
            .and_then(Value::as_str)
            != Some("native_calibration.exe")
    {
        return Err(
            "Tauri calibration resource must stage into the packaged application root".into(),
        );
    }

    let windows = config
        .get("bundle")
        .and_then(|bundle| bundle.get("windows"))
        .and_then(Value::as_object)
        .ok_or_else(|| "Tauri config is missing bundle.windows".to_owned())?;
    let sign_command = windows
        .get("signCommand")
        .and_then(Value::as_str)
        .ok_or_else(|| "Tauri signCommand is missing".to_owned())?;
    let signer = "../../../scripts/sign_v4_authenticode.ps1";
    if !sign_command.contains(signer)
        || canonical(config_root.join(signer), "Authenticode signing script")?
            != canonical(
                repo_root.join("scripts/sign_v4_authenticode.ps1"),
                "Authenticode signing script",
            )?
    {
        return Err(
            "Tauri Authenticode script path must resolve to the repository signing policy".into(),
        );
    }

    let hook = windows
        .get("nsis")
        .and_then(|nsis| nsis.get("installerHooks"))
        .and_then(Value::as_str)
        .ok_or_else(|| "Tauri NSIS installer hook path is missing".to_owned())?;
    if canonical(config_root.join(hook), "NSIS installer hook")?
        != canonical(config_root.join("nsis-hooks.nsh"), "NSIS installer hook")?
    {
        return Err("Tauri NSIS installer hook must resolve beside tauri.conf.json".into());
    }

    let watch_folders = build
        .get("additionalWatchFolders")
        .and_then(Value::as_array)
        .ok_or_else(|| "Tauri additionalWatchFolders is missing".to_owned())?;
    for (relative, expected) in [
        ("../../../rust/crates", repo_root.join("rust/crates")),
        (
            "../../../rust/Cargo.toml",
            repo_root.join("rust/Cargo.toml"),
        ),
        (
            "../../../rust/Cargo.lock",
            repo_root.join("rust/Cargo.lock"),
        ),
    ] {
        if !watch_folders
            .iter()
            .any(|path| path.as_str() == Some(relative))
            || canonical(config_root.join(relative), "Tauri watch folder")?
                != canonical(expected, "Tauri watch folder")?
        {
            return Err(format!("Tauri watch folder does not resolve to {relative}"));
        }
    }
    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    validate_layout(root).map_err(|error| format!("application layout contract: {error}"))?;
    for (app, manifest_path) in [
        (App::Desktop, "apps/desktop/package.json"),
        (App::Website, "apps/website/package.json"),
    ] {
        let manifest = fs::read_to_string(root.join(manifest_path))?;
        validate_package_manifest(app, &manifest)
            .map_err(|error| format!("application dependency contract: {error}"))?;
        validate_source_imports(app, &root.join(app.name()))?;
    }
    validate_website_build_sources(&root.join("apps/website"))?;
    let tauri_config = fs::read_to_string(root.join("apps/desktop/src-tauri/tauri.conf.json"))?;
    validate_relative_packaging_paths(root, &tauri_config)
        .map_err(|error| format!("relocated package path contract: {error}"))?;
    println!("[xtask] desktop/website application boundaries: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn desktop_and_website_manifests_reject_cross_application_build_dependencies() {
        let website = r#"{"scripts":{"build":"astro build"},"dependencies":{"astro":"7.4.0"}}"#;
        let desktop = r#"{"scripts":{"build":"vite build"},"dependencies":{"vite":"8.3.4"}}"#;
        assert!(validate_package_manifest(App::Website, website).is_ok());
        assert!(validate_package_manifest(App::Desktop, desktop).is_ok());

        for invalid in [
            r#"{"dependencies":{"@tauri-apps/api":"2.12.4"}}"#,
            r#"{"scripts":{"prebuild":"cargo xtask website"}}"#,
            r#"{"dependencies":{"desktop-ui":"file:../desktop"}}"#,
        ] {
            assert!(validate_package_manifest(App::Website, invalid).is_err());
        }
        for invalid in [
            r#"{"devDependencies":{"@astrojs/check":"0.9.10"}}"#,
            r#"{"scripts":{"check":"astro check"}}"#,
            r#"{"dependencies":{"website":"file:../website"}}"#,
        ] {
            assert!(validate_package_manifest(App::Desktop, invalid).is_err());
        }
        assert!(
            validate_package_manifest(App::Website, r#"{"workspaces":["../desktop"]}"#).is_err()
        );
    }

    #[test]
    fn app_source_imports_cannot_cross_the_desktop_website_boundary() {
        for (app, line) in [
            (
                App::Website,
                "import { invoke } from '@tauri-apps/api/core';",
            ),
            (
                App::Website,
                "export { default } from '../../desktop/src/App';",
            ),
            (App::Desktop, "import { defineConfig } from 'astro/config';"),
            (
                App::Desktop,
                "export { routes } from '../website/src/routes';",
            ),
        ] {
            assert!(
                imported_specifier(line).is_some_and(|specifier| app.forbidden_import(specifier)),
                "expected boundary violation for {line}"
            );
        }
        for (app, line) in [
            (App::Website, "import { defineConfig } from 'astro/config';"),
            (App::Desktop, "import React from 'react';"),
        ] {
            assert!(
                !imported_specifier(line).is_some_and(|specifier| app.forbidden_import(specifier))
            );
        }
    }

    #[test]
    fn astro_build_entrypoints_cannot_spawn_cargo_or_tauri_builds() {
        assert!(
            validate_website_build_source("import { defineConfig } from 'astro/config';").is_ok()
        );
        for invalid in [
            "import { execSync } from 'node:child_process'; execSync('cargo xtask calibration build');",
            "import { exec } from 'node:child_process'; exec('bunx tauri build');",
            "import catalog from '../../apps/desktop/src-tauri/assets';",
        ] {
            assert!(validate_website_build_source(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn application_layout_requires_independent_locks_and_rejects_duplicate_roots() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after the Unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sky-xtask-app-boundaries-{}-{nonce}",
            std::process::id()
        ));
        for path in REQUIRED_FILES {
            let file = root.join(path);
            fs::create_dir_all(file.parent().expect("fixture file has a parent")).unwrap();
            fs::write(file, "fixture").unwrap();
        }
        assert!(validate_layout(&root).is_ok());

        fs::create_dir_all(root.join("desktop")).unwrap();
        assert!(validate_layout(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relocated_tauri_paths_resolve_from_their_documented_windows_working_directories() {
        let root = crate::repo::root();
        let config_path = root.join("apps/desktop/src-tauri/tauri.conf.json");
        let source = fs::read_to_string(&config_path).unwrap();
        assert!(validate_relative_packaging_paths(&root, &source).is_ok());

        let stale_build_cwd = source.replace("cd ..\\\\..", "cd ..");
        assert!(validate_relative_packaging_paths(&root, &stale_build_cwd).is_err());
        let stale_signer = source.replace("../../../scripts", "../../scripts");
        assert!(validate_relative_packaging_paths(&root, &stale_signer).is_err());
        let stale_song_root = source.replace("../../../songs/*", "../../songs/*");
        assert!(validate_relative_packaging_paths(&root, &stale_song_root).is_err());
    }
}
