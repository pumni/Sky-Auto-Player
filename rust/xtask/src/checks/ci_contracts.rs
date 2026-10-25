use super::workflow::workflow_declares_only_dispatch;
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn packaged_ci_contract_source(source: &str) -> Result<()> {
    if source.replace("\r\n", "\n").contains("\n  candidate:\n") {
        return packaged_ci_build_once_contract_source(source);
    }
    Err("CI workflow must use the Rust candidate producer and build-once topology".into())
}

fn packaged_sccache_precedes_cargo_contract(packaged: &str) -> Result<()> {
    let setup_count = packaged.matches("name: Set up sccache").count();
    let setup_position = packaged
        .find("name: Set up sccache")
        .ok_or("packaged consumer must install sccache before invoking Cargo")?;
    let dependency_check_position = packaged
        .find("name: Fail fast on package qualification dependencies")
        .ok_or("packaged consumer must verify sccache before invoking Cargo")?;
    let first_cargo_position = packaged
        .find("cargo xtask ci candidate validate")
        .ok_or("packaged consumer must validate the exact candidate with Cargo")?;
    if setup_count != 1
        || setup_position >= dependency_check_position
        || dependency_check_position >= first_cargo_position
        || !packaged.contains("version: \"v0.17.0\"")
    {
        return Err(
            "packaged consumer must install and verify pinned sccache before its first Cargo command".into(),
        );
    }
    if packaged[..setup_position].contains("cargo ") {
        return Err("packaged consumer invoked Cargo before setting up sccache".into());
    }
    Ok(())
}

fn isolated_xtask_rust_runner_step_contract(step: &str) -> Result<()> {
    let copy_position = step
        .find("Copy-Item -LiteralPath \"rust/target/debug/xtask.exe\"")
        .ok_or("Rust verification must copy the built xtask executable to an isolated path")?;
    let run_position = step
        .find("& $isolatedXtask check rust")
        .ok_or("Rust verification must invoke the isolated xtask executable")?;
    let cleanup_position = step
        .find("Remove-Item -LiteralPath $isolatedXtask")
        .ok_or("Rust verification must remove its isolated xtask executable")?;
    if copy_position >= run_position
        || run_position >= cleanup_position
        || !step.contains("finally {")
        || step.contains("cargo xtask check rust")
    {
        return Err(
            "Rust verification must run and clean up a copied xtask without holding Cargo's target executable".into(),
        );
    }
    Ok(())
}

