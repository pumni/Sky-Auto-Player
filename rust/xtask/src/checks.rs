use crate::{Result, process, repo};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const FORBIDDEN_SECURITY_APIS: &[&str] = &[
    "SetWindowsHookEx",
    "SetWinEventHook",
    "ReadProcessMemory",
    "WriteProcessMemory",
    "VirtualAllocEx",
    "VirtualFreeEx",
    "VirtualProtectEx",
    "VirtualQueryEx",
    "CreateRemoteThread",
    "CreateRemoteThreadEx",
    "NtCreateThreadEx",
    "RtlCreateUserThread",
    "QueueUserAPC",
    "GetThreadContext",
    "SetThreadContext",
    "SuspendThread",
    "DebugActiveProcess",
    "keybd_event",
    "mouse_event",
];
const RETIRED_ACTIVE_TOKENS: &[&str] = &[
    "pyo3",
    "maturin",
    "PyInstaller",
    "sky_player_rs",
    "desktop_ipc",
    "Sky-Auto-Player-Core.exe",
    "build_rust_wheel.py",
    "scripts/check.py",
    "scripts/build_portable_release.py",
    "scripts/verify_release_manifest.py",
];

fn rust_manifest(root: &Path) -> Result<String> {
    Ok(fs::read_to_string(root.join("rust/Cargo.toml"))?)
}

fn active_files(root: &Path) -> impl Iterator<Item = std::path::PathBuf> {
    [
        root.join("rust/Cargo.toml"),
        root.join("rust/Cargo.lock"),
        root.join("desktop/src-tauri/Cargo.toml"),
        root.join("desktop/package.json"),
        root.join(".github/workflows/ci.yml"),
        root.join(".github/workflows/release.yml"),
    ]
    .into_iter()
}

fn walk_source(root: &Path, prefix: &str) -> Result<Vec<std::path::PathBuf>> {
    let directory = root.join(prefix);
    let mut files = Vec::new();
    for entry in WalkDir::new(directory).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_file()
            && matches!(
                entry.path().extension().and_then(|e| e.to_str()),
                Some("rs" | "toml" | "yml" | "yaml" | "json")
            )
        {
            files.push(entry.into_path());
        }
    }
    Ok(files)
}

fn architecture(root: &Path) -> Result<()> {
    let manifest = rust_manifest(root)?;
    let app_core = fs::read_to_string(root.join("rust/crates/sky_app_core/Cargo.toml"))?;
    for forbidden in [
        "tauri",
        "pyo3",
        "windows-sys",
        "sky_desktop_shell",
        "sky_player",
        "sky_native_adapters",
    ] {
        if app_core.contains(forbidden) {
            return Err(format!("sky_app_core has forbidden dependency: {forbidden}").into());
        }
    }
    if manifest.contains("sky_player_rs") || manifest.contains("pyo3") {
        return Err("production workspace contains a retired Python/player bridge".into());
    }
    let allowlist = root.join(".config/rust_architecture_allowlist.json");
    if !allowlist.is_file() {
        return Err(format!("architecture allowlist is missing: {}", allowlist.display()).into());
    }
    println!("[xtask] architecture checks: PASS");
    Ok(())
}

fn security(root: &Path) -> Result<()> {
    for prefix in ["rust", "desktop/src-tauri"] {
        for path in walk_source(root, prefix)? {
            if path.starts_with(root.join("rust/xtask")) {
                continue;
            }
            let content = fs::read_to_string(&path)?;
            for token in FORBIDDEN_SECURITY_APIS {
                if content.contains(token) {
                    return Err(
                        format!("forbidden security API {token} in {}", path.display()).into(),
                    );
                }
            }
        }
    }
    println!("[xtask] security checks: PASS");
    Ok(())
}

