use super::workflow::workflow_declares_only_dispatch;
use crate::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn packaged_ci_contract_source(source: &str) -> Result<()> {
    if source.replace("\r\n", "\n").contains("\n  candidate:\n") {
        return packaged_ci_build_once_contract_source(source);
    }
    packaged_ci_contract_source_legacy(source)
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
        "scripts/ci_validate_candidate.ps1",
        "-Mode Create",
        "-BundleDir",
        "-PublicKeyPath",
        "-CalibrationBinaryPath",
        "-OutputRoot",
        "-SourceSha",
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
        "scripts/ci_validate_candidate.ps1",
        "scripts/ci_validate_bridge.ps1",
        "-Mode Validate",
        "-CandidateInstallerPath",
        "-CandidateSignaturePath",
        "-CandidateVersion",
        "-CandidatePublicKeyPath",
        "Download updater bridge from this workflow run",
        "-BridgeRootPath",
        "-BridgeInstallerPath",
        "scripts/ci_tauri_update_e2e.ps1",
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
        "scripts/ci_validate_candidate.ps1",
        "-Mode Validate",
        "cargo xtask verify-tauri-bundle",
        "cargo xtask sbom generate",
        "cargo xtask sbom verify",
        "-Mode unsigned-zero-budget",
        "current-user install, launch, and uninstall",
        "cargo xtask builtin-catalog verify-installed",
        "installer_sha256",
        "updater_signature_sha256",
        "updater_public_key_sha256",
        "verify_installed_calibration.ps1",
        "INSTALLED_CALIBRATION_EVIDENCE.json",
        "calibration_binary_sha256",
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
    let install_position = packaged
        .find("installerRun = Start-Process")
        .ok_or("packaged consumer must execute the exact NSIS installer")?;
    let calibration_position = packaged
        .find("verify_installed_calibration.ps1")
        .ok_or("packaged consumer must verify installed native calibration")?;
    let launch_position = packaged
        .find("Start-Process -FilePath $appPath -WindowStyle Hidden -PassThru")
        .ok_or("packaged consumer must launch the installed application")?;
    if calibration_position <= install_position || calibration_position >= launch_position {
        return Err(
            "installed calibration verification must follow installation and precede app launch"
                .into(),
        );
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

fn packaged_ci_contract_source_legacy(source: &str) -> Result<()> {
    let normalized = source.replace("\r\n", "\n");
    let package_needs = "needs: [changes, static]";
    let fixture_start = normalized
        .find("  updater_e2e:\n")
        .ok_or("CI workflow is missing the isolated updater fixture job")?;
    let fixture_end = normalized[fixture_start..]
        .find("\n  packaged:\n")
        .map(|offset| fixture_start + offset)
        .ok_or("CI workflow updater fixture job must precede the canonical packaged job")?;
    let fixture = &normalized[fixture_start..fixture_end];
    let required_needs_line = format!("    {package_needs}");
    if !fixture.lines().any(|line| line == required_needs_line) {
        return Err(format!(
            "updater fixture job must declare the required dependency topology: {package_needs}"
        )
        .into());
    }
    for marker in [
        "name: Updater fixture qualification",
        "if: needs.changes.outputs.updater_required == 'true'",
        "tauri-update-fixture",
        "dangerousInsecureTransportProtocol",
        "scripts/ci_tauri_update_e2e.ps1",
        "FixtureTargetDir",
        "RUNNER_TEMP",
    ] {
        if !fixture.contains(marker) {
            return Err(format!(
                "isolated updater fixture CI is missing its required marker: {marker}"
            )
            .into());
        }
    }

    let start = normalized
        .find("  packaged:\n")
        .ok_or("CI workflow is missing the packaged job")?;
    let end = normalized[start..]
        .find("\n  status:\n")
        .map(|offset| start + offset)
        .ok_or("CI workflow packaged job is missing the status boundary")?;
    let packaged = &normalized[start..end];
    if !packaged.lines().any(|line| line == required_needs_line) {
        return Err(format!(
            "packaged job must declare the required dependency topology: {package_needs}"
        )
        .into());
    }

    for marker in [
        "name: Packaged v4 Tauri NSIS qualification",
        "Build and sign canonical Tauri NSIS artifact",
        "bun install --frozen-lockfile",
        "bun run build",
        "bun run tauri signer generate",
        "TAURI_SIGNING_PRIVATE_KEY",
        "bun run tauri build --ci --config",
        "name: Resolve GitHub CLI for artifact attestation verification",
        "Get-Command gh.exe -CommandType Application",
        "SKY_GH_PATH=$ghPath",
        "- name: Run Authenticode tamper regression",
        "scripts/test_v4_authenticode_integrity.ps1",
        "- name: Run V4 production signing contract test",
        "scripts/test_v4_production_signing_contract.ps1",
        "V4 production signing contract test failed with exit code",
        "- name: Verify Tauri Authenticode signature",
        "-Mode unsigned-zero-budget",
        "- name: Generate Tauri SPDX SBOM",
        "- name: Verify Tauri SPDX SBOM",
        "- name: Verify exact Tauri NSIS bundle",
        "Authenticode verification failed with exit code",
        "SBOM generation failed with exit code",
        "SBOM verification failed with exit code",
        "Tauri bundle verification failed with exit code",
        "& scripts/verify_v4_authenticode.ps1",
        "-Artifact $installedPe.FullName",
        "Installed Authenticode verification failed:",
        "scripts/test_installed_calibration.ps1",
        "verify_installed_calibration.ps1",
        "INSTALLED_CALIBRATION_EVIDENCE.json",
        "cargo xtask builtin-catalog verify-installed",
        "CI self-signed credentials remain test-only",
        "Tauri updater signer generation failed with exit code",
        "Tauri build failed with exit code",
        "Installer attestation verification failed with exit code",
        "Updater signature attestation verification failed with exit code",
        "SBOM attestation verification failed with exit code",
        "GH_TOKEN: ${{ github.token }}",
        "attestation verify --help",
        "--source-digest $env:GITHUB_SHA",
        "--signer-workflow $signerWorkflow",
        "& $env:SKY_GH_PATH attestation verify",
        "--predicate-type https://spdx.dev/Document/v2.3",
        "GitHub CLI absolute path is unavailable for attestation verification",
        "GH_TOKEN is unavailable for attestation verification",
        "Installed GitHub CLI lacks the required exact-source attestation options",
        "current-user install, launch, and uninstall",
        "sky_desktop_shell.exe",
        "uninstall.exe",
        "scripts/cleanup_v4_test_signing.ps1",
        "rust/target/dist/bundle/nsis",
        "actions/upload-artifact@",
    ] {
        if !packaged.contains(marker) {
            return Err(format!(
                "canonical v4 packaged CI is missing the Tauri qualification marker: {marker}"
            )
            .into());
        }
    }
    let test_signing_setup_marker =
        "      - name: Prepare bounded ephemeral Authenticode test certificate\n";
    let tamper_regression_marker = "      - name: Run Authenticode tamper regression\n";
    let test_signing_setup_position = packaged
        .find(test_signing_setup_marker)
        .ok_or("canonical v4 packaged CI is missing the isolated test-signing setup step")?;
    let tamper_regression_position = packaged
        .find(tamper_regression_marker)
        .ok_or("canonical v4 packaged CI is missing the Authenticode tamper regression step")?;
    if test_signing_setup_position >= tamper_regression_position {
        return Err(
            "canonical v4 packaged CI must prepare test signing credentials in a prior step".into(),
        );
    }
    let test_signing_setup = &packaged[test_signing_setup_position..tamper_regression_position];
    for marker in [
        "timeout-minutes: 2",
        "pwsh scripts/setup_v4_test_signing.ps1 -EnvFile $env:GITHUB_ENV -TimeoutSeconds 30",
    ] {
        if !test_signing_setup.contains(marker) {
            return Err(format!(
                "canonical v4 packaged CI test-signing setup is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let tamper_regression_end = packaged[tamper_regression_position..]
        .find("\n      - name: Run V4 production signing contract test\n")
        .map(|offset| tamper_regression_position + offset)
        .ok_or("canonical v4 packaged CI tamper regression step has no bounded end")?;
    let tamper_regression = &packaged[tamper_regression_position..tamper_regression_end];
    if tamper_regression.contains("setup_v4_test_signing.ps1") {
        return Err(
            "canonical v4 packaged CI must not configure test signing in the tamper regression step"
                .into(),
        );
    }
    let attestation_start = packaged
        .find("      - name: Verify exact GitHub artifact attestations\n")
        .ok_or("canonical v4 packaged CI is missing the attestation verification step")?;
    let attestation_end = packaged[attestation_start..]
        .find("\n      - name: Upload exact Tauri NSIS release candidate\n")
        .map(|offset| attestation_start + offset)
        .ok_or("canonical v4 packaged CI attestation step has no bounded end")?;
    let attestation = &packaged[attestation_start..attestation_end];
    if attestation
        .matches("--source-digest $env:GITHUB_SHA")
        .count()
        != 3
        || attestation
            .matches("--signer-workflow $signerWorkflow")
            .count()
            != 3
        || attestation.matches("-R $env:GITHUB_REPOSITORY").count() != 3
        || attestation
            .lines()
            .any(|line| line.trim_start().starts_with("gh attestation verify"))
    {
        return Err(
            "canonical v4 packaged CI attestation verification must use absolute gh, exact source digest, signer workflow, and repository binding for all three checks".into(),
        );
    }
    let validate_start = normalized
        .find("  validate:\n")
        .ok_or("CI workflow is missing the validate job")?;
    let validate_end = normalized[validate_start..]
        .find("\n  updater_e2e:\n")
        .map(|offset| validate_start + offset)
        .ok_or("CI workflow validate job must precede the updater fixture job")?;
    if normalized[validate_start..validate_end].contains("certutil.exe") {
        return Err("ordinary Windows validation must not require certutil.exe".into());
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
        "scripts/test_v4_production_orchestrator.ps1",
        "scripts/test_v4_updater_private_key.ps1",
    ] {
        if packaged.contains(forbidden) {
            return Err(format!(
                "canonical v4 packaged CI must not contain the legacy v3 artifact marker: {forbidden}"
            )
            .into());
        }
    }

    for marker in [
        "needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]",
        "UPDATER_REQUIRED",
        "UPDATER_BRIDGE_REQUIRED",
        "UPDATER_CONTRACT_REQUIRED",
        "DESKTOP_WEB_REQUIRED",
        "RELEASE_REQUIRED",
        "SUPPLY_CHAIN_REQUIRED",
        "UPDATER_CONTRACT_RESULT",
        "UPDATER_E2E_RESULT",
    ] {
        if !normalized.contains(marker) {
            return Err(format!(
                "CI required gate is missing updater fixture integration marker: {marker}"
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
    let validator = fs::read_to_string(root.join("scripts/ci_validate_candidate.ps1"))?;
    for marker in [
        "schema_version",
        "source_sha",
        "calibration_binary_size",
        "calibration_binary_sha256",
        "calibration_binary_architecture",
        "installer_sha256",
        "updater_signature_sha256",
        "updater_public_key_sha256",
        "Get-FileHash",
        "updater-public-key.pub",
        "PRIVATE KEY",
        "candidate artifact must contain exactly four files",
    ] {
        if !validator.contains(marker) {
            return Err(format!(
                "current-candidate validator is missing its fail-closed marker: {marker}"
            )
            .into());
        }
    }
    let validator_test = root.join("scripts/test_ci_validate_candidate.ps1");
    if !validator_test.exists() {
        return Err("current-candidate validator self-test is missing".into());
    }
    for script in [
        "scripts/build_native_calibration.ps1",
        "scripts/verify_installed_calibration.ps1",
        "scripts/test_installed_calibration.ps1",
    ] {
        if !root.join(script).exists() {
            return Err(
                format!("native calibration package contract script is missing: {script}").into(),
            );
        }
    }
    let bridge_validator = fs::read_to_string(root.join("scripts/ci_validate_bridge.ps1"))?;
    for marker in [
        "schema_version",
        "source_sha",
        "installer_sha256",
        "sentinel_id",
        "sentinel_content_sha256",
        "bridge artifact must contain exactly two files",
        "Get-FileHash",
        "PRIVATE KEY",
    ] {
        if !bridge_validator.contains(marker) {
            return Err(format!(
                "updater-bridge validator is missing its fail-closed marker: {marker}"
            )
            .into());
        }
    }
    if !root.join("scripts/test_ci_validate_bridge.ps1").exists() {
        return Err("updater-bridge validator self-test is missing".into());
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
        "scripts/ci_classify.ps1",
        "scripts/test_ci_classify.ps1",
        "github.event.pull_request.base.sha",
        "github.event.pull_request.head.sha",
        "github.event.before",
        "classifier_args+=(-Full)",
        "classifier_args+=(-BaseSha",
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
        "scripts/ci_build_updater_bridge.ps1",
        "scripts/ci_validate_bridge.ps1",
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
    let bridge_script = fs::read_to_string(root.join("scripts/ci_build_updater_bridge.ps1"))?;
    if bridge_script.matches("bun run tauri build").count() != 1
        || !bridge_script.contains("tauri-update-fixture")
        || !bridge_script.contains("RUNNER_TEMP")
        || !bridge_script.contains("ci_validate_bridge.ps1")
        || !bridge_script.contains("WriteAllBytes($cargoPath, $cargoSource)")
        || !bridge_script.contains("WriteAllBytes($lockPath, $lockSource)")
    {
        return Err("updater bridge producer must have one fixture build, bounded contract validation, and exact source restoration".into());
    }
    if bridge_script.contains("upload-artifact") || bridge_script.contains("bridge.json.sig") {
        return Err(
            "updater bridge producer must not upload signing material or extra artifact files"
                .into(),
        );
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
        "-CandidateInstallerPath $env:SKY_CANDIDATE_INSTALLER",
        "Download updater bridge from this workflow run",
        "scripts/ci_validate_bridge.ps1",
        "-BridgeRootPath $env:SKY_BRIDGE_ROOT",
        "-BridgeInstallerPath $env:SKY_BRIDGE_INSTALLER",
        "-BridgeSourceSha $env:SKY_BRIDGE_SOURCE_SHA",
        "-BridgeSentinelSha256 $env:SKY_BRIDGE_SENTINEL_SHA256",
        "zero bridge or candidate tauri build",
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
    if validate.matches("scripts/build_native_calibration.ps1").count() != 1
        || !validate.contains(
            "run: pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File scripts/build_native_calibration.ps1",
        )
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
        let source = r#"
  updater_bridge:
    name: Build updater bridge fixture
    needs: changes
    if: needs.changes.outputs.updater_required == 'true'
    steps:
      - run: scripts/ci_build_updater_bridge.ps1
      - run: scripts/ci_validate_bridge.ps1
      - uses: actions/upload-artifact@v7
  desktop_web:
    name: Desktop web and browser validation
    needs: changes
    if: needs.changes.outputs.desktop_required == 'true'
    runs-on: ubuntu-latest
    steps:
      - run: bun install --frozen-lockfile
      - run: bun run check
      - run: bun node_modules/playwright/cli.js --version
      - run: bun node_modules/playwright/cli.js install chromium
      - run: bun run test:e2e
  validate:
    name: Windows compatibility and unit tests
    steps:
      - run: cargo xtask check rust
  candidate:
    name: Build current Tauri candidate
    needs: changes
    if: needs.changes.outputs.package_required == 'true' || needs.changes.outputs.updater_required == 'true'
    steps:
      - run: bun install --frozen-lockfile
      - run: bun run tauri build --ci --config candidate.json -- --profile dist
      - run: bun run dev:build-info -- --exe ..\\rust\\target\\dist\\sky_desktop_shell.exe
      - run: Remove-Item -LiteralPath $keyPath, "$keyPath.pub", $configPath
      - run: scripts/ci_validate_candidate.ps1 -Mode Create -BundleDir $bundleDir -PublicKeyPath $publicKeyPath -CalibrationBinaryPath desktop/src-tauri/resources/native_calibration.exe -OutputRoot $candidateRoot -SourceSha $env:SKY_CI_SOURCE_SHA
      - uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9
        path: ${{ runner.temp }}/sky-auto-player-current-candidate
  updater_contract:
    name: Updater key-rotation contract
    needs: changes
    if: needs.changes.outputs.updater_required == 'true'
    runs-on: windows-latest
    steps:
      - run: rustup toolchain install 1.99.1
      - run: bun install --frozen-lockfile
      - run: scripts/test_v4_updater_key_rotation.ps1
  updater_e2e:
    name: Updater fixture qualification
    needs: [changes, static, candidate, updater_bridge]
    if: needs.changes.outputs.updater_required == 'true'
    steps:
      - name: Download updater bridge from this workflow run
        uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333
      - run: scripts/ci_validate_candidate.ps1 -Mode Validate -CandidateInstallerPath candidate.exe -CandidateSignaturePath candidate.sig -CandidateVersion 4.0.0-alpha.2 -CandidatePublicKeyPath candidate.pub
      - run: scripts/ci_validate_bridge.ps1 -Mode Validate -BridgeRoot bridge
      - run: dangerousInsecureTransportProtocol = true
      - run: $fixtureTarget = Join-Path $env:RUNNER_TEMP "sky-auto-player-v4-updater-fixture-target"; pwsh scripts/ci_tauri_update_e2e.ps1 -FixtureTargetDir $fixtureTarget -BridgeRootPath bridge -BridgeInstallerPath bridge.exe -BridgeSourceSha 1234567890abcdef1234567890abcdef12345678 -BridgeVersion 4.0.0-alpha.1 -BridgeSentinelId sentinel -BridgeSentinelSha256 abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd
  packaged:
    name: Packaged v4 Tauri NSIS qualification
    needs: [changes, static, candidate]
    steps:
      - uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333
      - run: scripts/ci_validate_candidate.ps1 -Mode Validate
      - name: Build and sign canonical Tauri NSIS artifact
      - run: bun install --frozen-lockfile
      - run: bun run build
      - run: bun run tauri signer generate
        # Tauri updater signer generation failed with exit code
      - name: Validate exact current candidate contract
        run: scripts/ci_validate_candidate.ps1 -Mode Validate
      - name: Rust cache
      - name: Stage and re-hash exact current candidate after Rust cache restore
        run: Get-FileHash -LiteralPath $stagedInstaller -Algorithm SHA256; Get-FileHash -LiteralPath $stagedSignature -Algorithm SHA256; Get-FileHash -LiteralPath $env:SKY_CANDIDATE_PUBLIC_KEY -Algorithm SHA256
        # Staged candidate hashes do not match the validated candidate contract
        # installer_sha256 updater_signature_sha256 updater_public_key_sha256
      - name: Verify Tauri Authenticode signature
        run: pwsh scripts/verify_v4_authenticode.ps1 -Mode unsigned-zero-budget
        # Authenticode verification failed with exit code
        # Installed Authenticode verification failed with exit code
        # CI self-signed credentials remain test-only
      - name: Generate Tauri SPDX SBOM
        run: cargo xtask sbom generate
        # SBOM generation failed with exit code
      - name: Verify Tauri SPDX SBOM
        run: cargo xtask sbom verify
        # SBOM verification failed with exit code
      - name: Verify exact Tauri NSIS bundle
        run: cargo xtask verify-tauri-bundle
        # Tauri bundle verification failed with exit code
      - name: Qualify current-user install, launch, and uninstall
        run: installerRun = Start-Process -FilePath $installer -ArgumentList @('/S', '/NS') -Wait -PassThru
      - run: pwsh scripts/verify_installed_calibration.ps1 -InstallRoot $installRoot -ExpectedSha256 $candidate.calibration_binary_sha256 -Evidence rust/target/dist/INSTALLED_CALIBRATION_EVIDENCE.json
      - run: Start-Process -FilePath $appPath -WindowStyle Hidden -PassThru
      - run: check sky_desktop_shell.exe uninstall.exe
      - run: cargo xtask builtin-catalog verify-installed --root installed/builtin-songs
      - name: Upload exact Tauri NSIS release candidate
        uses: actions/upload-artifact@v7
        path: rust/target/dist/bundle/nsis
  site:
    name: Website validation
  status:
    name: Sky Auto Player — required CI gate
    # Build bounded unsigned Authenticode PE fixture
    # rustc --edition 2021 --target x86_64-pc-windows-msvc
    # Get-AuthenticodeSignature SKY_AUTHENTICODE_FIXTURE
    # Run Authenticode tamper regression on controlled unsigned PE fixture
    # scripts/test_v4_authenticode_integrity.ps1
    # scripts/test_v4_production_signing_contract.ps1
    # scripts/setup_v4_test_signing.ps1
    # scripts/cleanup_v4_test_signing.ps1
    needs: [changes, static, release_contract, supply_chain, validate, desktop_web, candidate, updater_bridge, updater_contract, updater_e2e, packaged, site]
    env: { UPDATER_REQUIRED: true, RELEASE_REQUIRED: false, SUPPLY_CHAIN_REQUIRED: false, UPDATER_BRIDGE_REQUIRED: true, UPDATER_CONTRACT_REQUIRED: true, DESKTOP_WEB_REQUIRED: true, CANDIDATE_REQUIRED: true, CANDIDATE_RESULT: success, UPDATER_CONTRACT_RESULT: success, UPDATER_E2E_RESULT: success }
        "#;
        assert!(packaged_ci_contract_source(source).is_ok());
        let crlf_source = source.replace('\n', "\r\n");
        assert!(packaged_ci_contract_source(&crlf_source).is_ok());
        let unblocked_package_jobs =
            source.replace("needs: [changes, static, candidate]", "needs: changes");
        assert!(packaged_ci_contract_source(&unblocked_package_jobs).is_err());
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
                &format!("    # {forbidden}\n    name: Packaged v4 Tauri NSIS qualification"),
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
  validate:
    name: Windows compatibility and unit tests
    steps:
      - run: cargo xtask check rust
  packaged:
    name: Packaged v4 Tauri NSIS qualification
    needs: [changes, static]
    steps:
      - run: bun install --frozen-lockfile
      - run: bun run build
      - run: bun run tauri signer generate
      - run: bun run tauri build --ci --config test.json
      - run: cargo xtask verify-tauri-bundle
      - name: Qualify current-user install, launch, and uninstall
        run: check sky_desktop_shell.exe uninstall.exe
      - run: cargo xtask builtin-catalog verify-installed --root installed/builtin-songs
      - uses: actions/upload-artifact@v7
  status:
    needs: [changes, static, supply_chain, validate, packaged]
        "#;
        assert!(packaged_ci_contract_source(source).is_err());
    }
}
