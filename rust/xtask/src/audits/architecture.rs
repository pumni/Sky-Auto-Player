use super::source::strip_rust_comments;
#[path = "architecture/allowlist.rs"]
mod allowlist;
use crate::Result;
use allowlist::{
    ArchitectureAllowlist, ArchitectureMetric, architecture_metric_record, architecture_record,
    load_architecture_allowlist, stale_metric_allowlist_entries,
};
#[cfg(test)]
use allowlist::{ArchitectureException, parse_architecture_allowlist};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
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
const NATIVE_RUNTIME_MODULE_ADVISORY_THRESHOLDS: &[(&str, usize)] = &[
    ("apps/desktop/src-tauri/src/native_runtime.rs", 2_000),
    (
        "apps/desktop/src-tauri/src/native_runtime/calibration.rs",
        1_550,
    ),
    (
        "apps/desktop/src-tauri/src/native_runtime/catalog.rs",
        1_650,
    ),
    ("apps/desktop/src-tauri/src/native_runtime/tests.rs", 900),
    (
        "apps/desktop/src-tauri/src/native_runtime/tests/calibration.rs",
        650,
    ),
    (
        "apps/desktop/src-tauri/src/native_runtime/tests/catalog.rs",
        1_100,
    ),
    (
        "apps/desktop/src-tauri/src/native_runtime/tests/event_delivery.rs",
        800,
    ),
];
const PLAYBACK_INTEGRATION_ROOT_PATH: &str =
    "apps/desktop/src-tauri/src/native_runtime/tests/playback_integration.rs";
const ENGINE_TEST_ROOT_PATH: &str = "rust/crates/sky_player/src/engine/tests.rs";
const ENGINE_TEST_MODULE_ADVISORY_THRESHOLD: usize = 2_200;
const XTASK_MODULE_ADVISORY_THRESHOLDS: &[(&str, usize)] = &[
    ("rust/xtask/src/assembly_audit.rs", 1_300),
    ("rust/xtask/src/calibration.rs", 1_250),
    ("rust/xtask/src/checks.rs", 1_600),
    ("rust/xtask/src/checks/bindings.rs", 700),
    ("rust/xtask/src/checks/bun_pin.rs", 600),
    ("rust/xtask/src/checks/ci_contracts.rs", 2_100),
    ("rust/xtask/src/checks/hosted_malware.rs", 350),
    ("rust/xtask/src/checks/hosted_signing.rs", 300),
    ("rust/xtask/src/checks/recovery_auth.rs", 350),
    ("rust/xtask/src/checks/release_pipeline.rs", 1_200),
    ("rust/xtask/src/checks/release_runner.rs", 1_900),
    ("rust/xtask/src/checks/rust_toolchain.rs", 860),
    ("rust/xtask/src/checks/tauri_metadata.rs", 1_300),
    ("rust/xtask/src/checks/trust.rs", 1_050),
    ("rust/xtask/src/checks/workflow.rs", 850),
    ("rust/xtask/src/audits/security.rs", 700),
    ("rust/xtask/src/audits/architecture.rs", 1_600),
    ("rust/xtask/src/audits/architecture/allowlist.rs", 350),
    ("rust/xtask/src/audits/source.rs", 100),
];

fn module_line_advisory_warning(
    rule: &str,
    path: &str,
    source: &str,
    threshold: usize,
) -> Option<String> {
    let line_count = source.lines().count();
    (line_count > threshold).then(|| {
        format!(
            "[{rule}] {path}: {line_count} lines (> advisory threshold {threshold}); consider splitting or clarifying responsibilities if the module is hard to review"
        )
    })
}

fn is_engine_test_owner_path(path: &str) -> bool {
    path == ENGINE_TEST_ROOT_PATH || path.starts_with("rust/crates/sky_player/src/engine/tests/")
}

fn scan_module_advisory_warnings(
    root: &Path,
    required_files: &[&str],
    required_directories: &[&str],
    rule: &str,
    fallback_threshold: fn(&str) -> usize,
    threshold_overrides: &[(&str, usize)],
) -> Result<Vec<String>> {
    let mut sources = BTreeMap::<String, String>::new();
    for relative in required_files {
        let path = root.join(relative);
        if !path.is_file() {
            return Err(format!("required Rust source file is missing: {relative}").into());
        }
        sources.insert((*relative).to_owned(), fs::read_to_string(path)?);
    }

    for relative_directory in required_directories {
        let directory = required_source_directory(root, relative_directory)?;
        for entry in WalkDir::new(&directory).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file()
                || entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    != Some("rs")
            {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            if let std::collections::btree_map::Entry::Vacant(source) = sources.entry(relative) {
                source.insert(fs::read_to_string(entry.path())?);
            }
        }
    }

    Ok(sources
        .iter()
        .filter_map(|(path, source)| {
            let threshold = threshold_overrides
                .iter()
                .find_map(|(override_path, threshold)| {
                    (*override_path == path).then_some(*threshold)
                })
                .unwrap_or_else(|| fallback_threshold(path));
            module_line_advisory_warning(rule, path, source, threshold)
        })
        .collect())
}

