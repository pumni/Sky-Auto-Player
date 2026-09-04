use crate::{Result, manifest, repo, sbom, version};
use serde::Serialize;
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG_PATH: &str = "desktop/src-tauri/tauri.conf.json";
const V4_IDENTIFIER: &str = "io.github.pumni.skyautoplayer";
const PRODUCT_NAME: &str = "Sky Auto Player";
const NSIS_TARGET: &str = "nsis";
const CURRENT_USER_INSTALL_MODE: &str = "currentUser";
const WINDOWS_ARCH: &str = "x64";
// Public Tauri updater trust material. The matching private key is generated
// and stored outside the repository by the release operator.
pub const V4_TAURI_UPDATER_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEY2MzU1MjYwQTBDNjYzRDUKUldUVlk4YWdZRkkxOWdWRnNkRTNVY0habzA0YlQ4OFkxZk42WEM3OGVnSW5WNlc5SHlSbGF3QWEK";

#[derive(Debug, Serialize)]
struct ArtifactSummary {
    schema_version: u32,
    evidence_type: &'static str,
    product_name: &'static str,
    identifier: &'static str,
    version: String,
    target: &'static str,
    install_mode: &'static str,
    installer: String,
    updater_signature: String,
    installer_size: u64,
    signature_size: u64,
    installer_sha256: String,
    updater_signature_sha256: String,
}

fn object<'a>(value: &'a Value, key: &str) -> Result<&'a serde_json::Map<String, Value>> {
    value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| format!("Tauri config field {key} must be an object").into())
}

fn bool_field(value: &serde_json::Map<String, Value>, key: &str) -> Result<bool> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("Tauri config field {key} must be a boolean").into())
}

fn string_field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Tauri config field {key} must be a string").into())
}

fn validate_config_value(config: &Value, project_version: &str) -> Result<()> {
    let parsed_version = version::parse(project_version)?;
    if parsed_version.to_string() != project_version {
        return Err(
            format!("Cargo project version is not canonical SemVer: {project_version:?}").into(),
        );
    }
    if config.get("version").is_some() {
        return Err(
            "Tauri config must omit version; desktop/src-tauri/Cargo.toml is the canonical v4 version source".into(),
        );
    }
    if string_field(config, "productName")? != PRODUCT_NAME {
        return Err("Tauri productName does not match the v4 package contract".into());
    }
    if string_field(config, "identifier")? != V4_IDENTIFIER {
        return Err("Tauri identifier does not match ADR-0006".into());
    }

    let bundle = object(config, "bundle")?;
    if !bool_field(bundle, "active")? {
        return Err("Tauri bundling must be enabled for the v4 package".into());
    }
    if !bool_field(bundle, "createUpdaterArtifacts")? {
        return Err("Tauri updater artifacts must be enabled for the v4 package".into());
    }
    let targets = bundle
        .get("targets")
        .and_then(Value::as_array)
        .ok_or("Tauri bundle.targets must be an array containing only nsis")?;
    if targets.len() != 1 || targets[0].as_str() != Some(NSIS_TARGET) {
        return Err("Tauri bundle.targets must contain only nsis for v4.0".into());
    }
    let windows = bundle
        .get("windows")
        .and_then(Value::as_object)
        .ok_or("Tauri bundle.windows must be configured")?;
    let sign_command = windows
        .get("signCommand")
        .and_then(Value::as_str)
        .ok_or("Tauri bundle.windows.signCommand must be configured")?;
    if !sign_command.starts_with("pwsh ")
        || !sign_command.contains("sign_v4_authenticode.ps1")
        || !sign_command.contains("%1")
        || sign_command.contains(['&', '|', ';', '`'])
    {
        return Err(
            "Tauri Windows signing must use the fail-closed v4 Authenticode provider seam".into(),
        );
    }
    let nsis = windows
        .get("nsis")
        .and_then(Value::as_object)
        .ok_or("Tauri bundle.windows.nsis must be configured")?;
    if nsis.get("installMode").and_then(Value::as_str) != Some(CURRENT_USER_INSTALL_MODE) {
        return Err("Tauri NSIS installMode must be currentUser".into());
    }

    let updater = config
        .get("plugins")
        .and_then(Value::as_object)
        .and_then(|plugins| plugins.get("updater"))
        .and_then(Value::as_object)
        .ok_or("Tauri updater configuration must contain the v4 public trust root")?;
    let public_key = updater
        .get("pubkey")
        .and_then(Value::as_str)
        .ok_or("Tauri updater v4 public key is missing")?;
    if public_key != V4_TAURI_UPDATER_PUBLIC_KEY || !valid_public_key(public_key) {
        return Err(
            "Tauri updater public key is missing, malformed, or is not the independent v4 root"
                .into(),
        );
    }
    if updater.contains_key("endpoints") {
        return Err("Tauri updater endpoints are Rust-owned and must not be checked in".into());
    }
    if updater
        .get("dangerousInsecureTransportProtocol")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err("production Tauri updater transport must remain HTTPS-only".into());
    }
    Ok(())
}

