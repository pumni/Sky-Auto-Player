use std::fs;
use std::path::Path;

fn source(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
fn linux_process_runtime_contract(
    process: &str,
    unix_runtime: &str,
    smoke: &str,
    ci: &str,
    classifier: &str,
    main: &str,
) -> std::result::Result<(), String> {
    let process = source(process);
    let unix_runtime = source(unix_runtime);
    let smoke = source(smoke);
    let ci = source(ci);
    let classifier = source(classifier);
    let main = source(main);

    for marker in [
        "#[cfg(unix)]\nmod unix;",
        "unix::capture_timeout(program, args, cwd, env, remove_env, false, timeout)",
        "unix::capture_timeout(program, args, cwd, child_env, &[], true, timeout)",
        "unix::capture_timeout_to_file(",
        "bounded process capture is unavailable on this platform",
        "command.env_clear();",
        "RUNTIME_CHILD_ENV_ALLOWLIST",
    ] {
        if !process.contains(marker) {
            return Err(format!(
                "Rust process adapter is missing Linux production dispatch `{marker}`"
            ));
        }
    }
    if process.contains("bounded Windows process-tree execution is unavailable on this platform") {
        return Err(
            "Linux release subprocess adapter still has the unsupported-platform stub".into(),
        );
    }

    for marker in [
        ".process_group(0)",
        "kill_process_group(self.process_group, Signal::KILL)",
        "spawn_bounded_reader(",
        "collect_bounded_reader(",
        "READER_DRAIN_TIMEOUT",
        "PROCESS_TEARDOWN_TIMEOUT",
        ".create_new(true)",
        "remove_partial_file(stdout_path)",
        "diagnostic_secrets(env)",
        "redact_diagnostics(",
    ] {
        if !unix_runtime.contains(marker) {
            return Err(format!(
                "Unix process adapter is missing bounded cleanup contract `{marker}`"
            ));
        }
    }

    for marker in [
        "fn run_parent()",
        "let child_environment = [",
        "(\"PATH\".to_owned(), fake_bin.to_string_lossy().into_owned())",
        "process::capture_owned_timeout_runtime_only(",
        "write_executable(&fake_bin.join(\"gh\"), GH_SCRIPT)",
        "write_executable(&fake_bin.join(\"curl\"), CURL_SCRIPT)",
        "GhTransport::for_metadata_recovery(",
        "GitHubArtifactVerifier::new(",
        "capture_owned_timeout_runtime_only(",
        "capture_owned_timeout_to_file_runtime_only(",
        "require_descendant_stopped(",
        "channels/stable/latest.json",
    ] {
        if !smoke.contains(marker) {
            return Err(format!(
                "Linux release-mode fake subprocess smoke is missing `{marker}`"
            ));
        }
    }
    for secret in [
        "V4_TAURI_UPDATER_PRIVATE_KEY",
        "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
    ] {
        if smoke.contains(secret) {
            return Err(format!(
                "Linux fake subprocess smoke must not reference production signing secret `{secret}`"
            ));
        }
    }

    let job = ci
        .split_once("  linux_process_runtime:")
        .and_then(|(_, body)| body.split_once("\n  supply_chain:"))
        .map(|(job, _)| job)
        .ok_or("CI is missing the hosted Linux production subprocess job")?;
    for marker in [
        "runs-on: ubuntu-latest",
        "contents: read",
        "cargo test --locked --manifest-path rust/xtask/Cargo.toml",
        "cargo build --locked --manifest-path rust/xtask/Cargo.toml --release",
        "rust/target/release/xtask ci linux-process-smoke",
    ] {
        if !job.contains(marker) {
            return Err(format!(
                "hosted Linux production subprocess job is missing `{marker}`"
            ));
        }
    }
    for secret in [
        "V4_TAURI_UPDATER_PRIVATE_KEY",
        "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
    ] {
        if job.contains(secret) {
            return Err(format!(
                "hosted Linux subprocess job must not reference production signing secret `{secret}`"
            ));
        }
    }

    for path in [
        "rust/xtask/src/process/unix.rs",
        "rust/xtask/src/release_v4/recovery/process_smoke.rs",
    ] {
        if !classifier.contains(path) {
            return Err(format!(
                "CI classifier must route Linux subprocess source `{path}` through required release validation"
            ));
        }
    }
    if !main.contains("Some(\"linux-process-smoke\")")
        || !main.contains("release_v4::linux_process_smoke(&args[2..])")
        || !main.contains("cargo xtask ci linux-process-smoke")
    {
        return Err("xtask CLI does not expose the release-mode Linux subprocess smoke".into());
    }
    Ok(())
}

pub(super) fn validate(root: &Path) -> crate::Result<()> {
    let read = |path: &str| -> crate::Result<String> { Ok(fs::read_to_string(root.join(path))?) };
    linux_process_runtime_contract(
        &read("rust/xtask/src/process.rs")?,
        &read("rust/xtask/src/process/unix.rs")?,
        &read("rust/xtask/src/release_v4/recovery/process_smoke.rs")?,
        &read(".github/workflows/ci.yml")?,
        &read("rust/xtask/src/ci_classifier.rs")?,
        &read("rust/xtask/src/main.rs")?,
    )
    .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> { error.into() })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn replace_fixture(text: &str, expected: &str, replacement: &str) -> String {
        let original = source(text);
        assert!(
            original.contains(expected),
            "mutation target must exist: {expected}"
        );
        let mutated = original.replace(expected, replacement);
        assert_ne!(mutated, original, "mutation must change fixture");
        mutated
    }
    #[test]
    fn linux_production_subprocess_contract_rejects_missing_runtime_and_smoke() {
        let root = crate::repo::root();
        let read = |path: &str| fs::read_to_string(root.join(path)).expect("fixture source");
        let process = read("rust/xtask/src/process.rs");
        let unix_runtime = read("rust/xtask/src/process/unix.rs");
        let smoke = read("rust/xtask/src/release_v4/recovery/process_smoke.rs");
        let ci = read(".github/workflows/ci.yml");
        let classifier = read("rust/xtask/src/ci_classifier.rs");
        let main = read("rust/xtask/src/main.rs");

        linux_process_runtime_contract(&process, &unix_runtime, &smoke, &ci, &classifier, &main)
            .expect("Linux production subprocess contract must be complete");

        let unsupported = replace_fixture(
            &process,
            "unix::capture_timeout(program, args, cwd, env, remove_env, false, timeout)",
            "Err(\"bounded Windows process-tree execution is unavailable on this platform\".into())",
        );
        assert!(
            linux_process_runtime_contract(
                &unsupported,
                &unix_runtime,
                &smoke,
                &ci,
                &classifier,
                &main,
            )
            .is_err()
        );

        let unbounded = replace_fixture(
            &unix_runtime,
            "kill_process_group(self.process_group, Signal::KILL)",
            "kill_process_group(Pid::from_raw(1).expect(\"init PID\"), Signal::KILL)",
        );
        assert!(
            linux_process_runtime_contract(&process, &unbounded, &smoke, &ci, &classifier, &main,)
                .is_err()
        );

        let missing_smoke = replace_fixture(
            &ci,
            "rust/target/release/xtask ci linux-process-smoke",
            "rust/target/release/xtask --version",
        );
        assert!(
            linux_process_runtime_contract(
                &process,
                &unix_runtime,
                &smoke,
                &missing_smoke,
                &classifier,
                &main,
            )
            .is_err()
        );
    }
}
