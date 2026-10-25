#![forbid(unsafe_code)]

use crate::Result;
use crate::ci_artifacts::{
    StagingDirectory, direct_files, pe_machine, require_directory, require_regular_file,
    sha256_file, validate_commit_sha, validate_digest, validate_public_key, validate_safe_filename,
    validate_semver, write_new_file,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const PUBLIC_KEY_FILE: &str = "updater-public-key.pub";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CandidateManifest {
    schema_version: u32,
    source_sha: String,
    version: String,
    calibration_binary: String,
    calibration_binary_size: u64,
    calibration_binary_sha256: String,
    calibration_binary_architecture: String,
    installer: String,
    installer_sha256: String,
    updater_signature: String,
    updater_signature_sha256: String,
    updater_public_key: String,
    updater_public_key_sha256: String,
}

#[derive(Debug, Eq, PartialEq)]
struct CandidateContract {
    manifest: CandidateManifest,
    installer_sha256: String,
    signature_sha256: String,
    public_key_sha256: String,
}

fn project_version(root: &Path) -> Result<String> {
    let manifest_path = root.join("desktop/src-tauri/Cargo.toml");
    let source = fs::read_to_string(&manifest_path).map_err(|error| {
        format!(
            "canonical desktop manifest is missing or unreadable: {}: {error}",
            manifest_path.display()
        )
    })?;
    let manifest: toml::Value = toml::from_str(&source)
        .map_err(|error| format!("canonical desktop manifest is invalid TOML: {error}"))?;
    let version = manifest
        .get("package")
        .and_then(toml::Value::as_table)
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or("canonical desktop manifest must contain exactly one package version")?;
    validate_semver(version, "canonical desktop package version")?;
    Ok(version.to_owned())
}

fn read_manifest(path: &Path) -> Result<CandidateManifest> {
    let bytes = fs::read(path)
        .map_err(|error| format!("candidate.json is missing or unreadable: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("candidate.json is not the locked contract: {error}").into())
}

fn read_candidate(
    root: &Path,
    expected_source: &str,
    expected_version: &str,
) -> Result<CandidateContract> {
    require_directory(root, "candidate directory is missing")?;
    let root = fs::canonicalize(root)
        .map_err(|error| format!("candidate directory is missing: {error}"))?;
    let files = direct_files(&root, "candidate directory is missing")?;
    let metadata_path = root.join("candidate.json");
    if !metadata_path.is_file() {
        return Err("candidate.json is missing".into());
    }
    if files.len() != 4 {
        return Err(format!(
            "candidate artifact must contain exactly four files; found {}",
            files.len()
        )
        .into());
    }
    let manifest = read_manifest(&metadata_path)?;
    if manifest.schema_version != 2 {
        return Err(format!(
            "unsupported candidate contract schema: {}",
            manifest.schema_version
        )
        .into());
    }
    let expected_source = validate_commit_sha(expected_source, "expected source SHA")?;
    let observed_source = validate_commit_sha(&manifest.source_sha, "candidate source SHA")?;
    if observed_source != expected_source {
        return Err("candidate source SHA does not match the checked-out source".into());
    }
    validate_semver(&manifest.version, "candidate version")?;
    if manifest.version != expected_version {
        return Err("candidate version does not match the checked-out source version".into());
    }
    if manifest.calibration_binary != "native_calibration.exe" {
        return Err(
            "candidate calibration binary filename is not the locked executable name".into(),
        );
    }
    if manifest.calibration_binary_size == 0 {
        return Err("candidate calibration binary size must be positive".into());
    }
    let calibration_sha = validate_digest(
        &manifest.calibration_binary_sha256,
        "candidate calibration binary SHA-256",
    )?;
    if manifest.calibration_binary_architecture != "x86_64" {
        return Err("candidate calibration binary architecture is not x86_64".into());
    }

    validate_safe_filename(&manifest.installer, "installer")?;
    validate_safe_filename(&manifest.updater_signature, "updater_signature")?;
    validate_safe_filename(&manifest.updater_public_key, "updater_public_key")?;
    if !manifest
        .installer
        .to_ascii_lowercase()
        .ends_with("-setup.exe")
    {
        return Err("candidate installer is not an NSIS setup executable".into());
    }
    if manifest.updater_signature != format!("{}.sig", manifest.installer) {
        return Err("candidate updater signature is not the exact installer pair".into());
    }
    if manifest.updater_public_key != PUBLIC_KEY_FILE {
        return Err("candidate public key filename is not the locked public-key filename".into());
    }
    let installer_digest = validate_digest(&manifest.installer_sha256, "installer_sha256")?;
    let signature_digest = validate_digest(
        &manifest.updater_signature_sha256,
        "updater_signature_sha256",
    )?;
    let public_key_digest = validate_digest(
        &manifest.updater_public_key_sha256,
        "updater_public_key_sha256",
    )?;

    let expected_names = BTreeSet::from([
        "candidate.json".to_owned(),
        manifest.installer.clone(),
        manifest.updater_signature.clone(),
        PUBLIC_KEY_FILE.to_owned(),
    ]);
    let observed_names = files
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .ok_or_else(|| "candidate artifact contains a non-Unicode filename".into())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if observed_names != expected_names {
        return Err(format!(
            "candidate artifact contains unexpected files: {}",
            observed_names.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }

    let installer_path = root.join(&manifest.installer);
    let signature_path = root.join(&manifest.updater_signature);
    let public_key_path = root.join(PUBLIC_KEY_FILE);
    let observed_installer = sha256_file(&installer_path)?;
    let observed_signature = sha256_file(&signature_path)?;
    let observed_public_key = sha256_file(&public_key_path)?;
    if !observed_installer.eq_ignore_ascii_case(&installer_digest) {
        return Err("installer SHA-256 does not match candidate.json".into());
    }
    if !observed_signature.eq_ignore_ascii_case(&signature_digest) {
        return Err("updater signature SHA-256 does not match candidate.json".into());
    }
    if !observed_public_key.eq_ignore_ascii_case(&public_key_digest) {
        return Err("updater public-key SHA-256 does not match candidate.json".into());
    }
    validate_public_key(&public_key_path)?;

    Ok(CandidateContract {
        manifest: CandidateManifest {
            source_sha: observed_source,
            calibration_binary_sha256: calibration_sha,
            installer_sha256: installer_digest,
            updater_signature_sha256: signature_digest,
            updater_public_key_sha256: public_key_digest,
            ..manifest
        },
        installer_sha256: observed_installer,
        signature_sha256: observed_signature,
        public_key_sha256: observed_public_key,
    })
}

fn calibration_details(path: &Path) -> Result<(u64, String)> {
    let metadata = require_regular_file(path, "calibration binary")?;
    if path.file_name().and_then(|name| name.to_str()) != Some("native_calibration.exe")
        || metadata.len() == 0
    {
        return Err("calibration binary must be a non-empty native_calibration.exe".into());
    }
    if pe_machine(path, "calibration binary")? != 0x8664 {
        return Err("calibration binary must be an x86_64 PE executable".into());
    }
    Ok((metadata.len(), sha256_file(path)?))
}

fn create_candidate(
    bundle_dir: &Path,
    public_key_path: &Path,
    calibration_path: &Path,
    output_root: &Path,
    source_sha: &str,
    repository_root: &Path,
) -> Result<CandidateContract> {
    let source_sha = validate_commit_sha(source_sha, "source SHA")?;
    let version = project_version(repository_root)?;
    let (calibration_size, calibration_sha) = calibration_details(calibration_path)?;
    require_directory(bundle_dir, "candidate bundle is missing")?;
    let bundle_dir = fs::canonicalize(bundle_dir)
        .map_err(|error| format!("candidate bundle is missing: {error}"))?;
    let bundle_files = direct_files(&bundle_dir, "candidate bundle is missing")?;
    let installers = bundle_files
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with("-setup.exe"))
        })
        .collect::<Vec<_>>();
    if installers.len() != 1 || bundle_files.len() != 2 {
        return Err("candidate bundle must contain exactly one installer and one signature".into());
    }
    let installer_path = installers[0];
    let installer = installer_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("candidate installer filename is not Unicode")?
        .to_owned();
    validate_safe_filename(&installer, "installer")?;
    let signature = format!("{installer}.sig");
    let signature_path = bundle_dir.join(&signature);
    require_regular_file(&signature_path, "candidate updater signature")?;
    require_regular_file(public_key_path, "candidate public key")?;
    let public_key_path = fs::canonicalize(public_key_path)
        .map_err(|error| format!("candidate public key is missing: {error}"))?;
    validate_public_key(&public_key_path)?;

    let staging = StagingDirectory::create(output_root)?;
    let staged_installer = staging.path().join(&installer);
    fs::copy(installer_path, &staged_installer)?;
    fs::copy(&signature_path, staging.path().join(&signature))?;
    fs::copy(&public_key_path, staging.path().join(PUBLIC_KEY_FILE))?;
    let manifest = CandidateManifest {
        schema_version: 2,
        source_sha: source_sha.clone(),
        version: version.clone(),
        calibration_binary: "native_calibration.exe".to_owned(),
        calibration_binary_size: calibration_size,
        calibration_binary_sha256: calibration_sha,
        calibration_binary_architecture: "x86_64".to_owned(),
        installer,
        installer_sha256: sha256_file(&staged_installer)?,
        updater_signature: signature,
        updater_signature_sha256: sha256_file(
            &staging.path().join(signature_path.file_name().unwrap()),
        )?,
        updater_public_key: PUBLIC_KEY_FILE.to_owned(),
        updater_public_key_sha256: sha256_file(&staging.path().join(PUBLIC_KEY_FILE))?,
    };
    let manifest_bytes = serde_json::to_vec(&manifest)?;
    write_new_file(&staging.path().join("candidate.json"), &manifest_bytes)?;
    let contract = read_candidate(staging.path(), &source_sha, &version)?;
    staging.commit(output_root)?;
    Ok(contract)
}

fn print_contract(contract: &CandidateContract) {
    println!(
        "CI candidate contract: PASS (source={}; version={}; installer={}; signature={}; public_key={})",
        contract.manifest.source_sha,
        contract.manifest.version,
        contract.installer_sha256,
        contract.signature_sha256,
        contract.public_key_sha256,
    );
}

fn parse_options(
    args: &[String],
    allowed: &[&str],
) -> Result<std::collections::BTreeMap<String, String>> {
    let mut options = std::collections::BTreeMap::new();
    let mut index = 0;
    while index < args.len() {
        let name = args[index]
            .strip_prefix("--")
            .ok_or_else(|| format!("candidate option must start with --: {}", args[index]))?;
        if !allowed.contains(&name) {
            return Err(format!("unknown candidate option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if options.insert(name.to_owned(), value).is_some() {
            return Err(format!("candidate option was provided more than once: --{name}").into());
        }
        index += 1;
    }
    Ok(options)
}

fn required<'a>(
    options: &'a std::collections::BTreeMap<String, String>,
    name: &str,
) -> Result<&'a str> {
    options
        .get(name)
        .map(String::as_str)
        .ok_or_else(|| format!("candidate requires --{name}").into())
}

pub(crate) fn run(repository_root: &Path, args: &[String]) -> Result<()> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or("candidate requires create or validate")?;
    let allowed: &[&str] = match operation {
        "create" => &[
            "bundle-dir",
            "public-key",
            "calibration-binary",
            "output-root",
            "source-sha",
            "repository-root",
        ],
        "validate" => &["candidate-root", "source-sha", "repository-root"],
        _ => return Err("candidate requires create or validate".into()),
    };
    let options = parse_options(&args[1..], allowed)?;
    let root = options
        .get("repository-root")
        .map(PathBuf::from)
        .unwrap_or_else(|| repository_root.to_owned());
    let contract = match operation {
        "create" => create_candidate(
            Path::new(required(&options, "bundle-dir")?),
            Path::new(required(&options, "public-key")?),
            Path::new(required(&options, "calibration-binary")?),
            Path::new(required(&options, "output-root")?),
            required(&options, "source-sha")?,
            &root,
        )?,
        "validate" => {
            let version = project_version(&root)?;
            read_candidate(
                Path::new(required(&options, "candidate-root")?),
                required(&options, "source-sha")?,
                &version,
            )?
        }
        _ => return Err("candidate requires create or validate".into()),
    };
    print_contract(&contract);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ci_artifacts;

    fn temp_dir(name: &str) -> PathBuf {
        let path = ci_artifacts::unique_sibling(&std::env::temp_dir().join(name), "test");
        fs::create_dir(&path).unwrap();
        path
    }

    fn valid_pe() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x86];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_i32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes
    }

    fn setup(root: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let bundle = root.join("bundle");
        fs::create_dir(&bundle).unwrap();
        let version = project_version(&crate::repo::root()).unwrap();
        let installer = format!("Sky Auto Player_{version}_x64-setup.exe");
        fs::write(bundle.join(&installer), b"fixture installer bytes").unwrap();
        fs::write(
            bundle.join(format!("{installer}.sig")),
            b"fixture signature bytes",
        )
        .unwrap();
        let public_key = root.join("updater.pub");
        fs::write(&public_key, "dGVzdC1wdWJsaWMta2V5\n").unwrap();
        let calibration = root.join("native_calibration.exe");
        fs::write(&calibration, valid_pe()).unwrap();
        (bundle, public_key, calibration, root.join("candidate"))
    }

    fn create(root: &Path, source: &str) -> Result<CandidateContract> {
        let (bundle, public_key, calibration, output) = setup(root);
        create_candidate(
            &bundle,
            &public_key,
            &calibration,
            &output,
            source,
            &crate::repo::root(),
        )
    }

    #[test]
    fn create_and_validate_preserve_candidate_v2_schema_and_exact_inventory() {
        let root = temp_dir("sky-xtask-candidate-positive");
        let contract = create(&root, "1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(contract.manifest.schema_version, 2);
        assert_eq!(
            contract.manifest.source_sha,
            "1234567890abcdef1234567890abcdef12345678"
        );
        let output = root.join("candidate");
        let manifest_text = fs::read_to_string(output.join("candidate.json")).unwrap();
        let fields = serde_json::from_str::<serde_json::Value>(&manifest_text).unwrap();
        assert_eq!(fields.as_object().unwrap().len(), 13);
        assert_eq!(fs::read_dir(&output).unwrap().count(), 4);
        let reread = read_candidate(
            &output,
            "1234567890abcdef1234567890abcdef12345678",
            &project_version(&crate::repo::root()).unwrap(),
        )
        .unwrap();
        assert_eq!(reread.manifest, contract.manifest);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_tampered_installer_signature_public_key_and_source_identity() {
        let root = temp_dir("sky-xtask-candidate-tamper");
        create(&root, "1234567890abcdef1234567890abcdef12345678").unwrap();
        let candidate = root.join("candidate");
        let manifest: CandidateManifest = read_manifest(&candidate.join("candidate.json")).unwrap();
        let installer = candidate.join(&manifest.installer);
        let original = fs::read(&installer).unwrap();
        fs::write(&installer, b"tampered installer").unwrap();
        assert!(
            read_candidate(
                &candidate,
                "1234567890abcdef1234567890abcdef12345678",
                &manifest.version
            )
            .unwrap_err()
            .to_string()
            .contains("installer SHA-256")
        );
        fs::write(&installer, original).unwrap();

        assert!(
            read_candidate(
                &candidate,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &manifest.version
            )
            .unwrap_err()
            .to_string()
            .contains("source SHA")
        );
        assert!(
            read_candidate(
                &candidate,
                "1234567890abcdef1234567890abcdef12345678",
                "0.0.0"
            )
            .unwrap_err()
            .to_string()
            .contains("version")
        );
        let signature_path = candidate.join(&manifest.updater_signature);
        let signature = fs::read(&signature_path).unwrap();
        fs::write(&signature_path, b"corrupt signature").unwrap();
        assert!(
            read_candidate(
                &candidate,
                "1234567890abcdef1234567890abcdef12345678",
                &manifest.version
            )
            .unwrap_err()
            .to_string()
            .contains("updater signature SHA-256")
        );
        fs::write(&signature_path, signature).unwrap();
        fs::write(candidate.join(PUBLIC_KEY_FILE), "PRIVATE KEY").unwrap();
        assert!(
            read_candidate(
                &candidate,
                "1234567890abcdef1234567890abcdef12345678",
                &manifest.version
            )
            .unwrap_err()
            .to_string()
            .contains("updater public-key SHA-256")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_wrong_machine_malformed_pe_traversal_and_unexpected_inventory() {
        let root = temp_dir("sky-xtask-candidate-negative");
        let (_bundle, _public_key, calibration, output) = setup(&root);
        let wrong_machine = {
            let mut bytes = valid_pe();
            bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
            bytes
        };
        fs::write(&calibration, wrong_machine).unwrap();
        assert!(
            create_candidate(
                &root.join("bundle"),
                &root.join("updater.pub"),
                &calibration,
                &output,
                "1234567890abcdef1234567890abcdef12345678",
                &crate::repo::root(),
            )
            .unwrap_err()
            .to_string()
            .contains("x86_64 PE")
        );
        assert!(!output.exists());

        fs::write(&calibration, valid_pe()).unwrap();
        create_candidate(
            &root.join("bundle"),
            &root.join("updater.pub"),
            &calibration,
            &output,
            "1234567890abcdef1234567890abcdef12345678",
            &crate::repo::root(),
        )
        .unwrap();
        let candidate = root.join("candidate");
        fs::write(candidate.join("unexpected.txt"), b"extra").unwrap();
        assert!(
            read_candidate(
                &candidate,
                "1234567890abcdef1234567890abcdef12345678",
                &project_version(&crate::repo::root()).unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("exactly four files")
        );
        fs::remove_file(candidate.join("unexpected.txt")).unwrap();
        let path = candidate.join("candidate.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        manifest["installer"] = serde_json::Value::String("../escape.exe".to_owned());
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(
            read_candidate(
                &candidate,
                "1234567890abcdef1234567890abcdef12345678",
                &project_version(&crate::repo::root()).unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("safe artifact-relative")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_candidate_manifest_fields() {
        let root = temp_dir("sky-xtask-candidate-unknown-field");
        create(&root, "1234567890abcdef1234567890abcdef12345678").unwrap();
        let candidate = root.join("candidate");
        let manifest_path = candidate.join("candidate.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["future_field"] = serde_json::Value::String("not in schema v2".to_owned());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = read_candidate(
            &candidate,
            "1234567890abcdef1234567890abcdef12345678",
            &project_version(&crate::repo::root()).unwrap(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("unknown field"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn candidate_creation_rolls_back_output_on_error_and_rejects_extra_files() {
        let root = temp_dir("sky-xtask-candidate-rollback");
        let (bundle, public_key, calibration, output) = setup(&root);
        fs::write(bundle.join("extra.txt"), b"unexpected").unwrap();
        assert!(
            create_candidate(
                &bundle,
                &public_key,
                &calibration,
                &output,
                "1234567890abcdef1234567890abcdef12345678",
                &crate::repo::root(),
            )
            .is_err()
        );
        assert!(!output.exists());
        fs::remove_file(bundle.join("extra.txt")).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn public_key_and_safe_filename_rules_reject_private_material_and_paths() {
        let root = temp_dir("sky-xtask-candidate-key");
        let key = root.join("key.pub");
        let private_marker = ["BEGIN", "PRIVATE", "KEY"].join(" ");
        fs::write(&key, format!("-----{private_marker}-----")).unwrap();
        assert!(validate_public_key(&key).is_err());
        for name in [
            "../installer.exe",
            "C:\\installer.exe",
            "folder/name.exe",
            "bad..name.exe",
        ] {
            assert!(validate_safe_filename(name, "test").is_err(), "{name}");
        }
        let unknown = vec![
            "--candidate-root".to_owned(),
            "candidate".to_owned(),
            "--ignore-hashes".to_owned(),
            "true".to_owned(),
        ];
        assert!(
            parse_options(&unknown, &["candidate-root"])
                .unwrap_err()
                .to_string()
                .contains("unknown candidate option")
        );
        let duplicate = vec![
            "--source-sha".to_owned(),
            "a".repeat(40),
            "--source-sha".to_owned(),
            "b".repeat(40),
        ];
        assert!(
            parse_options(&duplicate, &["source-sha"])
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn candidate_source_identity_is_bound_to_git_head() {
        let root = crate::repo::root();
        let head =
            crate::process::capture_text("git", &["rev-parse", "--verify", "HEAD"], &root, &[])
                .unwrap();
        let version = project_version(&root).unwrap();
        let identity =
            crate::ci::candidate_source_identity(&root, &version, Some(&head), false).unwrap();
        assert_eq!(identity.source_sha, head);
        assert_eq!(identity.version, version);
        assert!(
            crate::ci::candidate_source_identity(
                &root,
                &version,
                Some("0000000000000000000000000000000000000000"),
                false
            )
            .is_err()
        );
    }
}
