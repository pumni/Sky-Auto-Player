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
    "site/src",
    "tests",
    ".config",
];

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".astro",
    ".turbo",
    ".cache",
];
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "ps1", "psm1", "psd1", "yml", "yaml", "toml",
    "json", "astro", "css", "html", "svg", "sh", "bat", "cmd",
];

// These names are part of a previously published evidence format and branch
// namespace. Each exception is tied to one active publisher/provenance file
// and one exact literal; unrelated names on the same line still fail the audit.
const RETAINED_EXTERNAL_REFERENCES: &[(&str, &str)] = &[
    (
        "scripts/run_rt_dispatch_evidence.ps1",
        "runs/r3-p4-20261016T145957238Z-be3ec0ab",
    ),
    (
        "scripts/run_rt_dispatch_evidence.ps1",
        "r3-p4-20261016T145957238Z-be3ec0ab",
    ),
    (
        "scripts/run_rt_dispatch_evidence.ps1",
        "rt-r3/evidence/p4-3a96be29eb2e",
    ),
    (
        "scripts/publish_rt_dispatch_evidence.ps1",
        "rt-r3/evidence/",
    ),
    (
        "scripts/lib/evidence_publication_contract.ps1",
        "rt-r3/p0-baseline",
    ),
    (
        "scripts/lib/evidence_publication_contract.ps1",
        "rt-r3/p1-lifecycle-release-barrier",
    ),
    (
        "scripts/lib/evidence_publication_contract.ps1",
        "rt-r3/p2-final-lease-age",
    ),
    (
        "scripts/lib/evidence_publication_contract.ps1",
        "rt-r3/p4-diagnostics",
    ),
    (
        "scripts/lib/evidence_publication_contract.ps1",
        "rt-r3/p5-final-qualification",
    ),
    ("scripts/test_evidence_contracts.ps1", "rt-r3/p0-baseline"),
    (
        "scripts/test_evidence_contracts.ps1",
        "rt-r3/p1-lifecycle-release-barrier",
    ),
    (
        "scripts/test_evidence_contracts.ps1",
        "rt-r3/p2-final-lease-age",
    ),
    (
        "scripts/test_evidence_contracts.ps1",
        "rt-r3/p4-diagnostics",
    ),
    (
        "scripts/test_evidence_contracts.ps1",
        "rt-r3/p5-final-qualification",
    ),
];

// The public updater trust root is versioned key material, not an identifier.
// Keep these exceptions tied to its three canonical declarations.
const RETAINED_VERSIONED_LITERALS: &[(&str, &str)] = &[
    (
        "desktop/src-tauri/src/native_update.rs",
        "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QUFCRDJFNzgzODgxOEMKUldTTWdUaDRMcjJxR2JxeE5kTUx5VlIxS1dhOHRrSTEzY2FMeE8wYldtckM2TjV2KzRwQUNaTEUK",
    ),
    (
        "desktop/src-tauri/tauri.conf.json",
        "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QUFCRDJFNzgzODgxOEMKUldTTWdUaDRMcjJxR2JxeE5kTUx5VlIxS1dhOHRrSTEzY2FMeE8wYldtckM2TjV2KzRwQUNaTEUK",
    ),
    (
        "rust/xtask/src/tauri_bundle.rs",
        "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE5QUFCRDJFNzgzODgxOEMKUldTTWdUaDRMcjJxR2JxeE5kTUx5VlIxS1dhOHRrSTEzY2FMeE8wYldtckM2TjV2KzRwQUNaTEUK",
    ),
];

// Stable fixtures may preserve identifiers that resemble implementation
// sequence markers; each exception remains tied to one exact source literal.
const RETAINED_FIXTURE_DATA_LITERALS: &[(&str, &str)] = &[
    ("tests/fixtures/catalog/search_cases.json", "Wave2"),
    (
        "rust/xtask/src/updater_trust.rs",
        "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDYzNkNBMzdDQzBDM0IyREQKUldUZHNzUEFmS05zWXpKMjRqcnRlR3FFY0pVMFVybEsydlhmdDhWV0RGYWYxc01wNlJSZ3dJWmQK",
    ),
];

fn identifier_boundary(bytes: &[u8], index: usize) -> bool {
    index == 0
        || !bytes[index - 1].is_ascii_alphanumeric()
        || (bytes[index].is_ascii_uppercase() && bytes[index - 1].is_ascii_lowercase())
}

