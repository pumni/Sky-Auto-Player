use crate::Result;
use crate::audits::source::walk_source;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Eq, PartialEq)]
struct TauriFeatureResolution {
    default: BTreeSet<String>,
    dev: BTreeSet<String>,
}

fn feature_entries(
    features: &toml::value::Table,
    name: &str,
) -> std::result::Result<Vec<String>, String> {
    let value = features
        .get(name)
        .ok_or_else(|| format!("feature `{name}` is missing"))?;
    let entries = value
        .as_array()
        .ok_or_else(|| format!("feature `{name}` must be an array"))?;
    entries
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("feature `{name}` contains a non-string entry"))
        })
        .collect()
}

fn collect_feature_closure(
    name: &str,
    features: &toml::value::Table,
    values: &mut BTreeSet<String>,
    visiting: &mut BTreeSet<String>,
) -> std::result::Result<(), String> {
    if !visiting.insert(name.to_owned()) {
        return Err(format!("feature graph contains a cycle at `{name}`"));
    }
    for entry in feature_entries(features, name)? {
        values.insert(entry.clone());
        if features.contains_key(&entry) {
            collect_feature_closure(&entry, features, values, visiting)?;
        }
    }
    visiting.remove(name);
    Ok(())
}

fn simulate_tauri_dev_features(
    default_entries: &[String],
    features: &toml::value::Table,
) -> std::result::Result<BTreeSet<String>, String> {
    let mut dev = BTreeSet::new();
    for feature in default_entries {
        let entries = feature_entries(features, feature)?;
        if !entries.iter().any(|entry| entry == "tauri/custom-protocol") {
            dev.insert(feature.clone());
        }
    }
    Ok(dev)
}

fn tauri_feature_contract_manifest(
    source: &str,
) -> std::result::Result<TauriFeatureResolution, String> {
    let manifest = toml::from_str::<toml::Value>(source.trim_start())
        .map_err(|error| format!("invalid Cargo.toml: {error}"))?;
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "Cargo.toml is missing [dependencies]".to_owned())?;
    let tauri_dependency = dependencies
        .get("tauri")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "tauri dependency must use an inline table".to_owned())?;
    if tauri_dependency
        .get("default-features")
        .and_then(toml::Value::as_bool)
        != Some(false)
    {
        return Err("tauri dependency must keep default-features = false".to_owned());
    }

    let features = manifest
        .get("features")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "Cargo.toml is missing [features]".to_owned())?;
    let default_entries = feature_entries(features, "default")?;
    let default = default_entries.iter().cloned().collect::<BTreeSet<_>>();
    let expected_default = ["desktop-runtime", "packaged-assets"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    if default != expected_default || default_entries.len() != default.len() {
        return Err(format!(
            "default features must directly contain exactly `desktop-runtime` and `packaged-assets`; found {default:?}"
        ));
    }

    let desktop_runtime = feature_entries(features, "desktop-runtime")?;
    if !desktop_runtime.iter().any(|entry| entry == "tauri/wry") {
        return Err("desktop-runtime must directly contain `tauri/wry`".to_owned());
    }
    let forbidden_runtime_entries = [
        "tauri/custom-protocol",
        "tauri/x11",
        "tauri/dbus",
        "tauri/dynamic-acl",
        "tauri/common-controls-v6",
    ];
    let mut runtime_closure = BTreeSet::new();
    collect_feature_closure(
        "desktop-runtime",
        features,
        &mut runtime_closure,
        &mut BTreeSet::new(),
    )?;
    if let Some(forbidden) = forbidden_runtime_entries
        .iter()
        .find(|entry| runtime_closure.contains(**entry))
    {
        return Err(format!(
            "desktop-runtime must not contain `{forbidden}` directly or through another feature"
        ));
    }

    let packaged_assets = feature_entries(features, "packaged-assets")?;
    for required in ["tauri/custom-protocol", "tauri/compression"] {
        if !packaged_assets.iter().any(|entry| entry == required) {
            return Err(format!(
                "packaged-assets must directly contain `{required}`"
            ));
        }
    }

    let dev = simulate_tauri_dev_features(&default_entries, features)?;
    let expected_dev = ["desktop-runtime".to_owned()]
        .into_iter()
        .collect::<BTreeSet<_>>();
    if dev != expected_dev {
        return Err(format!(
            "Tauri CLI dev feature simulation must resolve to `desktop-runtime`; found {dev:?}"
        ));
    }

    Ok(TauriFeatureResolution { default, dev })
}

