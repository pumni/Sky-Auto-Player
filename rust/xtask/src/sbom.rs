use crate::{Result, manifest, repo};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

const SPDX_VERSION: &str = "SPDX-2.3";
const DATA_LICENSE: &str = "CC0-1.0";
const ARTIFACT_SET_MARKER: &str = "artifact-set-sha256:";

#[derive(Debug, Clone, Eq, PartialEq)]
struct Artifact {
    name: String,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
}

pub fn generate(root: &Path, artifact_dir: &Path, output: &Path) -> Result<()> {
    let artifacts = collect_artifacts(artifact_dir)?;
    let artifact_set = artifact_set_sha256(&artifacts);
    let head = repo::git_head(root, false)?;
    let version = repo::project_version(root)?;
    let lockfile_sha256 = manifest::sha256(&root.join("rust/Cargo.lock"))?;
    let locked_packages = locked_packages(root)?;
    let files = artifacts
        .iter()
        .enumerate()
        .map(|(index, artifact)| {
            json!({
                "SPDXID": format!("SPDXRef-Artifact-{}", index + 1),
                "fileName": artifact.name,
                "checksums": [{
                    "algorithm": "SHA256",
                    "checksumValue": artifact.sha256,
                }],
                "fileSize": artifact.size,
                "licenseConcluded": "NOASSERTION",
                "copyrightText": "NOASSERTION",
            })
        })
        .collect::<Vec<_>>();
    let file_relationships = artifacts
        .iter()
        .enumerate()
        .map(|(index, _)| {
            json!({
                "spdxElementId": "SPDXRef-Package-SkyAutoPlayer",
                "relationshipType": "CONTAINS",
                "relatedSpdxElement": format!("SPDXRef-Artifact-{}", index + 1),
            })
        })
        .collect::<Vec<_>>();
    let mut packages = vec![json!({
        "SPDXID": "SPDXRef-Package-SkyAutoPlayer",
        "name": "Sky Auto Player",
        "versionInfo": version.clone(),
        "downloadLocation": "NOASSERTION",
        "filesAnalyzed": true,
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": "NOASSERTION",
        "copyrightText": "NOASSERTION",
    })];
    let mut package_relationships = Vec::new();
    for (index, package) in locked_packages.iter().enumerate() {
        let package_id = format!("SPDXRef-Package-Cargo-{}", index + 1);
        packages.push(json!({
            "SPDXID": package_id.clone(),
            "name": package.name,
            "versionInfo": package.version,
            "downloadLocation": package.source.as_deref().unwrap_or("NOASSERTION"),
            "filesAnalyzed": false,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
        }));
        package_relationships.push(json!({
            "spdxElementId": "SPDXRef-Package-SkyAutoPlayer",
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": package_id,
        }));
    }
    let mut relationships = file_relationships;
    relationships.extend(package_relationships);
    let document = json!({
        "spdxVersion": SPDX_VERSION,
        "dataLicense": DATA_LICENSE,
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("Sky Auto Player v{} canonical Windows candidate", version),
        "documentNamespace": format!(
            "https://github.com/pumni/Sky-Auto-Player/sbom/{head}/{artifact_set}/{lockfile_sha256}"
        ),
        "creationInfo": {
            "created": repo::commit_time_utc(root)?,
            "creators": ["Tool: sky_xtask"],
        },
        "documentDescribes": ["SPDXRef-Package-SkyAutoPlayer"],
        "packages": packages,
        "files": files,
        "relationships": relationships,
        "comment": format!(
            "{}{}; git-head:{}; lockfile-sha256:{}; candidate-artifact-directory:canonical-tauri-nsis",
            ARTIFACT_SET_MARKER, artifact_set, head, lockfile_sha256
        ),
    });
    let mut bytes = serde_json::to_vec_pretty(&document)?;
    bytes.push(b'\n');
    fs::write(output, bytes)?;
    println!(
        "[xtask] generated SPDX SBOM for {} exact artifact(s): {}",
        artifacts.len(),
        output.display()
    );
    Ok(())
}