fn native_runtime_advisory_warnings(root: &Path) -> Result<Vec<String>> {
    let overrides = NATIVE_RUNTIME_MODULE_ADVISORY_THRESHOLDS
        .iter()
        .map(|(path, threshold)| (*path, *threshold))
        .chain(std::iter::once((PLAYBACK_INTEGRATION_ROOT_PATH, 500)))
        .collect::<Vec<_>>();
    scan_module_advisory_warnings(
        root,
        &[
            "apps/desktop/src-tauri/src/native_runtime.rs",
            PLAYBACK_INTEGRATION_ROOT_PATH,
        ],
        &[
            "apps/desktop/src-tauri/src/native_runtime",
            "apps/desktop/src-tauri/src/native_runtime/tests/playback_integration",
        ],
        "native_runtime_module_lines",
        native_runtime_default_threshold,
        &overrides,
    )
}

fn native_runtime_default_threshold(path: &str) -> usize {
    if path.contains("/tests/playback_integration/") {
        1_600
    } else if path.contains("/tests/") {
        1_100
    } else {
        1_650
    }
}

fn engine_test_default_threshold(_: &str) -> usize {
    ENGINE_TEST_MODULE_ADVISORY_THRESHOLD
}

fn xtask_default_threshold(_: &str) -> usize {
    2_100
}

fn engine_test_advisory_warnings(root: &Path) -> Result<Vec<String>> {
    let overrides = [(ENGINE_TEST_ROOT_PATH, 750)];
    scan_module_advisory_warnings(
        root,
        &[ENGINE_TEST_ROOT_PATH],
        &["rust/crates/sky_player/src/engine/tests"],
        "engine_test_module_lines",
        engine_test_default_threshold,
        &overrides,
    )
}