fn valid_public_key(value: &str) -> bool {
    value.len() <= 4096
        && !value.is_empty()
        && !value.contains("release-2026")
        && !value.contains("PRIVATE KEY")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
}

pub fn validate_config(root: &Path) -> Result<()> {
    let config_path = root.join(CONFIG_PATH);
    let config: Value = serde_json::from_slice(&fs::read(&config_path)?)?;
    let project_version = repo::project_version(root)?;
    validate_config_value(&config, &project_version)
        .map_err(|error| format!("{}: {error}", config_path.display()).into())
}

fn is_installer(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase().ends_with("-setup.exe"))
        .unwrap_or(false)
}

fn signature_path(installer: &Path) -> PathBuf {
    let name = installer
        .file_name()
        .expect("installer path must have a filename")
        .to_string_lossy();
    installer.with_file_name(format!("{name}.sig"))
}

fn artifact_summary(bundle_dir: &Path, project_version: &str) -> Result<ArtifactSummary> {
    let bundle_dir = bundle_dir.canonicalize()?;
    if !bundle_dir.is_dir() {
        return Err(format!(
            "Tauri bundle directory is missing: {}",
            bundle_dir.display()
        )
        .into());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(&bundle_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!(
                "Tauri bundle directory contains a symlink: {}",
                entry.path().display()
            )
            .into());
        }
        if !file_type.is_file() {
            return Err(format!(
                "Tauri bundle directory contains an unexpected non-file: {}",
                entry.path().display()
            )
            .into());
        }
        files.push(entry.path());
    }
    files.sort();

    let installers = files
        .iter()
        .filter(|path| is_installer(path))
        .collect::<Vec<_>>();
    if installers.len() != 1 {
        return Err(format!(
            "expected exactly one Tauri NSIS setup executable, found {}",
            installers.len()
        )
        .into());
    }
    let installer = installers[0];
    let signature = signature_path(installer);
    if !files.iter().any(|path| path == &signature) {
        return Err(format!(
            "Tauri updater signature is missing: {}",
            signature.display()
        )
        .into());
    }
    if files.len() != 2 {
        let names = files
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        return Err(format!(
            "Tauri NSIS bundle must contain only the setup executable and its .sig: {names:?}"
        )
        .into());
    }
    if installer.metadata()?.len() == 0 {
        return Err("Tauri NSIS installer is empty".into());
    }
    let signature_text = String::from_utf8(fs::read(&signature)?)?;
    if signature_text.trim().is_empty() {
        return Err("Tauri updater signature is empty".into());
    }
    let installer_name = installer.file_name().unwrap().to_string_lossy();
    let expected_name = format!("{PRODUCT_NAME}_{project_version}_{WINDOWS_ARCH}-setup.exe");
    if installer_name != expected_name {
        return Err(format!(
            "Tauri installer name does not match canonical product/version {expected_name}: {installer_name}"
        )
        .into());
    }

    Ok(ArtifactSummary {
        schema_version: 2,
        evidence_type: "tauri-nsis-artifact",
        product_name: PRODUCT_NAME,
        identifier: V4_IDENTIFIER,
        version: project_version.to_owned(),
        target: NSIS_TARGET,
        install_mode: CURRENT_USER_INSTALL_MODE,
        installer: installer_name.into_owned(),
        updater_signature: signature
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned(),
        installer_size: installer.metadata()?.len(),
        signature_size: signature.metadata()?.len(),
        installer_sha256: manifest::sha256(installer)?,
        updater_signature_sha256: manifest::sha256(&signature)?,
    })
}