pub fn verify(root: &Path, artifact_dir: &Path, sbom_path: &Path) -> Result<()> {
    let artifacts = collect_artifacts(artifact_dir)?;
    let payload: Value = serde_json::from_slice(&fs::read(sbom_path)?)?;
    validate_document(root, &artifacts, &payload)?;
    println!(
        "[xtask] verified SPDX SBOM against {} exact artifact(s): {}",
        artifacts.len(),
        sbom_path.display()
    );
    Ok(())
}

fn collect_artifacts(artifact_dir: &Path) -> Result<Vec<Artifact>> {
    if !artifact_dir.is_dir() {
        return Err(format!(
            "SBOM artifact directory is missing: {}",
            artifact_dir.display()
        )
        .into());
    }
    let mut artifacts = Vec::new();
    for entry in fs::read_dir(artifact_dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(format!(
                "SBOM artifact directory must contain only regular files: {}",
                entry.path().display()
            )
            .into());
        }
        let name = entry
            .file_name()
            .to_str()
            .ok_or("SBOM artifact filename is not valid UTF-8")?
            .to_owned();
        if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
            return Err(format!("SBOM artifact filename is unsafe: {name:?}").into());
        }
        let path = entry.path();
        artifacts.push(Artifact {
            name,
            size: fs::metadata(&path)?.len(),
            sha256: manifest::sha256(&path)?,
        });
    }
    artifacts.sort_by(|left, right| left.name.cmp(&right.name));
    if artifacts.is_empty() {
        return Err("SBOM artifact directory is empty".into());
    }
    Ok(artifacts)
}

fn artifact_set_sha256(artifacts: &[Artifact]) -> String {
    let mut digest = Sha256::new();
    for artifact in artifacts {
        digest.update(artifact.name.as_bytes());
        digest.update([0]);
        digest.update(artifact.sha256.as_bytes());
        digest.update([0]);
    }
    repo::hex_digest(digest.finalize())
}