fn xtask_module_advisory_warnings(root: &Path) -> Result<Vec<String>> {
    scan_module_advisory_warnings(
        root,
        &[],
        &["rust/xtask/src", "rust/xtask/src/checks"],
        "xtask_module_lines",
        xtask_default_threshold,
        XTASK_MODULE_ADVISORY_THRESHOLDS,
    )
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

fn required_source_directory(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = root.join(relative);
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => Ok(path),
        Ok(_) => Err(format!(
            "architecture audit cannot traverse `{relative}`: expected a directory"
        )
        .into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(format!(
            "architecture audit cannot traverse `{relative}`: required scan root is missing"
        )
        .into()),
        Err(error) => {
            Err(format!("architecture audit cannot inspect `{relative}`: {error}").into())
        }
    }
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
    let source_root = required_source_directory(root, "rust/crates/sky_app_core/src")?;
    for entry in WalkDir::new(source_root).follow_links(false) {
        let entry = entry?;
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

const FACADE_ADVISORY_LIMIT: usize = 250;
const REGULAR_ADVISORY_LIMIT: usize = 700;
const WORKER_FUNCTION_HARD_LIMIT: usize = 350;
const CONTEXT_FIELD_ADVISORY_LIMIT: usize = 12;
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
    "hold_forensics.rs",
    "mod.rs",
    "observation.rs",
    "observer.rs",
    "observer_trace.rs",
    "observer_wake.rs",
    "timing.rs",
];
const UNSAFE_AUDIT_SOURCE_ROOTS: &[&str] = &[
    "rust/crates/sky_dispatch_core/src",
    "rust/crates/sky_dispatch_win32/src",
    "rust/crates/sky_app_core/src",
    "rust/crates/sky_player/src",
    "rust/crates/sky_native_adapters/src",
    "rust/crates/sky_windows_security/src",
    "apps/desktop/src-tauri/src",
];

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

fn context_violations(lines: &[String]) -> Vec<(String, usize)> {
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
        if fields > CONTEXT_FIELD_ADVISORY_LIMIT {
            result.push((name.to_owned(), fields));
        }
    }
    result
}

fn context_advisory_warnings(path: &str, lines: &[String]) -> Vec<String> {
    context_violations(lines)
        .into_iter()
        .map(|(name, fields)| {
            format!(
                "[context_fields] {path}: {name} has {fields} fields (> advisory threshold {CONTEXT_FIELD_ADVISORY_LIMIT}); consider whether the options remain clear to callers"
            )
        })
        .collect()
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

fn top_level_glob_import_count(lines: &[String]) -> usize {
    let mut depth = 0isize;
    let mut count = 0;
    for line in lines {
        if depth == 0 && line.trim() == "use super::*;" {
            count += 1;
        }
        depth = (depth + line.matches('{').count() as isize - line.matches('}').count() as isize)
            .max(0);
    }
    count
}

fn is_test_source_path(path: &str, source: &str) -> bool {
    path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs")
        || path.contains("/tests/")
        || source.lines().any(|line| line.trim() == "#![cfg(test)]")
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

fn dependency_direction_violations(crate_name: &str, joined_source: &str) -> Vec<&'static str> {
    let mut violations = Vec::new();
    if crate_name == "sky_dispatch_core"
        && (joined_source.contains("sky_dispatch_win32::")
            || joined_source.contains("use sky_dispatch_win32"))
    {
        violations.push("core imports sky_dispatch_win32");
    }
    if ["sky_dispatch_core", "sky_dispatch_win32"].contains(&crate_name)
        && (joined_source.contains("sky_player::") || joined_source.contains("use sky_player"))
    {
        violations.push("lower crate imports sky_player");
    }
    violations
}

fn runtime_schedule_clone_violation(relative_path: &str, joined_source: &str) -> bool {
    (relative_path == "rust/crates/sky_player/src/engine/worker.rs"
        || relative_path.starts_with("rust/crates/sky_player/src/engine/worker/"))
        && WORKER_SCHEDULE_CLONE_PATTERNS
            .iter()
            .any(|pattern| joined_source.contains(pattern))
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
    let file_is_test = is_test_source_path(path, source);
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
    allowlist: &ArchitectureAllowlist,
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
    let allowlist = load_architecture_allowlist(root)?;
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    warnings.extend(native_runtime_advisory_warnings(root)?);
    warnings.extend(engine_test_advisory_warnings(root)?);
    warnings.extend(xtask_module_advisory_warnings(root)?);
    let mut used_metric_allowlist = BTreeSet::new();
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
    let app_core_source = required_source_directory(root, "rust/crates/sky_app_core/src")?;
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
    let dispatch_dir =
        required_source_directory(root, "rust/crates/sky_player/src/engine/worker/dispatch")?;
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(&dispatch_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("rs")
        {
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| "dispatch module name must be valid UTF-8")?;
            if !name.ends_with("_tests.rs") {
                actual.insert(name);
            }
        }
    }
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

    for crate_name in [
        "sky_dispatch_core",
        "sky_dispatch_win32",
        "sky_app_core",
        "sky_player",
    ] {
        let source_root =
            required_source_directory(root, &format!("rust/crates/{crate_name}/src"))?;
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
                let (rule, threshold) = if FACADES.contains(
                    &absolute
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(""),
                ) {
                    ("facade_lines", FACADE_ADVISORY_LIMIT)
                } else {
                    ("regular_module_lines", REGULAR_ADVISORY_LIMIT)
                };
                if let Some(warning) =
                    module_line_advisory_warning(rule, &relative, &source, threshold)
                {
                    warnings.push(warning);
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
                    architecture_metric_record(
                        &mut errors,
                        &mut warnings,
                        &allowlist,
                        &mut used_metric_allowlist,
                        ArchitectureMetric {
                            path: &relative,
                            rule: "dispatch_function_lines",
                            subject: &name,
                            observed: count,
                            unit: "lines",
                            hard_limit: DISPATCH_FUNCTION_HARD_LIMIT,
                        },
                    );
                }
            }
            for violation in dependency_direction_violations(crate_name, &joined) {
                architecture_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &relative,
                    "dependency_direction",
                    violation,
                );
            }
            let glob_imports = top_level_glob_import_count(&clean);
            if glob_imports > 0 && !is_test_source_path(&relative, &source) {
                architecture_metric_record(
                    &mut errors,
                    &mut warnings,
                    &allowlist,
                    &mut used_metric_allowlist,
                    ArchitectureMetric {
                        path: &relative,
                        rule: "production_glob_import",
                        subject: "",
                        observed: glob_imports,
                        unit: "top-level `use super::*` imports",
                        hard_limit: 0,
                    },
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
            if runtime_schedule_clone_violation(&relative, &joined) {
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
            warnings.extend(context_advisory_warnings(&relative, &clean));
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
        let absolute_root = required_source_directory(root, source_root)?;
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
    for (path, rule, _) in allowlist.keys() {
        if rule == "unsafe_boundary" && !production_unsafe_modules.contains(path) {
            errors.push(format!(
                "[stale_unsafe_allowlist_entry] {path}: no production unsafe code remains"
            ));
        }
    }
    errors.extend(stale_metric_allowlist_entries(
        &allowlist,
        &used_metric_allowlist,
    ));
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
    if !errors.is_empty() {
        return Err(format!(
            "architecture audit failed: {} ({} error(s))",
            errors.join("; "),
            errors.len(),
        )
        .into());
    }
    println!(
        "[xtask] architecture checks: PASS ({} warning(s))",
        warnings.len()
    );
    Ok(())
}

pub(crate) fn run(root: &Path) -> Result<()> {
    app_core_boundary(root)?;
    adapter_boundary(root)?;
    architecture(root)
}

#[cfg(test)]
#[path = "architecture/tests.rs"]
mod tests;