fn retirement(root: &Path) -> Result<()> {
    let mut files = active_files(root).collect::<Vec<_>>();
    files.extend(walk_source(root, "desktop/src")?);
    files.extend(walk_source(root, "rust/crates")?);
    for path in files {
        if !path.is_file() {
            continue;
        }
        if path
            .components()
            .any(|component| component.as_os_str() == "tests")
        {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        for token in RETIRED_ACTIVE_TOKENS {
            if content.contains(token) {
                return Err(format!(
                    "retired token {token} remains in active file {}",
                    path.display()
                )
                .into());
            }
        }
    }
    validate_tooling_ledger(root)?;
    println!("[xtask] retirement checks: PASS");
    Ok(())
}

fn validate_tooling_ledger(root: &Path) -> Result<()> {
    let ledger_path = root.join("docs/migration/wave6-tooling-retirement-ledger.json");
    let payload: Value = serde_json::from_slice(&fs::read(&ledger_path)?)?;
    let entries = payload
        .get("entries")
        .and_then(Value::as_array)
        .ok_or("Wave 6 ledger entries must be an array")?;
    let evidence_classes = [
        "MIGRATED_XTASK",
        "MIGRATED_RUST",
        "MIGRATED_TYPESCRIPT",
        "DUPLICATE",
        "FIXTURE_FROZEN",
    ];
    let placeholders = [
        "generic evidence",
        "native covers",
        "named native/frontend/updater tests",
        "direct Rust/native build evidence is stronger",
        "native Rust/Tauri services now own",
    ];
    let mut ledger_paths = std::collections::BTreeSet::new();
    for entry in entries {
        let object = entry
            .as_object()
            .ok_or("Wave 6 ledger entry must be an object")?;
        let path = object
            .get("path")
            .and_then(Value::as_str)
            .ok_or("Wave 6 ledger path missing")?;
        ledger_paths.insert(path.to_owned());
        let classification = object
            .get("classification")
            .and_then(Value::as_str)
            .ok_or("Wave 6 ledger classification missing")?;
        let exists = root.join(path).exists();
        if classification == "NONCANONICAL_RETAINED" && !exists {
            return Err(format!("retained ledger path does not exist: {path}").into());
        }
        if evidence_classes.contains(&classification) {
            let invariants = object
                .get("invariants")
                .and_then(Value::as_array)
                .ok_or(format!("{path}: invariants must be a non-empty array"))?;
            let evidence = object
                .get("evidence")
                .and_then(Value::as_array)
                .ok_or(format!("{path}: evidence must be a non-empty array"))?;
            if invariants.is_empty()
                || evidence.is_empty()
                || invariants
                    .iter()
                    .any(|value| value.as_str().is_none_or(str::is_empty))
            {
                return Err(format!(
                    "{path}: migrated ledger entries need concrete invariants/evidence"
                )
                .into());
            }
            for item in evidence {
                let target = item
                    .as_str()
                    .ok_or(format!("{path}: evidence target must be a string"))?;
                if placeholders.iter().any(|placeholder| {
                    target
                        .to_ascii_lowercase()
                        .contains(&placeholder.to_ascii_lowercase())
                }) {
                    return Err(
                        format!("{path}: placeholder evidence is not permitted: {target}").into(),
                    );
                }
                let (file, symbol) = target
                    .split_once("::")
                    .ok_or(format!("{path}: evidence must use path::symbol: {target}"))?;
                let evidence_path = root.join(file);
                if !evidence_path.is_file() {
                    return Err(format!("{path}: evidence file does not exist: {file}").into());
                }
                let source = fs::read_to_string(&evidence_path)?;
                if !symbol.is_empty() && !source.contains(symbol) {
                    return Err(format!("{path}: evidence symbol is absent: {target}").into());
                }
            }
        } else if matches!(
            classification,
            "OBSOLETE" | "TRANSPORT_ONLY" | "TOOLING_RETAINED" | "NONCANONICAL_RETAINED"
        ) {
            if !exists && classification != "OBSOLETE" && classification != "TRANSPORT_ONLY" {
                return Err(format!("{path}: retained tooling entry does not exist").into());
            }
        } else {
            return Err(
                format!("{path}: unknown Wave 6 ledger classification {classification}").into(),
            );
        }
    }
    let tracked_python = process::capture_text("git", &["ls-files", "--", "*.py"], root, &[])?;
    for path in tracked_python
        .lines()
        .filter(|path| root.join(path).is_file())
    {
        if !ledger_paths.contains(path) {
            return Err(format!("Python file is missing from Wave 6 ledger: {path}").into());
        }
    }
    Ok(())
}

pub fn bindings() -> Result<()> {
    let root = repo::root();
    let export_dir = prepare_binding_export_dir(&root)?;
    let export_dir = export_dir
        .to_str()
        .ok_or("binding export directory is not valid UTF-8")?
        .to_owned();
    let export_env = [("TS_RS_EXPORT_DIR", export_dir.as_str())];
    process::run(
        "cargo",
        &[
            "test",
            "--manifest-path",
            "rust/Cargo.toml",
            "-p",
            "sky_desktop_shell",
            "--lib",
            "--all-features",
            "--locked",
        ],
        &root,
        &export_env,
    )?;
    compare_generated_bindings(&root, Path::new(&export_dir))?;
    Ok(())
}

fn prepare_binding_export_dir(root: &Path) -> Result<std::path::PathBuf> {
    let export_dir = root.join("rust/target/xtask-bindings");
    if export_dir.exists() {
        if fs::symlink_metadata(&export_dir)?.file_type().is_symlink() {
            return Err("binding export directory must not be a symlink".into());
        }
        fs::remove_dir_all(&export_dir)?;
    }
    fs::create_dir_all(&export_dir)?;
    Ok(export_dir)
}

fn collect_binding_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    if !root.is_dir() {
        return Err(format!("binding export directory is missing: {}", root.display()).into());
    }
    let mut files = BTreeMap::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() {
            return Err(format!(
                "binding export contains a symlink: {}",
                entry.path().display()
            )
            .into());
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        files.insert(relative, fs::read(entry.path())?);
    }
    Ok(files)
}