pub(super) fn tauri_feature_contract(root: &Path) -> Result<()> {
    let manifest_path = root.join("apps/desktop/src-tauri/Cargo.toml");
    let resolution = tauri_feature_contract_manifest(&fs::read_to_string(&manifest_path)?)
        .map_err(|error| format!("{}: {error}", manifest_path.display()))?;
    println!(
        "[xtask] Tauri feature contract: PASS (default={:?}, dev={:?})",
        resolution.default, resolution.dev
    );
    Ok(())
}

fn validate_tauri_inner_cargo_args(
    args: &[String],
    surface: &str,
) -> std::result::Result<(), String> {
    let separator = args
        .iter()
        .position(|argument| argument == "--")
        .ok_or_else(|| format!("{surface} is missing Tauri's Cargo argument separator"))?;
    if !args[..separator]
        .windows(2)
        .any(|pair| pair[0] == "tauri" && pair[1] == "build")
    {
        return Err(format!("{surface} does not invoke `tauri build`"));
    }
    if args[..separator]
        .iter()
        .any(|argument| argument == "--locked")
    {
        return Err(format!(
            "{surface} places `--locked` before Tauri's Cargo separator"
        ));
    }
    let cargo_args = &args[separator + 1..];
    let locked_count = cargo_args
        .iter()
        .filter(|argument| argument.as_str() == "--locked")
        .count();
    if locked_count != 1 {
        return Err(format!(
            "{surface} must pass exactly one `--locked` to Cargo after Tauri's separator"
        ));
    }
    if !cargo_args
        .windows(2)
        .any(|pair| pair[0] == "--profile" && pair[1] == "dist")
    {
        return Err(format!(
            "{surface} must preserve the `dist` Cargo profile after Tauri's separator"
        ));
    }
    Ok(())
}

fn validate_tauri_shell_command(command: &str, surface: &str) -> std::result::Result<(), String> {
    let args = command
        .split_whitespace()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    validate_tauri_inner_cargo_args(&args, surface)
}

fn rust_string_array_arguments(
    source: &str,
    function_marker: &str,
    array_marker: &str,
    array_end: &str,
    surface: &str,
) -> std::result::Result<Vec<String>, String> {
    let function = source
        .find(function_marker)
        .map(|position| &source[position..])
        .ok_or_else(|| format!("{surface} source function is missing"))?;
    let array = function
        .split_once(array_marker)
        .map(|(_, remaining)| remaining)
        .ok_or_else(|| format!("{surface} argument array is missing"))?;
    let body = array
        .split_once(array_end)
        .map(|(body, _)| body)
        .ok_or_else(|| format!("{surface} argument array is not terminated"))?;
    let args = body
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if args.is_empty() {
        return Err(format!("{surface} has no literal command arguments"));
    }
    Ok(args)
}

fn validate_rust_tauri_build(
    source: &str,
    function_marker: &str,
    array_marker: &str,
    array_end: &str,
    surface: &str,
) -> std::result::Result<(), String> {
    let args =
        rust_string_array_arguments(source, function_marker, array_marker, array_end, surface)?;
    validate_tauri_inner_cargo_args(&args, surface)
}

