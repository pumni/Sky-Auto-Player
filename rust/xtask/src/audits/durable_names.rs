use crate::Result;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const ROOTS: &[&str] = &[
    "src",
    "desktop",
    "rust",
    "scripts",
    ".github/workflows",
    ".github/actions",
];
const EXTRA_FILES: &[&str] = &["pyproject.toml", "uv.lock"];
const HISTORICAL_PREFIXES: &[&str] = &[
    "scripts/bench_phase",
    "docs/evidence/desktop-phase",
    "tests/test_phase",
];
const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".pytest_cache",
    "__pycache__",
];

fn is_historical(path: &str) -> bool {
    HISTORICAL_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
}

pub(crate) fn contains_phase_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index + 5 <= bytes.len() {
        if !bytes[index..index + 5].eq_ignore_ascii_case(b"phase") {
            index += 1;
            continue;
        }
        let before_ok = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
        let mut cursor = index + 5;
        if bytes
            .get(cursor)
            .is_some_and(|byte| *byte == b'_' || *byte == b'-')
        {
            cursor += 1;
        }
        let digit_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        let after_ok = cursor == bytes.len() || !bytes[cursor].is_ascii_alphanumeric();
        if before_ok && cursor > digit_start && after_ok {
            return true;
        }
        index += 5;
    }
    false
}

pub(crate) fn path_has_durable_phase_name(relative: &str) -> bool {
    !is_historical(relative) && contains_phase_name(relative)
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut output = EXTRA_FILES
        .iter()
        .map(|path| root.join(path))
        .collect::<Vec<_>>();
    for relative in ROOTS {
        let base = root.join(relative);
        if !base.is_dir() {
            continue;
        }
        output.extend(
            WalkDir::new(base)
                .follow_links(false)
                .into_iter()
                .filter_entry(|entry| {
                    !entry.file_name().to_str().is_some_and(|name| {
                        IGNORED_DIRS.contains(&name) || name.ends_with(".egg-info")
                    })
                })
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.file_type().is_file())
                .map(|entry| entry.into_path()),
        );
    }
    output.sort();
    output.dedup();
    output
}

pub(crate) fn run(root: &Path) -> Result<()> {
    let audit_paths = [
        root.join("rust/xtask/src/audits/durable_names.rs"),
        root.join("scripts/audit_durable_phase_names.py"),
    ];
    let mut violations = Vec::new();
    for path in files(root) {
        if audit_paths.contains(&path) || !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if is_historical(&relative) {
            continue;
        }
        if path_has_durable_phase_name(&relative) {
            violations.push(format!("{relative}:0: durable phase identifier in path"));
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (line, value) in text.lines().enumerate() {
            if contains_phase_name(value) {
                violations.push(format!(
                    "{relative}:{}: durable phase identifier: {}",
                    line + 1,
                    value.trim()
                ));
            }
        }
    }
    if let Some(first) = violations.first() {
        return Err(format!(
            "durable-name audit failed: {first} ({} violation(s))",
            violations.len()
        )
        .into());
    }
    println!("[xtask] durable-name checks: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_boundary_semantics_match_python_contract() {
        for value in [
            "SKY_PHASE8_RESTART_SELFTEST",
            "__SKY_PHASE8_GUI_SMOKE__",
            "some_phase9_runtime_flag",
            "PHASE8_ARTIFACT_SUMMARY",
            "scripts/build_phase8.py",
        ] {
            assert!(contains_phase_name(value));
            assert!(path_has_durable_phase_name(value));
        }
        assert!(!path_has_durable_phase_name(
            "tests/test_phase9_gui_canonical.py"
        ));
        assert!(!path_has_durable_phase_name("rust/xtask/src/dist.rs"));
    }

    #[test]
    fn embedded_phase_without_separator_is_not_a_match() {
        assert!(!contains_phase_name("phase_name9"));
        assert!(!contains_phase_name("notphase9"));
    }
}