fn has_phase_style_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    for index in 0..bytes.len().saturating_sub(4) {
        if !identifier_boundary(bytes, index)
            || !bytes[index..index + 5].eq_ignore_ascii_case(b"phase")
        {
            continue;
        }
        let mut cursor = index + 5;
        let underscore_suffix = bytes.get(cursor) == Some(&b'_');
        if underscore_suffix {
            cursor += 1;
        }
        if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            return true;
        }
        if underscore_suffix && bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
            cursor += 1;
            if cursor == bytes.len()
                || matches!(bytes.get(cursor), Some(b'_') | Some(b'-'))
                || bytes.get(cursor).is_some_and(u8::is_ascii_digit)
            {
                return true;
            }
        } else if bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
                cursor += 1;
            }
            if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
                return true;
            }
        } else if (bytes.get(cursor) == Some(&b'-')
            && bytes.get(cursor + 1).is_some_and(u8::is_ascii_digit))
            || (bytes.get(cursor).is_some_and(u8::is_ascii_uppercase)
                && (cursor + 1 == bytes.len()
                    || bytes.get(cursor + 1).is_some_and(|next| {
                        next.is_ascii_uppercase()
                            || next.is_ascii_digit()
                            || matches!(next, b'_' | b'-')
                    })))
        {
            return true;
        }
    }
    false
}

fn has_ordinal_marker(value: &str) -> bool {
    let bytes = value.as_bytes();
    for index in 0..bytes.len() {
        if !identifier_boundary(bytes, index) {
            continue;
        }

        let (prefix_len, prefix) =
            if bytes[index..].len() >= 2 && bytes[index..index + 2].eq_ignore_ascii_case(b"wo") {
                (2, "wo")
            } else if let Some(byte) = bytes.get(index) {
                let prefix = match byte.to_ascii_lowercase() {
                    b'r' => "r",
                    b'p' => "p",
                    b'h' => "h",
                    b'w' => "w",
                    _ => continue,
                };
                (1, prefix)
            } else {
                continue;
            };

        let mut cursor = index + prefix_len;
        let digits_start = cursor;
        while bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            cursor += 1;
        }
        if cursor == digits_start {
            continue;
        }

        let digits = &value[digits_start..cursor];
        let mut suffix_letter = None;
        if bytes.get(cursor).is_some_and(u8::is_ascii_alphabetic) {
            suffix_letter = bytes.get(cursor).copied();
            cursor += 1;
        }
        let has_suffix_letter = suffix_letter.is_some();

        // Percentile names are measurements, not optimization-wave markers.
        if prefix == "p" && matches!(digits, "50" | "75" | "90" | "95" | "99") {
            continue;
        }

        // PascalCase identifiers such as `W4Noncanonical` encode the marker
        // before the descriptive suffix, so the uppercase suffix starts the
        // evidence even though the rest of the word remains alphanumeric.
        if suffix_letter.is_some_and(|byte| byte.is_ascii_uppercase()) {
            return true;
        }
        if prefix == "r"
            && has_suffix_letter
            && matches!(digits, "8" | "9" | "10" | "11" | "12" | "13" | "14" | "15")
            && matches!(suffix_letter, Some(b'd') | Some(b'w') | Some(b'b'))
        {
            continue;
        }

        let next = bytes.get(cursor);
        if matches!(next, Some(b'_') | Some(b'-'))
            || next.is_some_and(u8::is_ascii_uppercase)
            || (has_suffix_letter && next.is_some_and(|byte| !byte.is_ascii_alphanumeric()))
        {
            return true;
        }
    }
    false
}

fn has_work_order_or_sprint_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for index in 0..bytes.len() {
        if !identifier_boundary(value.as_bytes(), index) {
            continue;
        }

        let mut cursor = if bytes[index..].starts_with(b"sprint") {
            index + "sprint".len()
        } else if bytes[index..].starts_with(b"work") {
            let mut cursor = index + "work".len();
            if matches!(bytes.get(cursor), Some(b'_') | Some(b'-')) {
                cursor += 1;
            }
            if !bytes[cursor..].starts_with(b"order") {
                continue;
            }
            cursor + "order".len()
        } else if bytes[index..].starts_with(b"wo") {
            index + "wo".len()
        } else {
            continue;
        };

        while matches!(bytes.get(cursor), Some(b'_') | Some(b'-')) {
            cursor += 1;
        }
        if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            return true;
        }
    }
    false
}