pub(super) fn tauri_locked_build_contract(root: &Path) -> Result<()> {
    let package_path = root.join("apps/desktop/package.json");
    let package = serde_json::from_str::<Value>(&fs::read_to_string(&package_path)?)?;
    let package_command = package
        .get("scripts")
        .and_then(Value::as_object)
        .and_then(|scripts| scripts.get("tauri:build"))
        .and_then(Value::as_str)
        .ok_or("desktop package must define its tauri:build script")?;
    validate_tauri_shell_command(package_command, "apps/desktop tauri:build")
        .map_err(|error| format!("{}: {error}", package_path.display()))?;

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path)?;
    let ci_builds = ci
        .lines()
        .map(str::trim)
        .filter(|line| line.contains("bun run tauri build"))
        .collect::<Vec<_>>();
    if ci_builds.len() != 2 {
        return Err(format!(
            "CI must retain exactly two direct Tauri dist builders; found {}",
            ci_builds.len()
        )
        .into());
    }
    for (index, command) in ci_builds.iter().enumerate() {
        validate_tauri_shell_command(command, &format!("CI Tauri builder {}", index + 1))?;
    }

    let packaging_docs_path = root.join("docs/v4-tauri-packaging.md");
    let packaging_docs = fs::read_to_string(&packaging_docs_path)?;
    let documented_builds = packaging_docs
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("bun run tauri build"))
        .collect::<Vec<_>>();
    if documented_builds.len() != 1 {
        return Err(format!(
            "{} must keep exactly one current Tauri packaging command example; found {}",
            packaging_docs_path.display(),
            documented_builds.len()
        )
        .into());
    }
    validate_tauri_shell_command(documented_builds[0], "v4 Tauri packaging example")?;

    let release_path = root.join("xtask/src/release_v4.rs");
    let release = fs::read_to_string(&release_path)?;
    validate_rust_tauri_build(
        &release,
        "fn build_unsigned(",
        "let build_args = vec![",
        "];",
        "release_v4::build_unsigned",
    )
    .map_err(|error| format!("{}: {error}", release_path.display()))?;

    let credential_path = root.join("xtask/src/credential_cli.rs");
    let credential = fs::read_to_string(&credential_path)?;
    validate_rust_tauri_build(
        &credential,
        "fn build_candidate(",
        ".args([",
        "])",
        "credential CLI candidate builder",
    )
    .map_err(|error| format!("{}: {error}", credential_path.display()))?;

    let fixture_path = root.join("xtask/src/updater_fixture.rs");
    let fixture = fs::read_to_string(&fixture_path)?;
    validate_rust_tauri_build(
        &fixture,
        "fn build_bridge_internal(",
        "let build_args = vec![",
        "];",
        "updater fixture bridge builder",
    )
    .map_err(|error| format!("{}: {error}", fixture_path.display()))?;

    println!("[xtask] Tauri Cargo lock mode: PASS (6 active builders)");
    Ok(())
}

fn tauri_build_freshness_contract_manifest(source: &str) -> std::result::Result<(), String> {
    let config = serde_json::from_str::<Value>(source)
        .map_err(|error| format!("invalid Tauri config JSON: {error}"))?;
    let build = config
        .get("build")
        .and_then(Value::as_object)
        .ok_or_else(|| "Tauri config is missing its build object".to_owned())?;
    let before_build = build.get("beforeBuildCommand").and_then(Value::as_str);
    if before_build != Some("bun run build") {
        return Err(
            "Tauri packaging must build the frontend without legacy calibration tooling".to_owned(),
        );
    }
    let watch_folders = build
        .get("additionalWatchFolders")
        .and_then(Value::as_array)
        .ok_or_else(|| "Tauri config is missing build.additionalWatchFolders".to_owned())?;
    let watch_folders = watch_folders
        .iter()
        .map(|path| {
            path.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Tauri additionalWatchFolders must contain strings".to_owned())
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for required in [
        "../../../crates",
        "../../../Cargo.toml",
        "../../../Cargo.lock",
    ] {
        if !watch_folders.iter().any(|path| path == required) {
            return Err(format!(
                "Tauri additionalWatchFolders is missing `{required}`"
            ));
        }
    }
    if watch_folders
        .iter()
        .any(|path| path == "../../../" || path == "../../../.")
    {
        return Err(
            "Tauri additionalWatchFolders must not watch the entire repository workspace"
                .to_owned(),
        );
    }
    Ok(())
}

pub(super) fn tauri_build_freshness_contract(root: &Path) -> Result<()> {
    let path = root.join("apps/desktop/src-tauri/tauri.conf.json");
    tauri_build_freshness_contract_manifest(&fs::read_to_string(&path)?)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let diagnostic_path = root.join("apps/desktop/scripts/dev-build-info.mjs");
    let diagnostic = fs::read_to_string(&diagnostic_path)?;
    for marker in [
        "git",
        "status",
        "--selftest-build-info",
        "expected_debug_executable",
        "rustc",
        "freshness",
        "INCOMPLETE",
        "process.exitCode",
    ] {
        if !diagnostic.contains(marker) {
            return Err(format!(
                "build freshness diagnostic is missing its required marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in [
        "rmSync",
        "writeFile",
        "unlink",
        "cargo clean",
        "Remove-Item",
    ] {
        if diagnostic.contains(forbidden) {
            return Err(format!(
                "build freshness diagnostic must remain read-only; found {forbidden}"
            )
            .into());
        }
    }
    let acceptance_path = root.join("scripts/test_build_provenance_switch.ps1");
    let acceptance = fs::read_to_string(&acceptance_path)?;
    for marker in [
        "checkout --quiet --detach",
        "CARGO_TARGET_DIR",
        "--selftest-build-info",
        "embedded_native_build_commit",
        "same target, no cargo clean",
    ] {
        if !acceptance.contains(marker) {
            return Err(format!(
                "build provenance switch acceptance helper is missing its marker: {marker}"
            )
            .into());
        }
    }
    println!("[xtask] Tauri build freshness contract: PASS");
    Ok(())
}

const LEGACY_RELEASE_TOPOLOGY_MARKERS: &[&str] = &[
    "Sky-Auto-Player-Releases",
    "V4_RELEASE_AUTHORITY_TOKEN",
    "V4_RELEASE_AUTHORITY_REPOSITORY",
    "Invoke-AuthorityApi",
    "AuthorityTokenEnv",
    "AuthorityCheckout",
    "release-authority",
    "release_authority",
];

fn find_legacy_release_topology_marker(source: &str) -> Option<&'static str> {
    LEGACY_RELEASE_TOPOLOGY_MARKERS
        .iter()
        .copied()
        .find(|marker| source.contains(marker))
}

const ACTIVE_RELEASE_SURFACES: &[&str] = &[
    "xtask/src/main.rs",
    "xtask/src/release_metadata.rs",
    "xtask/src/release_v4.rs",
    "xtask/src/nsis_harness.rs",
    "xtask/src/updater_fixture.rs",
    ".github/workflows/ci.yml",
    ".github/workflows/release-v4.yml",
    ".github/workflows/v4-metadata-recovery.yml",
    "apps/desktop/src-tauri/src/native_update.rs",
    "apps/desktop/src-tauri/tauri.conf.json",
    "apps/desktop/src-tauri/Cargo.toml",
];

fn active_release_surface_source<'a>(path: &Path, source: &'a str) -> &'a str {
    if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(production, _)| production)
    } else {
        source
    }
}

