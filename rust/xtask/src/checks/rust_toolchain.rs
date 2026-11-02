use crate::Result;
use semver::Version;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

const TOOLCHAIN_FILE: &str = "rust/rust-toolchain.toml";
const RESOLVER: &str = "scripts/toolchain-pin.mjs";
const SETUP_ACTION: &str = ".github/actions/setup-rust-pinned/action.yml";
const LOCAL_SETUP: &str = "uses: ./.github/actions/setup-rust-pinned";
const WORKFLOWS: &[&str] = &[
    ".github/workflows/ci.yml",
    ".github/workflows/release-v4.yml",
    ".github/workflows/hosted-v4-signing-proof.yml",
    ".github/workflows/v4-metadata-recovery.yml",
];
const ACTIVE_PIN_CONSUMERS: &[&str] = &[
    RESOLVER,
    SETUP_ACTION,
    "scripts/run_rt_dispatch_evidence.ps1",
    "desktop/scripts/profile-builtin-library.mjs",
    "desktop/src-tauri/src/native_runtime/tests/catalog_viewport_benchmark.rs",
    "docs/rust-toolchain-policy.md",
    "docs/v4-hosted-signing-proof.md",
    "rust/xtask/src/checks/ci_contracts.rs",
    "rust/xtask/src/checks/hosted_signing.rs",
];

fn contract_error<T>(message: impl Into<String>) -> Result<T> {
    Err(message.into().into())
}

fn canonical_rust_version(source: &str) -> Result<String> {
    let document: toml::Value = toml::from_str(source)?;
    let root = document
        .as_table()
        .ok_or("Rust toolchain file must be a TOML table")?;
    if root.len() != 1 || !root.contains_key("toolchain") {
        return contract_error("rust/rust-toolchain.toml must contain only one [toolchain] table");
    }
    let toolchain = root["toolchain"]
        .as_table()
        .ok_or("Rust toolchain [toolchain] must be a table")?;
    let expected = BTreeSet::from(["channel", "profile", "targets", "components"]);
    let actual = toolchain
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return contract_error(format!(
            "Rust toolchain pin must define exactly channel/profile/targets/components (found {actual:?})"
        ));
    }

    let channel = toolchain["channel"]
        .as_str()
        .ok_or("Rust toolchain channel must be a quoted exact version")?;
    let version = Version::parse(channel)
        .map_err(|error| format!("Rust toolchain channel is invalid: {error}"))?;
    if !version.pre.is_empty() || !version.build.is_empty() || version.to_string() != channel {
        return contract_error("Rust toolchain channel must be an exact stable numeric patch");
    }
    if toolchain["profile"].as_str() != Some("minimal") {
        return contract_error("Rust toolchain profile must remain minimal");
    }
    for key in ["targets", "components"] {
        let values = toolchain[key]
            .as_array()
            .ok_or_else(|| format!("Rust toolchain {key} must be a string array"))?;
        if values.is_empty()
            || values.iter().any(|value| {
                value.as_str().is_none_or(|text| {
                    text.trim().is_empty() || text.chars().any(char::is_whitespace)
                })
            })
        {
            return contract_error(format!(
                "Rust toolchain {key} must contain nonempty strings"
            ));
        }
        let unique = values
            .iter()
            .filter_map(toml::Value::as_str)
            .collect::<BTreeSet<_>>();
        if unique.len() != values.len() {
            return contract_error(format!("Rust toolchain {key} contains duplicate entries"));
        }
    }
    Ok(version.to_string())
}