fn compare_generated_bindings(root: &Path, export_dir: &Path) -> Result<()> {
    let checked_in_dir = root.join("desktop/src/bridge/generated");
    let mut expected = collect_binding_files(&checked_in_dir)?;
    // These are maintained frontend support files rather than ts-rs exports.
    expected.remove("index.ts");
    expected.remove("serde_json/JsonValue.ts");
    let actual = collect_binding_files(export_dir)?;
    if expected != actual {
        let expected_paths = expected.keys().cloned().collect::<Vec<_>>();
        let actual_paths = actual.keys().cloned().collect::<Vec<_>>();
        let changed = expected_paths
            .iter()
            .chain(actual_paths.iter())
            .filter(|path| expected.get(*path) != actual.get(*path))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        return Err(format!(
            "generated Tauri bindings differ from committed output: {}",
            changed.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    Ok(())
}

pub fn run(group: &str) -> Result<()> {
    let root = repo::root();
    match group {
        "static" => {
            architecture(&root)?;
            security(&root)?;
            retirement(&root)?;
        }
        "rust" => {
            // The canonical Windows qualification runs workspace tests in a
            // restricted environment.  Keep process-global test fixtures
            // deterministic there; this does not change product concurrency.
            let export_dir = prepare_binding_export_dir(&root)?;
            let export_dir = export_dir
                .to_str()
                .ok_or("binding export directory is not valid UTF-8")?
                .to_owned();
            let test_env = [
                ("RUST_TEST_THREADS", "1"),
                ("TS_RS_EXPORT_DIR", export_dir.as_str()),
            ];
            process::run(
                "cargo",
                &[
                    "fmt",
                    "--manifest-path",
                    "rust/Cargo.toml",
                    "--all",
                    "--",
                    "--check",
                ],
                &root,
                &[],
            )?;
            process::run(
                "cargo",
                &[
                    "clippy",
                    "--manifest-path",
                    "rust/Cargo.toml",
                    "--workspace",
                    "--all-targets",
                    "--all-features",
                    "--locked",
                    "--",
                    "-D",
                    "warnings",
                ],
                &root,
                &[],
            )?;
            process::run(
                "cargo",
                &[
                    "test",
                    "--manifest-path",
                    "rust/Cargo.toml",
                    "--workspace",
                    "--all-features",
                    "--locked",
                ],
                &root,
                &test_env,
            )?;
        }
        "desktop" => {
            process::run(
                "bun",
                &["install", "--frozen-lockfile"],
                &root.join("desktop"),
                &[],
            )?;
            process::run("bun", &["run", "check"], &root.join("desktop"), &[])?;
            process::run("bun", &["run", "test:e2e"], &root.join("desktop"), &[])?;
            process::run(
                "cargo",
                &[
                    "check",
                    "--manifest-path",
                    "rust/Cargo.toml",
                    "-p",
                    "sky_desktop_shell",
                    "--all-features",
                    "--locked",
                ],
                &root,
                &[],
            )?;
            bindings()?;
        }
        "all" => {
            run("static")?;
            run("rust")?;
            run("desktop")?;
        }
        other => return Err(format!("unknown check group: {other}").into()),
    }
    println!("[xtask] check {group}: PASS");
    Ok(())
}