fn validate_active_release_surface(relative: &str, source: &str) -> Result<()> {
    if let Some(forbidden) = find_legacy_release_topology_marker(source) {
        return Err(format!(
            "legacy release-topology marker `{forbidden}` remains in active surface {relative}"
        )
        .into());
    }
    Ok(())
}

pub(super) fn release_metadata_contract(root: &Path) -> Result<()> {
    let native = fs::read_to_string(root.join("apps/desktop/src-tauri/src/native_update.rs"))?;
    for marker in [
        "V4_STABLE_METADATA_ENDPOINT",
        "V4_BETA_METADATA_ENDPOINT",
        "endpoints(vec![endpoint])",
        "V4_TAURI_UPDATER_PUBLIC_KEY",
        ".pubkey(public_key)",
    ] {
        if !native.contains(marker) {
            return Err(format!(
                "Rust updater metadata is missing fixed contract marker: {marker}"
            )
            .into());
        }
    }
    if native.contains("api.github.com/repos/pumni/Sky-Auto-Player/releases") {
        return Err("runtime updater must not use the release-management API".into());
    }
    let generator = fs::read_to_string(root.join("xtask/src/release_metadata.rs"))?;
    for marker in [
        "RELEASE_REPOSITORY: &str = \"pumni/Sky-Auto-Player\"",
        "STABLE_METADATA_PATH",
        "BETA_METADATA_PATH",
        "canonical_asset_url",
        "validate_monotonic",
        "candidate_version <= current_version",
    ] {
        if !generator.contains(marker) {
            return Err(format!("Rust metadata generator is missing {marker}").into());
        }
    }
    let release = fs::read_to_string(root.join("xtask/src/release_v4.rs"))?;
    for marker in [
        "release_metadata::generate",
        "release_metadata::validate_monotonic",
        "raw.githubusercontent.com",
        "validate_beta_keeps_latest",
        "make_latest",
    ] {
        if !release.contains(marker) {
            return Err(format!("Rust release transaction is missing {marker}").into());
        }
    }
    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?;
    if !ci.contains("cargo xtask release-v4 self-test") {
        return Err("release_contract CI must execute the Rust mock transport scenarios".into());
    }
    for relative in ACTIVE_RELEASE_SURFACES {
        let path = root.join(relative);
        let content = fs::read_to_string(&path)?;
        validate_active_release_surface(relative, active_release_surface_source(&path, &content))?;
    }
    let bundle = fs::read_to_string(root.join("xtask/src/tauri_bundle.rs"))?;
    for marker in [
        "schema_version: 2",
        "evidence_type: \"tauri-nsis-artifact\"",
        "installer_sha256",
        "updater_signature_sha256",
    ] {
        if !bundle.contains(marker) {
            return Err(
                format!("Tauri artifact evidence is missing exact-byte marker: {marker}").into(),
            );
        }
    }
    println!("[xtask] Rust v4 metadata and release authority contract: PASS");
    Ok(())
}
fn v4_legacy_updater_source_contract(source: &str, surface: &str) -> Result<()> {
    for forbidden in [
        "sky_updater",
        "Sky-Auto-Player-Updater.exe",
        "sky_updater_e2e",
        "cargo xtask dist",
        "verify-dist",
        "MANIFEST.json",
        "MANIFEST.json.sig",
        "SKY_UPDATE_SIGNING_KEY_HEX",
        "pep440_rs",
        "packaging.version",
        "ActiveUpdateState",
        "active_update_for_install",
        "legacy_old_dir_sweep_pending",
        "last_notified_version",
        "pending_update_version",
        "auto_apply",
    ] {
        if source.contains(forbidden) {
            return Err(format!(
                "retired v3 updater marker `{forbidden}` remains in current v4 surface {surface}"
            )
            .into());
        }
    }
    Ok(())
}