fn validate_root_overrides(root: &Path) -> Result<()> {
    for candidate in [
        "rust-toolchain.toml",
        "rust-toolchain",
        "rust/rust-toolchain",
    ] {
        if root.join(candidate).exists() {
            return contract_error(format!(
                "conflicting Rust toolchain configuration: {candidate}"
            ));
        }
    }
    for config_name in [".cargo/config.toml", ".cargo/config"] {
        let config_path = root.join(config_name);
        if config_path.exists() {
            let config = fs::read_to_string(&config_path)?;
            for line in config.lines() {
                let code = line.split('#').next().unwrap_or_default().trim();
                if let Some((key, _)) = code.split_once('=')
                    && matches!(key.trim(), "rustc" | "RUSTUP_TOOLCHAIN")
                {
                    return contract_error(format!(
                        "{config_name} contains a conflicting Rust override: {}",
                        key.trim()
                    ));
                }
            }
        }
    }
    Ok(())
}

fn validate_action_source(source: &str) -> Result<()> {
    let inputs = source
        .split_once("inputs:\n")
        .and_then(|(_, tail)| tail.split_once("runs:\n").map(|(inputs, _)| inputs))
        .ok_or("setup-rust-pinned action must define inputs before runs")?;
    let input_names = inputs
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let (key, _) = trimmed.split_once(':')?;
            (line.starts_with("  ") && !line.starts_with("    ")).then_some(key)
        })
        .collect::<BTreeSet<_>>();
    if input_names != BTreeSet::from(["components", "targets"]) {
        return contract_error(format!(
            "setup-rust-pinned must accept only per-job targets/components (found {input_names:?})"
        ));
    }
    let read_pin = source
        .find("node scripts/toolchain-pin.mjs read rust")
        .ok_or("setup-rust-pinned must read the checked-out Rust pin")?;
    let install = source
        .find("'toolchain', 'install', $version")
        .ok_or("setup-rust-pinned must install the parsed Rust version")?;
    let export = source
        .find("RUSTUP_TOOLCHAIN=$version")
        .ok_or("setup-rust-pinned must export the parsed Rust version through GITHUB_ENV")?;
    let verify = source
        .find("node scripts/toolchain-pin.mjs verify rust")
        .ok_or("setup-rust-pinned must verify Rustup/rustc/Cargo")?;
    if !(read_pin < install && install < export && export < verify)
        || !source.contains("shell: pwsh")
        || !source.contains("$env:GITHUB_ENV")
        || !source.contains("if ($readExit -ne 0) { exit $readExit }")
        || !source.contains("if ($installExit -ne 0) { exit $installExit }")
        || !source.contains("if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }")
    {
        return contract_error(
            "setup-rust-pinned must fail closed in read/install/export/verify order",
        );
    }
    Ok(())
}

fn source_part_is_triplet(value: &str) -> bool {
    let mut components = value.split('.');
    components
        .next()
        .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && components
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && components
            .next()
            .is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        && components.next().is_none()
}

fn rust_version_assertion(line: &str) -> bool {
    let normalized = line.replace("\\.", ".");
    let lower = normalized.to_ascii_lowercase();
    for marker in [
        "rustup_toolchain",
        "rust_toolchain_version",
        "rustc_version",
        "rust_version",
        "rustcversion",
        "rustversion",
        "rustup toolchain install",
        "rustc",
        "rust/cargo version",
        "rust/cargo",
        "rust compiler version",
        "rust toolchain version",
        "rust toolchain ",
        "rust version",
        "compiler version",
        "compiler:",
        "cargo_version",
        "cargo version",
        "cargo +",
        "cargo ",
        "rust ",
    ] {
        for (index, _) in lower.match_indices(marker) {
            let mut tail = &normalized[index + marker.len()..];
            for _ in 0..4 {
                tail = tail.trim_start_matches(|character: char| {
                    character.is_whitespace()
                        || matches!(character, ':' | '=' | '\'' | '"' | '`' | 'v')
                });
                let token = tail
                    .split(|character: char| {
                        character.is_whitespace()
                            || matches!(character, ':' | '=' | '\'' | '"' | '`' | ',' | ';')
                    })
                    .next()
                    .unwrap_or_default();
                let candidate = token
                    .chars()
                    .take_while(|character| character.is_ascii_digit() || *character == '.')
                    .collect::<String>();
                if source_part_is_triplet(&candidate) {
                    return true;
                }
                if !matches!(
                    token.to_ascii_lowercase().as_str(),
                    "stable"
                        | "production"
                        | "pinned"
                        | "expected"
                        | "current"
                        | "version"
                        | "compiler"
                        | "toolchain"
                        | "is"
                ) {
                    break;
                }
                tail = &tail[token.len()..];
            }
        }
    }
    false
}