fn has_wave_or_batch_name(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for index in 0..bytes.len() {
        if !identifier_boundary(value.as_bytes(), index) {
            continue;
        }

        let mut cursor = if bytes[index..].starts_with(b"optimization") {
            let mut cursor = index + "optimization".len();
            while matches!(bytes.get(cursor), Some(b'_' | b'-' | b' ')) {
                cursor += 1;
            }
            if bytes[cursor..].starts_with(b"wave") {
                cursor + "wave".len()
            } else if bytes[cursor..].starts_with(b"batch") {
                cursor + "batch".len()
            } else {
                continue;
            }
        } else if bytes[index..].starts_with(b"wave") {
            index + "wave".len()
        } else if bytes[index..].starts_with(b"batch") {
            index + "batch".len()
        } else {
            continue;
        };

        while matches!(bytes.get(cursor), Some(b'_' | b'-' | b' ')) {
            cursor += 1;
        }
        if bytes.get(cursor).is_some_and(u8::is_ascii_digit) {
            return true;
        }
    }
    false
}

fn contains_implementation_history_name(value: &str) -> bool {
    has_phase_style_name(value)
        || has_ordinal_marker(value)
        || has_work_order_or_sprint_name(value)
        || has_wave_or_batch_name(value)
}

pub(crate) fn path_has_durable_history_name(relative: &str) -> bool {
    contains_implementation_history_name(relative)
}

#[cfg(test)]
fn is_retained_external_reference(relative: &str, line: &str) -> bool {
    RETAINED_EXTERNAL_REFERENCES
        .iter()
        .any(|(path, literal)| *path == relative && line.contains(literal))
}

fn line_has_unretained_history_name(relative: &str, line: &str) -> bool {
    if !contains_implementation_history_name(line) {
        return false;
    }

    let mut remaining = line.to_owned();
    for (path, literal) in RETAINED_EXTERNAL_REFERENCES {
        if *path == relative {
            remaining = remaining.replace(literal, "");
        }
    }
    for (path, literal) in RETAINED_VERSIONED_LITERALS {
        if *path == relative {
            remaining = remaining.replace(literal, "");
        }
    }
    for (path, literal) in RETAINED_FIXTURE_DATA_LITERALS {
        if *path == relative {
            remaining = remaining.replace(literal, "");
        }
    }
    contains_implementation_history_name(&remaining)
}

fn has_source_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            SOURCE_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str())
        })
}

