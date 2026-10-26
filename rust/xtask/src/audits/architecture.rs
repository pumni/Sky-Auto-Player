use super::source::strip_rust_comments;
use crate::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
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
const NATIVE_RUNTIME_MODULE_BUDGETS: &[(&str, usize)] = &[
    ("desktop/src-tauri/src/native_runtime.rs", 2_000),
    ("desktop/src-tauri/src/native_runtime/calibration.rs", 1_550),
    ("desktop/src-tauri/src/native_runtime/catalog.rs", 1_650),
    ("desktop/src-tauri/src/native_runtime/tests.rs", 900),
    (
        "desktop/src-tauri/src/native_runtime/tests/calibration.rs",
        650,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/catalog.rs",
        1_100,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/event_delivery.rs",
        800,
    ),
];
const PLAYBACK_INTEGRATION_ROOT_PATH: &str =
    "desktop/src-tauri/src/native_runtime/tests/playback_integration.rs";
const PLAYBACK_INTEGRATION_ROOT_BUDGET: usize = 500;
const PLAYBACK_INTEGRATION_MODULE_BUDGETS: &[(&str, usize)] = &[
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/supervisor_lease.rs",
        1_600,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/terminal_release.rs",
        1_600,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/presentation_events.rs",
        1_600,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/diagnostics_trace.rs",
        1_600,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/start_prepare.rs",
        1_600,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/storage_bootstrap.rs",
        1_600,
    ),
    (
        "desktop/src-tauri/src/native_runtime/tests/playback_integration/commands_status.rs",
        1_600,
    ),
];
const ENGINE_TEST_ROOT_PATH: &str = "rust/crates/sky_player/src/engine/tests.rs";
const ENGINE_TEST_ROOT_BUDGET: usize = 750;
const ENGINE_TEST_MODULE_BUDGETS: &[(&str, usize)] = &[
    (
        "rust/crates/sky_player/src/engine/tests/startup_admission.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/lease_supervision.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/preparation_dispatch.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/release_cleanup.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/native_dispatch.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/telemetry_metrics.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/target_focus_admission.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/pause_suspend.rs",
        2_200,
    ),
    (
        "rust/crates/sky_player/src/engine/tests/timing_wait.rs",
        2_200,
    ),
    ("rust/crates/sky_player/src/engine/tests/support.rs", 2_200),
];

fn native_runtime_line_budget_violation(path: &str, source: &str, budget: usize) -> Option<String> {
    module_line_budget_violation("native_runtime_module_lines", path, source, budget)
}

fn module_line_budget_violation(
    rule: &str,
    path: &str,
    source: &str,
    budget: usize,
) -> Option<String> {
    let line_count = source.lines().count();
    (line_count > budget).then(|| format!("[{rule}] {path}: {line_count} lines (> {budget})"))
}
fn native_runtime_module_budgets(root: &Path) -> Result<()> {
    for (path, budget) in NATIVE_RUNTIME_MODULE_BUDGETS {
        let source = fs::read_to_string(root.join(path))?;
        if let Some(violation) = native_runtime_line_budget_violation(path, &source, *budget) {
            return Err(violation.into());
        }
    }
    Ok(())
}

fn playback_integration_module_inventory_violation(
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) -> Option<String> {
    (expected != actual).then(|| {
        format!(
            "[native_playback_test_module_ownership] native_runtime/tests/playback_integration/: observed={actual:?}, expected={expected:?}"
        )
    })
}

