use crate::{Result, tauri_bundle};
use base64::{Engine, engine::general_purpose::STANDARD};
use minisign_verify::PublicKey;
use serde_json::Value;
use std::fs;
use std::path::Path;
use walkdir::WalkDir;

fn v4_production_calibration_gate_contract(harness: &str) -> Result<()> {
    let qualification = harness
        .split_once("fn qualify_package(")
        .map(|(_, body)| body)
        .ok_or("Rust NSIS owner is missing its package qualification function")?;
    let qualification = qualification
        .split_once("\n#[cfg(windows)]\nfn verify_calibration_artifact(")
        .map(|(body, _)| body)
        .ok_or("Rust package qualification body is not bounded")?;
    let staged_gate = qualification
        .find("&staged_calibration_root")
        .ok_or("Rust NSIS owner must verify the staged calibration resource")?;
    let install_position = qualification
        .find("run_installer(\n        installer,")
        .ok_or("Rust NSIS owner must install the exact candidate")?;
    let installed_gate = qualification
        .find("verify_calibration_artifact(\n        repository_root,\n        &install_root")
        .ok_or("Rust NSIS owner must verify installed native calibration")?;
    let launch_position = qualification
        .find("launch_smoke(scope, &app")
        .ok_or("Rust NSIS owner must launch the installed application")?;
    if staged_gate >= install_position
        || install_position >= installed_gate
        || installed_gate >= launch_position
    {
        return Err("calibration qualification must verify staged bytes, install, verify installed bytes, then launch".into());
    }

    let verifier = harness
        .split_once("fn verify_calibration_artifact(")
        .map(|(_, remainder)| remainder)
        .ok_or("Rust NSIS owner is missing its typed calibration verifier")?;
    let verifier = verifier
        .split_once("\n#[cfg(windows)]")
        .map(|(function, _)| function)
        .ok_or("Rust calibration verifier must be a bounded Windows owner function")?;
    for marker in [
        "verify-installed",
        "--install-root",
        "--expected-size",
        "candidate.manifest.calibration_binary_size",
        "--expected-sha256",
        "candidate.manifest.calibration_binary_sha256",
        "crate::calibration::run(repository_root, &args)",
    ] {
        if !verifier.contains(marker) {
            return Err(format!(
                "Rust installed calibration verifier is missing required marker: {marker}"
            )
            .into());
        }
    }

    Ok(())
}