fn files(root: &Path) -> Vec<PathBuf> {
    let mut output = Vec::new();
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
                    !entry
                        .file_name()
                        .to_str()
                        .is_some_and(|name| IGNORED_DIRS.contains(&name))
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
    let audit_paths = [root.join("rust/xtask/src/audits/durable_names.rs")];
    let mut violations = Vec::new();
    for path in files(root) {
        if audit_paths.contains(&path) || !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if path_has_durable_history_name(&relative) {
            violations.push(format!(
                "{relative}:0: implementation-history name in active path"
            ));
        }
        if !has_source_extension(&path) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (line, value) in text.lines().enumerate() {
            if line_has_unretained_history_name(&relative, value) {
                violations.push(format!(
                    "{relative}:{}: implementation-history name in active source: {}",
                    line + 1,
                    value.trim()
                ));
            }
        }
    }
    if !violations.is_empty() {
        return Err(format!("durable-name audit failed:\n{}", violations.join("\n")).into());
    }
    println!("[xtask] durable-name checks: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_phase_and_ordinal_implementation_names() {
        for value in [
            "phase8_runtime",
            "phase_a_sender_only",
            "PhaseASenderOnly",
            "rt_r3_probe.rs",
            "p4_final_qualification.rs",
            "p4-final-qualification.rs",
            "r3-dispatch-probe.rs",
            "h4_watchdog",
            "h4-watchdog",
            "wo04_runner",
            "wo04-runner",
            "work_order_04_runner",
            "WO-04_release_contract",
            "sprint-12-runner",
            "wave2_fixtures.rs",
            "Wave3SongFixtures",
            "optimization_wave_4",
            "optimizationBatch5",
            "W4Noncanonical",
            "P7aPhysicalStudy",
        ] {
            assert!(contains_implementation_history_name(value), "{value}");
        }
    }

    #[test]
    fn detects_spaced_phase_ordinals_but_not_prose() {
        assert!(contains_implementation_history_name(
            "Phase 8 compatibility fixture"
        ));
        assert!(contains_implementation_history_name("phase-8-runtime"));
        assert!(!contains_implementation_history_name(
            "the phase-based wait policy"
        ));
        assert!(!contains_implementation_history_name(
            "phase name in a state machine"
        ));
    }

    #[test]
    fn ignores_percentiles_versions_and_embedded_words() {
        for value in [
            "p50_us",
            "p75_us",
            "p90_us",
            "p95_us",
            "p99_us",
            "p99Values",
            "p99_9_us",
            "pep440",
            "win32_error",
            "v4_release",
            "notphase8",
            "phase_name9",
            "phaseStartName",
            "phaseElapsed",
            "phaseMarkers",
            "PhaseStartName",
            "phase_units",
            "phase_label",
            "SKY_UPDATE_SAFETY_PHASE_LOG",
            "poll_phase",
            "non-ASCII — punctuation before a work-order word",
            "%r8d",
            "waveform_samples",
            "batch_size_4",
            "batching_workers",
            "world-04_map",
        ] {
            assert!(!contains_implementation_history_name(value), "{value}");
        }
    }

    #[test]
    fn active_paths_with_historical_markers_are_rejected() {
        assert!(path_has_durable_history_name(
            "tests/test_phase9_gui_canonical_fixture"
        ));
        assert!(path_has_durable_history_name(
            "scripts/bench_phase3/committed-run.json"
        ));
        assert!(path_has_durable_history_name("tests/phase9_regression.rs"));
        assert!(!path_has_durable_history_name(
            "rust/crates/sky_player/src/engine/phase_units.rs"
        ));
        assert!(path_has_durable_history_name(
            "rust/crates/sky_player/src/engine/phase8_runtime.rs"
        ));
    }

    #[test]
    fn retained_external_references_are_path_and_literal_scoped() {
        let branch = "rt-r3/p4-diagnostics";
        assert!(is_retained_external_reference(
            "scripts/lib/evidence_publication_contract.ps1",
            branch
        ));
        assert!(!is_retained_external_reference(
            "scripts/run_rt_dispatch_evidence.ps1",
            branch
        ));
        assert!(line_has_unretained_history_name(
            "scripts/lib/evidence_publication_contract.ps1",
            "rt-r3/p4-diagnostics and r3_future_probe"
        ));
        assert!(!line_has_unretained_history_name(
            "scripts/lib/evidence_publication_contract.ps1",
            branch
        ));

        let fixture_key = RETAINED_FIXTURE_DATA_LITERALS[1].1;
        assert!(!line_has_unretained_history_name(
            "rust/xtask/src/updater_trust.rs",
            fixture_key
        ));
        assert!(line_has_unretained_history_name(
            "rust/xtask/src/updater_trust.rs",
            &format!("{fixture_key} r3_future_probe")
        ));
    }

    #[test]
    fn retained_versioned_key_material_is_path_scoped() {
        let public_key = RETAINED_VERSIONED_LITERALS[0].1;
        assert!(!line_has_unretained_history_name(
            "desktop/src-tauri/src/native_update.rs",
            public_key
        ));
        assert!(line_has_unretained_history_name(
            "desktop/src-tauri/src/native_update.rs",
            &format!("{public_key} r3_future_probe")
        ));
        assert!(line_has_unretained_history_name(
            "desktop/src-tauri/src/native_runtime.rs",
            public_key
        ));
    }

    #[test]
    fn retained_fixture_path_is_an_exact_data_exception() {
        assert!(!line_has_unretained_history_name(
            "tests/fixtures/catalog/search_cases.json",
            r#""C:\\Wave2\\Songs\\Dandelions.txt""#
        ));
        assert!(line_has_unretained_history_name(
            "tests/fixtures/catalog/search_cases.json",
            r#""C:\\Wave2\\Songs\\Dandelions.txt p4_future""#
        ));
        assert!(line_has_unretained_history_name(
            "tests/fixtures/settings/compatibility_cases.json",
            r#""C:\\Wave2\\Songs\\Dandelions.txt""#
        ));
    }
}