fn first_rust_command(source: &str) -> Option<usize> {
    ["cargo ", "rustc ", "rustup ", "RUSTUP_TOOLCHAIN"]
        .iter()
        .filter_map(|marker| source.find(marker))
        .min()
}

fn is_rust_consumer(source: &str) -> bool {
    first_rust_command(source).is_some() || source.contains(LOCAL_SETUP)
}

fn job_id(line: &str) -> Option<&str> {
    let tail = line.strip_prefix("  ")?;
    if tail.starts_with(' ') {
        return None;
    }
    let (name, value) = tail.split_once(':')?;
    (value.trim().is_empty()
        && !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'))
    .then_some(name)
}

fn workflow_jobs(source: &str) -> Result<Vec<(String, String)>> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    let lines = normalized.lines().collect::<Vec<_>>();
    let jobs_start = lines
        .iter()
        .position(|line| *line == "jobs:")
        .ok_or("workflow must contain a jobs: map")?;
    let mut starts = Vec::new();
    for (index, line) in lines.iter().enumerate().skip(jobs_start + 1) {
        if line.starts_with(' ') {
            if let Some(id) = job_id(line) {
                starts.push((index, id.to_owned()));
            }
        } else if !line.trim().is_empty() {
            break;
        }
    }
    if starts.is_empty() {
        return contract_error("workflow jobs: map must not be empty");
    }
    Ok(starts
        .iter()
        .enumerate()
        .map(|(position, (start, id))| {
            let end = starts
                .get(position + 1)
                .map_or(lines.len(), |(next, _)| *next);
            (id.clone(), lines[*start..end].join("\n"))
        })
        .collect())
}