fn validate_document(root: &Path, artifacts: &[Artifact], payload: &Value) -> Result<()> {
    if payload.get("spdxVersion").and_then(Value::as_str) != Some(SPDX_VERSION) {
        return Err("SBOM must be SPDX-2.3 JSON".into());
    }
    if payload.get("dataLicense").and_then(Value::as_str) != Some(DATA_LICENSE) {
        return Err("SBOM dataLicense must be CC0-1.0".into());
    }
    if payload.get("SPDXID").and_then(Value::as_str) != Some("SPDXRef-DOCUMENT") {
        return Err("SBOM document SPDXID is invalid".into());
    }
    let expected_set = artifact_set_sha256(artifacts);
    let comment = payload
        .get("comment")
        .and_then(Value::as_str)
        .ok_or("SBOM exact-artifact binding comment is missing")?;
    let expected_marker = format!("{ARTIFACT_SET_MARKER}{expected_set}");
    if !comment.contains(&expected_marker) {
        return Err("SBOM artifact-set SHA-256 does not match the candidate".into());
    }
    let head = repo::git_head(root, false)?;
    let lockfile_sha256 = manifest::sha256(&root.join("rust/Cargo.lock"))?;
    if !comment.contains(&format!("lockfile-sha256:{lockfile_sha256}")) {
        return Err("SBOM is not bound to the current Cargo.lock".into());
    }
    let namespace = payload
        .get("documentNamespace")
        .and_then(Value::as_str)
        .ok_or("SBOM documentNamespace is missing")?;
    if !namespace.contains(&head)
        || !namespace.contains(&expected_set)
        || !namespace.contains(&lockfile_sha256)
    {
        return Err("SBOM namespace is not bound to this source and artifact set".into());
    }
    let packages = payload
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("SBOM packages must be an array")?;
    let locked = locked_packages(root)?;
    if packages.len() != locked.len() + 1 {
        return Err("SBOM package set does not match Cargo.lock".into());
    }
    if packages[0].get("SPDXID").and_then(Value::as_str) != Some("SPDXRef-Package-SkyAutoPlayer") {
        return Err("SBOM product package is missing".into());
    }
    for (index, expected) in locked.iter().enumerate() {
        let package = &packages[index + 1];
        if package.get("SPDXID").and_then(Value::as_str)
            != Some(format!("SPDXRef-Package-Cargo-{}", index + 1).as_str())
            || package.get("name").and_then(Value::as_str) != Some(expected.name.as_str())
            || package.get("versionInfo").and_then(Value::as_str) != Some(expected.version.as_str())
        {
            return Err(format!(
                "SBOM Cargo package mismatch: {} {}",
                expected.name, expected.version
            )
            .into());
        }
    }
    let files = payload
        .get("files")
        .and_then(Value::as_array)
        .ok_or("SBOM files must be an array")?;
    let mut observed = BTreeMap::new();
    for file in files {
        let name = file
            .get("fileName")
            .and_then(Value::as_str)
            .ok_or("SBOM fileName is missing")?;
        let checksums = file
            .get("checksums")
            .and_then(Value::as_array)
            .ok_or("SBOM file checksums are missing")?;
        if checksums.len() != 1 {
            return Err(format!("SBOM file has unexpected checksum count: {name}").into());
        }
        let checksum = &checksums[0];
        if checksum.get("algorithm").and_then(Value::as_str) != Some("SHA256") {
            return Err(format!("SBOM file checksum algorithm is not SHA256: {name}").into());
        }
        let value = checksum
            .get("checksumValue")
            .and_then(Value::as_str)
            .ok_or("SBOM checksumValue is missing")?;
        if !is_sha256(value) {
            return Err(format!("SBOM checksumValue is not SHA-256: {name}").into());
        }
        if observed
            .insert(name.to_owned(), value.to_ascii_lowercase())
            .is_some()
        {
            return Err(format!("SBOM contains a duplicate file: {name}").into());
        }
    }
    if observed.len() != artifacts.len() {
        return Err("SBOM file set does not match the exact candidate artifact set".into());
    }
    for artifact in artifacts {
        if observed.get(&artifact.name) != Some(&artifact.sha256) {
            return Err(format!(
                "SBOM digest mismatch for candidate artifact: {}",
                artifact.name
            )
            .into());
        }
    }
    Ok(())
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn locked_packages(root: &Path) -> Result<Vec<LockedPackage>> {
    let lockfile: toml::Value = toml::from_str(&fs::read_to_string(root.join("rust/Cargo.lock"))?)?;
    let packages = lockfile
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or("Cargo.lock package list is missing")?;
    packages
        .iter()
        .map(|package| {
            let package = package
                .as_table()
                .ok_or("Cargo.lock package entry must be a table")?;
            Ok(LockedPackage {
                name: package
                    .get("name")
                    .and_then(toml::Value::as_str)
                    .ok_or("Cargo.lock package name is missing")?
                    .to_owned(),
                version: package
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .ok_or("Cargo.lock package version is missing")?
                    .to_owned(),
                source: package
                    .get("source")
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "sky-xtask-sbom-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    #[test]
    fn generated_spdx_binds_exact_files_and_rejects_tampering() {
        let root = temp_root();
        let _ = fs::remove_dir_all(&root);
        let artifacts = root.join("bundle");
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join("candidate.exe"), b"candidate").unwrap();
        fs::write(artifacts.join("candidate.exe.sig"), b"signature").unwrap();
        let output = root.join("SBOM.spdx.json");
        generate(&crate::repo::root(), &artifacts, &output).unwrap();
        verify(&crate::repo::root(), &artifacts, &output).unwrap();
        fs::write(artifacts.join("candidate.exe"), b"tampered").unwrap();
        assert!(verify(&crate::repo::root(), &artifacts, &output).is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn sha256_validation_is_strict() {
        assert!(is_sha256(&"a".repeat(64)));
        assert!(!is_sha256(&"g".repeat(64)));
        assert!(!is_sha256(&"a".repeat(63)));
    }
}