pub fn verify(
    root: &Path,
    bundle_dir: &Path,
    summary_path: Option<&Path>,
    authenticode_evidence_path: Option<&Path>,
    sbom_path: Option<&Path>,
) -> Result<()> {
    validate_config(root)?;
    let project_version = repo::project_version(root)?;
    let summary = artifact_summary(bundle_dir, &project_version)?;
    let authenticode_evidence_path = authenticode_evidence_path
        .ok_or("canonical Tauri qualification requires Authenticode evidence")?;
    validate_authenticode_evidence(authenticode_evidence_path, &summary)?;
    let sbom_path = sbom_path.ok_or("canonical Tauri qualification requires an SPDX SBOM")?;
    sbom::verify(root, bundle_dir, sbom_path)?;
    let payload = serde_json::to_vec_pretty(&json!(summary))?;
    if let Some(path) = summary_path {
        fs::write(path, &payload)?;
        println!("[xtask] Tauri artifact summary: {}", path.display());
    }
    println!("{}", String::from_utf8(payload)?);
    Ok(())
}

fn validate_authenticode_evidence(path: &Path, summary: &ArtifactSummary) -> Result<()> {
    let payload: Value = serde_json::from_slice(&fs::read(path)?)?;
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .ok_or("Authenticode evidence mode is missing")?;
    let expected_thumbprint = expected_authenticode_signer_thumbprint(mode)?;
    validate_authenticode_evidence_value(&payload, summary, mode, &expected_thumbprint)
}

fn expected_authenticode_signer_thumbprint(mode: &str) -> Result<String> {
    let variable = match mode {
        "test" => "SKY_AUTHENTICODE_TEST_THUMBPRINT",
        "production" => "SKY_AUTHENTICODE_APPROVED_SIGNER_THUMBPRINT",
        _ => return Err("Authenticode evidence mode is invalid".into()),
    };
    let thumbprint = std::env::var(variable)
        .map_err(|_| format!("Authenticode verification requires {variable}"))?
        .trim()
        .to_ascii_uppercase();
    if thumbprint.len() != 40
        || !thumbprint
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("{variable} must be a 40-character SHA-1 thumbprint").into());
    }
    Ok(thumbprint)
}

fn is_rejected_authenticode_status(status: &str) -> bool {
    matches!(
        status,
        "NotSigned"
            | "HashMismatch"
            | "Incompatible"
            | "NotSupported"
            | "PublisherMismatch"
            | "Error"
    )
}

