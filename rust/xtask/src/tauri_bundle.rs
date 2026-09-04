use crate::{Result, manifest, repo, version};
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
    let nsis = windows
        .get("nsis")
        .and_then(Value::as_object)
        .ok_or("Tauri bundle.windows.nsis must be configured")?;
    if nsis.get("installMode").and_then(Value::as_str) != Some(CURRENT_USER_INSTALL_MODE) {
        return Err("Tauri NSIS installMode must be currentUser".into());
    }

    if let Some(updater) = config
        .get("plugins")
        .and_then(Value::as_object)
        .and_then(|plugins| plugins.get("updater"))
        && updater.as_object().is_some_and(|updater| {
            updater.contains_key("endpoints") || updater.contains_key("pubkey")
        })
    {
        return Err(
            "checked-in Tauri updater endpoints and trust keys are forbidden: endpoints are Rust-owned and the v4 trust root belongs to WO-05".into(),
        );
    }
    Ok(())
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

pub fn verify(root: &Path, bundle_dir: &Path, summary_path: Option<&Path>) -> Result<()> {
    validate_config(root)?;
    let project_version = repo::project_version(root)?;
    let summary = artifact_summary(bundle_dir, &project_version)?;
    let payload = serde_json::to_vec_pretty(&json!(summary))?;
    if let Some(path) = summary_path {
        fs::write(path, &payload)?;
        println!("[xtask] Tauri artifact summary: {}", path.display());
    }
    println!("{}", String::from_utf8(payload)?);
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
                "windows": {"nsis": {"installMode": CURRENT_USER_INSTALL_MODE}}
            }
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
}
