use crate::{Result, hash, repo};
use semver::{Version, VersionReq};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Debug, Clone, Eq, PartialEq)]
struct FrontendPackage {
    key: String,
    name: String,
    version: String,
    integrity: String,
    dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FrontendGraph {
    packages: Vec<FrontendPackage>,
    root_packages: Vec<String>,
    edges: Vec<(String, String)>,
}

impl FrontendGraph {
    fn package_index(&self, key: &str) -> Result<usize> {
        self.packages
            .iter()
            .position(|package| package.key == key)
            .ok_or_else(|| format!("frontend package is missing from its graph: {key}").into())
    }
}

fn frontend_package_id(index: usize) -> String {
    format!("SPDXRef-Package-Npm-{}", index + 1)
}

fn npm_purl(package: &FrontendPackage) -> String {
    format!(
        "pkg:npm/{}@{}",
        package.name.replace('@', "%40").replace('/', "%2F"),
        package.version
    )
}

pub fn generate(root: &Path, artifact_dir: &Path, output: &Path) -> Result<()> {
    let artifacts = collect_artifacts(artifact_dir)?;
    let artifact_set = artifact_set_sha256(&artifacts);
    let head = repo::git_head(root, false)?;
    let version = repo::project_version(root)?;
    let cargo_lockfile_sha256 = hash::sha256(&root.join("rust/Cargo.lock"))?;
    let bun_lockfile_sha256 = hash::sha256(&root.join("desktop/bun.lock"))?;
    let locked_packages = locked_packages(root)?;
    let frontend = frontend_graph(root)?;
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
    for (index, package) in frontend.packages.iter().enumerate() {
        let package_id = frontend_package_id(index);
        packages.push(json!({
            "SPDXID": package_id,
            "name": package.name,
            "versionInfo": package.version,
            "downloadLocation": "NOASSERTION",
            "filesAnalyzed": false,
            "licenseConcluded": "NOASSERTION",
            "licenseDeclared": "NOASSERTION",
            "copyrightText": "NOASSERTION",
            "externalRefs": [{
                "referenceCategory": "PACKAGE-MANAGER",
                "referenceType": "purl",
                "referenceLocator": npm_purl(package),
            }],
            "comment": format!("bun-lock-key:{}; integrity:{}", package.key, package.integrity),
        }));
    }
    for package_key in &frontend.root_packages {
        let package_id = frontend_package_id(frontend.package_index(package_key)?);
        package_relationships.push(json!({
            "spdxElementId": "SPDXRef-Package-SkyAutoPlayer",
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": package_id,
        }));
    }
    for (from, to) in &frontend.edges {
        package_relationships.push(json!({
            "spdxElementId": frontend_package_id(frontend.package_index(from)?),
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": frontend_package_id(frontend.package_index(to)?),
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
            "https://github.com/pumni/Sky-Auto-Player/sbom/{head}/{artifact_set}/{cargo_lockfile_sha256}/{bun_lockfile_sha256}"
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
            "{}{}; git-head:{}; cargo-lockfile-sha256:{}; bun-lockfile-sha256:{}; candidate-artifact-directory:canonical-tauri-nsis",
            ARTIFACT_SET_MARKER, artifact_set, head, cargo_lockfile_sha256, bun_lockfile_sha256
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
    let locked_packages = locked_packages(root)?;
    let frontend = frontend_graph(root)?;
    let payload: Value = serde_json::from_slice(&fs::read(sbom_path)?)?;
    validate_document(root, &artifacts, &locked_packages, &frontend, &payload)?;
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
            sha256: hash::sha256(&path)?,
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

fn validate_document(
    root: &Path,
    artifacts: &[Artifact],
    locked_packages: &[LockedPackage],
    frontend: &FrontendGraph,
    payload: &Value,
) -> Result<()> {
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
    let cargo_lockfile_sha256 = hash::sha256(&root.join("rust/Cargo.lock"))?;
    let bun_lockfile_sha256 = hash::sha256(&root.join("desktop/bun.lock"))?;
    if !comment.contains(&format!("cargo-lockfile-sha256:{cargo_lockfile_sha256}")) {
        return Err("SBOM is not bound to the current Cargo.lock".into());
    }
    if !comment.contains(&format!("bun-lockfile-sha256:{bun_lockfile_sha256}")) {
        return Err("SBOM is not bound to the current desktop/bun.lock".into());
    }
    let namespace = payload
        .get("documentNamespace")
        .and_then(Value::as_str)
        .ok_or("SBOM documentNamespace is missing")?;
    if !namespace.contains(&head)
        || !namespace.contains(&expected_set)
        || !namespace.contains(&cargo_lockfile_sha256)
        || !namespace.contains(&bun_lockfile_sha256)
    {
        return Err("SBOM namespace is not bound to this source and artifact set".into());
    }
    let packages = payload
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("SBOM packages must be an array")?;
    if packages.len() != locked_packages.len() + frontend.packages.len() + 1 {
        return Err("SBOM package set does not match the dependency lockfiles".into());
    }
    if packages[0].get("SPDXID").and_then(Value::as_str) != Some("SPDXRef-Package-SkyAutoPlayer") {
        return Err("SBOM product package is missing".into());
    }
    for (index, expected) in locked_packages.iter().enumerate() {
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
    let frontend_offset = locked_packages.len() + 1;
    for (index, expected) in frontend.packages.iter().enumerate() {
        let package = &packages[frontend_offset + index];
        if package.get("SPDXID").and_then(Value::as_str)
            != Some(frontend_package_id(index).as_str())
            || package.get("name").and_then(Value::as_str) != Some(expected.name.as_str())
            || package.get("versionInfo").and_then(Value::as_str) != Some(expected.version.as_str())
            || package
                .get("externalRefs")
                .and_then(Value::as_array)
                .and_then(|refs| refs.first())
                .and_then(|reference| reference.get("referenceLocator"))
                .and_then(Value::as_str)
                != Some(npm_purl(expected).as_str())
            || package
                .get("comment")
                .and_then(Value::as_str)
                .map(|comment| {
                    comment.contains(&format!("bun-lock-key:{}", expected.key))
                        && comment.contains(&format!("integrity:{}", expected.integrity))
                })
                != Some(true)
        {
            return Err(format!(
                "SBOM frontend package mismatch: {} {}",
                expected.name, expected.version
            )
            .into());
        }
    }
    let expected_relationships = expected_relationships(artifacts, locked_packages, frontend)?;
    if payload.get("relationships") != Some(&Value::Array(expected_relationships)) {
        return Err("SBOM dependency/artifact relationships do not match the candidate".into());
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

fn frontend_graph(root: &Path) -> Result<FrontendGraph> {
    let source = fs::read_to_string(root.join("desktop/bun.lock"))?;
    let payload: Value = serde_json::from_str(&strip_trailing_commas(&source)?)?;
    if payload.get("lockfileVersion").and_then(Value::as_u64) != Some(2) {
        return Err("desktop/bun.lock must use Bun lockfile version 2".into());
    }
    let package_values = payload
        .get("packages")
        .and_then(Value::as_object)
        .ok_or("desktop/bun.lock package map is missing")?;
    let mut packages = BTreeMap::new();
    for (key, value) in package_values {
        let tuple = value
            .as_array()
            .ok_or_else(|| format!("Bun lock package entry is not an array: {key}"))?;
        let descriptor = tuple
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Bun lock package descriptor is missing: {key}"))?;
        let (name, version) = split_package_descriptor(descriptor)?;
        let metadata = tuple
            .get(2)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("Bun lock package metadata is missing: {key}"))?;
        let integrity = tuple
            .get(3)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("Bun lock package integrity is missing: {key}"))?;
        if integrity.is_empty() || integrity.len() > 4096 {
            return Err(format!("Bun lock package integrity is invalid: {key}").into());
        }
        packages.insert(
            key.clone(),
            FrontendPackage {
                key: key.clone(),
                name,
                version,
                integrity: integrity.to_owned(),
                dependencies: dependency_map(metadata)?,
            },
        );
    }

    let workspace = payload
        .get("workspaces")
        .and_then(Value::as_object)
        .and_then(|workspaces| workspaces.get(""))
        .and_then(Value::as_object)
        .ok_or("desktop/bun.lock root workspace is missing")?;
    let root_dependencies = workspace
        .get("dependencies")
        .and_then(Value::as_object)
        .ok_or("desktop/bun.lock production dependencies are missing")?;
    let mut root_packages = Vec::new();
    for (name, requirement) in root_dependencies {
        let requirement = requirement
            .as_str()
            .ok_or_else(|| format!("Bun root dependency requirement is invalid: {name}"))?;
        let key = resolve_frontend_package(&packages, "", name, requirement)?;
        root_packages.push(key);
    }
    root_packages.sort();

    let mut selected = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut pending = root_packages.clone();
    while let Some(key) = pending.pop() {
        if !selected.insert(key.clone()) {
            continue;
        }
        let package = packages
            .get(&key)
            .ok_or_else(|| format!("Bun lock selected package is missing: {key}"))?;
        for (name, requirement) in &package.dependencies {
            let dependency = resolve_frontend_package(&packages, &key, name, requirement)?;
            edges.insert((key.clone(), dependency.clone()));
            if !selected.contains(&dependency) {
                pending.push(dependency);
            }
        }
    }
    let graph_packages = selected
        .into_iter()
        .map(|key| {
            packages
                .get(&key)
                .cloned()
                .ok_or_else(|| format!("Bun lock selected package is missing: {key}"))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?;
    Ok(FrontendGraph {
        packages: graph_packages,
        root_packages,
        edges: edges.into_iter().collect(),
    })
}

fn dependency_map(metadata: &serde_json::Map<String, Value>) -> Result<BTreeMap<String, String>> {
    let mut dependencies = BTreeMap::new();
    let optional_peers = metadata
        .get("optionalPeers")
        .and_then(Value::as_array)
        .map(|peers| {
            peers
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();
    for field in ["dependencies", "optionalDependencies", "peerDependencies"] {
        let Some(entries) = metadata.get(field) else {
            continue;
        };
        let entries = entries
            .as_object()
            .ok_or_else(|| format!("Bun lock {field} must be an object"))?;
        for (name, requirement) in entries {
            if field == "peerDependencies" && optional_peers.contains(name.as_str()) {
                continue;
            }
            let requirement = requirement
                .as_str()
                .ok_or_else(|| format!("Bun lock dependency requirement is invalid: {name}"))?;
            dependencies.insert(name.clone(), requirement.to_owned());
        }
    }
    Ok(dependencies)
}

fn split_package_descriptor(descriptor: &str) -> Result<(String, String)> {
    let separator = descriptor
        .rfind('@')
        .ok_or_else(|| format!("Bun lock package descriptor has no version: {descriptor}"))?;
    let name = &descriptor[..separator];
    let version = &descriptor[separator + 1..];
    if name.is_empty() || version.is_empty() {
        return Err(format!("Bun lock package descriptor is malformed: {descriptor}").into());
    }
    Ok((name.to_owned(), version.to_owned()))
}

fn resolve_frontend_package(
    packages: &BTreeMap<String, FrontendPackage>,
    parent_key: &str,
    name: &str,
    requirement: &str,
) -> Result<String> {
    let preferred = if parent_key.is_empty() {
        name.to_owned()
    } else {
        format!("{parent_key}/{name}")
    };
    if let Some(package) = packages.get(&preferred)
        && package.name == name
        && frontend_version_matches(&package.version, requirement)
    {
        return Ok(preferred);
    }
    let candidates = packages
        .iter()
        .filter(|(_, package)| {
            package.name == name && frontend_version_matches(&package.version, requirement)
        })
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    match candidates.as_slice() {
        [candidate] => Ok(candidate.clone()),
        [] => Err(format!(
            "Bun lock dependency cannot be resolved: {name} {requirement} from {parent_key}"
        )
        .into()),
        _ => Err(format!(
            "Bun lock dependency resolution is ambiguous: {name} {requirement} from {parent_key}"
        )
        .into()),
    }
}

fn frontend_version_matches(version: &str, requirement: &str) -> bool {
    let Ok(version) = Version::parse(version) else {
        return false;
    };
    requirement.split("||").any(|alternative| {
        if VersionReq::parse(alternative.trim())
            .map(|requirement| requirement.matches(&version))
            .unwrap_or(false)
        {
            return true;
        }
        let normalized = alternative
            .split_whitespace()
            .map(|token| token.split_once('-').map_or(token, |(prefix, _)| prefix))
            .collect::<Vec<_>>()
            .join(" ");
        VersionReq::parse(&normalized)
            .map(|requirement| requirement.matches(&version))
            .unwrap_or(false)
    })
}

fn strip_trailing_commas(source: &str) -> Result<String> {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            output.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            output.push(byte);
            index += 1;
            continue;
        }
        if byte == b',' {
            let mut next = index + 1;
            while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                next += 1;
            }
            if matches!(bytes.get(next), Some(b'}' | b']')) {
                index += 1;
                continue;
            }
        }
        output.push(byte);
        index += 1;
    }
    String::from_utf8(output).map_err(Into::into)
}

fn expected_relationships(
    artifacts: &[Artifact],
    locked_packages: &[LockedPackage],
    frontend: &FrontendGraph,
) -> Result<Vec<Value>> {
    let mut relationships = artifacts
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
    relationships.extend(locked_packages.iter().enumerate().map(|(index, _)| {
        json!({
            "spdxElementId": "SPDXRef-Package-SkyAutoPlayer",
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": format!("SPDXRef-Package-Cargo-{}", index + 1),
        })
    }));
    for package_key in &frontend.root_packages {
        relationships.push(json!({
            "spdxElementId": "SPDXRef-Package-SkyAutoPlayer",
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": frontend_package_id(frontend.package_index(package_key)?),
        }));
    }
    for (from, to) in &frontend.edges {
        relationships.push(json!({
            "spdxElementId": frontend_package_id(frontend.package_index(from)?),
            "relationshipType": "DEPENDS_ON",
            "relatedSpdxElement": frontend_package_id(frontend.package_index(to)?),
        }));
    }
    Ok(relationships)
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

    #[test]
    fn frontend_graph_covers_production_workspace_dependencies() {
        let graph = frontend_graph(&crate::repo::root()).unwrap();
        assert!(
            graph
                .root_packages
                .iter()
                .any(|key| graph.packages.iter().any(|package| package.key == *key))
        );
        assert!(graph.packages.iter().any(|package| package.name == "react"));
        assert!(
            graph
                .packages
                .iter()
                .any(|package| package.name == "@tauri-apps/api")
        );
        assert!(!graph.edges.is_empty());
    }
}