fn v4_native_adapter_source_contract(source: &str, surface: &str) -> Result<()> {
    for forbidden in [
        "sky_updater",
        "Sky-Auto-Player-Updater.exe",
        "ActiveUpdateState",
        "active_update_for_install",
        "pep440_rs",
    ] {
        if source.contains(forbidden) {
            return Err(format!(
                "retired v3 updater marker `{forbidden}` remains in sky_native_adapters surface {surface}"
            )
            .into());
        }
    }
    Ok(())
}

fn v4_native_adapter_required_drop_source_contract(source: &str) -> Result<()> {
    for required_drop in [
        "last_notified_version",
        "legacy_old_dir_sweep_pending",
        "pending_update_version",
        "auto_apply",
    ] {
        if !source.contains(required_drop) {
            return Err(format!(
                "sky_native_adapters settings migration must explicitly drop retired update key `{required_drop}` on save/migrate"
            )
            .into());
        }
    }
    Ok(())
}

fn v4_native_adapter_retirement(root: &Path) -> Result<()> {
    const SOURCE_ROOT: &str = "crates/sky_native_adapters/src";
    let settings_store_path = "crates/sky_native_adapters/src/settings_store.rs";
    let mut settings_store_found = false;

    for source_path in walk_source(root, SOURCE_ROOT)? {
        let file_name = source_path.file_name().and_then(|name| name.to_str());
        if file_name == Some("tests.rs")
            || file_name.is_some_and(|name| name.ends_with("_tests.rs"))
        {
            continue;
        }
        let relative = source_path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(&source_path)?;
        let production = source
            .split_once("\n#[cfg(test)]")
            .map(|(production, _)| production)
            .unwrap_or(&source);
        v4_native_adapter_source_contract(production, &relative)?;
        if relative == settings_store_path {
            v4_native_adapter_required_drop_source_contract(production)?;
            settings_store_found = true;
        }
    }

    if !settings_store_found {
        return Err(format!(
            "sky_native_adapters settings store source is missing: {settings_store_path}"
        )
        .into());
    }
    Ok(())
}

fn is_xtask_contract_source(root: &Path, source_path: &Path) -> bool {
    source_path.file_name().and_then(|name| name.to_str()) == Some("checks.rs")
        || source_path.starts_with(root.join("xtask/src/checks"))
}

