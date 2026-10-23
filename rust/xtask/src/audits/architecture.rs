use crate::Result;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const FORBIDDEN_APP_CORE_DEPENDENCIES: &[&str] = &[
    "tauri",
    "windows-sys",
    "sky_desktop_shell",
    "sky_player",
    "sky_native_adapters",
];
const ADAPTER_DEPENDENCY_KINDS: &[&str] =
    &["dependencies", "build-dependencies", "dev-dependencies"];

fn dependency_table_contains(table: Option<&toml::Value>, dependency: &str) -> bool {
    table
        .and_then(toml::Value::as_table)
        .is_some_and(|entries| {
            entries.iter().any(|(name, value)| {
                name == dependency
                    || value.get("package").and_then(toml::Value::as_str) == Some(dependency)
            })
        })
}

fn adapter_dependency_boundary(manifest: &toml::Value) -> Result<()> {
    for kind in ADAPTER_DEPENDENCY_KINDS {
        if dependency_table_contains(manifest.get(kind), "sky_player") {
            return Err("sky_native_adapters must not depend on sky_player".into());
        }
        if dependency_table_contains(manifest.get(kind), "sky_dispatch_win32") {
            return Err(
                "sky_native_adapters may depend on sky_dispatch_win32 only for Windows".into(),
            );
        }
    }

    let mut has_windows_dispatch_dependency = false;
    if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
        for (target, config) in targets {
            for kind in ADAPTER_DEPENDENCY_KINDS {
                if dependency_table_contains(config.get(kind), "sky_player") {
                    return Err("sky_native_adapters must not depend on sky_player".into());
                }
                if dependency_table_contains(config.get(kind), "sky_dispatch_win32") {
                    if target != "cfg(windows)" || *kind != "dependencies" {
                        return Err(
                            "sky_native_adapters may depend on sky_dispatch_win32 only for Windows"
                                .into(),
                        );
                    }
                    has_windows_dispatch_dependency = true;
                }
            }
        }
    }
    if !has_windows_dispatch_dependency {
        return Err(
            "sky_native_adapters must declare its sky_dispatch_win32 dependency for Windows".into(),
        );
    }
    Ok(())
}

fn adapter_boundary(root: &Path) -> Result<()> {
    let manifest_path = root.join("rust/crates/sky_native_adapters/Cargo.toml");
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(manifest_path)?)?;
    adapter_dependency_boundary(&manifest)
}

fn app_core_boundary(root: &Path) -> Result<()> {
    let manifest_path = root.join("rust/crates/sky_app_core/Cargo.toml");
    let manifest: toml::Value = toml::from_str(&fs::read_to_string(&manifest_path)?)?;
    if let Some(dependencies) = manifest.get("dependencies").and_then(toml::Value::as_table)
        && let Some(dependency) = FORBIDDEN_APP_CORE_DEPENDENCIES
            .iter()
            .find(|name| dependencies.contains_key(**name))
    {
        return Err(format!("sky_app_core must not depend directly on {dependency}").into());
    }
    let source_root = root.join("rust/crates/sky_app_core/src");
    for entry in WalkDir::new(source_root)
        .follow_links(false)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        if !entry.file_type().is_file()
            || entry.path().extension().and_then(|ext| ext.to_str()) != Some("rs")
        {
            continue;
        }
        let source = fs::read_to_string(entry.path())?;
        let clean = crate::checks::strip_rust_comments(&source);
        let tokens = [
            "tauri",
            "windows-sys",
            "windows_sys",
            "sky_desktop_shell",
            "sky_player",
            "sky_native_adapters",
            "std::fs",
            "std::net",
        ];
        if let Some(token) = tokens.iter().find(|token| clean.contains(**token)) {
            return Err(format!(
                "sky_app_core source contains forbidden boundary reference `{token}` in {}",
                entry.path().display()
            )
            .into());
        }
        let without_forbid = clean.replace("#![forbid(unsafe_code)]", "");
        if without_forbid
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| word == "unsafe")
        {
            return Err(format!(
                "sky_app_core contains unsafe code in {}",
                entry.path().display()
            )
            .into());
        }
    }
    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    app_core_boundary(root)?;
    adapter_boundary(root)?;
    crate::checks::architecture(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_core_boundary_passes() {
        app_core_boundary(&crate::repo::root()).expect("sky_app_core boundary");
    }

    #[test]
    fn current_adapter_dependency_boundary_passes() {
        adapter_boundary(&crate::repo::root()).expect("sky_native_adapters dependency boundary");
    }

    #[test]
    fn adapter_dependency_boundary_rejects_player_and_non_windows_dispatch_edges() {
        let fixtures = [
            r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                sky_player = { path = "../sky_player" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
            "#,
            r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
                [target.'cfg(unix)'.dev-dependencies]
                sky_player = { path = "../sky_player" }
            "#,
            r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
            "#,
            r#"
                [dependencies]
                sky_app_core = { path = "../sky_app_core" }
                [target.'cfg(windows)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
                [target.'cfg(unix)'.dependencies]
                sky_dispatch_win32 = { path = "../sky_dispatch_win32" }
            "#,
        ];
        for fixture in fixtures {
            let manifest: toml::Value = toml::from_str(fixture).expect("fixture manifest");
            assert!(adapter_dependency_boundary(&manifest).is_err());
        }
    }
}
