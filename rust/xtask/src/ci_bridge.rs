#![forbid(unsafe_code)]

use crate::Result;
use crate::ci_artifacts::{
    StagingDirectory, direct_files, require_directory, require_regular_file, sha256_file,
    validate_commit_sha, validate_digest, validate_safe_filename, validate_semver, write_new_file,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct BridgeManifest {
    schema_version: u32,
    source_sha: String,
    version: String,
    publisher: String,
    identifier: String,
    installer: String,
    installer_sha256: String,
    sentinel_id: String,
    sentinel_content_sha256: String,
}

#[derive(Clone, Debug, Default)]
struct BridgeBindings {
    version: Option<String>,
    publisher: Option<String>,
    identifier: Option<String>,
    sentinel_id: Option<String>,
    sentinel_content_sha256: Option<String>,
}

#[derive(Clone, Debug)]
struct BridgeProducer {
    source_sha: String,
    version: String,
    publisher: String,
    identifier: String,
    sentinel_id: String,
    sentinel_content_sha256: String,
}

#[derive(Debug, Eq, PartialEq)]
struct BridgeContract {
    manifest: BridgeManifest,
    installer_sha256: String,
}

fn validate_publisher(value: &str) -> Result<()> {
    let mut chars = value.chars();
    let valid_first = chars
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric());
    if !valid_first
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'.' | b'_' | b'-'))
    {
        return Err(format!("bridge publisher is not bounded: {value}").into());
    }
    Ok(())
}

fn validate_identifier(value: &str) -> Result<()> {
    let valid_first = value
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric());
    if !valid_first
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!("bridge identifier is not bounded: {value}").into());
    }
    Ok(())
}

fn validate_sentinel(value: &str, name: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.encode_utf16().count() > 256
        || value.contains('\0')
        || value
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
    {
        return Err(format!("{name} is not a bounded stable sentinel ID").into());
    }
    Ok(())
}

fn manifest_path(root: &Path) -> PathBuf {
    root.join("bridge.json")
}

fn read_manifest(path: &Path) -> Result<BridgeManifest> {
    let bytes =
        fs::read(path).map_err(|error| format!("bridge.json is missing or unreadable: {error}"))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("bridge.json is not valid locked JSON: {error}").into())
}