pub(super) fn v4_legacy_updater_retirement(root: &Path) -> Result<()> {
    let desktop_manifest = root.join("apps/desktop/src-tauri/Cargo.toml");
    v4_legacy_updater_source_contract(
        &fs::read_to_string(&desktop_manifest)?,
        desktop_manifest.to_string_lossy().as_ref(),
    )?;

    let workspace_manifest = root.join("Cargo.toml");
    v4_legacy_updater_source_contract(
        &fs::read_to_string(&workspace_manifest)?,
        workspace_manifest.to_string_lossy().as_ref(),
    )?;

    let lockfile = root.join("Cargo.lock");
    let lockfile_source = fs::read_to_string(&lockfile)?;
    for forbidden in ["name = \"sky_updater\"", "name = \"pep440_rs\""] {
        if lockfile_source.contains(forbidden) {
            return Err(format!(
                "retired v3 dependency `{forbidden}` remains in {}",
                lockfile.display()
            )
            .into());
        }
    }

    let startup_guard = root.join("apps/desktop/src-tauri/src/startup_guard.rs");
    if startup_guard.exists() {
        return Err(format!(
            "retired custom updater startup admission path remains: {}",
            startup_guard.display()
        )
        .into());
    }

    let current_v4_surfaces = [
        "apps/desktop/src-tauri/src",
        "apps/desktop/src",
        "crates/sky_app_core/src",
        "crates/sky_player/src",
        "crates/sky_dispatch_core/src",
        "crates/sky_dispatch_win32/src",
        "xtask/src",
        ".github/workflows/ci.yml",
        ".github/workflows/release-v4.yml",
        "xtask/src/updater_fixture.rs",
        "xtask/src/release_v4.rs",
        "tests/fixtures/settings/compatibility_cases.json",
    ];
    for relative in current_v4_surfaces {
        let path = root.join(relative);
        if path.is_dir() {
            for source_path in walk_source(root, relative)? {
                if is_xtask_contract_source(root, &source_path) {
                    continue;
                }
                let source = fs::read_to_string(&source_path)?;
                let source = if relative == "xtask/src" {
                    source
                        .split_once("\n#[cfg(test)]")
                        .map(|(production, _)| production.to_owned())
                        .unwrap_or(source)
                } else {
                    source
                };
                v4_legacy_updater_source_contract(&source, source_path.to_string_lossy().as_ref())?;
            }
        } else if path.is_file() {
            v4_legacy_updater_source_contract(&fs::read_to_string(&path)?, relative)?;
        }
    }

    v4_native_adapter_retirement(root)?;

    println!("[xtask] v4 legacy updater retirement guards: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo;

    const VALID_TAURI_FEATURE_MANIFEST: &str = r#"
[dependencies]
tauri = { version = "2.11.5", default-features = false }

[features]
default = ["desktop-runtime", "packaged-assets"]
desktop-runtime = ["tauri/wry"]
packaged-assets = ["tauri/custom-protocol", "tauri/compression"]
tauri-test = ["tauri/test"]
"#;

    fn fixture_features(source: &str) -> toml::value::Table {
        toml::from_str::<toml::Value>(source.trim_start())
            .unwrap()
            .get("features")
            .and_then(toml::Value::as_table)
            .unwrap()
            .clone()
    }

    #[test]
    fn tauri_feature_contract_accepts_split_runtime_and_packaged_assets() {
        let resolution = tauri_feature_contract_manifest(VALID_TAURI_FEATURE_MANIFEST).unwrap();
        assert_eq!(
            resolution.default,
            ["desktop-runtime", "packaged-assets"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        assert_eq!(
            resolution.dev,
            ["desktop-runtime".to_owned()].into_iter().collect()
        );
    }

    #[test]
    fn tauri_builds_pass_locked_once_after_the_cargo_separator() {
        let valid = [
            "bun",
            "run",
            "tauri",
            "build",
            "--ci",
            "--",
            "--locked",
            "--profile",
            "dist",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert!(validate_tauri_inner_cargo_args(&valid, "fixture").is_ok());

        let missing = ["tauri", "build", "--", "--profile", "dist"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert!(validate_tauri_inner_cargo_args(&missing, "fixture").is_err());

        let misplaced = ["tauri", "build", "--locked", "--", "--profile", "dist"]
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let error = validate_tauri_inner_cargo_args(&misplaced, "fixture").unwrap_err();
        assert!(error.contains("before Tauri's Cargo separator"));

        let duplicate = [
            "tauri",
            "build",
            "--",
            "--locked",
            "--locked",
            "--profile",
            "dist",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        assert!(validate_tauri_inner_cargo_args(&duplicate, "fixture").is_err());
    }

    #[test]
    fn active_tauri_dist_builders_all_pass_locked_to_cargo() {
        tauri_locked_build_contract(&repo::root()).unwrap();
    }

    #[test]
    fn tauri_build_freshness_contract_accepts_scoped_watchers_and_frontend_hook() {
        let source = r#"{
            "build": {
                "beforeBuildCommand": "bun run build",
                "additionalWatchFolders": [
                    "../../../crates",
                    "../../../Cargo.toml",
                    "../../../Cargo.lock"
                ]
            }
        }"#;
        assert!(tauri_build_freshness_contract_manifest(source).is_ok());
    }

    #[test]
    fn tauri_build_hook_rejects_nested_build_commands() {
        let source = r#"{
            "build": {
                "beforeBuildCommand": "bun run build && cd .. && cargo tauri build",
                "additionalWatchFolders": ["../../../crates", "../../../Cargo.toml", "../../../Cargo.lock"]
            }
        }"#;
        assert!(tauri_build_freshness_contract_manifest(source).is_err());

        let source = r#"{
            "build": {
                "beforeBuildCommand": "bun run build && cargo tauri build",
                "additionalWatchFolders": ["../../../crates", "../../../Cargo.toml", "../../../Cargo.lock"]
            }
        }"#;
        assert!(tauri_build_freshness_contract_manifest(source).is_err());
    }

    #[test]
    fn tauri_build_freshness_contract_rejects_the_workspace_root_watch() {
        let source = r#"{
            "build": {
                "beforeBuildCommand": "bun run build",
                "additionalWatchFolders": [
                    "../../../",
                    "../../../crates",
                    "../../../Cargo.toml",
                    "../../../Cargo.lock"
                ]
            }
        }"#;
        let error = tauri_build_freshness_contract_manifest(source).unwrap_err();
        assert!(error.contains("must not watch the entire repository workspace"));
    }

    #[test]
    fn tauri_feature_contract_rejects_the_old_combined_runtime_topology() {
        let source = r#"
[dependencies]
tauri = { version = "2.11.5", default-features = false }

[features]
default = ["desktop-runtime"]
desktop-runtime = ["tauri/wry", "tauri/custom-protocol", "tauri/compression"]
"#;
        let features = fixture_features(source);
        let default = feature_entries(&features, "default").unwrap();
        assert!(
            simulate_tauri_dev_features(&default, &features)
                .unwrap()
                .is_empty()
        );
        let error = tauri_feature_contract_manifest(source).unwrap_err();
        assert!(error.contains("default features must directly contain"));
    }

    #[test]
    fn tauri_feature_contract_rejects_a_nested_production_alias() {
        let source = r#"
[dependencies]
tauri = { version = "2.11.5", default-features = false }

[features]
default = ["production"]
production = ["desktop-runtime", "packaged-assets"]
desktop-runtime = ["tauri/wry"]
packaged-assets = ["tauri/custom-protocol", "tauri/compression"]
"#;
        let error = tauri_feature_contract_manifest(source).unwrap_err();
        assert!(error.contains("default features must directly contain"));
    }

    #[test]
    fn tauri_feature_contract_rejects_missing_wry() {
        let source = VALID_TAURI_FEATURE_MANIFEST
            .replace("desktop-runtime = [\"tauri/wry\"]", "desktop-runtime = []");
        let error = tauri_feature_contract_manifest(&source).unwrap_err();
        assert!(error.contains("desktop-runtime must directly contain `tauri/wry`"));
    }

    #[test]
    fn tauri_feature_contract_rejects_protocol_inside_runtime_or_its_aliases() {
        let source = VALID_TAURI_FEATURE_MANIFEST.replace(
            "desktop-runtime = [\"tauri/wry\"]",
            "desktop-runtime = [\"tauri/wry\", \"runtime-packaging\"]\nruntime-packaging = [\"tauri/custom-protocol\"]",
        );
        let error = tauri_feature_contract_manifest(&source).unwrap_err();
        assert!(error.contains("desktop-runtime must not contain `tauri/custom-protocol`"));
    }

    #[test]
    fn v4_legacy_updater_contract_rejects_retired_runtime_and_release_markers() {
        assert!(
            v4_legacy_updater_source_contract(
                "official Tauri NSIS and UpdateService only",
                "fixture"
            )
            .is_ok()
        );
        for marker in [
            "sky_updater",
            "Sky-Auto-Player-Updater.exe",
            "sky_updater_e2e",
            "cargo xtask dist",
            "verify-dist",
            "MANIFEST.json",
            "MANIFEST.json.sig",
            "SKY_UPDATE_SIGNING_KEY_HEX",
            "pep440_rs",
            "packaging.version",
            "ActiveUpdateState",
            "active_update_for_install",
        ] {
            assert!(
                v4_legacy_updater_source_contract(marker, "fixture").is_err(),
                "{marker}"
            );
        }
    }

    #[test]
    fn v4_native_adapter_retirement_guards_scan_extracted_owners_and_required_drops() {
        let root = repo::root();
        v4_native_adapter_retirement(&root)
            .expect("all moved native-adapter production owners retain retirement guards");

        for marker in [
            "sky_updater",
            "Sky-Auto-Player-Updater.exe",
            "ActiveUpdateState",
            "active_update_for_install",
            "pep440_rs",
        ] {
            assert!(
                v4_native_adapter_source_contract(marker, "fixture").is_err(),
                "{marker}"
            );
        }

        for required_drop in [
            "last_notified_version",
            "legacy_old_dir_sweep_pending",
            "pending_update_version",
            "auto_apply",
        ] {
            assert!(
                v4_native_adapter_required_drop_source_contract(required_drop).is_err(),
                "missing migration key {required_drop} must fail closed"
            );
        }
    }

    #[test]
    fn v4_legacy_retirement_skips_only_the_xtask_contract_owners() {
        let root = repo::root();
        assert!(is_xtask_contract_source(
            &root,
            &root.join("xtask/src/checks.rs")
        ));
        assert!(is_xtask_contract_source(
            &root,
            &root.join("xtask/src/checks/ci_contracts.rs")
        ));
        assert!(!is_xtask_contract_source(
            &root,
            &root.join("xtask/src/tauri_bundle.rs")
        ));
    }

    #[test]
    fn release_metadata_contract_requires_rust_owned_channels_and_release_transitions() {
        let native = r#"
const V4_STABLE_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json";
const V4_BETA_METADATA_ENDPOINT: &str = "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json";
fn validate_official_metadata_endpoint() {}
endpoints(vec![endpoint])
fn production_metadata_endpoints_are_fixed_and_channel_isolated() {}
"#;
        for marker in [
            "V4_STABLE_METADATA_ENDPOINT",
            "V4_BETA_METADATA_ENDPOINT",
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json",
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json",
            "endpoints(vec![endpoint])",
            "validate_official_metadata_endpoint",
            "production_metadata_endpoints_are_fixed_and_channel_isolated",
        ] {
            assert!(native.contains(marker), "{marker}");
        }
        assert!(!native.contains("api.github.com/repos/pumni/Sky-Auto-Player/releases"));

        let release = r#"
trait Transport
fn publish_release
fn promote_metadata
fn final_verify
validate_beta_keeps_latest
releases/latest
"#;
        for marker in [
            "trait Transport",
            "fn publish_release",
            "fn promote_metadata",
            "fn final_verify",
            "validate_beta_keeps_latest",
            "releases/latest",
        ] {
            assert!(release.contains(marker));
        }
        assert!(!release.contains("gh release create"));
    }

    #[test]
    fn release_metadata_negative_guard_rejects_legacy_topology_markers() {
        for marker in LEGACY_RELEASE_TOPOLOGY_MARKERS {
            assert_eq!(find_legacy_release_topology_marker(marker), Some(*marker));
        }
        assert_eq!(
            find_legacy_release_topology_marker(
                "canonical release-metadata contract and same-repository pipeline"
            ),
            None
        );
    }

    #[test]
    fn release_metadata_negative_guard_covers_runtime_and_release_surfaces() {
        for surface in [
            "apps/desktop/src-tauri/src/native_update.rs",
            "apps/desktop/src-tauri/tauri.conf.json",
            ".github/workflows/ci.yml",
            ".github/workflows/release-v4.yml",
        ] {
            assert!(ACTIVE_RELEASE_SURFACES.contains(&surface));
        }

        let runtime_error = validate_active_release_surface(
            "apps/desktop/src-tauri/src/native_update.rs",
            "const ENDPOINT = \"https://github.com/pumni/Sky-Auto-Player-Releases/releases\";",
        )
        .expect_err("legacy repository marker must be rejected in the updater runtime surface");
        assert!(runtime_error.to_string().contains("native_update.rs"));

        let config_error = validate_active_release_surface(
            "apps/desktop/src-tauri/tauri.conf.json",
            "V4_RELEASE_AUTHORITY_REPOSITORY",
        )
        .expect_err("legacy authority marker must be rejected in updater configuration");
        assert!(config_error.to_string().contains("tauri.conf.json"));

        let workflow_error = validate_active_release_surface(
            ".github/workflows/release-v4.yml",
            "AuthorityCheckout",
        )
        .expect_err("legacy authority marker must be rejected in release workflow surfaces");
        assert!(workflow_error.to_string().contains("release-v4.yml"));
    }
}