fn packaged_ci_build_once_contract_source(source: &str) -> Result<()> {
    let normalized = source.replace("\r\n", "\n");
    let ordinary_ci = normalized
        .split("  deploy_pages:")
        .next()
        .unwrap_or(normalized.as_str());
    if ordinary_ci.matches("actions/attest@").count() != 0
        || ordinary_ci.contains("id-token: write")
        || ordinary_ci.contains("attestations: write")
        || ordinary_ci.contains("artifact-metadata: write")
    {
        return Err("ordinary CI must not create or verify GitHub attestations or request attestation permissions".into());
    }

    let candidate_start = normalized
        .find("  candidate:\n")
        .ok_or("CI workflow is missing the current-candidate producer job")?;
    let candidate_end = normalized[candidate_start..]
        .find("\n  updater_e2e:\n")
        .map(|offset| candidate_start + offset)
        .ok_or("current-candidate producer must precede the updater consumer")?;
    let candidate = &normalized[candidate_start..candidate_end];
    for marker in [
        "name: Build current Tauri candidate",
        "needs: changes",
        "if: needs.changes.outputs.package_required == 'true' || needs.changes.outputs.updater_required == 'true'",
        "bun run tauri build --ci --config",
        "dev:build-info",
        "--profile dist",
        "cargo xtask ci candidate create",
        "--bundle-dir",
        "--public-key",
        "--calibration-binary",
        "--output-root",
        "--source-sha",
        "actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9",
        "path: ${{ runner.temp }}/sky-auto-player-current-candidate",
    ] {
        if !candidate.contains(marker) {
            return Err(format!(
                "current-candidate producer is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let cleanup_position = candidate
        .find("Remove-Item -LiteralPath $keyPath, \"$keyPath.pub\", $configPath")
        .ok_or("current-candidate producer must delete its private key and build config")?;
    let upload_position = candidate
        .find("actions/upload-artifact@")
        .ok_or("current-candidate producer must upload one workflow artifact")?;
    if cleanup_position >= upload_position {
        return Err("current-candidate private key cleanup must precede artifact upload".into());
    }

    let updater_start = candidate_end;
    let updater_end = normalized[updater_start..]
        .find("\n  packaged:\n")
        .map(|offset| updater_start + offset)
        .ok_or("CI workflow updater consumer must precede the packaged consumer")?;
    let updater = &normalized[updater_start..updater_end];
    for marker in [
        "name: Updater fixture qualification",
        "needs: [changes, static, candidate, updater_bridge]",
        "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333",
        "cargo xtask ci candidate validate",
        "cargo xtask ci bridge validate",
        "--candidate-root",
        "--bridge-root",
        "Download updater bridge from this workflow run",
        "cargo xtask ci updater-fixture run",
        "--allow-hosted-disposable-runner",
    ] {
        if !updater.contains(marker) {
            return Err(
                format!("updater consumer is missing its required marker: {marker}").into(),
            );
        }
    }
    if updater.contains("bun run tauri build") {
        return Err("updater consumer must not build a current candidate".into());
    }

    let package_start = updater_end;
    let package_end = normalized[package_start..]
        .find("\n  site:\n")
        .map(|offset| package_start + offset)
        .ok_or("CI workflow packaged consumer must precede the site job")?;
    let packaged = &normalized[package_start..package_end];
    for marker in [
        "name: Packaged v4 Tauri NSIS qualification",
        "needs: [changes, static, candidate]",
        "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333",
        "cargo xtask ci candidate validate",
        "cargo xtask verify-tauri-bundle",
        "cargo xtask sbom generate",
        "cargo xtask sbom verify",
        "--mode unsigned-zero-budget",
        "current-user install, launch, and uninstall",
        "cargo xtask ci nsis smoke",
        "--allow-hosted-disposable-runner",
        "--registry-mode fresh-install",
        "installer_sha256",
        "updater_signature_sha256",
        "updater_public_key_sha256",
        "INSTALLED_CALIBRATION_EVIDENCE.json",
        "actions/upload-artifact@",
    ] {
        if !packaged.contains(marker) {
            return Err(
                format!("packaged consumer is missing its required marker: {marker}").into(),
            );
        }
    }
    for forbidden in [
        "bun run tauri build",
        "TAURI_SIGNING_PRIVATE_KEY",
        "scripts/test_v4_authenticode_integrity.ps1",
        "scripts/test_v4_production_signing_contract.ps1",
        "actions/attest@",
        "id-token: write",
        "attestations: write",
        "name: Resolve GitHub CLI for artifact attestation verification",
        "tauri-update-fixture",
        "dangerousInsecureTransportProtocol",
        "127.0.0.1:17845",
        "CARGO_TARGET_DIR",
        "--features",
        "cargo xtask dist",
        "verify-dist",
        "Sky-Auto-Player-v",
        "Sky-Auto-Player-Updater.exe",
        "MANIFEST.json",
        "PORTABLE_ARTIFACT",
        "portable",
    ] {
        if packaged.contains(forbidden) {
            return Err(format!(
                "packaged consumer contains forbidden producer/attestation work: {forbidden}"
            )
            .into());
        }
    }

    let validate_position = packaged
        .find("name: Validate exact current candidate contract")
        .ok_or("packaged consumer must validate candidate.json before qualification")?;
    packaged_sccache_precedes_cargo_contract(packaged)?;
    let cache_position = packaged
        .find("name: Rust cache")
        .ok_or("packaged consumer must restore Rust cache before staging")?;
    let stage_position = packaged
        .find("name: Stage and re-hash exact current candidate after Rust cache restore")
        .ok_or("packaged consumer must stage the candidate after Rust cache restore")?;
    if validate_position >= cache_position || stage_position <= cache_position {
        return Err(
            "packaged candidate validation/staging order does not preserve the restored target tree"
                .into(),
        );
    }
    for marker in [
        "Get-FileHash -LiteralPath $stagedInstaller -Algorithm SHA256",
        "Get-FileHash -LiteralPath $stagedSignature -Algorithm SHA256",
        "Get-FileHash -LiteralPath $env:SKY_CANDIDATE_PUBLIC_KEY -Algorithm SHA256",
        "Staged candidate hashes do not match the validated candidate contract",
    ] {
        if !packaged.contains(marker) {
            return Err(format!(
                "packaged consumer is missing post-cache staging hash verification: {marker}"
            )
            .into());
        }
    }

    let nsis_harness = include_str!("../nsis_harness.rs");
    for marker in [
        "verify_calibration_artifact(",
        "INSTALLED_CALIBRATION_EVIDENCE.json",
        "INSTALLED_AUTHENTICODE_EVIDENCE.json",
        "candidate.manifest.calibration_binary_sha256",
        "installed Tauri tree contains no project PE files",
        "--selftest-update-active-playback",
        "assert_uninstall_registration_absent()",
    ] {
        if !nsis_harness.contains(marker) {
            return Err(format!(
                "Rust packaged NSIS owner is missing its required qualification marker: {marker}"
            )
            .into());
        }
    }
    let install_position = packaged
        .find("cargo xtask ci nsis smoke")
        .ok_or("packaged consumer must invoke the Rust-owned NSIS qualification")?;
    let evidence_position = packaged
        .find("name: Emit exact Tauri qualification evidence")
        .ok_or("packaged consumer must emit qualification evidence after NSIS smoke")?;
    if install_position >= evidence_position {
        return Err("Rust NSIS qualification must precede packaging evidence emission".into());
    }

    for marker in [
        "Build bounded unsigned Authenticode PE fixture",
        "rustc --edition 2021 --target x86_64-pc-windows-msvc",
        "Get-AuthenticodeSignature",
        "SKY_AUTHENTICODE_FIXTURE",
        "Run Authenticode tamper regression on controlled unsigned PE fixture",
        "scripts/test_v4_authenticode_integrity.ps1",
        "scripts/test_v4_production_signing_contract.ps1",
        "scripts/setup_v4_test_signing.ps1",
        "scripts/cleanup_v4_test_signing.ps1",
        "needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]",
        "CANDIDATE_REQUIRED",
        "CANDIDATE_RESULT",
        "name: Sky Auto Player — required CI gate",
    ] {
        if !normalized.contains(marker) {
            return Err(format!(
                "CI build-once control-plane contract is missing its marker: {marker}"
            )
            .into());
        }
    }
    for forbidden in ["SystemRoot", "notepad.exe"] {
        if normalized.contains(forbidden) {
            return Err(format!(
                "release contract Authenticode fixture must not depend on a system PE: {forbidden}"
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn packaged_ci_contract(root: &Path) -> Result<()> {
    let path = root.join(".github/workflows/ci.yml");
    packaged_ci_contract_source(&fs::read_to_string(&path)?)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let candidate = fs::read_to_string(root.join("rust/xtask/src/ci_candidate.rs"))?;
    for marker in [
        "schema_version",
        "source_sha",
        "calibration_binary_size",
        "calibration_binary_sha256",
        "calibration_binary_architecture",
        "installer_sha256",
        "updater_signature_sha256",
        "updater_public_key_sha256",
        "deny_unknown_fields",
        "direct_files",
        "sha256_file",
        "validate_public_key",
        "validate_safe_filename",
        "candidate artifact must contain exactly four files",
        "create_and_validate_preserve_candidate_v2_schema_and_exact_inventory",
        "rejects_tampered_installer_signature_public_key_and_source_identity",
        "rejects_wrong_machine_malformed_pe_traversal_and_unexpected_inventory",
        "rejects_unknown_candidate_manifest_fields",
        "public_key_and_safe_filename_rules_reject_private_material_and_paths",
    ] {
        if !candidate.contains(marker) {
            return Err(format!(
                "Rust current-candidate contract is missing its fail-closed marker: {marker}"
            )
            .into());
        }
    }
    let bridge = fs::read_to_string(root.join("rust/xtask/src/ci_bridge.rs"))?;
    for marker in [
        "schema_version",
        "source_sha",
        "installer_sha256",
        "sentinel_id",
        "sentinel_content_sha256",
        "deny_unknown_fields",
        "direct_files",
        "PRIVATE KEY",
        "bridge artifact must contain exactly two files",
        "create_and_validate_preserve_bridge_v1_schema_and_exact_files",
        "rejects_tampered_installer_wrong_source_and_expected_binding_mismatch",
        "rejects_extra_private_material_signature_paths_and_unknown_json_fields",
        "operation_options_reject_unknown_names_and_duplicates",
    ] {
        if !bridge.contains(marker) {
            return Err(format!(
                "Rust updater-bridge contract is missing its fail-closed marker: {marker}"
            )
            .into());
        }
    }
    let artifacts = fs::read_to_string(root.join("rust/xtask/src/ci_artifacts.rs"))?;
    for marker in [
        "validate_commit_sha",
        "validate_digest",
        "validate_safe_filename",
        "is_reparse_point",
        "direct_files",
        "pe_machine",
        "replace_file_with_rollback",
        "replacement_restores_the_original_when_staging_rename_fails",
        "staging_directory_is_removed_after_an_error",
    ] {
        if !artifacts.contains(marker) {
            return Err(format!(
                "Rust artifact primitives are missing their fail-closed marker: {marker}"
            )
            .into());
        }
    }
    let calibration = fs::read_to_string(root.join("rust/xtask/src/calibration.rs"))?;
    for marker in [
        "x86_64-pc-windows-msvc",
        "--profile",
        "dist",
        "replace_file_with_rollback",
        "ProcessTree::attach",
        "owner.terminate()",
        "cleanup_timed_out_process",
        "wait_reader_until",
        "installed_calibration_timeout_kills_descendants_and_bounds_pipe_readers",
        "installed_calibration_early_exit_kills_descendant_holding_output_pipes",
        "installed calibration --help exceeded the 5-second smoke bound",
        "installed_verification_fails_closed_for_missing_binary_invalid_size_digest_and_pe",
        "successful_evidence_keeps_the_installed_calibration_schema",
        "installed_calibration_cli_rejects_unknown_and_duplicate_options",
    ] {
        if !calibration.contains(marker) {
            return Err(format!(
                "Rust native calibration integration is missing its fail-closed marker: {marker}"
            )
            .into());
        }
    }
    let classifier = fs::read_to_string(root.join("rust/xtask/src/ci_classifier.rs"))?;
    for marker in [
        "rust_required",
        "desktop_e2e_required",
        "classification_reason",
        "changed_paths_from_git",
        "classify_git_paths",
        "decode_git_paths",
        "git_paths_are_exact_and_malformed_names_fail_closed",
        "git_path_decoding_preserves_trailing_space_and_rejects_invalid_utf8",
        "--depth=1",
        "preserves_exact_lane_output_shape_and_reason_golden",
        "matches_positive_and_union_classifier_matrix",
        "unknown_or_malformed_paths_fail_closed_and_workflow_dispatch_is_full",
        "explicit_base_head_works_in_detached_shallow_checkout_and_fetches_missing_base",
    ] {
        if !classifier.contains(marker) {
            return Err(format!(
                "Rust CI classifier is missing its compatibility marker: {marker}"
            )
            .into());
        }
    }
    let process_tree =
        fs::read_to_string(root.join("rust/crates/sky_windows_security/src/process_tree.rs"))?;
    if !process_tree.contains("#[cfg(not(windows))]\n    pub fn terminate")
        || !process_tree.contains("Windows process-tree termination is unavailable")
    {
        return Err(
            "Windows process-tree boundary must expose a fail-closed cross-platform API".into(),
        );
    }
    for script in [
        "scripts/ci_classify.ps1",
        "scripts/test_ci_classify.ps1",
        "scripts/ci_validate_candidate.ps1",
        "scripts/test_ci_validate_candidate.ps1",
        "scripts/ci_validate_bridge.ps1",
        "scripts/test_ci_validate_bridge.ps1",
        "scripts/build_native_calibration.ps1",
        "scripts/verify_installed_calibration.ps1",
        "scripts/test_installed_calibration.ps1",
    ] {
        if root.join(script).exists() {
            return Err(
                format!("migrated PowerShell CI implementation remains present: {script}").into(),
            );
        }
    }
    println!("[xtask] canonical v4 packaged CI Tauri contract: PASS");
    Ok(())
}

const ACTIVE_CI_WORKFLOW_FILES: &[&str] = &[
    ".github/workflows/ci.yml",
    ".github/workflows/pages.yml",
    ".github/workflows/release-v4.yml",
    ".github/workflows/rehearse-v4.yml",
];

fn validate_active_github_hosted_ubuntu_labels(source: &str) -> Result<()> {
    for line in source.lines() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("runs-on:") || !trimmed.contains("ubuntu-") {
            continue;
        }
        if !trimmed.contains("ubuntu-latest") {
            return Err(format!(
                "active GitHub-hosted Linux job must use runs-on: ubuntu-latest: {trimmed}"
            )
            .into());
        }
    }
    Ok(())
}

pub(super) fn ci_control_plane_contract(root: &Path) -> Result<()> {
    let workflows_root = root.join(".github/workflows");
    let mut observed = BTreeSet::new();
    for entry in fs::read_dir(&workflows_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if matches!(
            entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) {
            observed.insert(relative);
        }
    }
    let expected = ACTIVE_CI_WORKFLOW_FILES
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    if observed != expected {
        return Err(format!(
            "active CI workflow set is not locked: observed={observed:?}, expected={expected:?}"
        )
        .into());
    }
    for relative in ACTIVE_CI_WORKFLOW_FILES {
        let source = fs::read_to_string(root.join(relative))?;
        validate_active_github_hosted_ubuntu_labels(&source)
            .map_err(|error| format!("{relative}: {error}"))?;
    }

    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))?.replace("\r\n", "\n");
    for output in [
        "rust_required",
        "desktop_required",
        "desktop_native_required",
        "desktop_e2e_required",
        "package_required",
        "updater_required",
        "release_required",
        "supply_chain_required",
        "site_required",
        "classification_reason",
    ] {
        let marker = format!("{output}: ${{{{ steps.classify.outputs.{output} }}}}");
        if !ci.contains(&marker) {
            return Err(format!("CI is missing classifier output projection: {output}").into());
        }
    }
    for marker in [
        "cargo xtask ci classify",
        "cargo test --locked --manifest-path rust/xtask/Cargo.toml",
        "github.event.pull_request.base.sha",
        "github.event.pull_request.head.sha",
        "github.event.before",
        "classifier_args+=(--full)",
        "classifier_args+=(--base-sha",
        "static_required:",
        "contains(steps.classify.outputs.classification_reason, 'static-only')",
        "desktop_e2e_required == 'true'",
        "name: Website validation",
        "if: needs.changes.outputs.site_required == 'true'",
        "name: Sky Auto Player — required CI gate",
        "needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]",
    ] {
        if !ci.contains(marker) {
            return Err(
                format!("CI control-plane contract is missing its marker: {marker}").into(),
            );
        }
    }
    for forbidden in [
        "cargo run --manifest-path rust/tools",
        "static_required: ${{ steps.classify.outputs.static_required }}",
    ] {
        if ci.contains(forbidden) {
            return Err(format!(
                "CI control-plane contract contains retired classifier wiring: {forbidden}"
            )
            .into());
        }
    }

    let bridge_start = ci
        .find("\n  updater_bridge:\n")
        .ok_or("CI is missing the updater_bridge producer job")?;
    let static_start = ci
        .find("\n  static:\n")
        .ok_or("CI is missing the static job boundary")?;
    let updater_contract_start = ci
        .find("\n  updater_contract:\n")
        .ok_or("CI is missing the updater_contract job")?;
    if bridge_start >= updater_contract_start || updater_contract_start >= static_start {
        return Err("updater_bridge and updater_contract must fan out before static".into());
    }
    let bridge = &ci[bridge_start..updater_contract_start];
    for marker in [
        "name: Build updater bridge fixture",
        "needs: changes",
        "if: needs.changes.outputs.updater_required == 'true'",
        "runs-on: windows-latest",
        "Build exactly one updater bridge fixture",
        "cargo xtask ci bridge build-fixture",
        "cargo xtask ci bridge validate",
        "bridge.json",
        "actions/upload-artifact@",
    ] {
        if !bridge.contains(marker) {
            return Err(format!("updater_bridge producer is missing its marker: {marker}").into());
        }
    }
    if bridge.matches("bun run tauri build").count() != 0 {
        return Err(
            "updater_bridge workflow must delegate its single build to the producer script".into(),
        );
    }
    let updater_fixture = fs::read_to_string(root.join("rust/xtask/src/updater_fixture.rs"))?;
    for marker in [
        "fn build_bridge_internal(",
        "git",
        "worktree",
        "--detach",
        "RUNNER_TEMP",
        "tauri-update-fixture",
        "run_bun(",
        "crate::ci_bridge::create_bridge",
        "crate::ci_bridge::read_bridge",
        "source SHA must match the checked-out Git HEAD",
    ] {
        if !updater_fixture.contains(marker) {
            return Err(format!(
                "Rust updater bridge owner is missing its required marker: {marker}"
            )
            .into());
        }
    }
    if updater_fixture.matches("bun run tauri build").count() != 0
        || !updater_fixture.contains("\"tauri\".to_owned(),\n            \"build\".to_owned()")
    {
        return Err("Rust updater bridge producer must own one isolated tauri-update-fixture build and typed contract validation".into());
    }
    let updater_contract = &ci[updater_contract_start..static_start];
    for marker in [
        "name: Updater key-rotation contract",
        "needs: changes",
        "if: needs.changes.outputs.updater_required == 'true'",
        "runs-on: windows-latest",
        "rustup toolchain install 1.99.1",
        "bun-version: 1.4.3",
        "bun install --frozen-lockfile",
        "scripts/test_v4_updater_key_rotation.ps1",
        "contents: read",
    ] {
        if !updater_contract.contains(marker) {
            return Err(
                format!("updater_contract is missing its required marker: {marker}").into(),
            );
        }
    }
    if updater_contract
        .lines()
        .filter(|line| line.contains("needs.changes.outputs."))
        .count()
        != 1
        || updater_contract.contains("actions/upload-artifact@")
        || updater_contract.contains("attestations:")
        || updater_contract.contains("id-token:")
    {
        return Err(
            "updater_contract must be keyed only by updater_required and must not upload or attest"
                .into(),
        );
    }
    if updater_contract
        .matches("scripts/test_v4_updater_key_rotation.ps1")
        .count()
        != 1
    {
        return Err("updater_contract must run key rotation exactly once".into());
    }
    let candidate_start = ci
        .find("\n  candidate:\n")
        .ok_or("CI is missing the candidate producer job")?;
    let updater_start = ci
        .find("\n  updater_e2e:\n")
        .ok_or("CI is missing the updater consumer job")?;
    if bridge_start >= candidate_start || candidate_start >= updater_start {
        return Err(
            "candidate and updater_bridge producers must precede the updater consumer".into(),
        );
    }
    let candidate = &ci[candidate_start..updater_start];
    if !candidate.contains("needs: changes")
        || candidate.matches("bun run tauri build").count() != 1
    {
        return Err("candidate must have one direct current-candidate tauri build".into());
    }
    let updater_end = ci
        .find("\n  packaged:\n")
        .ok_or("CI is missing the packaged job boundary")?;
    let updater = &ci[updater_start..updater_end];
    for marker in [
        "needs: [changes, static, candidate, updater_bridge]",
        "Download current candidate from this workflow run",
        "Validate and bind exact current candidate",
        "cargo xtask ci candidate validate",
        "Download updater bridge from this workflow run",
        "cargo xtask ci bridge validate",
        "cargo xtask ci updater-fixture run",
        "--candidate-root $env:SKY_CANDIDATE_ROOT",
        "--bridge-root $env:SKY_BRIDGE_ROOT",
        "--allow-hosted-disposable-runner",
    ] {
        if !updater.contains(marker) {
            return Err(format!(
                "updater consumer is missing its provided-bridge marker: {marker}"
            )
            .into());
        }
    }
    if updater.contains("bun run tauri build") || updater.contains("bun run build") {
        return Err(
            "provided updater consumer must not build a bridge, candidate, or frontend".into(),
        );
    }
    if updater.contains("scripts/test_v4_updater_key_rotation.ps1")
        || updater.contains("updater_contract")
    {
        return Err(
            "updater_e2e must retain runtime qualification without depending on updater_contract"
                .into(),
        );
    }
    let packaged_start = updater_end;
    let status_boundary = ci
        .find("\n  status:\n")
        .ok_or("CI is missing the required aggregate gate")?;
    let packaged = &ci[packaged_start..status_boundary];
    if packaged.contains("updater_contract") {
        return Err("packaged must not depend on updater_contract".into());
    }
    if candidate.contains("updater_contract") || bridge.contains("updater_contract") {
        return Err("candidate and updater_bridge must not depend on updater_contract".into());
    }
    let validate_start = ci
        .find("\n  validate:\n")
        .ok_or("CI is missing the validate job")?;
    let desktop_web_start = ci
        .find("\n  desktop_web:\n")
        .ok_or("CI is missing the desktop_web job")?;
    if validate_start >= desktop_web_start || desktop_web_start >= candidate_start {
        return Err("desktop_web must be a separate lane before candidate qualification".into());
    }
    let validate = &ci[validate_start..desktop_web_start];
    if !validate.contains("cargo xtask check desktop-native")
        || validate.contains("oven-sh/setup-bun")
        || validate.contains("bun install")
        || validate.contains("playwright")
        || validate.contains("Chromium")
        || validate.contains("test:e2e")
    {
        return Err("Windows validation must contain native-only desktop ownership".into());
    }
    if validate
        .matches("name: Fail fast on Windows validation dependencies")
        .count()
        != 1
        || validate
            .matches(
                "pwsh scripts/ci_require_windows_tools.ps1 -Tool cargo,rustc,rustup,git,sccache",
            )
            .count()
            != 1
        || validate.contains("Fail fast on Rust validation dependencies")
        || validate.contains("Fail fast on native desktop validation dependencies")
    {
        return Err(
            "Windows validation must consolidate its shared dependency check exactly once".into(),
        );
    }
    let rust_check_position = validate
        .find("name: Repository verification — Rust")
        .ok_or("Windows validation is missing its Rust repository verification step")?;
    let rust_check_step_end = validate[rust_check_position..]
        .find("\n      - name:")
        .map_or(validate.len(), |offset| rust_check_position + offset);
    isolated_xtask_rust_runner_step_contract(&validate[rust_check_position..rust_check_step_end])?;
    let rust_cache_position = validate
        .find("name: Rust cache")
        .ok_or("Windows validation is missing its Rust cache step")?;
    let dependency_check_position = validate
        .find("name: Fail fast on Windows validation dependencies")
        .ok_or("Windows validation is missing its shared dependency check")?;
    let first_repository_check_position = validate
        .find("name: Repository verification — Rust")
        .ok_or("Windows validation is missing its first repository verification step")?;
    let calibration_resource_position = validate
        .find("name: Stage calibration resource for Tauri validation")
        .ok_or("Windows validation must stage the calibration resource before Tauri checks")?;
    if rust_cache_position >= dependency_check_position
        || dependency_check_position >= calibration_resource_position
        || calibration_resource_position >= first_repository_check_position
    {
        return Err(
            "Windows validation must cache dependencies, stage the calibration resource, then run repository checks".into(),
        );
    }
    if validate.matches("cargo xtask calibration build").count() != 1
        || !validate.contains("run: cargo xtask calibration build")
    {
        return Err(
            "Windows validation must stage the locked calibration resource exactly once".into(),
        );
    }
    if validate.contains(
        "        if: needs.changes.outputs.rust_required == 'true' || needs.changes.outputs.desktop_native_required == 'true'",
    ) {
        return Err(
            "Windows validation toolchain setup must not repeat the job-level predicate".into(),
        );
    }
    for marker in [
        "      - name: Repository verification — Rust\n        if: needs.changes.outputs.rust_required == 'true'",
        "      - name: Repository verification — native desktop\n        if: needs.changes.outputs.desktop_native_required == 'true'",
    ] {
        if !validate.contains(marker) {
            return Err(format!(
                "Windows validation lost its lane-specific repository predicate: {marker}"
            )
            .into());
        }
    }
    let desktop_web_end = ci
        .find("\n  site:\n")
        .ok_or("CI is missing the site job boundary")?;
    let desktop_web = &ci[desktop_web_start..desktop_web_end];
    for marker in [
        "name: Desktop web and browser validation",
        "needs: changes",
        "if: needs.changes.outputs.desktop_required == 'true'",
        "runs-on: ubuntu-latest",
        "bun-version: 1.4.3",
        "bun install --frozen-lockfile",
        "bun run check",
        "desktop_e2e_required == 'true'",
        "node_modules/playwright/cli.js --version",
        "node_modules/playwright/cli.js install chromium",
        "bun run test:e2e",
    ] {
        if !desktop_web.contains(marker) {
            return Err(format!("desktop_web job is missing its marker: {marker}").into());
        }
    }
    let status_start = ci
        .find("\n  status:\n")
        .ok_or("CI is missing the required aggregate gate")?;
    let deploy_start = ci
        .find("\n  deploy_pages:\n")
        .ok_or("CI is missing the deploy_pages job")?;
    if desktop_web_end >= status_start || status_start >= deploy_start {
        return Err("CI site validation, status, and Pages deployment must remain ordered".into());
    }
    let status = &ci[status_start..deploy_start];
    for marker in [
        "UPDATER_BRIDGE_REQUIRED",
        "UPDATER_BRIDGE_RESULT",
        "UPDATER_CONTRACT_REQUIRED",
        "UPDATER_CONTRACT_RESULT",
        "DESKTOP_WEB_REQUIRED",
        "DESKTOP_WEB_RESULT",
        "if [[ \"$UPDATER_BRIDGE_REQUIRED\" == \"true\" ]]",
        "if [[ \"$UPDATER_CONTRACT_REQUIRED\" == \"true\" ]]",
        "if [[ \"$DESKTOP_WEB_REQUIRED\" == \"true\" ]]",
        "Sky Auto Player — required CI gate",
    ] {
        if !status.contains(marker) {
            return Err(format!(
                "required aggregate gate is missing its fail-closed marker: {marker}"
            )
            .into());
        }
    }
    if status.contains("DESKTOP_E2E_REQUIRED") {
        return Err(
            "required aggregate gate must not project the unused DESKTOP_E2E_REQUIRED variable"
                .into(),
        );
    }
    if status.contains("deploy_pages") {
        return Err("required CI status gate must not depend on deploy_pages".into());
    }
    let site = &ci[desktop_web_end..status_start];
    for marker in [
        "name: Website validation",
        "uses: ./.github/actions/site-validate",
        "name: Upload validated Pages artifact",
        "actions/upload-pages-artifact@fc324d3547104276b827a68afc52ff2a11cc49c9",
        "path: site/dist",
    ] {
        if !site.contains(marker) {
            return Err(
                format!("CI site artifact contract is missing its marker: {marker}").into(),
            );
        }
    }
    for forbidden in [
        "pages: write",
        "id-token: write",
        "oven-sh/setup-bun",
        "bun install",
        "bun run build",
        "actions/download-artifact@",
    ] {
        if site.contains(forbidden) {
            return Err(
                format!("CI site validation job contains deploy-only work: {forbidden}").into(),
            );
        }
    }
    let deploy = &ci[deploy_start..];
    for marker in [
        "name: Deploy validated website",
        "needs: [changes, site, status]",
        "always() &&",
        "github.event_name == 'push'",
        "github.ref == 'refs/heads/main'",
        "needs.changes.outputs.site_required == 'true'",
        "needs.site.result == 'success'",
        "needs.status.result == 'success'",
        "contents: read",
        "pages: write",
        "id-token: write",
        "name: github-pages",
        "url: ${{ steps.deployment.outputs.page_url }}",
        "actions/configure-pages@45bfe0192ca1faeb007ade9deae92b16b8254a0d",
        "id: deployment",
        "actions/deploy-pages@368f82528645a54fb793d4d04e342629a3f51346",
    ] {
        if !deploy.contains(marker) {
            return Err(
                format!("CI Pages deployment contract is missing its marker: {marker}").into(),
            );
        }
    }
    for forbidden in [
        "actions/checkout@",
        "oven-sh/setup-bun",
        "bun install",
        "bun run build",
        "site-validate",
        "actions/download-artifact@",
    ] {
        if deploy.contains(forbidden) {
            return Err(format!("CI Pages deployment job must not contain: {forbidden}").into());
        }
    }
    let site_action = fs::read_to_string(root.join(".github/actions/site-validate/action.yml"))?;
    if site_action.matches("run: bun run sync:version").count() != 1
        || site_action
            .lines()
            .any(|line| line.trim() == "run: bun run check")
        || site_action
            .lines()
            .any(|line| line.trim() == "run: bun run build")
    {
        return Err(
            "CI site validation must synchronize the site version exactly once and use CI-only scripts".into(),
        );
    }
    let action_steps = [
        "name: Install dependencies",
        "name: Sync site application version",
        "name: Type and Astro checks",
        "name: Lint",
        "name: Formatting",
        "name: Build static site",
        "name: Verify required output",
        "name: Verify SEO contracts",
        "name: Install Playwright Chromium",
        "name: Run Functional E2E and accessibility tests",
    ];
    let mut previous_position = None;
    for step in action_steps {
        let position = site_action
            .find(step)
            .ok_or_else(|| format!("site validation action is missing its step: {step}"))?;
        if previous_position.is_some_and(|previous| previous >= position) {
            return Err("site validation action steps are out of order".into());
        }
        previous_position = Some(position);
    }
    let site_package = fs::read_to_string(root.join("site/package.json"))?;
    for marker in [
        "\"precheck\": \"bun run sync:version\"",
        "\"check\": \"astro check\"",
        "\"check:ci\": \"astro check\"",
        "\"prebuild\": \"bun run sync:version\"",
        "\"build\": \"astro build\"",
        "\"build:ci\": \"astro build\"",
    ] {
        if !site_package.contains(marker) {
            return Err(format!(
                "site package scripts must preserve local self-contained commands and CI-only variants: {marker}"
            )
            .into());
        }
    }
    let topology = fs::read_to_string(root.join("docs/ci-validation-topology.md"))?;
    for marker in [
        "# CI validation topology",
        "## Triggers",
        "## Classifier outputs",
        "## Representative path matrix",
        "## Build-once boundaries",
        "## Deliberate non-deduplication",
        "## Safe classifier extension procedure",
        "docs/releases/**",
        "site/dist",
        "workflow_dispatch",
        "Unknown paths",
    ] {
        if !topology.contains(marker) {
            return Err(format!(
                "CI validation topology documentation is missing its marker: {marker}"
            )
            .into());
        }
    }
    let xtask_checks = fs::read_to_string(root.join("rust/xtask/src/checks.rs"))?;
    let desktop_branch_start = xtask_checks
        .find("        \"desktop\" => {")
        .ok_or("xtask is missing the full desktop check branch")?;
    let native_branch_start = xtask_checks
        .find("        \"desktop-native\" => {")
        .ok_or("xtask is missing the desktop-native check branch")?;
    let all_branch_start = xtask_checks
        .find("        \"all\" => {")
        .ok_or("xtask is missing the all check branch")?;
    let desktop_branch = &xtask_checks[desktop_branch_start..native_branch_start];
    let native_branch = &xtask_checks[native_branch_start..all_branch_start];
    if !desktop_branch.contains("bun")
        || !desktop_branch.contains("test:e2e")
        || !native_branch.contains("check_desktop_native")
        || native_branch.contains("bun")
        || native_branch.contains("test:e2e")
    {
        return Err("xtask desktop/full and desktop-native check ownership is not locked".into());
    }

    let pages = fs::read_to_string(root.join(".github/workflows/pages.yml"))?;
    for marker in [
        "bun run build",
        "bun run verify:dist",
        "actions/upload-pages-artifact@",
        "actions/deploy-pages@",
    ] {
        if !pages.contains(marker) {
            return Err(format!("Pages deploy contract is missing its marker: {marker}").into());
        }
    }
    if !workflow_declares_only_dispatch(&pages) {
        return Err(
            "Pages recovery workflow must retain workflow_dispatch as its only trigger".into(),
        );
    }
    if pages.contains("./.github/actions/site-validate") || pages.contains("test:functional") {
        return Err("Pages deploy workflow must not repeat the full site validation suite".into());
    }

    let release = fs::read_to_string(root.join(".github/workflows/release-v4.yml"))?;
    if release.contains("inputs:") || release.contains("inputs.") {
        return Err("production release workflow must have zero semantic dispatch inputs".into());
    }
    let rehearsal = fs::read_to_string(root.join(".github/workflows/rehearse-v4.yml"))?;
    if rehearsal.contains("mode: topology") || rehearsal.contains("mode: draft") {
        return Err("rehearsal workflow must not expose a topology/draft mode input".into());
    }

    println!("[xtask] CI/release control-plane contract: PASS");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_github_hosted_ubuntu_contract_accepts_latest() {
        validate_active_github_hosted_ubuntu_labels(
            "jobs:\n  build:\n    runs-on: ubuntu-latest\n",
        )
        .expect("active GitHub-hosted Ubuntu jobs should use ubuntu-latest");
    }

    #[test]
    fn active_github_hosted_ubuntu_contract_rejects_numbered_labels() {
        let error = validate_active_github_hosted_ubuntu_labels(
            "jobs:\n  build:\n    runs-on: ubuntu-24.04\n",
        )
        .expect_err("numbered GitHub-hosted Ubuntu labels must be rejected");
        assert!(error.to_string().contains("ubuntu-latest"));
    }

    #[test]
    fn packaged_ci_build_once_contract_accepts_locked_workflow() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        packaged_ci_contract(&root).expect("locked build-once CI contract must pass");

        let source = fs::read_to_string(root.join(".github/workflows/ci.yml"))
            .expect("locked CI workflow fixture must exist");
        for (current_pin, previous_pin, action) in [
            (
                "actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9",
                "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a",
                "upload-artifact",
            ),
            (
                "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333",
                "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c",
                "download-artifact",
            ),
        ] {
            let stale_pin = source.replace(current_pin, previous_pin);
            assert_ne!(stale_pin, source, "{action} current pin must be present");
            assert!(
                packaged_ci_contract_source(&stale_pin).is_err(),
                "packaged CI contract must reject the previous {action} SHA"
            );
        }
    }

    #[test]
    fn packaged_ci_contract_requires_tauri_and_rejects_v3_artifacts() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = fs::read_to_string(root.join(".github/workflows/ci.yml"))
            .expect("locked CI workflow fixture must exist");
        packaged_ci_contract_source(&source).expect("locked Rust-owned CI workflow must pass");

        for retired in [
            "cargo xtask ci candidate create",
            "cargo xtask ci candidate validate",
            "cargo xtask ci bridge validate",
        ] {
            let without_rust_consumer = source.replace(retired, "retired consumer removed");
            assert_ne!(without_rust_consumer, source, "{retired} must be present");
            assert!(
                packaged_ci_contract_source(&without_rust_consumer).is_err(),
                "CI contract must reject removing {retired}"
            );
        }

        for forbidden in [
            "tauri-update-fixture",
            "dangerousInsecureTransportProtocol",
            "127.0.0.1:17845",
            "CARGO_TARGET_DIR",
            "--features",
            "cargo xtask dist",
            "verify-dist",
            "Sky-Auto-Player-v",
            "Sky-Auto-Player-Updater.exe",
            "MANIFEST.json",
            "PORTABLE_ARTIFACT",
            "portable",
        ] {
            let source_with_legacy_marker = source.replace(
                "    name: Packaged v4 Tauri NSIS qualification",
                &format!("    # {forbidden}\\n    name: Packaged v4 Tauri NSIS qualification"),
            );
            assert!(
                packaged_ci_contract_source(&source_with_legacy_marker).is_err(),
                "{forbidden}"
            );
        }
    }
    #[test]
    fn packaged_ci_contract_requires_the_isolated_fixture_job() {
        let source = r#"
  candidate:
    name: Build current Tauri candidate
    needs: changes
  packaged:
    name: Packaged v4 Tauri NSIS qualification
    needs: [changes, static, candidate]
"#;
        assert!(packaged_ci_contract_source(source).is_err());
    }

    #[test]
    fn packaged_candidate_contract_requires_verified_sccache_before_cargo() {
        let valid = r#"
      - name: Set up sccache
        with:
          version: "v0.17.0"
      - name: Fail fast on package qualification dependencies
      - name: Validate exact current candidate contract
        run: cargo xtask ci candidate validate
"#;
        packaged_sccache_precedes_cargo_contract(valid)
            .expect("pinned sccache setup and verification must precede Cargo");

        let missing_setup = valid.replace("name: Set up sccache", "name: missing sccache");
        assert!(
            packaged_sccache_precedes_cargo_contract(&missing_setup).is_err(),
            "packaged contract must reject a missing sccache setup"
        );

        let late_setup = r#"
      - name: Validate exact current candidate contract
        run: cargo xtask ci candidate validate
      - name: Set up sccache
        with:
          version: "v0.17.0"
      - name: Fail fast on package qualification dependencies
"#;
        assert!(
            packaged_sccache_precedes_cargo_contract(late_setup).is_err(),
            "packaged contract must reject Cargo before sccache setup"
        );

        let early_cargo = valid.replace(
            "      - name: Set up sccache",
            "      - name: Earlier Cargo command\n        run: cargo metadata\n      - name: Set up sccache",
        );
        assert!(
            packaged_sccache_precedes_cargo_contract(&early_cargo).is_err(),
            "packaged contract must reject any Cargo command before sccache"
        );
    }

    #[test]
    fn windows_rust_ci_invokes_an_isolated_xtask_copy() {
        let valid = r#"
          Copy-Item -LiteralPath "rust/target/debug/xtask.exe" -Destination $isolatedXtask
          try {
            & $isolatedXtask check rust
          } finally {
            Remove-Item -LiteralPath $isolatedXtask -Force
          }
"#;
        isolated_xtask_rust_runner_step_contract(valid)
            .expect("the copied xtask must run and clean up outside Cargo's target path");

        let missing_copy = valid.replace(
            "Copy-Item -LiteralPath \"rust/target/debug/xtask.exe\" -Destination $isolatedXtask",
            "# copy removed",
        );
        assert!(
            isolated_xtask_rust_runner_step_contract(&missing_copy).is_err(),
            "Rust CI contract must reject removal of the isolated copy"
        );

        let cargo_alias = valid.replace("& $isolatedXtask check rust", "cargo xtask check rust");
        assert!(
            isolated_xtask_rust_runner_step_contract(&cargo_alias).is_err(),
            "Rust CI contract must reject the self-locking cargo alias"
        );

        let no_finally = valid.replace("} finally {", "} else {");
        assert!(
            isolated_xtask_rust_runner_step_contract(&no_finally).is_err(),
            "Rust CI contract must require cleanup on every exit path"
        );
    }
}