fn read_bridge(root: &Path, source_sha: &str, bindings: &BridgeBindings) -> Result<BridgeContract> {
    require_directory(root, "bridge artifact directory is missing")?;
    let root = fs::canonicalize(root)
        .map_err(|error| format!("bridge artifact directory is missing: {error}"))?;
    let files = direct_files(&root, "bridge artifact directory is missing")?;
    if files.len() != 2 {
        return Err(format!(
            "bridge artifact must contain exactly two files; found {}",
            files.len()
        )
        .into());
    }
    for path in &files {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("bridge artifact contains a non-Unicode filename")?;
        let folded = name.to_ascii_lowercase();
        if folded.contains("private key")
            || folded.contains("secret key")
            || folded.ends_with(".sig")
            || folded.ends_with(".pub")
            || folded.ends_with(".key")
        {
            return Err(
                "bridge artifact contains private-key material or a detached signature".into(),
            );
        }
    }
    let metadata_path = manifest_path(&root);
    require_regular_file(&metadata_path, "bridge.json")?;
    let manifest = read_manifest(&metadata_path)?;
    if manifest.schema_version != 1 {
        return Err(format!(
            "unsupported bridge contract schema: {}",
            manifest.schema_version
        )
        .into());
    }
    let source_sha = validate_commit_sha(source_sha, "expected source SHA")?;
    let manifest_source = validate_commit_sha(&manifest.source_sha, "bridge source SHA")?;
    if source_sha != manifest_source {
        return Err("bridge source SHA does not match the checked-out source".into());
    }
    validate_semver(&manifest.version, "bridge version")?;
    if let Some(expected) = &bindings.version {
        validate_semver(expected, "expected bridge version")?;
        if manifest.version != *expected {
            return Err("bridge version does not match the producer/consumer binding".into());
        }
    }
    validate_publisher(&manifest.publisher)?;
    validate_identifier(&manifest.identifier)?;
    if let Some(expected) = &bindings.publisher
        && manifest.publisher != *expected
    {
        return Err("bridge publisher does not match the producer/consumer binding".into());
    }
    if let Some(expected) = &bindings.identifier
        && manifest.identifier != *expected
    {
        return Err("bridge identifier does not match the producer/consumer binding".into());
    }
    validate_safe_filename(&manifest.installer, "installer")?;
    if !manifest
        .installer
        .to_ascii_lowercase()
        .ends_with("-setup.exe")
    {
        return Err("bridge installer is not an NSIS setup executable".into());
    }
    let installer_digest = validate_digest(&manifest.installer_sha256, "bridge installer_sha256")?;
    validate_sentinel(&manifest.sentinel_id, "sentinel_id")?;
    let sentinel_digest = validate_digest(
        &manifest.sentinel_content_sha256,
        "bridge sentinel_content_sha256",
    )?;
    if let Some(expected) = &bindings.sentinel_id {
        validate_sentinel(expected, "expected sentinel_id")?;
        if manifest.sentinel_id != *expected {
            return Err("bridge sentinel ID does not match the producer/consumer binding".into());
        }
    }
    if let Some(expected) = &bindings.sentinel_content_sha256 {
        let expected = validate_digest(expected, "expected sentinel content SHA-256")?;
        if !sentinel_digest.eq_ignore_ascii_case(&expected) {
            return Err(
                "bridge sentinel content SHA does not match the producer/consumer binding".into(),
            );
        }
    }

    let installer_path = root.join(&manifest.installer);
    require_regular_file(&installer_path, "bridge installer")?;
    let expected_names = BTreeSet::from(["bridge.json".to_owned(), manifest.installer.clone()]);
    let observed_names = files
        .iter()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_owned)
                .ok_or_else(|| "bridge artifact contains a non-Unicode filename".into())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if observed_names != expected_names {
        return Err(format!(
            "bridge artifact contains unexpected files: {}",
            observed_names.into_iter().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    let observed_digest = sha256_file(&installer_path)?;
    if !observed_digest.eq_ignore_ascii_case(&installer_digest) {
        return Err("bridge installer SHA-256 does not match bridge.json".into());
    }
    Ok(BridgeContract {
        manifest: BridgeManifest {
            source_sha: manifest_source,
            installer_sha256: installer_digest,
            sentinel_content_sha256: sentinel_digest,
            ..manifest
        },
        installer_sha256: observed_digest,
    })
}

fn create_bridge(
    bundle_dir: &Path,
    output_root: &Path,
    producer: &BridgeProducer,
) -> Result<BridgeContract> {
    let source_sha = validate_commit_sha(&producer.source_sha, "source SHA")?;
    validate_semver(&producer.version, "bridge version")?;
    validate_publisher(&producer.publisher)?;
    validate_identifier(&producer.identifier)?;
    validate_sentinel(&producer.sentinel_id, "sentinel_id")?;
    let sentinel_content_sha256 =
        validate_digest(&producer.sentinel_content_sha256, "sentinel content SHA")?;
    require_directory(bundle_dir, "bridge build bundle is missing")?;
    let bundle_dir = fs::canonicalize(bundle_dir)
        .map_err(|error| format!("bridge build bundle is missing: {error}"))?;
    let bundle_files = direct_files(&bundle_dir, "bridge build bundle is missing")?;
    let installers = bundle_files
        .iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().ends_with("-setup.exe"))
        })
        .collect::<Vec<_>>();
    if installers.len() != 1 {
        return Err("bridge build bundle must contain exactly one NSIS installer".into());
    }
    let source_installer = installers[0];
    let installer = source_installer
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("bridge installer filename is not Unicode")?
        .to_owned();
    validate_safe_filename(&installer, "installer")?;

    let staging = StagingDirectory::create(output_root)?;
    fs::copy(source_installer, staging.path().join(&installer))?;
    let manifest = BridgeManifest {
        schema_version: 1,
        source_sha,
        version: producer.version.clone(),
        publisher: producer.publisher.clone(),
        identifier: producer.identifier.clone(),
        installer: installer.clone(),
        installer_sha256: sha256_file(&staging.path().join(&installer))?,
        sentinel_id: producer.sentinel_id.clone(),
        sentinel_content_sha256,
    };
    write_new_file(
        &manifest_path(staging.path()),
        &serde_json::to_vec(&manifest)?,
    )?;
    let bindings = BridgeBindings {
        version: Some(producer.version.clone()),
        publisher: Some(producer.publisher.clone()),
        identifier: Some(producer.identifier.clone()),
        sentinel_id: Some(producer.sentinel_id.clone()),
        sentinel_content_sha256: Some(manifest.sentinel_content_sha256.clone()),
    };
    let contract = read_bridge(staging.path(), &manifest.source_sha, &bindings)?;
    staging.commit(output_root)?;
    Ok(contract)
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
            .ok_or_else(|| format!("bridge option must start with --: {}", args[index]))?;
        if !allowed.contains(&name) {
            return Err(format!("unknown bridge option: --{name}").into());
        }
        index += 1;
        let value = args
            .get(index)
            .ok_or_else(|| format!("missing value for --{name}"))?
            .clone();
        if options.insert(name.to_owned(), value).is_some() {
            return Err(format!("bridge option was provided more than once: --{name}").into());
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
        .ok_or_else(|| format!("bridge requires --{name}").into())
}