pub(super) fn v4_trust_material_contract(root: &Path) -> Result<()> {
    let config_path = root.join("desktop/src-tauri/tauri.conf.json");
    let config = fs::read_to_string(&config_path)?;
    for marker in ["sign_v4_authenticode.ps1", "plugins", "updater", "pubkey"] {
        if !config.contains(marker) {
            return Err(
                format!("v4 Tauri trust config is missing its required marker: {marker}").into(),
            );
        }
    }
    if config.contains("release-2026") || config.contains("PRIVATE KEY") {
        return Err("v4 Tauri config contains legacy or private key material".into());
    }

    let config_json: Value = serde_json::from_str(&config)?;
    let config_key = config_json
        .get("plugins")
        .and_then(Value::as_object)
        .and_then(|plugins| plugins.get("updater"))
        .and_then(Value::as_object)
        .and_then(|updater| updater.get("pubkey"))
        .and_then(Value::as_str)
        .ok_or("v4 Tauri config public trust root is not a string")?;
    let native_path = root.join("desktop/src-tauri/src/native_update.rs");
    let native = fs::read_to_string(&native_path)?;
    let native_key = extract_rust_string_constant(&native, "V4_TAURI_UPDATER_PUBLIC_KEY")?;
    if config_key != tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY
        || native_key != tauri_bundle::V4_TAURI_UPDATER_PUBLIC_KEY
        || !native.contains(
            "const V4_TAURI_UPDATER_PUBLIC_KEYS: &[&str] = &[V4_TAURI_UPDATER_PUBLIC_KEY];",
        )
    {
        return Err("v4 production updater public-root copies do not match byte-for-byte".into());
    }
    let decoded = STANDARD.decode(config_key)?;
    let decoded = String::from_utf8(decoded)?;
    PublicKey::decode(&decoded)?;
    crate::updater_trust::inventory_public_trust_roots(root)?;

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path)?.replace("\r\n", "\n");
    for marker in [
        "scripts/setup_v4_test_signing.ps1",
        "cargo xtask authenticode verify",
        "scripts/test_v4_authenticode_integrity.ps1",
        "scripts/cleanup_v4_test_signing.ps1",
        "scripts/test_v4_updater_key_rotation.ps1",
        "cargo xtask ci updater-fixture run",
        "cargo xtask ci nsis smoke",
        "--allow-hosted-disposable-runner",
        "cargo xtask ci bridge validate",
        "scripts/ci_require_windows_tools.ps1",
        "TimeoutSeconds 30",
        "qualify packaged Tauri updater rotation",
        "cargo xtask ci candidate validate",
        "--candidate-root",
        "--bridge-root",
        "--source-sha",
        "--fixture-root",
        "--allow-hosted-disposable-runner",
        "cargo xtask sbom generate",
        "cargo xtask sbom verify",
        "workflow_dispatch:",
    ] {
        if !ci.contains(marker) {
            return Err(format!("v4 trust CI is missing its required marker: {marker}").into());
        }
    }
    let ordinary_ci = ci.split("  deploy_pages:").next().unwrap_or(ci.as_str());
    if ordinary_ci.contains("actions/attest@")
        || ordinary_ci.contains("id-token: write")
        || ordinary_ci.contains("attestations: write")
    {
        return Err(
            "ordinary CI must not retain artifact attestation creation or permissions".into(),
        );
    }
    for marker in [
        "#[cfg(feature = \"tauri-update-fixture\")]",
        "FIXTURE_NEW_ONLY_ARG",
        "FIXTURE_PORT_ARG",
        "FIXTURE_PUBLIC_KEY_ARG",
        "fixture_runtime_config_from_args",
        "read_fixture_public_key",
        "fixture_public_keys",
        ".last()",
        "fixture_new_only_mode_selects_only_the_last_supplied_root",
    ] {
        if !native.contains(marker) {
            return Err(format!(
                "fixture-only updater new-root runtime seam is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let updater_fixture = fs::read_to_string(root.join("rust/xtask/src/updater_fixture.rs"))?;
    for marker in [
        "127.0.0.1",
        "loopback fixture returned",
        "validate_manifest_bytes",
        "response_bytes_are_exact_source_bytes",
        "body_sha256",
        "content_length",
        "application/octet-stream",
        "updater-preserved-user.json",
        "user_song_sha256_unchanged",
        "built_in_manifest_sha256_before",
        "built_in_manifest_sha256_after",
        "selected_builtin_id_before",
        "selected_builtin_id_after",
        "selected_builtin_content_sha256_before",
        "selected_builtin_content_sha256_after",
        "source_tree_restore",
        "crate::builtin_catalog::run",
        "candidate.installer_sha256",
        "old-root detached-signature operation changed candidate installer bytes",
        "git",
        "worktree",
        "--detach",
        "preserved_bridge_root",
        "n_to_n_plus_1_installer_sha256",
        "old_root_rejection",
        "assert_migration_registry_roots_absent",
        "--selftest-update-fixture-new-only",
        "--selftest-update-fixture-port",
        "--selftest-update-fixture-public-key",
        "write_http_evidence",
        "MAX_HTTP_REQUESTS",
        "source SHA must match the checked-out Git HEAD",
    ] {
        if !updater_fixture.contains(marker) {
            return Err(format!(
                "updater fixture is missing its required preservation marker: {marker}"
            )
            .into());
        }
    }
    if native.contains("option_env!(\"SKY_TAURI_UPDATE_FIXTURE")
        || updater_fixture.contains("SKY_TAURI_UPDATE_FIXTURE_PUBLIC_KEYS")
        || updater_fixture.contains("SKY_TAURI_UPDATE_FIXTURE_PORT")
    {
        return Err(
            "fixture updater must not use compile-time roots or port environment inputs".into(),
        );
    }
    if ci.matches("cargo install cargo-vet").count() != 1 {
        return Err("CI must install cargo-vet exactly once in the supply-chain job".into());
    }
    if ci
        .matches("google/osv-scanner-action/osv-scanner-action@")
        .count()
        != 1
    {
        return Err("CI must run exactly one pinned OSV-Scanner action".into());
    }
    if !ci.contains("--lockfile=./desktop/bun.lock") {
        return Err("CI OSV-Scanner must audit the desktop Bun lockfile".into());
    }

    let verifier = fs::read_to_string(root.join("scripts/verify_v4_authenticode.ps1"))?;
    for marker in [
        "authenticode-windows-inspection",
        "Get-AuthenticodeSignature",
        "ExpectedThumbprint",
        "v4_authenticode_crypto.ps1",
        "Get-AuthenticodeIntegrityProof",
        "ArtifactManifest",
        "maximumArtifactCount",
        "maximumManifestBytes",
        "Authenticode artifact manifest contains a malformed entry",
        "inspection file does not match its manifest SHA-256",
        "platform_status",
        "signed_digest_algorithm",
        "computed_digest",
    ] {
        if !verifier.contains(marker) {
            return Err(format!(
                "v4 Authenticode verifier is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let rust_policy = fs::read_to_string(root.join("rust/xtask/src/authenticode.rs"))?;
    for marker in [
        "unsigned-zero-budget",
        "validate_thumbprint",
        "validate_test_pfx",
        "validate_test_pfx_password",
        "validate_production_provider",
        "accepted_status",
        "test Authenticode PFX passwords are forbidden because signtool exposes argv",
        "authenticode-verification",
        "unsigned-zero-budget accepts only a real NotSigned PE with no signer",
        "Authenticode signed CMS digest does not match the computed PE image digest",
        "MAX_AUTHENTICODE_ARTIFACTS",
        "MAX_ARTIFACT_MANIFEST_BYTES",
        "artifact_manifest_bytes",
        "adapter_evidence_must_cover_every_requested_file_and_exact_hash",
        "artifact_manifest_is_bounded_and_preserves_exact_unicode_inventory",
        "write_evidence",
    ] {
        if !rust_policy.contains(marker) {
            return Err(format!(
                "Rust Authenticode policy is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let crypto = fs::read_to_string(root.join("scripts/v4_authenticode_crypto.ps1"))?;
    for marker in [
        "Get-AuthenticodePeLayout",
        "Get-AuthenticodeImageDigest",
        "Get-AuthenticodeSpcDigest",
        "SizeOfHeaders",
        "sumOfBytesHashed",
        "Sort-Object PointerToRawData",
        "SignedCms",
        "CheckSignature($true)",
        "signature-valid-independent-cryptographic-integrity",
        "signedcms-spc-indirect-data-authenticode-hash",
    ] {
        if !crypto.contains(marker) {
            return Err(format!(
                "v4 independent Authenticode verifier is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let setup = fs::read_to_string(root.join("scripts/setup_v4_test_signing.ps1"))?;
    for marker in [
        "CertificateRequest",
        "X509ContentType]::Pfx",
        "EphemeralKeySet",
        "Passwordless ephemeral PFX",
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD=",
        "RUNNER_TEMP",
    ] {
        if !setup.contains(marker) {
            return Err(format!(
                "v4 Authenticode test PFX setup is missing its required marker: {marker}"
            )
            .into());
        }
    }
    if setup.contains("::add-mask::")
        || setup.contains("SKY_AUTHENTICODE_TEST_PFX_PASSWORD=$pfxPassword")
    {
        return Err(
            "ephemeral Authenticode PFX passwords must not be generated or published".into(),
        );
    }
    for forbidden in [
        "New-SelfSignedCertificate",
        "certutil.exe",
        "Cert:\\CurrentUser",
        "TrustedPublisher",
        "CurrentUser/${store}",
    ] {
        if setup.contains(forbidden) {
            return Err(format!(
                "v4 Authenticode test PFX setup must not depend on certificate stores: {forbidden}"
            )
            .into());
        }
    }
    let cleanup = fs::read_to_string(root.join("scripts/cleanup_v4_test_signing.ps1"))?;
    for marker in [
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "RUNNER_TEMP",
        "sky-v4-test-signing-[0-9a-fA-F]{32}\\.pfx",
        "Clear-TestSigningEnvironment",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD",
    ] {
        if !cleanup.contains(marker) {
            return Err(format!(
                "v4 Authenticode test PFX cleanup is missing its required marker: {marker}"
            )
            .into());
        }
    }

    for forbidden in [
        "Cert:\\CurrentUser",
        "TrustedPublisher",
        "CurrentUser/${store}",
    ] {
        if cleanup.contains(forbidden) {
            return Err(format!(
                "v4 Authenticode test PFX cleanup must not depend on certificate stores: {forbidden}"
            )
            .into());
        }
    }
    let signer = fs::read_to_string(root.join("scripts/sign_v4_authenticode.ps1"))?;
    for marker in [
        "unsigned-zero-budget",
        "no signing performed",
        "SKY_AUTHENTICODE_TEST_PFX_PATH",
        "SKY_AUTHENTICODE_TEST_PFX_PASSWORD",
        "/f $pfxPath",
        "test Authenticode signing rejects PFX passwords because signtool exposes argv",
        "sign /fd SHA256 /f $pfxPath $Path",
        "EphemeralKeySet",
        "SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT",
        "SKY_AUTHENTICODE_PROVIDER",
        "SKY_AUTHENTICODE_PROVIDER_COMMAND",
    ] {
        if !signer.contains(marker) {
            return Err(
                format!("v4 Authenticode signer is missing its required marker: {marker}").into(),
            );
        }
    }
    if signer.contains(" /p ") {
        return Err("signtool must never receive a PFX password on its command line".into());
    }
    let tamper = fs::read_to_string(root.join("scripts/test_v4_authenticode_integrity.ps1"))?;
    for marker in [
        "sign_v4_authenticode.ps1",
        "Get-AuthenticodeSignature",
        "clean signed PE PASS",
        "Tampered signed PE unexpectedly passed independent Authenticode verification",
        "Get-AuthenticodePeLayout",
        "WriteAllBytes",
        "cargo xtask authenticode verify",
    ] {
        if !tamper.contains(marker) {
            return Err(format!(
                "v4 Authenticode tamper regression is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let contract_test =
        fs::read_to_string(root.join("scripts/test_v4_production_signing_contract.ps1"))?;
    for marker in [
        "unsigned-zero-budget mode succeeds without a provider",
        "Production signing rejects test credentials",
        "Production verification rejects CI test certificate",
        "unsigned-zero-budget verification rejects signed binary",
        "Production verification rejects test thumbprint",
        "Contract Test 3b: Rust CLI passes two signed synthetic PE files through the manifest adapter",
        "Contract Test 3c: missing artifact fails closed",
        "Contract Test 3d: tampered signed PE fails closed through the manifest adapter",
        "v4_authenticode_crypto.ps1",
        "Get-AuthenticodePeLayout",
        "cargo @cargoArguments",
        r#"@("xtask", "authenticode", "verify", "--mode", $Mode)"#,
        "CI test certificate cannot satisfy production mode or zero-budget unsigned state",
    ] {
        if !contract_test.contains(marker) {
            return Err(format!(
                "v4 production signing contract test is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let key_verifier = fs::read_to_string(root.join("scripts/verify_v4_updater_private_key.ps1"))?;
    if !key_verifier.contains("updater-trust verify-private-key") {
        return Err(
            "v4 updater key verification script must delegate to updater-trust verify-private-key"
                .into(),
        );
    }
    for forbidden in ["::add-mask::", "19AABD2E7838818C"] {
        if key_verifier.contains(forbidden) {
            return Err(format!(
                "v4 updater key verification script must not emit or hard-code production secret/output data: {forbidden}"
            )
            .into());
        }
    }
    let key_verifier_test =
        fs::read_to_string(root.join("scripts/test_v4_updater_private_key.ps1"))?;
    for marker in [
        "V4_TEST_ONLY_PASS_PHRASE_MARKER",
        "verify_v4_updater_private_key.ps1",
        "Assert-NoPasswordMarker",
        "Verifier mismatch path",
        "Verifier success path",
        "throwaway.key",
    ] {
        if !key_verifier_test.contains(marker) {
            return Err(format!(
                "v4 updater key verifier regression is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let orchestrator =
        fs::read_to_string(root.join("scripts/orchestrate_v4_production_release.ps1"))?;
    for marker in [
        "ExpectedSourceSha",
        "Version",
        "Channel",
        "UpdaterPrivateKeyPath",
        "ApprovedSignerThumbprint",
        "updater-trust verify-private-key",
        "Redact-UpdaterVerifierOutput",
        "updater-trust verify-signature",
        "V4_QUALIFICATION_EVIDENCE.json",
        "V4_PRODUCTION_RELEASE_EVIDENCE.json",
        "sign_v4_authenticode.ps1",
        "cargo xtask authenticode verify",
        "unsigned-zero-budget",
        "Invoke-PrePackagingStaleOutputPurge",
        "[Pre-Packaging Purge] Stale candidate artifacts and evidence successfully purged: PASS",
        "cargo xtask ci nsis smoke",
        "--allow-protected-release-runner",
    ] {
        if !orchestrator.contains(marker) {
            return Err(format!(
                "v4 production release orchestrator is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let nsis_harness = fs::read_to_string(root.join("rust/xtask/src/nsis_harness.rs"))?;
    v4_production_calibration_gate_contract(&nsis_harness)?;
    let installed_calibration_verifier =
        fs::read_to_string(root.join("rust/xtask/src/calibration.rs"))?;
    for marker in [
        "expected calibration binary size must be positive",
        "installed calibration SHA-256 mismatch",
        "installed calibration executable is not x86_64",
        "Duration::from_secs(5)",
        "installed calibration --help exited with",
        "InstalledCalibrationEvidence",
        "successful_evidence_keeps_the_installed_calibration_schema",
        "installed_verification_fails_closed_for_missing_binary_invalid_size_digest_and_pe",
    ] {
        if !installed_calibration_verifier.contains(marker) {
            return Err(format!(
                "Rust production calibration verifier is missing its fail-closed check: {marker}"
            )
            .into());
        }
    }
    let evidence_builder = fs::read_to_string(root.join("scripts/v4_qualification_evidence.ps1"))?;
    for marker in [
        "New-V4CanonicalQualificationEvidence",
        "tauri-nsis-qualified-release",
        "install-launch-uninstall",
    ] {
        if !evidence_builder.contains(marker) {
            return Err(format!(
                "v4 qualification evidence builder is missing required marker: {marker}"
            )
            .into());
        }
    }
    let orchestrator_test =
        fs::read_to_string(root.join("scripts/test_v4_production_orchestrator.ps1"))?;
    for marker in [
        "[PASS] All V4 production orchestrator contract tests passed",
        "Parameter validation fails closed on missing parameters",
        "Source SHA mismatch fails closed before packaging",
        "Channel policy validation fails closed on invalid SemVer / channel",
        "Mutually exclusive provider configuration fails closed",
        "Wrong updater private key fails pre-flight verification before packaging",
        "Secret values are not emitted by expected error paths",
        "Inherited signing key environment fails closed",
        "Stale candidate artifacts and evidence are purged before packaging",
        "[Pre-Packaging Purge] Stale candidate artifacts and evidence successfully purged: PASS",
        "Stale-output purge did not execute strictly BEFORE pre-packaging updater key verification",
        "Updater signature verification rejects corrupted signature",
        "Tampered candidate binary is detected",
        "Production verification rejects CI test certificate",
    ] {
        if !orchestrator_test.contains(marker) {
            return Err(format!(
                "v4 production orchestrator test is missing its required marker: {marker}"
            )
            .into());
        }
    }
    let topology_doc = fs::read_to_string(root.join("docs/v4-release-execution-topology.md"))?;
    for marker in [
        "Build Once, Qualify Exact Bytes",
        "Runner Trust Boundaries and Key Custody",
        "V4_QUALIFICATION_EVIDENCE.json",
        "V4_PRODUCTION_RELEASE_EVIDENCE.json",
    ] {
        if !topology_doc.contains(marker) {
            return Err(format!(
                "v4 release execution topology documentation is missing required marker: {marker}"
            )
            .into());
        }
    }

    let private_begin = ["BEGIN", "PRIVATE", "KEY"].join(" ");
    let rsa_private_begin = ["BEGIN", "RSA", "PRIVATE", "KEY"].join(" ");
    let ec_private_begin = ["BEGIN", "EC", "PRIVATE", "KEY"].join(" ");
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            !entry.path().components().any(|component| {
                matches!(
                    component.as_os_str().to_str(),
                    Some(".git" | "target" | "node_modules" | "dist")
                )
            })
        })
    {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let filename = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if [".key", ".pem", ".pfx", ".p12"]
            .iter()
            .any(|suffix| filename.ends_with(suffix))
        {
            return Err(format!(
                "private signing material file is present in the repository tree: {}",
                path.display()
            )
            .into());
        }
        let bytes = fs::read(path)?;
        if is_tauri_minisign_private_key(&bytes) {
            return Err(format!(
                "Tauri/minisign private key material found at {}",
                path.display()
            )
            .into());
        }
        let Ok(content) = String::from_utf8(bytes) else {
            continue;
        };
        for (line_number, line) in content.lines().enumerate() {
            if line.contains(&private_begin)
                || line.contains(&rsa_private_begin)
                || line.contains(&ec_private_begin)
            {
                return Err(format!(
                    "private key material marker found at {}:{}",
                    path.display(),
                    line_number + 1
                )
                .into());
            }
            let secret_name = ["TAURI_SIGNING_PRIVATE", "_KEY"].concat();
            if line.contains(&secret_name)
                && ["Write-Host", "Write-Output", "echo", "Add-Content"]
                    .iter()
                    .any(|sink| line.contains(sink))
            {
                return Err(format!(
                    "signing secret is sent to a logging/output sink at {}:{}",
                    path.display(),
                    line_number + 1
                )
                .into());
            }
        }
    }
    println!("[xtask] v4 trust-material and secret-output guards: PASS");
    Ok(())
}

fn extract_rust_string_constant(source: &str, name: &str) -> Result<String> {
    let marker = format!("const {name}: &str = \"");
    let values = source
        .lines()
        .filter_map(|line| {
            let start = line.find(&marker)? + marker.len();
            let value = line.get(start..)?.split_once('"')?.0;
            Some(value.to_owned())
        })
        .collect::<Vec<_>>();
    match values.as_slice() {
        [value] => Ok(value.clone()),
        _ => Err(format!("Rust source must contain exactly one {name} string constant").into()),
    }
}

fn is_tauri_minisign_private_key(bytes: &[u8]) -> bool {
    let mut candidate = bytes.to_vec();
    for _ in 0..3 {
        if is_minisign_secret_text(&candidate) {
            return true;
        }
        let Ok(text) = std::str::from_utf8(&candidate) else {
            return false;
        };
        let Ok(decoded) = STANDARD.decode(text.trim()) else {
            return false;
        };
        candidate = decoded;
    }
    false
}

fn is_minisign_secret_text(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let lines = text.lines().collect::<Vec<_>>();
    if lines.len() != 2 {
        return false;
    }
    let comment = lines[0].to_ascii_lowercase();
    if !comment.starts_with("untrusted comment:")
        || (!comment.contains("secret key") && !comment.contains("private key"))
    {
        return false;
    }
    STANDARD
        .decode(lines[1])
        .map(|payload| (64..=1024).contains(&payload.len()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_release_calibration_gate_rejects_removal_or_post_evidence_move() {
        let source = include_str!("../nsis_harness.rs");
        v4_production_calibration_gate_contract(source)
            .expect("production release must verify staged and installed calibration");

        let install_start = source
            .find("run_installer(\n        installer,")
            .expect("installer invocation marker");
        let gate_start = source[install_start..]
            .find("verify_calibration_artifact(\n        repository_root,\n        &install_root")
            .map(|offset| install_start + offset)
            .expect("installed calibration gate marker");
        let launch_start = source[gate_start..]
            .find("launch_smoke(scope, &app")
            .map(|offset| gate_start + offset)
            .expect("application launch marker");
        let installed_gate = source[gate_start..launch_start].to_owned();

        let mut removed_gate = source.to_owned();
        removed_gate.replace_range(gate_start..launch_start, "");
        assert!(
            v4_production_calibration_gate_contract(&removed_gate).is_err(),
            "release contract must reject removal of installed calibration verification"
        );

        let mut moved_gate = removed_gate;
        let evidence_start = moved_gate
            .find("pub(crate) fn assert_uninstall_registration_absent")
            .expect("qualification evidence boundary");
        let evidence_line_end = moved_gate[evidence_start..]
            .find('\n')
            .map(|offset| evidence_start + offset + 1)
            .expect("qualification evidence line end");
        moved_gate.insert_str(evidence_line_end, &installed_gate);
        assert!(
            v4_production_calibration_gate_contract(&moved_gate).is_err(),
            "release contract must reject installed calibration verification moved after evidence"
        );
    }

    #[test]
    fn trust_guard_rejects_renamed_and_encoded_minisign_secret_keys() {
        let secret_payload = STANDARD.encode([0_u8; 158]);
        let secret_text =
            format!("untrusted comment: minisign encrypted secret key\n{secret_payload}");
        assert!(is_tauri_minisign_private_key(secret_text.as_bytes()));
        assert!(is_tauri_minisign_private_key(
            STANDARD.encode(&secret_text).as_bytes()
        ));

        let public_text = format!(
            "untrusted comment: minisign public key: fixture\n{}",
            STANDARD.encode([0_u8; 32])
        );
        assert!(!is_tauri_minisign_private_key(
            STANDARD.encode(public_text).as_bytes()
        ));

        let fixture = std::env::temp_dir().join(format!(
            "sky-xtask-renamed-secret-{}-{:?}.txt",
            std::process::id(),
            std::thread::current().id()
        ));
        fs::write(&fixture, STANDARD.encode(secret_text)).unwrap();
        let renamed_fixture = fs::read(&fixture).unwrap();
        assert!(is_tauri_minisign_private_key(&renamed_fixture));
        fs::remove_file(fixture).unwrap();
    }
}