fn validate_setup_use(job: &str, path: &Path) -> Result<()> {
    let lines = job.lines().collect::<Vec<_>>();
    let uses = lines
        .iter()
        .position(|line| line.trim() == LOCAL_SETUP)
        .ok_or_else(|| {
            format!(
                "{} Rust consumer job is missing local setup action",
                path.display()
            )
        })?;
    let step_start = (0..uses)
        .rev()
        .find(|index| lines[*index].starts_with("      - name:"))
        .ok_or("Rust setup action must be inside a named workflow step")?;
    let step_end = (uses + 1..lines.len())
        .find(|index| lines[*index].starts_with("      - name:"))
        .unwrap_or(lines.len());
    let mut step_fields = BTreeSet::new();
    for line in &lines[step_start + 1..step_end] {
        if line.bytes().take_while(|byte| *byte == b' ').count() != 8 {
            continue;
        }
        let Some((key, value)) = line.trim().split_once(':') else {
            return contract_error("Rust setup step contains an unsupported YAML field");
        };
        if !step_fields.insert(key) || !matches!(key, "name" | "uses" | "with") {
            return contract_error(format!(
                "{} Rust setup step has duplicate/unsupported field {key}",
                path.display()
            ));
        }
        if key == "uses" && value.trim() != LOCAL_SETUP.strip_prefix("uses:").unwrap().trim() {
            return contract_error("Rust setup step must use the exact local pinned action");
        }
    }
    let with_start = (uses + 1..step_end).find(|index| lines[*index].trim() == "with:");
    let with_count = (uses + 1..step_end)
        .filter(|index| lines[*index].trim() == "with:")
        .count();
    if with_count > 1 {
        return contract_error(format!(
            "{} Rust setup step has ambiguous duplicate with: blocks",
            path.display()
        ));
    }
    if let Some(with_start) = with_start {
        let input_names = lines[with_start + 1..step_end]
            .iter()
            .filter_map(|line| {
                if line.starts_with("          ") && !line.starts_with("            ") {
                    line.trim().split_once(':').map(|(key, _)| key)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        let unique_names = input_names.iter().copied().collect::<BTreeSet<_>>();
        if unique_names.len() != input_names.len()
            || input_names
                .iter()
                .any(|key| !matches!(*key, "targets" | "components"))
        {
            return contract_error(format!(
                "{} Rust setup action has unsupported inputs: {input_names:?}",
                path.display()
            ));
        }
    }
    Ok(())
}

fn exact_checkout_position(job: &str, path: &Path, id: &str) -> Result<usize> {
    let lines = job.lines().collect::<Vec<_>>();
    let checkout_use = lines
        .iter()
        .position(|line| line.trim().starts_with("uses: actions/checkout@"))
        .ok_or_else(|| format!("{} job {id} must check out exact source", path.display()))?;
    let step_start = (0..checkout_use)
        .rev()
        .find(|index| lines[*index].starts_with("      - name:"))
        .ok_or("checkout action must be inside a named workflow step")?;
    let step_end = (checkout_use + 1..lines.len())
        .find(|index| lines[*index].starts_with("      - name:"))
        .unwrap_or(lines.len());
    let mut step_fields = BTreeSet::new();
    for line in &lines[step_start + 1..step_end] {
        if line.bytes().take_while(|byte| *byte == b' ').count() != 8 {
            continue;
        }
        let Some((key, _)) = line.trim().split_once(':') else {
            return contract_error("checkout step contains an unsupported YAML field");
        };
        if !step_fields.insert(key) || !matches!(key, "name" | "uses" | "with") {
            return contract_error(format!(
                "{} exact checkout step has duplicate/unsupported field {key}",
                path.display()
            ));
        }
    }
    let checkout_inputs = lines[checkout_use + 1..step_end]
        .iter()
        .filter_map(|line| {
            if line.starts_with("          ") && !line.starts_with("            ") {
                line.trim()
                    .split_once(':')
                    .map(|(key, value)| (key, value.trim()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let refs = checkout_inputs
        .iter()
        .filter(|(key, _)| *key == "ref")
        .map(|(_, value)| *value)
        .collect::<Vec<_>>();
    if refs.len() != 1
        || !matches!(
            refs[0],
            "${{ github.sha }}" | "${{ github.event.pull_request.head.sha || github.sha }}"
        )
    {
        return contract_error(format!(
            "{} job {id} must check out its exact event/source SHA before Rust setup",
            path.display()
        ));
    }
    job.find("uses: actions/checkout@")
        .ok_or_else(|| "exact checkout step position is missing".into())
}

fn validate_workflow_source(source: &str, path: &Path) -> Result<usize> {
    let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalized.lines() {
        let lower = line.to_ascii_lowercase();
        if line.contains("RUSTUP_TOOLCHAIN:")
            || lower.contains("rustup toolchain install")
            || lower.contains("cargo +")
            || rust_version_assertion(line)
        {
            return contract_error(format!(
                "{} contains an independent/stale Rust toolchain selection or assertion: {}",
                path.display(),
                line.trim()
            ));
        }
    }

    let mut action_count = 0;
    for (id, job) in workflow_jobs(&normalized)? {
        let has_action = job.matches(LOCAL_SETUP).count();
        let rust_consumer = is_rust_consumer(&job);
        let expected_non_rust = match path.to_string_lossy().replace('\\', "/").as_str() {
            ".github/workflows/ci.yml" => {
                ["desktop_web", "site", "status", "deploy_pages"].contains(&id.as_str())
            }
            ".github/workflows/v4-metadata-recovery.yml" => id == "validate",
            ".github/workflows/hosted-v4-signing-proof.yml" => id == "malware-scan-proof",
            _ => false,
        };
        if !rust_consumer && !expected_non_rust {
            return contract_error(format!(
                "{} job {id} is not classified as a Rust consumer; update the explicit job inventory",
                path.display()
            ));
        }
        if rust_consumer || !expected_non_rust {
            if has_action != 1 {
                return contract_error(format!(
                    "{} Rust consumer job {id} must use setup-rust-pinned exactly once (found {has_action})",
                    path.display()
                ));
            }
            let checkout = exact_checkout_position(&job, path, &id)?;
            let setup = job
                .find(LOCAL_SETUP)
                .ok_or_else(|| format!("{} job {id} is missing Rust setup", path.display()))?;
            let command = first_rust_command(&job).ok_or_else(|| {
                format!(
                    "{} job {id} has no recognizable Rust command",
                    path.display()
                )
            })?;
            if !(checkout < setup && setup < command) {
                return contract_error(format!(
                    "{} job {id} must check out exact source, set up Rust, then run Rust commands",
                    path.display()
                ));
            }
            validate_setup_use(&job, path)?;
            action_count += 1;
        } else if has_action != 0 {
            return contract_error(format!(
                "{} non-Rust job {id} unexpectedly installs the Rust toolchain",
                path.display()
            ));
        }
    }
    if action_count == 0 {
        return contract_error(format!(
            "{} contains no checked Rust setup consumers",
            path.display()
        ));
    }
    Ok(action_count)
}

fn validate_cache_keys(source: &str) -> Result<()> {
    for (name, expected) in [("cargo-audit", 2), ("cargo-vet", 2)] {
        let keys = source
            .lines()
            .filter(|line| line.trim_start().starts_with(&format!("key: {name}-")))
            .collect::<Vec<_>>();
        if keys.len() != expected
            || keys
                .iter()
                .any(|line| !line.contains("hashFiles('rust/rust-toolchain.toml')"))
        {
            return contract_error(format!(
                "{name} cache restore/save keys must bind to rust/rust-toolchain.toml"
            ));
        }
    }
    Ok(())
}

fn validate_active_consumers(root: &Path) -> Result<()> {
    for relative in ACTIVE_PIN_CONSUMERS {
        let source = fs::read_to_string(root.join(relative))?;
        validate_active_consumer(relative, &source)?;
    }
    let resolver = fs::read_to_string(root.join(RESOLVER))?;
    for marker in [
        "args[0] === 'read'",
        "['rust', 'bun'].includes(args[1])",
        "args[0] === 'verify' && args[1] === 'rust'",
        "args[0] === 'exec' && args[1] === '--'",
        "RUSTUP_TOOLCHAIN",
        "key.toUpperCase()",
        "['RUSTC', 'CARGO_BUILD_RUSTC']",
        "CARGO_BUILD_RUSTC",
        "Explicit cargo +toolchain overrides are not supported",
        "Explicit cargo --config build.rustc overrides are not supported",
        "Explicit rustup run/override commands are not supported",
    ] {
        if !resolver.contains(marker) {
            return contract_error(format!(
                "toolchain resolver is missing fail-closed behavior: {marker}"
            ));
        }
    }
    let ci = fs::read_to_string(root.join(WORKFLOWS[0]))?;
    if ci
        .matches("node --test scripts/toolchain-pin.test.mjs")
        .count()
        != 1
    {
        return contract_error("CI static gate must run the shared toolchain resolver tests once");
    }
    validate_cache_keys(&ci)
}

fn is_yaml(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yml" | "yaml")
    )
}

fn workflow_has_rust_consumer(source: &str) -> bool {
    is_rust_consumer(source)
        || source.to_ascii_lowercase().contains("setup-rust")
        || source.contains("RUSTUP_TOOLCHAIN")
}

fn validate_unrecognized_workflow(path: &Path, source: &str) -> Result<()> {
    let relative = path.to_string_lossy().replace('\\', "/");
    if !WORKFLOWS.contains(&relative.as_str()) && workflow_has_rust_consumer(source) {
        return contract_error(format!(
            "unrecognized Rust toolchain consumer workflow: {relative}"
        ));
    }
    Ok(())
}

fn validate_active_consumer(relative: &str, source: &str) -> Result<()> {
    for line in source.lines() {
        if rust_version_assertion(line) {
            return contract_error(format!(
                "{relative} contains a hardcoded Rust version assertion: {}",
                line.trim()
            ));
        }
    }
    Ok(())
}

fn validate_repository(root: &Path) -> Result<usize> {
    validate_root_overrides(root)?;
    let version = canonical_rust_version(&fs::read_to_string(root.join(TOOLCHAIN_FILE))?)?;
    validate_action_source(&fs::read_to_string(root.join(SETUP_ACTION))?)?;

    let mut consumers = 0;
    for relative in WORKFLOWS {
        consumers += validate_workflow_source(
            &fs::read_to_string(root.join(relative))?,
            Path::new(relative),
        )?;
    }

    for entry in WalkDir::new(root.join(".github/workflows")).follow_links(false) {
        let entry = entry?;
        if !is_yaml(entry.path()) {
            continue;
        }
        if entry.file_type().is_symlink() {
            return contract_error(format!(
                "Rust toolchain audit does not follow workflow symlinks: {}",
                entry.path().display()
            ));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| "workflow path escaped repository root")?;
        let source = fs::read_to_string(entry.path())?;
        validate_unrecognized_workflow(relative, &source)?;
    }
    validate_active_consumers(root)?;
    println!(
        "[xtask] Rust toolchain pin contract: PASS (Rust {version}; {consumers} workflow consumers verified)"
    );
    Ok(consumers)
}

pub(crate) fn rust_toolchain_contract(root: &Path) -> Result<()> {
    validate_repository(root)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ci_artifacts;
    use std::fs;

    const FIXTURE_PIN: &str = "[toolchain]\nchannel = \"7.8.9\"\nprofile = \"minimal\"\ntargets = [\"x86_64-pc-windows-msvc\"]\ncomponents = [\"clippy\"]\n";
    const VALID_ACTION: &str = "inputs:\n  targets:\n  components:\nruns:\n  using: composite\n  steps:\n    - shell: pwsh\n      run: |\n        node scripts/toolchain-pin.mjs read rust\n        $readExit = $LASTEXITCODE\n        if ($readExit -ne 0) { exit $readExit }\n        $rustupInstallArgs = @('toolchain', 'install', $version, '--profile', 'minimal')\n        $installExit = $LASTEXITCODE\n        if ($installExit -ne 0) { exit $installExit }\n        RUSTUP_TOOLCHAIN=$version >> $env:GITHUB_ENV\n    - shell: pwsh\n      run: node scripts/toolchain-pin.mjs verify rust\n        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }\n";
    const VALID_WORKFLOW: &str = "jobs:\n  check:\n    steps:\n      - name: Checkout\n        uses: actions/checkout@0123456789abcdef0123456789abcdef01234567\n        with:\n          ref: ${{ github.sha }}\n      - name: Rust setup\n        uses: ./.github/actions/setup-rust-pinned\n        with:\n          targets: x86_64-pc-windows-msvc\n      - name: Run checks\n        run: cargo xtask check static\n";

    fn assert_rejected<T>(result: Result<T>, label: &str) {
        assert!(result.is_err(), "{label}");
    }

    #[test]
    fn accepts_exact_nested_rust_pin_and_ordered_shared_setup() {
        assert_eq!(canonical_rust_version(FIXTURE_PIN).unwrap(), "7.8.9");
        validate_action_source(VALID_ACTION).unwrap();
        assert_eq!(
            validate_workflow_source(VALID_WORKFLOW, Path::new("fixture.yml")).unwrap(),
            1
        );
    }

    #[test]
    fn rejects_missing_floating_duplicate_and_ambiguous_pins() {
        for source in [
            "",
            "[toolchain]\nchannel = \"stable\"\nprofile = \"minimal\"\ntargets = [\"x86_64-pc-windows-msvc\"]\ncomponents = [\"clippy\"]\n",
            "[toolchain]\nchannel = \"7.8.9\"\nchannel = \"7.8.10\"\nprofile = \"minimal\"\ntargets = [\"x86_64-pc-windows-msvc\"]\ncomponents = [\"clippy\"]\n",
            "[toolchain]\nchannel = \"7.8.9\"\nprofile = \"minimal\"\ntargets = [\"x86_64-pc-windows-msvc\"]\ncomponents = [\"clippy\"]\n\n[toolchain]\nchannel = \"7.8.9\"\n",
            "[toolchain]\nchannel = \"7.8.9-beta.1\"\nprofile = \"minimal\"\ntargets = [\"x86_64-pc-windows-msvc\"]\ncomponents = [\"clippy\"]\n",
        ] {
            assert_rejected(canonical_rust_version(source), source);
        }
    }

    #[test]
    fn rejects_missing_or_ambiguous_setup_and_stale_selection() {
        let missing = VALID_WORKFLOW.replace(LOCAL_SETUP, "uses: actions/checkout@deadbeef");
        assert_rejected(
            validate_workflow_source(&missing, Path::new("missing.yml")),
            "missing setup",
        );
        let duplicate = VALID_WORKFLOW.replace(
            "      - name: Run checks",
            "      - name: Second Rust setup\n        uses: ./.github/actions/setup-rust-pinned\n      - name: Run checks",
        );
        assert_rejected(
            validate_workflow_source(&duplicate, Path::new("duplicate.yml")),
            "duplicate setup",
        );
        let conditional = VALID_WORKFLOW.replace(
            "      - name: Rust setup\n",
            "      - name: Rust setup\n        if: false\n",
        );
        assert_rejected(
            validate_workflow_source(&conditional, Path::new("conditional.yml")),
            "conditional setup",
        );
        let ignored = VALID_WORKFLOW.replace(
            "      - name: Rust setup\n",
            "      - name: Rust setup\n        continue-on-error: true\n",
        );
        assert_rejected(
            validate_workflow_source(&ignored, Path::new("ignored.yml")),
            "ignored setup failure",
        );
        let unsupported_input = VALID_WORKFLOW.replace(
            "          targets: x86_64-pc-windows-msvc",
            "          targets: x86_64-pc-windows-msvc\n          version: 7.8.9",
        );
        assert_rejected(
            validate_workflow_source(&unsupported_input, Path::new("override.yml")),
            "unsupported version override",
        );
        let wrong_source = VALID_WORKFLOW.replace("${{ github.sha }}", "refs/heads/main");
        assert_rejected(
            validate_workflow_source(&wrong_source, Path::new("wrong-source.yml")),
            "non-exact checkout",
        );
        for stale in [
            VALID_WORKFLOW.replace(
                "run: cargo xtask",
                "env:\n          RUSTUP_TOOLCHAIN: stable\n        run: cargo xtask",
            ),
            VALID_WORKFLOW.replace("run: cargo xtask", "run: cargo +stable xtask"),
            VALID_WORKFLOW.replace(
                "run: cargo xtask",
                "run: echo Expected Rust 7.8.9; cargo xtask",
            ),
        ] {
            assert_rejected(
                validate_workflow_source(&stale, Path::new("stale.yml")),
                "stale selection",
            );
        }
    }

    #[test]
    fn compiler_version_scan_avoids_unrelated_tool_and_document_versions() {
        assert!(rust_version_assertion(
            "Expected stable Rust 7.8.9 from the source pin"
        ));
        assert!(rust_version_assertion("CARGO_VERSION: 7.8.9"));
        assert!(!rust_version_assertion("CARGO_AUDIT_VERSION: 0.22.2"));
        assert!(!rust_version_assertion(
            "Tauri signer then signs bytes; Rust verifier checks the CLI version 2.12.3."
        ));
        assert_rejected(
            validate_active_consumer(
                "desktop/scripts/profile-builtin-library.mjs",
                "const rustcVersion = '7.8.8'; // packaged artifact provenance",
            ),
            "stale packaged build compiler provenance",
        );
    }

    #[test]
    fn rejects_unrecognized_rust_setup_workflow_consumers() {
        let path = Path::new(".github/workflows/future-rust.yml");
        for source in [
            "jobs:\n  build:\n    steps:\n      - uses: actions-rust-lang/setup-rust@v1\n",
            "jobs:\n  build:\n    steps:\n      - run: cargo test\n",
        ] {
            assert_rejected(
                validate_unrecognized_workflow(path, source),
                "unrecognized Rust consumer workflow",
            );
        }
        validate_unrecognized_workflow(
            Path::new(".github/workflows/non-rust.yml"),
            "jobs:\n  build:\n    steps:\n      - run: npm run build\n",
        )
        .unwrap();
    }

    #[test]
    fn rejects_rust_setup_split_across_job_boundaries() {
        let split = "jobs:\n  cargo_job:\n    steps:\n      - name: Checkout\n        uses: actions/checkout@0123456789abcdef0123456789abcdef01234567\n        with:\n          ref: ${{ github.sha }}\n      - name: Cargo\n        run: cargo test\n  setup_job:\n    steps:\n      - name: Checkout\n        uses: actions/checkout@0123456789abcdef0123456789abcdef01234567\n        with:\n          ref: ${{ github.sha }}\n      - name: Rust setup\n        uses: ./.github/actions/setup-rust-pinned\n      - name: Rust verifier\n        run: rustc --version\n";
        assert!(validate_workflow_source(split, Path::new("split.yml")).is_err());
    }

    #[test]
    fn root_toolchain_and_compiler_overrides_are_rejected() {
        let root = ci_artifacts::unique_sibling(&std::env::temp_dir().join("sky-rust-pin"), "test");
        fs::create_dir_all(root.join(".cargo")).unwrap();
        fs::write(root.join("rust-toolchain.toml"), FIXTURE_PIN).unwrap();
        assert_rejected(validate_root_overrides(&root), "root toolchain file");
        fs::remove_file(root.join("rust-toolchain.toml")).unwrap();
        fs::create_dir_all(root.join("rust")).unwrap();
        fs::write(root.join(TOOLCHAIN_FILE), FIXTURE_PIN).unwrap();
        validate_root_overrides(&root).unwrap();
        fs::write(root.join("rust/rust-toolchain"), "7.8.10\n").unwrap();
        assert!(validate_root_overrides(&root).is_err());
        fs::remove_dir_all(root.join("rust")).unwrap();
        fs::write(
            root.join(".cargo/config.toml"),
            "[env]\nRUSTUP_TOOLCHAIN=\"stable\"",
        )
        .unwrap();
        assert_rejected(validate_root_overrides(&root), "cargo compiler override");
        fs::remove_file(root.join(".cargo/config.toml")).unwrap();
        fs::write(root.join(".cargo/config"), "[build]\nrustc=\"rustc.exe\"").unwrap();
        assert!(validate_root_overrides(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn setup_action_cannot_bypass_pin_install_or_verification() {
        for (label, invalid) in [
            (
                "missing source read",
                VALID_ACTION.replace("node scripts/toolchain-pin.mjs read rust", "echo 7.8.9"),
            ),
            (
                "missing environment export",
                VALID_ACTION.replace(
                    "RUSTUP_TOOLCHAIN=$version >> $env:GITHUB_ENV",
                    "echo skipped",
                ),
            ),
            (
                "missing runtime verification",
                VALID_ACTION.replace(
                    "node scripts/toolchain-pin.mjs verify rust",
                    "Write-Host ready",
                ),
            ),
            (
                "unsupported input",
                VALID_ACTION.replace("  components:\n", "  components:\n  version: 7.8.9\n"),
            ),
        ] {
            assert_rejected(validate_action_source(&invalid), label);
        }
    }
}