fn validate_authenticode_evidence_value(
    payload: &Value,
    summary: &ArtifactSummary,
    mode: &str,
    expected_thumbprint: &str,
) -> Result<()> {
    if payload.get("schema_version").and_then(Value::as_u64) != Some(1)
        || payload.get("evidence_type").and_then(Value::as_str) != Some("authenticode-verification")
    {
        return Err("Authenticode evidence schema is invalid".into());
    }
    if !matches!(mode, "test" | "production") {
        return Err("Authenticode evidence mode is invalid".into());
    }
    if payload
        .get("expected_signer_thumbprint")
        .and_then(Value::as_str)
        .map(|value| value.eq_ignore_ascii_case(expected_thumbprint))
        != Some(true)
    {
        return Err(
            "Authenticode evidence approved signer identity is missing or mismatched".into(),
        );
    }
    let files = payload
        .get("files")
        .and_then(Value::as_array)
        .ok_or("Authenticode evidence files are missing")?;
    if files.len() != 1 {
        return Err("canonical Tauri bundle evidence must cover exactly one installer".into());
    }
    let file = &files[0];
    if file.get("name").and_then(Value::as_str) != Some(summary.installer.as_str()) {
        return Err("Authenticode evidence does not cover the canonical installer".into());
    }
    let status = file
        .get("status")
        .and_then(Value::as_str)
        .ok_or("canonical installer Authenticode status is missing")?;
    if status.is_empty() {
        return Err("canonical installer Authenticode status is empty".into());
    }
    let platform_status = file
        .get("platform_status")
        .and_then(Value::as_str)
        .ok_or("canonical installer platform Authenticode status is missing")?;
    if status != platform_status {
        return Err(
            "canonical installer Authenticode status disagrees with platform status".into(),
        );
    }
    let verification = file
        .get("verification")
        .and_then(Value::as_str)
        .ok_or("canonical installer Authenticode verification result is missing")?;
    if verification != "signature-valid-independent-cryptographic-integrity"
        || file.get("integrity_verifier").and_then(Value::as_str)
            != Some("signedcms-spc-indirect-data-authenticode-hash")
        || file.get("integrity_status").and_then(Value::as_str) != Some("Valid")
        || file
            .get("signed_digest")
            .and_then(Value::as_str)
            .zip(file.get("computed_digest").and_then(Value::as_str))
            .map(|(signed, computed)| signed.eq_ignore_ascii_case(computed))
            != Some(true)
    {
        return Err(
            "canonical installer Authenticode cryptographic integrity proof is invalid".into(),
        );
    }
    let trust_exception = file.get("trust_exception");
    if is_rejected_authenticode_status(platform_status) {
        return Err(format!(
            "canonical installer platform Authenticode status is not accepted: {platform_status}"
        )
        .into());
    }
    match platform_status {
        "Valid"
            if verification == "signature-valid-independent-cryptographic-integrity"
                && trust_exception.map(Value::is_null).unwrap_or(true) => {}
        _ if mode == "test"
            && trust_exception.and_then(Value::as_str)
                == Some("test-platform-status-not-used-for-integrity") => {}
        _ => {
            return Err(format!(
                "canonical installer Authenticode status is not accepted: {status}"
            )
            .into());
        }
    }
    if file
        .get("signer_thumbprint")
        .and_then(Value::as_str)
        .map(|value| value.eq_ignore_ascii_case(expected_thumbprint))
        != Some(true)
    {
        return Err(
            "canonical installer Authenticode signer identity is missing or mismatched".into(),
        );
    }
    if file
        .get("sha256")
        .and_then(Value::as_str)
        .map(|value| value.eq_ignore_ascii_case(&summary.installer_sha256))
        != Some(true)
    {
        return Err("Authenticode evidence installer digest does not match the candidate".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> Value {
        json!({
            "productName": PRODUCT_NAME,
            "identifier": V4_IDENTIFIER,
            "build": {},
            "bundle": {
                "active": true,
                "targets": [NSIS_TARGET],
                "createUpdaterArtifacts": true,
                "windows": {
                    "signCommand": "pwsh -File ../../scripts/sign_v4_authenticode.ps1 %1",
                    "nsis": {"installMode": CURRENT_USER_INSTALL_MODE}
                }
            },
            "plugins": {"updater": {"pubkey": V4_TAURI_UPDATER_PUBLIC_KEY}}
        })
    }

    #[test]
    fn config_contract_requires_the_canonical_v4_shape() {
        assert!(validate_config_value(&valid_config(), "4.0.0-alpha.1").is_ok());

        for (field, replacement) in [
            ("identifier", json!("com.skyautoplayer.desktop")),
            ("productName", json!("Other App")),
        ] {
            let mut config = valid_config();
            config[field] = replacement;
            assert!(
                validate_config_value(&config, "4.0.0-alpha.1").is_err(),
                "{field}"
            );
        }

        let mut config = valid_config();
        config["version"] = json!("4.0.0-alpha.1");
        assert!(validate_config_value(&config, "4.0.0-alpha.1").is_err());
    }

    #[test]
    fn config_contract_rejects_non_nsis_or_elevated_installers() {
        let mut config = valid_config();
        config["bundle"]["targets"] = json!(["nsis", "msi"]);
        assert!(validate_config_value(&config, "4.0.0-alpha.1").is_err());

        let mut config = valid_config();
        config["bundle"]["windows"]["nsis"]["installMode"] = json!("both");
        assert!(validate_config_value(&config, "4.0.0-alpha.1").is_err());
    }

    #[test]
    fn config_contract_rejects_unowned_v4_updater_configuration() {
        let mut config = valid_config();
        config["plugins"] = json!({"updater": {"endpoints": ["https://example.invalid"]}});
        assert!(validate_config_value(&config, "4.0.0-alpha.1").is_err());
    }

    #[test]
    fn config_contract_rejects_the_legacy_update_root_and_signing_bypass() {
        let mut config = valid_config();
        config["plugins"]["updater"]["pubkey"] = json!("release-2026");
        assert!(validate_config_value(&config, "4.0.0-alpha.1").is_err());

        let mut config = valid_config();
        config["bundle"]["windows"]["signCommand"] = json!("true %1");
        assert!(validate_config_value(&config, "4.0.0-alpha.1").is_err());
    }

    #[test]
    fn artifact_verifier_requires_setup_executable_and_signature_pair() {
        let root =
            std::env::temp_dir().join(format!("sky-xtask-tauri-bundle-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let installer = root.join("Sky Auto Player_4.0.0-alpha.1_x64-setup.exe");
        let signature = root.join("Sky Auto Player_4.0.0-alpha.1_x64-setup.exe.sig");
        fs::write(&installer, b"installer").unwrap();
        fs::write(&signature, b"test-signature\n").unwrap();
        let summary = artifact_summary(&root, "4.0.0-alpha.1").unwrap();
        assert_eq!(
            summary.installer,
            installer.file_name().unwrap().to_string_lossy()
        );

        fs::remove_file(&installer).unwrap();
        fs::remove_file(&signature).unwrap();
        let wrong_installer = root.join("Sky Auto Player_4.0.0-alpha.10_x64-setup.exe");
        let wrong_signature = root.join("Sky Auto Player_4.0.0-alpha.10_x64-setup.exe.sig");
        fs::write(&wrong_installer, b"installer").unwrap();
        fs::write(&wrong_signature, b"test-signature\n").unwrap();
        assert!(artifact_summary(&root, "4.0.0-alpha.1").is_err());

        fs::remove_file(wrong_installer).unwrap();
        fs::remove_file(wrong_signature).unwrap();
        fs::write(
            root.join("Sky Auto Player_4.0.0-alpha.1_x64-setup.msi"),
            b"wrong target",
        )
        .unwrap();
        assert!(artifact_summary(&root, "4.0.0-alpha.1").is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn authenticode_evidence_binds_approved_signer_and_installer_bytes() {
        let root = std::env::temp_dir().join(format!(
            "sky-xtask-tauri-authenticode-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let installer = root.join("Sky Auto Player_4.0.0-alpha.1_x64-setup.exe");
        let signature = root.join("Sky Auto Player_4.0.0-alpha.1_x64-setup.exe.sig");
        fs::write(&installer, b"installer").unwrap();
        fs::write(&signature, b"test-signature\n").unwrap();
        let summary = artifact_summary(&root, "4.0.0-alpha.1").unwrap();
        let evidence_path = root.join("authenticode.json");
        fs::write(
            &evidence_path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "evidence_type": "authenticode-verification",
                "mode": "test",
                "expected_signer_thumbprint": "A".repeat(40),
                "files": [{
                    "name": summary.installer.clone(),
                    "status": "Valid",
                    "platform_status": "Valid",
                    "verification": "signature-valid-independent-cryptographic-integrity",
                    "trust_exception": null,
                    "integrity_verifier": "signedcms-spc-indirect-data-authenticode-hash",
                    "integrity_status": "Valid",
                    "signed_digest": "A".repeat(64),
                    "computed_digest": "a".repeat(64),
                    "signer_thumbprint": "A".repeat(40),
                    "sha256": summary.installer_sha256.clone(),
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&fs::read(&evidence_path).unwrap()).unwrap();
        validate_authenticode_evidence_value(&payload, &summary, "test", &"A".repeat(40)).unwrap();

        let untrusted_payload = json!({
            "schema_version": 1,
            "evidence_type": "authenticode-verification",
            "mode": "test",
            "expected_signer_thumbprint": "A".repeat(40),
            "files": [{
                "name": summary.installer.clone(),
                "status": "NotTrusted",
                "platform_status": "NotTrusted",
                "verification": "signature-valid-independent-cryptographic-integrity",
                "trust_exception": "test-platform-status-not-used-for-integrity",
                "integrity_verifier": "signedcms-spc-indirect-data-authenticode-hash",
                "integrity_status": "Valid",
                "signed_digest": "A".repeat(64),
                "computed_digest": "a".repeat(64),
                "signer_thumbprint": "A".repeat(40),
                "sha256": summary.installer_sha256.clone(),
            }]
        });
        validate_authenticode_evidence_value(&untrusted_payload, &summary, "test", &"A".repeat(40))
            .unwrap();
        assert!(
            validate_authenticode_evidence_value(
                &untrusted_payload,
                &summary,
                "production",
                &"A".repeat(40),
            )
            .is_err()
        );

        let mut untrusted_without_exception = untrusted_payload.clone();
        untrusted_without_exception["files"][0]["trust_exception"] = Value::Null;
        assert!(
            validate_authenticode_evidence_value(
                &untrusted_without_exception,
                &summary,
                "test",
                &"A".repeat(40),
            )
            .is_err()
        );

        let mut unsupported_status = untrusted_payload.clone();
        unsupported_status["files"][0]["status"] = json!("NotSigned");
        unsupported_status["files"][0]["platform_status"] = json!("NotSigned");
        assert!(
            validate_authenticode_evidence_value(
                &unsupported_status,
                &summary,
                "test",
                &"A".repeat(40),
            )
            .is_err()
        );
        fs::write(
            &evidence_path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "evidence_type": "authenticode-verification",
                "mode": "test",
                "expected_signer_thumbprint": "A".repeat(40),
                "files": [{
                    "name": summary.installer.clone(),
                    "status": "Valid",
                    "platform_status": "Valid",
                    "verification": "signature-valid-independent-cryptographic-integrity",
                    "trust_exception": null,
                    "integrity_verifier": "signedcms-spc-indirect-data-authenticode-hash",
                    "integrity_status": "Valid",
                    "signed_digest": "A".repeat(64),
                    "computed_digest": "a".repeat(64),
                    "signer_thumbprint": "B".repeat(40),
                    "sha256": summary.installer_sha256.clone(),
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&fs::read(&evidence_path).unwrap()).unwrap();
        assert!(
            validate_authenticode_evidence_value(&payload, &summary, "test", &"A".repeat(40))
                .is_err()
        );
        fs::write(
            &evidence_path,
            serde_json::to_vec(&json!({
                "schema_version": 1,
                "evidence_type": "authenticode-verification",
                "mode": "test",
                "expected_signer_thumbprint": "A".repeat(40),
                "files": [{
                    "name": summary.installer.clone(),
                    "status": "Valid",
                    "platform_status": "Valid",
                    "verification": "signature-valid-independent-cryptographic-integrity",
                    "trust_exception": null,
                    "integrity_verifier": "signedcms-spc-indirect-data-authenticode-hash",
                    "integrity_status": "Valid",
                    "signed_digest": "A".repeat(64),
                    "computed_digest": "a".repeat(64),
                    "signer_thumbprint": "A".repeat(40),
                    "sha256": "0".repeat(64),
                }]
            }))
            .unwrap(),
        )
        .unwrap();
        let payload: Value = serde_json::from_slice(&fs::read(&evidence_path).unwrap()).unwrap();
        assert!(
            validate_authenticode_evidence_value(&payload, &summary, "test", &"A".repeat(40))
                .is_err()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