fn optional_bindings(options: &std::collections::BTreeMap<String, String>) -> BridgeBindings {
    BridgeBindings {
        version: options.get("version").cloned(),
        publisher: options.get("publisher").cloned(),
        identifier: options.get("identifier").cloned(),
        sentinel_id: options.get("sentinel-id").cloned(),
        sentinel_content_sha256: options.get("sentinel-content-sha256").cloned(),
    }
}

pub(crate) fn run(args: &[String]) -> Result<()> {
    let operation = args
        .first()
        .map(String::as_str)
        .ok_or("bridge requires create or validate")?;
    let allowed: &[&str] = match operation {
        "create" => &[
            "bundle-dir",
            "output-root",
            "source-sha",
            "version",
            "publisher",
            "identifier",
            "sentinel-id",
            "sentinel-content-sha256",
        ],
        "validate" => &[
            "bridge-root",
            "source-sha",
            "version",
            "publisher",
            "identifier",
            "sentinel-id",
            "sentinel-content-sha256",
        ],
        _ => return Err("bridge requires create or validate".into()),
    };
    let options = parse_options(&args[1..], allowed)?;
    let contract = match operation {
        "create" => create_bridge(
            Path::new(required(&options, "bundle-dir")?),
            Path::new(required(&options, "output-root")?),
            &BridgeProducer {
                source_sha: required(&options, "source-sha")?.to_owned(),
                version: required(&options, "version")?.to_owned(),
                publisher: required(&options, "publisher")?.to_owned(),
                identifier: required(&options, "identifier")?.to_owned(),
                sentinel_id: required(&options, "sentinel-id")?.to_owned(),
                sentinel_content_sha256: required(&options, "sentinel-content-sha256")?.to_owned(),
            },
        )?,
        "validate" => read_bridge(
            Path::new(required(&options, "bridge-root")?),
            required(&options, "source-sha")?,
            &optional_bindings(&options),
        )?,
        _ => return Err("bridge requires create or validate".into()),
    };
    println!(
        "CI updater-bridge contract: PASS (source={}; version={}; publisher={}; identifier={}; installer={}; sentinel={}; sentinel_sha256={})",
        contract.manifest.source_sha,
        contract.manifest.version,
        contract.manifest.publisher,
        contract.manifest.identifier,
        contract.installer_sha256,
        contract.manifest.sentinel_id,
        contract.manifest.sentinel_content_sha256,
    );
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

    fn setup(root: &Path) -> (PathBuf, PathBuf) {
        let bundle = root.join("bundle");
        fs::create_dir(&bundle).unwrap();
        fs::write(
            bundle.join("Sky-Auto-Player_4.0.0-alpha.1_x64-setup.exe"),
            [1, 2, 3, 4],
        )
        .unwrap();
        (bundle, root.join("bridge"))
    }

    fn create(root: &Path) -> BridgeContract {
        let (bundle, output) = setup(root);
        create_bridge(&bundle, &output, &producer()).unwrap()
    }

    fn producer() -> BridgeProducer {
        BridgeProducer {
            source_sha: "1234567890abcdef1234567890abcdef12345678".to_owned(),
            version: "4.0.0-alpha.1".to_owned(),
            publisher: "github".to_owned(),
            identifier: "io.github.pumni.skyautoplayer".to_owned(),
            sentinel_id: "catalog-sentinel".to_owned(),
            sentinel_content_sha256:
                "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_owned(),
        }
    }

    fn bindings() -> BridgeBindings {
        BridgeBindings {
            version: Some("4.0.0-alpha.1".to_owned()),
            publisher: Some("github".to_owned()),
            identifier: Some("io.github.pumni.skyautoplayer".to_owned()),
            sentinel_id: Some("catalog-sentinel".to_owned()),
            sentinel_content_sha256: Some(
                "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd".to_owned(),
            ),
        }
    }

    #[test]
    fn create_and_validate_preserve_bridge_v1_schema_and_exact_files() {
        let root = temp_dir("sky-xtask-bridge-positive");
        let contract = create(&root);
        let output = root.join("bridge");
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("bridge.json")).unwrap()).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 9);
        assert_eq!(value["schema_version"], 1);
        assert_eq!(fs::read_dir(&output).unwrap().count(), 2);
        assert_eq!(
            read_bridge(&output, &contract.manifest.source_sha, &bindings()).unwrap(),
            contract
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_tampered_installer_wrong_source_and_expected_binding_mismatch() {
        let root = temp_dir("sky-xtask-bridge-tamper");
        let contract = create(&root);
        let output = root.join("bridge");
        let path = output.join(&contract.manifest.installer);
        let original = fs::read(&path).unwrap();
        fs::write(&path, [9, 9, 9]).unwrap();
        assert!(
            read_bridge(&output, &contract.manifest.source_sha, &bindings())
                .unwrap_err()
                .to_string()
                .contains("installer SHA-256")
        );
        fs::write(&path, original).unwrap();
        assert!(
            read_bridge(
                &output,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                &bindings()
            )
            .unwrap_err()
            .to_string()
            .contains("source SHA")
        );
        let mut wrong_binding = bindings();
        wrong_binding.identifier = Some("io.example.other".to_owned());
        assert!(
            read_bridge(&output, &contract.manifest.source_sha, &wrong_binding)
                .unwrap_err()
                .to_string()
                .contains("producer/consumer binding")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_extra_private_material_signature_paths_and_unknown_json_fields() {
        let root = temp_dir("sky-xtask-bridge-negative");
        let contract = create(&root);
        let output = root.join("bridge");
        fs::write(output.join("leaked-private.key"), "PRIVATE KEY").unwrap();
        assert!(
            read_bridge(&output, &contract.manifest.source_sha, &bindings())
                .unwrap_err()
                .to_string()
                .contains("exactly two files")
        );
        fs::remove_file(output.join("leaked-private.key")).unwrap();
        fs::write(output.join("detached.sig"), b"signature").unwrap();
        assert!(
            read_bridge(&output, &contract.manifest.source_sha, &bindings())
                .unwrap_err()
                .to_string()
                .contains("exactly two files")
        );
        fs::remove_file(output.join("detached.sig")).unwrap();

        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(output.join("bridge.json")).unwrap()).unwrap();
        manifest["installer"] = serde_json::Value::String("../escape.exe".to_owned());
        fs::write(
            output.join("bridge.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(
            read_bridge(&output, &contract.manifest.source_sha, &bindings())
                .unwrap_err()
                .to_string()
                .contains("safe artifact-relative")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_failure_leaves_no_partial_bridge_directory() {
        let root = temp_dir("sky-xtask-bridge-rollback");
        let (bundle, output) = setup(&root);
        fs::write(bundle.join("second-setup.exe"), [4, 3, 2, 1]).unwrap();
        assert!(create_bridge(&bundle, &output, &producer()).is_err());
        assert!(!output.exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn operation_options_reject_unknown_names_and_duplicates() {
        let args = vec![
            "--bridge-root".to_owned(),
            "bridge".to_owned(),
            "--force".to_owned(),
            "true".to_owned(),
        ];
        let error = parse_options(&args, &["bridge-root", "source-sha"]).unwrap_err();
        assert!(error.to_string().contains("unknown bridge option"));
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
    }
}