fn playback_integration_module_budgets(root: &Path) -> Result<()> {
    let root_source = fs::read_to_string(root.join(PLAYBACK_INTEGRATION_ROOT_PATH))?;
    if let Some(violation) = module_line_budget_violation(
        "native_playback_test_root_lines",
        PLAYBACK_INTEGRATION_ROOT_PATH,
        &root_source,
        PLAYBACK_INTEGRATION_ROOT_BUDGET,
    ) {
        return Err(violation.into());
    }

    let tests_root = root.join("desktop/src-tauri/src/native_runtime/tests/playback_integration");
    let expected = PLAYBACK_INTEGRATION_MODULE_BUDGETS
        .iter()
        .filter_map(|(path, _)| {
            path.strip_prefix("desktop/src-tauri/src/native_runtime/tests/playback_integration/")
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(&tests_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("rs")
        {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "native playback test module name must be valid UTF-8")?;
        actual.insert(name);
    }
    if let Some(violation) = playback_integration_module_inventory_violation(&expected, &actual) {
        return Err(violation.into());
    }

    for (path, budget) in PLAYBACK_INTEGRATION_MODULE_BUDGETS {
        let source = fs::read_to_string(root.join(path))?;
        if let Some(violation) = module_line_budget_violation(
            "native_playback_test_module_lines",
            path,
            &source,
            *budget,
        ) {
            return Err(violation.into());
        }
    }
    Ok(())
}

fn engine_test_module_inventory_violation(
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) -> Option<String> {
    (expected != actual).then(|| {
        format!(
            "[engine_test_module_ownership] engine/tests/: observed={actual:?}, expected={expected:?}"
        )
    })
}

fn engine_test_module_budgets(root: &Path) -> Result<()> {
    let root_source = fs::read_to_string(root.join(ENGINE_TEST_ROOT_PATH))?;
    if let Some(violation) = module_line_budget_violation(
        "engine_test_root_lines",
        ENGINE_TEST_ROOT_PATH,
        &root_source,
        ENGINE_TEST_ROOT_BUDGET,
    ) {
        return Err(violation.into());
    }

    let tests_root = root.join("rust/crates/sky_player/src/engine/tests");
    let expected = ENGINE_TEST_MODULE_BUDGETS
        .iter()
        .filter_map(|(path, _)| path.strip_prefix("rust/crates/sky_player/src/engine/tests/"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(&tests_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("rs")
        {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "engine test module name must be valid UTF-8")?;
        actual.insert(name);
    }
    if let Some(violation) = engine_test_module_inventory_violation(&expected, &actual) {
        return Err(violation.into());
    }

    for (path, budget) in ENGINE_TEST_MODULE_BUDGETS {
        let source = fs::read_to_string(root.join(path))?;
        if let Some(violation) =
            module_line_budget_violation("engine_test_module_lines", path, &source, *budget)
        {
            return Err(violation.into());
        }
    }
    Ok(())
}

fn is_engine_test_owner_path(path: &str) -> bool {
    ENGINE_TEST_MODULE_BUDGETS
        .iter()
        .any(|(owner_path, _)| *owner_path == path)
}

const XTASK_MODULE_BUDGETS: &[(&str, usize)] = &[
    ("rust/xtask/src/assembly_audit.rs", 1_300),
    ("rust/xtask/src/calibration.rs", 1_250),
    ("rust/xtask/src/checks.rs", 1_600),
    ("rust/xtask/src/checks/bindings.rs", 700),
    ("rust/xtask/src/checks/ci_contracts.rs", 2_100),
    ("rust/xtask/src/checks/release_pipeline.rs", 1_200),
    ("rust/xtask/src/checks/release_runner.rs", 1_900),
    ("rust/xtask/src/checks/tauri_metadata.rs", 1_300),
    ("rust/xtask/src/checks/trust.rs", 1_050),
    ("rust/xtask/src/checks/workflow.rs", 850),
    ("rust/xtask/src/audits/security.rs", 700),
    ("rust/xtask/src/audits/architecture.rs", 1_600),
    ("rust/xtask/src/audits/source.rs", 100),
];

fn xtask_module_inventory_violation(
    expected: &BTreeSet<String>,
    actual: &BTreeSet<String>,
) -> Option<String> {
    (expected != actual).then(|| {
        format!("[xtask_module_ownership] checks/: observed={actual:?}, expected={expected:?}")
    })
}

fn xtask_check_module_inventory(root: &Path) -> Result<()> {
    let checks_root = root.join("rust/xtask/src/checks");
    let expected = XTASK_MODULE_BUDGETS
        .iter()
        .filter_map(|(path, _)| path.strip_prefix("rust/xtask/src/checks/"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(checks_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file()
            || entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("rs")
        {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "xtask check module name must be valid UTF-8")?;
        actual.insert(name);
    }
    if let Some(violation) = xtask_module_inventory_violation(&expected, &actual) {
        return Err(violation.into());
    }
    Ok(())
}

fn xtask_module_budgets(root: &Path) -> Result<()> {
    xtask_check_module_inventory(root)?;
    for (path, budget) in XTASK_MODULE_BUDGETS {
        let source = fs::read_to_string(root.join(path))?;
        if let Some(violation) =
            module_line_budget_violation("xtask_module_lines", path, &source, *budget)
        {
            return Err(violation.into());
        }
    }
    Ok(())
}
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
        let clean = super::source::strip_rust_comments(&source);
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

const FACADE_HARD_LIMIT: usize = 250;
const REGULAR_SOFT_LIMIT: usize = 700;
const REGULAR_HARD_LIMIT: usize = 900;
const WORKER_FUNCTION_HARD_LIMIT: usize = 350;
const CONTEXT_FIELD_HARD_LIMIT: usize = 12;
const DISPATCH_FUNCTION_HARD_LIMIT: usize = 180;
const WORKER_SCHEDULE_CLONE_PATTERNS: &[&str] = &[
    "schedule.clone()",
    "Clone::clone(&schedule",
    "Clone::clone(&config.schedule",
];
const FACADES: &[&str] = &["engine.rs", "input.rs", "wait.rs", "lib.rs"];
const LEGACY_DISPATCH_PATHS: &[&str] = &[
    "rust/crates/sky_player/src/engine/worker/downs.rs",
    "rust/crates/sky_player/src/engine/worker/down_outcome.rs",
    "rust/crates/sky_player/src/engine/worker/releases.rs",
];
const CANONICAL_DISPATCH_FILES: &[&str] = &[
    "authored.rs",
    "mod.rs",
    "observation.rs",
    "observer.rs",
    "recovery.rs",
    "timing.rs",
    "hold_forensics.rs",
    "observer_wake.rs",
    "observer_trace.rs",
    "prepared.rs",
];
const UNSAFE_AUDIT_SOURCE_ROOTS: &[&str] = &[
    "rust/crates/sky_dispatch_core/src",
    "rust/crates/sky_dispatch_win32/src",
    "rust/crates/sky_app_core/src",
    "rust/crates/sky_player/src",
    "rust/crates/sky_native_adapters/src",
    "rust/crates/sky_windows_security/src",
    "desktop/src-tauri/src",
];

fn load_architecture_allowlist(root: &Path) -> Result<BTreeMap<(String, String), String>> {
    let path = root.join(".config/rust_architecture_allowlist.json");
    if !path.is_file() {
        return Err(format!("architecture allowlist is missing: {}", path.display()).into());
    }
    let payload: Value = serde_json::from_slice(&fs::read(path)?)?;
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("architecture allowlist entries must be an array")?;
    let mut result = BTreeMap::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or("architecture allowlist entry must be an object")?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist path missing")?;
        let rule = object
            .get("rule")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist rule missing")?;
        let reason = object
            .get("reason")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist reason missing")?;
        let expires = object
            .get("review_trigger")
            .and_then(Value::as_str)
            .ok_or("architecture allowlist review trigger missing")?;
        if !root.join(path).is_file() {
            return Err(format!("architecture allowlist path does not exist: {path}").into());
        }
        result.insert(
            (path.replace('\\', "/"), rule.to_owned()),
            format!("{reason} ({expires})"),
        );
    }
    Ok(result)
}

fn architecture_record(
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
    allowlist: &BTreeMap<(String, String), String>,
    path: &str,
    rule: &str,
    message: impl Into<String>,
) {
    let message = message.into();
    if let Some(debt) = allowlist.get(&(path.to_owned(), rule.to_owned())) {
        warnings.push(format!(
            "[{rule}] {path}: {message}; temporary allowlist: {debt}"
        ));
    } else {
        errors.push(format!("[{rule}] {path}: {message}"));
    }
}

fn clean_lines(source: &str) -> Vec<String> {
    strip_rust_comments(source)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn brace_end(lines: &[String], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut opened = false;
    for (index, line) in lines.iter().enumerate().skip(start) {
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        opened |= line.contains('{');
        if opened && depth <= 0 {
            return Some(index);
        }
    }
    None
}

fn context_violations(lines: &[String]) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let Some(struct_position) = trimmed.find("struct ") else {
            continue;
        };
        let name = trimmed[struct_position + "struct ".len()..]
            .split(['<', '{'])
            .next()
            .unwrap_or("")
            .trim();
        if !(name.ends_with("Context")
            || name.ends_with("Inputs")
            || name.ends_with("Config")
            || name.ends_with("Options")
            || name.ends_with("Shared"))
        {
            continue;
        }
        let Some(end) = brace_end(lines, index) else {
            continue;
        };
        let fields = lines[index + 1..end]
            .iter()
            .filter(|field| {
                let field = field.trim();
                !field.starts_with("fn ") && field.contains(':') && !field.starts_with("#")
            })
            .count();
        if fields > CONTEXT_FIELD_HARD_LIMIT {
            result.push((name.to_owned(), fields.to_string()));
        }
    }
    result
}

fn function_line_violations(lines: &[String], hard_limit: usize) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    for (start, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let Some(position) = trimmed.find("fn ") else {
            continue;
        };
        let name = trimmed[position + 3..]
            .split(['(', '<', ' '])
            .next()
            .unwrap_or("");
        if name.is_empty() {
            continue;
        }
        let Some(end) = brace_end(lines, start) else {
            continue;
        };
        let count = end - start + 1;
        if count > hard_limit {
            result.push((name.to_owned(), count));
        }
    }
    result
}

fn top_level_glob_import(lines: &[String]) -> bool {
    lines
        .iter()
        .map(|line| line.trim())
        .find(|line| !line.is_empty() && !line.starts_with("#![") && !line.starts_with("#["))
        == Some("use super::*;")
}

fn gated_test_support(lines: &[String], path: &str) -> bool {
    if !path.contains("/test_support/") && !path.ends_with("/test_support.rs") {
        return true;
    }
    lines
        .iter()
        .any(|line| line.contains("cfg(any(test, feature = \"test-support\"))"))
}

fn line_is_gated(lines: &[String], index: usize) -> bool {
    lines[..=index]
        .iter()
        .rev()
        .take(3)
        .any(|line| line.contains("cfg(any(test, feature = \"test-support\"))"))
}

#[cfg(test)]
fn contains_unsafe_code(source: &str) -> bool {
    let source = source.replace("#![forbid(unsafe_code)]", "");
    source
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|word| word == "unsafe")
}

#[derive(Debug, Default, PartialEq, Eq)]
struct UnsafeCodeInventory {
    production_lines: Vec<usize>,
    cfg_test_lines: Vec<usize>,
}

fn unsafe_code_inventory(path: &str, source: &str) -> UnsafeCodeInventory {
    let file_is_test = path.contains("/tests/")
        || path.ends_with("/tests.rs")
        || source.lines().any(|line| line.trim() == "#![cfg(test)]");
    let source = source
        .replace("#![forbid(unsafe_code)]", "")
        .replace("#![cfg(test)]", "");
    let clean = strip_rust_comments(&source);
    let lines = clean.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut test_only = vec![file_is_test; lines.len()];

    for (attribute, line) in lines.iter().enumerate() {
        if line.trim() != "#[cfg(test)]" {
            continue;
        }
        let mut declaration = None;
        for (index, candidate) in lines.iter().enumerate().skip(attribute + 1).take(5) {
            let candidate = candidate.trim();
            if candidate.is_empty() || candidate.starts_with("#[") {
                continue;
            }
            let item = candidate
                .strip_prefix("pub(crate) ")
                .or_else(|| candidate.strip_prefix("pub(super) "))
                .or_else(|| candidate.strip_prefix("pub "))
                .unwrap_or(candidate);
            if item.starts_with("mod ")
                || item.starts_with("fn ")
                || item.starts_with("const ")
                || item.starts_with("static ")
                || item.starts_with("struct ")
                || item.starts_with("enum ")
                || item.starts_with("impl ")
                || item.starts_with("trait ")
            {
                declaration = Some(index);
            }
            break;
        }
        if let Some(start) = declaration {
            let end = brace_end(&lines, start).unwrap_or(start);
            for line in test_only
                .iter_mut()
                .take(end.saturating_add(1))
                .skip(attribute)
            {
                *line = true;
            }
        }
    }

    let mut inventory = UnsafeCodeInventory::default();
    for (index, line) in lines.iter().enumerate() {
        let contains_token = line
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|word| word == "unsafe");
        if contains_token {
            let line_number = index + 1;
            if test_only.get(index).copied().unwrap_or(false) {
                inventory.cfg_test_lines.push(line_number);
            } else {
                inventory.production_lines.push(line_number);
            }
        }
    }
    inventory
}

fn audit_unsafe_source(
    path: &str,
    source: &str,
    allowlist: &BTreeMap<(String, String), String>,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> UnsafeCodeInventory {
    let inventory = unsafe_code_inventory(path, source);
    if !inventory.production_lines.is_empty() {
        architecture_record(
            errors,
            warnings,
            allowlist,
            path,
            "unsafe_boundary",
            format!(
                "production unsafe code requires a documented module exception at lines {:?}",
                inventory.production_lines
            ),
        );
    }
    inventory
}

pub(crate) fn architecture(root: &Path) -> Result<()> {
    engine_test_module_budgets(root)?;
    let allowlist = load_architecture_allowlist(root)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let app_core_manifest = root.join("rust/crates/sky_app_core/Cargo.toml");
    let app_core: toml::Value = toml::from_str(&fs::read_to_string(&app_core_manifest)?)?;
    if let Some(dependencies) = app_core.get("dependencies").and_then(toml::Value::as_table) {
        for forbidden in [
            "tauri",
            "windows-sys",
            "sky_desktop_shell",
            "sky_player",
            "sky_native_adapters",
        ] {
            if dependencies.contains_key(forbidden) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    "rust/crates/sky_app_core/Cargo.toml",
                    "app_core_dependency",
                    format!("sky_app_core must not depend directly on {forbidden}"),
                );
            }
        }
    }
    let app_core_source = root.join("rust/crates/sky_app_core/src");
    if app_core_source.is_dir() {
        for path in WalkDir::new(&app_core_source).follow_links(false) {
            let path = path?;
            if !path.file_type().is_file()
                || path
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("rs")
            {
                continue;
            }
            let relative = path
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let joined = clean_lines(&fs::read_to_string(path.path())?).join("");
            if [
                "tauri",
                "windows-sys",
                "windows_sys",
                "sky_desktop_shell",
                "sky_player",
            ]
            .iter()
            .any(|marker| joined.contains(marker))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "app_core_dependency",
                    "sky_app_core source references a forbidden delivery/platform/player dependency",
                );
            }
        }
    }
    let dispatch_dir = root.join("rust/crates/sky_player/src/engine/worker/dispatch");
    if dispatch_dir.is_dir() {
        let actual: BTreeSet<String> = fs::read_dir(&dispatch_dir)?
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                (entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("rs"))
                .then(|| entry.file_name().to_string_lossy().into_owned())
            })
            .filter(|name| !name.ends_with("_tests.rs"))
            .collect();
        let expected: BTreeSet<String> = CANONICAL_DISPATCH_FILES
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        for name in actual.difference(&expected) {
            errors.push(format!(
                "[unexpected_dispatch_module] {name}: dispatch module is not canonical"
            ));
        }
        for name in expected.difference(&actual) {
            errors.push(format!(
                "[missing_dispatch_module] {name}: canonical dispatch module is missing"
            ));
        }
    }

    for crate_name in [
        "sky_dispatch_core",
        "sky_dispatch_win32",
        "sky_app_core",
        "sky_player",
    ] {
        let source_root = root.join("rust/crates").join(crate_name).join("src");
        if !source_root.is_dir() {
            continue;
        }
        for path in WalkDir::new(&source_root).follow_links(false) {
            let path = path?;
            if !path.file_type().is_file()
                || path
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("rs")
            {
                continue;
            }
            let absolute = path.path();
            let relative = absolute
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(absolute)?;
            let lines = source
                .lines()
                .map(|line| format!("{line}\n"))
                .collect::<Vec<_>>();
            let clean = clean_lines(&source);
            let joined = clean.join("");
            if LEGACY_DISPATCH_PATHS.contains(&relative.as_str()) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "legacy_dispatch_path",
                    "legacy dispatch path must be removed",
                );
                continue;
            }
            if !is_engine_test_owner_path(&relative) {
                let limit = if FACADES.contains(
                    &absolute
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(""),
                ) {
                    FACADE_HARD_LIMIT
                } else {
                    REGULAR_HARD_LIMIT
                };
                if clean.len() > limit {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        if limit == FACADE_HARD_LIMIT {
                            "facade_lines"
                        } else {
                            "regular_module_lines"
                        },
                        format!("{} lines (> {limit})", clean.len()),
                    );
                }
                if limit == REGULAR_HARD_LIMIT
                    && clean.len() > REGULAR_SOFT_LIMIT
                    && clean.len() <= REGULAR_HARD_LIMIT
                {
                    warnings.push(format!(
                        "[regular_module_soft_lines] {relative}: {} lines (> {REGULAR_SOFT_LIMIT})",
                        clean.len()
                    ));
                }
            }
            if crate_name == "sky_player"
                && relative == "rust/crates/sky_player/src/engine/worker/orchestration.rs"
            {
                for (name, count) in function_line_violations(&clean, WORKER_FUNCTION_HARD_LIMIT) {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        "worker_function_lines",
                        format!("{name} has {count} lines (> {WORKER_FUNCTION_HARD_LIMIT})"),
                    );
                }
            }
            if relative.starts_with("rust/crates/sky_player/src/engine/worker/dispatch/") {
                for (name, count) in function_line_violations(&clean, DISPATCH_FUNCTION_HARD_LIMIT)
                {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        "dispatch_function_lines",
                        format!("{name} has {count} lines (> {DISPATCH_FUNCTION_HARD_LIMIT})"),
                    );
                }
            }
            if crate_name == "sky_dispatch_core"
                && (joined.contains("sky_dispatch_win32::")
                    || joined.contains("use sky_dispatch_win32"))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "dependency_direction",
                    "core imports sky_dispatch_win32",
                );
            }
            if ["sky_dispatch_core", "sky_dispatch_win32"].contains(&crate_name)
                && (joined.contains("sky_player::") || joined.contains("use sky_player"))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "dependency_direction",
                    "lower crate imports sky_player",
                );
            }
            if top_level_glob_import(&clean)
                && !relative.ends_with("/tests.rs")
                && !relative.contains("/tests/")
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "production_glob_import",
                    "top-level use super::* in production module",
                );
            }
            for (index, line) in clean.iter().enumerate() {
                if line.contains("Box<dyn Fn") && !line_is_gated(&lines, index) {
                    architecture_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &relative,
                        "production_dynamic_emitter",
                        "dynamic emitter in production source",
                    );
                }
            }
            if (relative == "rust/crates/sky_player/src/engine/worker.rs"
                || relative.starts_with("rust/crates/sky_player/src/engine/worker/"))
                && WORKER_SCHEDULE_CLONE_PATTERNS
                    .iter()
                    .any(|pattern| joined.contains(pattern))
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "runtime_schedule_clone",
                    "production worker must move RuntimeSchedule into the coordinator; cloning the schedule is forbidden",
                );
            }
            if !gated_test_support(&clean, &relative) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "test_support_cfg",
                    "test-support source is not cfg-gated",
                );
            }
            for (name, fields) in context_violations(&clean) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "context_fields",
                    format!("{name} has {fields} fields (> {CONTEXT_FIELD_HARD_LIMIT})"),
                );
            }
            if (relative == "rust/crates/sky_player/src/engine.rs")
                && clean.iter().enumerate().any(|(index, line)| {
                    line.trim() == "mod test_support;" && !line_is_gated(&clean, index)
                })
            {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "test_support_cfg",
                    "test_support module is not cfg-gated",
                );
            }
        }
    }
    let mut production_unsafe_modules = BTreeSet::new();
    let mut cfg_test_unsafe_modules = Vec::new();
    for source_root in UNSAFE_AUDIT_SOURCE_ROOTS {
        let absolute_root = root.join(source_root);
        if !absolute_root.is_dir() {
            continue;
        }
        for path in WalkDir::new(&absolute_root).follow_links(false) {
            let path = path?;
            if !path.file_type().is_file()
                || path
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("rs")
            {
                continue;
            }
            let relative = path
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(path.path())?;
            let inventory =
                audit_unsafe_source(&relative, &source, &allowlist, &mut errors, &mut warnings);
            if !inventory.production_lines.is_empty() {
                production_unsafe_modules.insert(relative.clone());
            }
            if !inventory.cfg_test_lines.is_empty() {
                cfg_test_unsafe_modules.push(format!(
                    "{relative}:{}",
                    inventory
                        .cfg_test_lines
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ));
            }
        }
    }
    for (path, rule) in allowlist.keys() {
        if rule == "unsafe_boundary" && !production_unsafe_modules.contains(path) {
            errors.push(format!(
                "[stale_unsafe_allowlist_entry] {path}: no production unsafe code remains"
            ));
        }
    }
    let production_inventory = production_unsafe_modules
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "[xtask] unsafe boundary inventory: production={} [{}]; cfg(test)-only={} [{}]",
        production_unsafe_modules.len(),
        production_inventory,
        cfg_test_unsafe_modules.len(),
        cfg_test_unsafe_modules.join("; ")
    );
    if !warnings.is_empty() {
        for warning in &warnings {
            println!("[xtask] architecture warning: {warning}");
        }
    }
    if let Some(error) = errors.first() {
        return Err(format!(
            "architecture audit failed: {error} ({} error(s))",
            errors.len()
        )
        .into());
    }
    println!(
        "[xtask] architecture checks: PASS ({} allowlisted warning(s))",
        warnings.len()
    );
    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    app_core_boundary(root)?;
    adapter_boundary(root)?;
    architecture(root)?;
    native_runtime_module_budgets(root)?;
    playback_integration_module_budgets(root)?;
    xtask_module_budgets(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo;

    #[test]
    fn current_core_boundary_passes() {
        app_core_boundary(&crate::repo::root()).expect("sky_app_core boundary");
    }

    #[test]
    fn current_adapter_dependency_boundary_passes() {
        adapter_boundary(&crate::repo::root()).expect("sky_native_adapters dependency boundary");
    }

    #[test]
    fn native_runtime_module_budgets_reject_regrowth_and_cover_the_current_tree() {
        assert_eq!(
            native_runtime_line_budget_violation("native_runtime.rs", "one\ntwo\n", 1),
            Some("[native_runtime_module_lines] native_runtime.rs: 2 lines (> 1)".to_owned())
        );
        assert_eq!(
            native_runtime_line_budget_violation("native_runtime.rs", "one\ntwo\n", 2),
            None
        );
        native_runtime_module_budgets(&crate::repo::root())
            .expect("native runtime ownership modules stay within their budgets");
    }

    #[test]
    fn playback_integration_module_budgets_reject_regrowth_and_cover_the_current_tree() {
        assert_eq!(
            module_line_budget_violation(
                "native_playback_test_root_lines",
                PLAYBACK_INTEGRATION_ROOT_PATH,
                "one\ntwo\n",
                1,
            ),
            Some(format!(
                "[native_playback_test_root_lines] {PLAYBACK_INTEGRATION_ROOT_PATH}: 2 lines (> 1)"
            ))
        );
        assert_eq!(
            module_line_budget_violation(
                "native_playback_test_module_lines",
                PLAYBACK_INTEGRATION_MODULE_BUDGETS[0].0,
                "one\ntwo\n",
                1,
            ),
            Some(format!(
                "[native_playback_test_module_lines] {}: 2 lines (> 1)",
                PLAYBACK_INTEGRATION_MODULE_BUDGETS[0].0
            ))
        );
        playback_integration_module_budgets(&crate::repo::root())
            .expect("native playback test owners stay within budgets and exact inventory");
    }

    #[test]
    fn playback_integration_module_ownership_rejects_unbudgeted_siblings() {
        let expected = ["supervisor_lease.rs", "terminal_release.rs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let actual = ["supervisor_lease.rs", "terminal_release.rs", "unowned.rs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert!(playback_integration_module_inventory_violation(&expected, &actual).is_some());
        assert!(playback_integration_module_inventory_violation(&expected, &expected).is_none());
    }

    #[test]
    fn engine_test_module_budgets_reject_regrowth_and_cover_the_current_tree() {
        assert_eq!(
            module_line_budget_violation(
                "engine_test_root_lines",
                ENGINE_TEST_ROOT_PATH,
                "one\ntwo\n",
                1,
            ),
            Some(format!(
                "[engine_test_root_lines] {ENGINE_TEST_ROOT_PATH}: 2 lines (> 1)"
            ))
        );
        assert_eq!(
            module_line_budget_violation(
                "engine_test_module_lines",
                ENGINE_TEST_MODULE_BUDGETS[0].0,
                "one\ntwo\n",
                1,
            ),
            Some(format!(
                "[engine_test_module_lines] {}: 2 lines (> 1)",
                ENGINE_TEST_MODULE_BUDGETS[0].0
            ))
        );
        engine_test_module_budgets(&crate::repo::root())
            .expect("engine test owners stay within their budgets and exact inventory");
    }

    #[test]
    fn engine_test_module_ownership_rejects_unbudgeted_modules() {
        let expected = ["startup_admission.rs", "support.rs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let actual = ["startup_admission.rs", "support.rs", "unowned.rs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert!(engine_test_module_inventory_violation(&expected, &actual).is_some());
        assert!(engine_test_module_inventory_violation(&expected, &expected).is_none());
    }

    #[test]
    fn xtask_module_budgets_reject_regrowth_and_cover_the_current_tree() {
        assert_eq!(
            module_line_budget_violation("xtask_module_lines", "checks.rs", "one\ntwo\n", 1),
            Some("[xtask_module_lines] checks.rs: 2 lines (> 1)".to_owned())
        );
        assert_eq!(
            module_line_budget_violation("xtask_module_lines", "checks.rs", "one\ntwo\n", 2),
            None
        );
        xtask_module_budgets(&crate::repo::root())
            .expect("xtask ownership modules stay within their budgets");
    }

    #[test]
    fn xtask_module_ownership_rejects_unbudgeted_modules() {
        let expected = ["bindings.rs", "workflow.rs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        let actual = ["bindings.rs", "workflow.rs", "unowned.rs"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert!(xtask_module_inventory_violation(&expected, &actual).is_some());
        assert!(xtask_module_inventory_violation(&expected, &expected).is_none());
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

    #[test]
    fn architecture_allowlist_is_loaded_and_current_tree_passes() {
        let root = repo::root();
        let allowlist = load_architecture_allowlist(&root).unwrap();
        assert!(
            !allowlist.contains_key(&(ENGINE_TEST_ROOT_PATH.into(), "regular_module_lines".into()))
        );
        architecture(&root).unwrap();
    }

    #[test]
    fn architecture_helpers_cover_context_function_glob_and_schedule_rules() {
        let context = (0..13)
            .map(|index| format!("    field_{index}: u8,\n"))
            .collect::<String>();
        let context = format!("struct WorkerContext {{\n{context}}}\n");
        assert_eq!(context_violations(&clean_lines(&context)).len(), 1);
        let function = std::iter::once("fn oversized() {\n".to_owned())
            .chain((0..181).map(|_| "    let _value = 1;\n".to_owned()))
            .chain(std::iter::once("}\n".to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(function_line_violations(&function, 180).len(), 1);
        assert!(top_level_glob_import(&clean_lines(
            "use super::*;\nfn f() {}\n"
        )));
        assert!(contains_unsafe_code("unsafe { value(); }"));
        assert!(
            WORKER_SCHEDULE_CLONE_PATTERNS
                .iter()
                .any(|pattern| "schedule.clone()".contains(pattern))
        );
    }

    #[test]
    fn unsafe_inventory_separates_production_from_cfg_test_code() {
        let source = r#"
fn production_boundary() {
    unsafe { core::hint::unreachable_unchecked() }
}

#[cfg(test)]
mod tests {
    fn test_boundary() {
        unsafe { core::hint::unreachable_unchecked() }
    }
}
"#;
        let inventory = unsafe_code_inventory("desktop/src-tauri/src/inventory_fixture.rs", source);
        assert_eq!(inventory.production_lines, vec![3]);
        assert_eq!(inventory.cfg_test_lines, vec![9]);

        let file_test = unsafe_code_inventory(
            "rust/crates/sky_native_adapters/src/fixture.rs",
            "#![cfg(test)]\nfn test_boundary() { unsafe { core::hint::unreachable_unchecked() } }\n",
        );
        assert!(file_test.production_lines.is_empty());
        assert_eq!(file_test.cfg_test_lines, vec![2]);
    }
    #[test]
    fn unauthorized_unsafe_is_rejected_in_shell_and_adapter_sources() {
        let source = "fn injected() { unsafe { core::hint::unreachable_unchecked() } }\n";
        for path in [
            "desktop/src-tauri/src/injected.rs",
            "rust/crates/sky_native_adapters/src/injected.rs",
        ] {
            let mut errors = Vec::new();
            let mut warnings = Vec::new();
            audit_unsafe_source(path, source, &BTreeMap::new(), &mut errors, &mut warnings);
            assert_eq!(warnings.len(), 0, "{path}");
            assert_eq!(errors.len(), 1, "{path}");
            assert!(
                errors[0].contains("[unsafe_boundary]"),
                "{path}: {errors:?}"
            );
        }
    }
}
