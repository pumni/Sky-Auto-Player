#![forbid(unsafe_code)]

use crate::{Result, ci_artifacts, hash, release_metadata, repo};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const MAX_HANDOFF_MANIFEST_BYTES: u64 = 32 * 1024;
const MAX_INSTALLER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CALIBRATION_BYTES: u64 = 64 * 1024 * 1024;
const CALIBRATION_BINARY: &str = "native_calibration.exe";
const RELEASE_WORKFLOW_REF: &str =
    "pumni/Sky-Auto-Player/.github/workflows/release-v4.yml@refs/heads/main";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UnsignedArtifactManifest {
    pub(crate) schema_version: u32,
    pub(crate) repository: String,
    pub(crate) source_sha: String,
    pub(crate) workflow_sha: String,
    pub(crate) run_id: String,
    pub(crate) run_attempt: u32,
    pub(crate) version: String,
    pub(crate) installer_name: String,
    pub(crate) installer_size: u64,
    pub(crate) installer_sha256: String,
    pub(crate) calibration_binary: String,
    pub(crate) calibration_binary_size: u64,
    pub(crate) calibration_binary_sha256: String,
    pub(crate) calibration_binary_architecture: String,
    pub(crate) producer_job: String,
    pub(crate) artifact_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorkflowIdentity {
    pub(crate) repository: String,
    pub(crate) source_sha: String,
    pub(crate) run_id: String,
    pub(crate) run_attempt: u32,
    pub(crate) event_name: String,
    pub(crate) reference: String,
    pub(crate) workflow_ref: String,
    pub(crate) job: String,
}

pub(crate) struct ExpectedHandoffIdentity<'a> {
    pub(crate) repository: &'a str,
    pub(crate) source_sha: &'a str,
    pub(crate) workflow_sha: &'a str,
    pub(crate) run_id: &'a str,
    pub(crate) run_attempt: u32,
    pub(crate) version: &'a str,
}

impl WorkflowIdentity {
    pub(crate) fn from_environment() -> Result<Self> {
        let attempt = std::env::var("GITHUB_RUN_ATTEMPT")
            .map_err(|_| "GITHUB_RUN_ATTEMPT is required for release handoff")?
            .parse::<u32>()
            .map_err(|_| "GITHUB_RUN_ATTEMPT must be a positive decimal integer")?;
        if attempt == 0 || attempt > 100 {
            return Err("GITHUB_RUN_ATTEMPT is outside the supported bound".into());
        }
        Ok(Self {
            repository: std::env::var("GITHUB_REPOSITORY")
                .map_err(|_| "GITHUB_REPOSITORY is required for release handoff")?,
            source_sha: std::env::var("GITHUB_SHA")
                .map_err(|_| "GITHUB_SHA is required for release handoff")?
                .to_ascii_lowercase(),
            run_id: std::env::var("GITHUB_RUN_ID")
                .map_err(|_| "GITHUB_RUN_ID is required for release handoff")?,
            run_attempt: attempt,
            event_name: std::env::var("GITHUB_EVENT_NAME")
                .map_err(|_| "GITHUB_EVENT_NAME is required for release handoff")?,
            reference: std::env::var("GITHUB_REF")
                .map_err(|_| "GITHUB_REF is required for release handoff")?,
            workflow_ref: std::env::var("GITHUB_WORKFLOW_REF")
                .map_err(|_| "GITHUB_WORKFLOW_REF is required for release handoff")?,
            job: std::env::var("GITHUB_JOB")
                .map_err(|_| "GITHUB_JOB is required for release handoff")?,
        })
    }

    pub(crate) fn validate_release_context(
        &self,
        repository: &str,
        source_sha: &str,
        workflow_sha: &str,
        run_id: &str,
        run_attempt: u32,
        expected_job: &str,
    ) -> Result<()> {
        let source_sha = ci_artifacts::validate_commit_sha(source_sha, "release source SHA")?;
        let workflow_sha = ci_artifacts::validate_commit_sha(workflow_sha, "workflow SHA")?;
        if repository != "pumni/Sky-Auto-Player"
            || self.repository != repository
            || self.source_sha != source_sha
            || self.source_sha != workflow_sha
            || self.run_id != run_id
            || self.run_attempt != run_attempt
            || self.event_name != "workflow_dispatch"
            || self.reference != "refs/heads/main"
            || self.workflow_ref != RELEASE_WORKFLOW_REF
            || self.job != expected_job
        {
            return Err(
                "release handoff runtime identity does not match the exact canonical workflow job"
                    .into(),
            );
        }
        validate_run_id(&self.run_id)?;
        Ok(())
    }
}

pub(crate) fn artifact_name(run_id: &str, run_attempt: u32) -> Result<String> {
    validate_run_id(run_id)?;
    if run_attempt == 0 || run_attempt > 100 {
        return Err("release run attempt is outside the supported bound".into());
    }
    Ok(format!("v4-unsigned-{run_id}-{run_attempt}"))
}

pub(crate) fn create_unsigned_handoff(
    state_root: &Path,
    installer: &Path,
    calibration_binary: &Path,
    identity: &ExpectedHandoffIdentity<'_>,
) -> Result<UnsignedArtifactManifest> {
    ci_artifacts::validate_semver(identity.version, "unsigned installer version")?;
    let source_sha = ci_artifacts::validate_commit_sha(identity.source_sha, "unsigned source SHA")?;
    let workflow_sha = ci_artifacts::validate_commit_sha(identity.workflow_sha, "workflow SHA")?;
    if source_sha != workflow_sha {
        return Err("unsigned handoff workflow and source SHAs differ".into());
    }
    if identity.repository != "pumni/Sky-Auto-Player" {
        return Err(
            "unsigned handoff repository differs from the canonical release repository".into(),
        );
    }
    validate_run_id(identity.run_id)?;
    if identity.run_attempt == 0 || identity.run_attempt > 100 {
        return Err("release run attempt is outside the supported bound".into());
    }
    let installer_name = release_metadata::canonical_installer_name(identity.version);
    let metadata = ci_artifacts::require_regular_file(installer, "unsigned NSIS installer")?;
    if metadata.len() == 0 || metadata.len() > MAX_INSTALLER_BYTES {
        return Err("unsigned NSIS installer is empty or exceeds the handoff size bound".into());
    }
    if installer.file_name().and_then(|name| name.to_str()) != Some(installer_name.as_str()) {
        return Err(
            "unsigned installer name differs from the canonical versioned NSIS name".into(),
        );
    }
    let calibration_metadata =
        ci_artifacts::require_regular_file(calibration_binary, "unsigned calibration executable")?;
    if calibration_metadata.len() == 0 || calibration_metadata.len() > MAX_CALIBRATION_BYTES {
        return Err(
            "unsigned calibration executable is empty or exceeds the handoff size bound".into(),
        );
    }
    if calibration_binary
        .file_name()
        .and_then(|name| name.to_str())
        != Some(CALIBRATION_BINARY)
    {
        return Err("unsigned calibration executable has a non-canonical filename".into());
    }
    if ci_artifacts::pe_machine(calibration_binary, "unsigned calibration executable")? != 0x8664 {
        return Err("unsigned calibration executable must be an x86_64 PE file".into());
    }
    let handoff = state_root.join("unsigned-handoff");
    fs::create_dir(&handoff)?;
    let output_installer = handoff.join(&installer_name);
    fs::copy(installer, &output_installer)?;
    let output_calibration = handoff.join(CALIBRATION_BINARY);
    fs::copy(calibration_binary, &output_calibration)?;
    let manifest = UnsignedArtifactManifest {
        schema_version: 2,
        repository: identity.repository.to_owned(),
        source_sha,
        workflow_sha,
        run_id: identity.run_id.to_owned(),
        run_attempt: identity.run_attempt,
        version: identity.version.to_owned(),
        installer_name,
        installer_size: metadata.len(),
        installer_sha256: hash::sha256(&output_installer)?,
        calibration_binary: CALIBRATION_BINARY.to_owned(),
        calibration_binary_size: calibration_metadata.len(),
        calibration_binary_sha256: hash::sha256(&output_calibration)?,
        calibration_binary_architecture: "x86_64".to_owned(),
        producer_job: "build_unsigned".to_owned(),
        artifact_name: artifact_name(identity.run_id, identity.run_attempt)?,
    };
    write_manifest(&handoff.join("manifest.json"), &manifest)?;
    validate_handoff_directory(&handoff, &manifest)?;
    Ok(manifest)
}

pub(crate) fn validate_unsigned_handoff(
    root: &Path,
    incoming: &Path,
    expected: &ExpectedHandoffIdentity<'_>,
    job: &WorkflowIdentity,
) -> Result<(UnsignedArtifactManifest, PathBuf)> {
    job.validate_release_context(
        expected.repository,
        expected.source_sha,
        expected.workflow_sha,
        expected.run_id,
        expected.run_attempt,
        "sign_qualify",
    )?;
    let head = repo::git_head(root, true)?;
    if !head.eq_ignore_ascii_case(expected.source_sha) {
        return Err("signer checkout HEAD is not the exact clean release source SHA".into());
    }
    let manifest_path = incoming.join("manifest.json");
    let metadata = ci_artifacts::require_regular_file(&manifest_path, "unsigned handoff manifest")?;
    if metadata.len() > MAX_HANDOFF_MANIFEST_BYTES {
        return Err("unsigned handoff manifest exceeds its size bound".into());
    }
    let manifest: UnsignedArtifactManifest = serde_json::from_slice(&fs::read(&manifest_path)?)
        .map_err(|_| "unsigned handoff manifest is malformed or has unsupported fields")?;
    validate_manifest_identity(&manifest, expected)?;
    validate_handoff_directory(incoming, &manifest)?;
    Ok((manifest.clone(), incoming.join(manifest.installer_name)))
}

pub(crate) fn validate_manifest_identity(
    manifest: &UnsignedArtifactManifest,
    expected: &ExpectedHandoffIdentity<'_>,
) -> Result<()> {
    let source_sha = ci_artifacts::validate_commit_sha(expected.source_sha, "release source SHA")?;
    let workflow_sha = ci_artifacts::validate_commit_sha(expected.workflow_sha, "workflow SHA")?;
    ci_artifacts::validate_semver(expected.version, "release version")?;
    let expected_installer = release_metadata::canonical_installer_name(expected.version);
    if manifest.schema_version != 2
        || expected.repository != "pumni/Sky-Auto-Player"
        || manifest.repository != expected.repository
        || manifest.source_sha != source_sha
        || manifest.workflow_sha != workflow_sha
        || manifest.workflow_sha != manifest.source_sha
        || manifest.run_id != expected.run_id
        || manifest.run_attempt != expected.run_attempt
        || manifest.version != expected.version
        || manifest.installer_name != expected_installer
        || manifest.installer_size == 0
        || manifest.installer_size > MAX_INSTALLER_BYTES
        || manifest.installer_sha256.len() != 64
        || !manifest
            .installer_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || manifest.calibration_binary != CALIBRATION_BINARY
        || manifest.calibration_binary_size == 0
        || manifest.calibration_binary_size > MAX_CALIBRATION_BYTES
        || manifest.calibration_binary_sha256.len() != 64
        || !manifest
            .calibration_binary_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        || manifest.calibration_binary_architecture != "x86_64"
        || manifest.producer_job != "build_unsigned"
        || manifest.artifact_name != artifact_name(expected.run_id, expected.run_attempt)?
    {
        return Err(
            "unsigned artifact manifest identity differs from this release dispatch".into(),
        );
    }
    Ok(())
}

fn validate_handoff_directory(directory: &Path, manifest: &UnsignedArtifactManifest) -> Result<()> {
    ci_artifacts::require_directory(directory, "unsigned handoff directory is missing")?;
    let mut names = fs::read_dir(directory)?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    names.sort();
    let mut expected = vec![
        "manifest.json".to_owned(),
        manifest.installer_name.clone(),
        manifest.calibration_binary.clone(),
    ];
    expected.sort();
    if names != expected {
        return Err("unsigned handoff must contain exactly a manifest, installer, and calibration executable".into());
    }
    let installer = directory.join(&manifest.installer_name);
    let metadata = ci_artifacts::require_regular_file(&installer, "unsigned handoff installer")?;
    if metadata.len() != manifest.installer_size
        || hash::sha256(&installer)? != manifest.installer_sha256.to_ascii_lowercase()
    {
        return Err("unsigned installer bytes differ from the same-run handoff manifest".into());
    }
    let calibration_binary = directory.join(&manifest.calibration_binary);
    let calibration_metadata = ci_artifacts::require_regular_file(
        &calibration_binary,
        "unsigned handoff calibration executable",
    )?;
    if calibration_metadata.len() != manifest.calibration_binary_size
        || hash::sha256(&calibration_binary)?
            != manifest.calibration_binary_sha256.to_ascii_lowercase()
    {
        return Err(
            "calibration executable bytes differ from the same-run handoff manifest".into(),
        );
    }
    if ci_artifacts::pe_machine(
        &calibration_binary,
        "unsigned handoff calibration executable",
    )? != 0x8664
    {
        return Err("unsigned handoff calibration executable must be an x86_64 PE file".into());
    }
    Ok(())
}

fn write_manifest(path: &Path, manifest: &UnsignedArtifactManifest) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    if bytes.len() as u64 > MAX_HANDOFF_MANIFEST_BYTES {
        return Err("unsigned handoff manifest exceeds its size bound".into());
    }
    fs::write(path, [bytes.as_slice(), b"\n"].concat())?;
    Ok(())
}

fn validate_run_id(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("release run ID must contain 1 to 20 decimal digits".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("sky-v4-handoff-{}-{nonce}", std::process::id()))
    }

    fn valid_x64_pe() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x86];
        bytes[0..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_i32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x84..0x86].copy_from_slice(&0x8664_u16.to_le_bytes());
        bytes
    }

    fn fixture() -> (PathBuf, UnsignedArtifactManifest) {
        let root = temp_dir();
        let directory = root.join("incoming");
        fs::create_dir_all(&directory).expect("create fixture directory");
        let installer_name = release_metadata::canonical_installer_name("4.3.0");
        let installer = directory.join(&installer_name);
        fs::write(&installer, b"same-build-bytes").expect("write installer fixture");
        let calibration_binary = directory.join(CALIBRATION_BINARY);
        fs::write(&calibration_binary, valid_x64_pe()).expect("write calibration fixture");
        let manifest = UnsignedArtifactManifest {
            schema_version: 2,
            repository: "pumni/Sky-Auto-Player".to_owned(),
            source_sha: "a".repeat(40),
            workflow_sha: "a".repeat(40),
            run_id: "123456".to_owned(),
            run_attempt: 2,
            version: "4.3.0".to_owned(),
            installer_name,
            installer_size: b"same-build-bytes".len() as u64,
            installer_sha256: hash::sha256(&installer).expect("hash fixture"),
            calibration_binary: CALIBRATION_BINARY.to_owned(),
            calibration_binary_size: fs::metadata(&calibration_binary)
                .expect("calibration metadata")
                .len(),
            calibration_binary_sha256: hash::sha256(&calibration_binary).expect("calibration hash"),
            calibration_binary_architecture: "x86_64".to_owned(),
            producer_job: "build_unsigned".to_owned(),
            artifact_name: "v4-unsigned-123456-2".to_owned(),
        };
        write_manifest(&directory.join("manifest.json"), &manifest).expect("write manifest");
        (root, manifest)
    }

    fn expected_identity(source_sha: &str, run_attempt: u32) -> ExpectedHandoffIdentity<'_> {
        ExpectedHandoffIdentity {
            repository: "pumni/Sky-Auto-Player",
            source_sha,
            workflow_sha: source_sha,
            run_id: "123456",
            run_attempt,
            version: "4.3.0",
        }
    }

    #[test]
    fn unsigned_handoff_binds_exact_bytes_and_attempt() {
        let (root, manifest) = fixture();
        let directory = root.join("incoming");
        let source_sha = "a".repeat(40);
        validate_manifest_identity(&manifest, &expected_identity(&source_sha, 2))
            .expect("valid identity");
        validate_handoff_directory(&directory, &manifest).expect("valid file inventory");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsigned_handoff_exports_the_build_origin_calibration_as_the_third_file() {
        let root = temp_dir();
        fs::create_dir_all(&root).expect("create state root");
        let installer = root.join(release_metadata::canonical_installer_name("4.3.0"));
        fs::write(&installer, b"same Windows installer bytes").expect("write installer");
        let calibration = root.join(CALIBRATION_BINARY);
        fs::write(&calibration, valid_x64_pe()).expect("write build-origin calibration");
        let source_sha = "a".repeat(40);
        let manifest = create_unsigned_handoff(
            &root,
            &installer,
            &calibration,
            &ExpectedHandoffIdentity {
                repository: "pumni/Sky-Auto-Player",
                source_sha: &source_sha,
                workflow_sha: &source_sha,
                run_id: "123456",
                run_attempt: 2,
                version: "4.3.0",
            },
        )
        .expect("create exact unsigned handoff");
        let handoff = root.join("unsigned-handoff");
        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.calibration_binary, CALIBRATION_BINARY);
        assert_eq!(manifest.calibration_binary_architecture, "x86_64");
        assert_eq!(
            fs::read(handoff.join(CALIBRATION_BINARY)).expect("read handed-off calibration"),
            fs::read(&calibration).expect("read source calibration")
        );
        assert_eq!(
            fs::read_dir(&handoff).expect("handoff inventory").count(),
            3
        );
        validate_handoff_directory(&handoff, &manifest).expect("validate exact three-file handoff");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsigned_handoff_rejects_stale_or_tampered_inputs() {
        let (root, manifest) = fixture();
        let directory = root.join("incoming");
        let different_sha = "b".repeat(40);
        let source_sha = "a".repeat(40);
        assert!(
            validate_manifest_identity(&manifest, &expected_identity(&different_sha, 2)).is_err()
        );
        assert!(validate_manifest_identity(&manifest, &expected_identity(&source_sha, 3)).is_err());
        fs::write(
            directory.join(&manifest.installer_name),
            b"changed-after-build",
        )
        .expect("tamper fixture");
        assert!(validate_handoff_directory(&directory, &manifest).is_err());
        fs::write(
            directory.join(&manifest.installer_name),
            b"same-build-bytes",
        )
        .expect("restore installer fixture");
        fs::write(
            directory.join(CALIBRATION_BINARY),
            valid_x64_pe().into_iter().chain([0x7f]).collect::<Vec<_>>(),
        )
        .expect("tamper calibration fixture");
        assert!(
            validate_handoff_directory(&directory, &manifest)
                .unwrap_err()
                .to_string()
                .contains("calibration executable bytes")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unsigned_handoff_rejects_sidecars_and_unbounded_json() {
        let (root, manifest) = fixture();
        let directory = root.join("incoming");
        fs::write(directory.join("unexpected.sig"), b"sidecar").expect("write sidecar");
        assert!(validate_handoff_directory(&directory, &manifest).is_err());
        fs::remove_file(directory.join("unexpected.sig")).expect("remove sidecar");
        let wrong_architecture = {
            let mut bytes = valid_x64_pe();
            bytes[0x84..0x86].copy_from_slice(&0x014c_u16.to_le_bytes());
            bytes
        };
        fs::write(directory.join(CALIBRATION_BINARY), wrong_architecture)
            .expect("write wrong architecture");
        let mut wrong_machine_manifest = manifest.clone();
        wrong_machine_manifest.calibration_binary_size =
            fs::metadata(directory.join(CALIBRATION_BINARY))
                .expect("wrong machine file metadata")
                .len();
        wrong_machine_manifest.calibration_binary_sha256 =
            hash::sha256(&directory.join(CALIBRATION_BINARY)).expect("wrong machine hash");
        assert!(validate_handoff_directory(&directory, &wrong_machine_manifest).is_err());
        let oversized = json!({ "padding": "x".repeat(MAX_HANDOFF_MANIFEST_BYTES as usize) });
        assert!(
            serde_json::to_vec(&oversized).expect("serialize").len() as u64
                > MAX_HANDOFF_MANIFEST_BYTES
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn workflow_identity_rejects_non_main_jobs_and_attempts() {
        let valid = WorkflowIdentity {
            repository: "pumni/Sky-Auto-Player".to_owned(),
            source_sha: "a".repeat(40),
            run_id: "123456".to_owned(),
            run_attempt: 2,
            event_name: "workflow_dispatch".to_owned(),
            reference: "refs/heads/main".to_owned(),
            workflow_ref: RELEASE_WORKFLOW_REF.to_owned(),
            job: "sign_qualify".to_owned(),
        };
        valid
            .validate_release_context(
                "pumni/Sky-Auto-Player",
                &"a".repeat(40),
                &"a".repeat(40),
                "123456",
                2,
                "sign_qualify",
            )
            .expect("canonical job identity");
        for invalid in [
            WorkflowIdentity {
                reference: "refs/heads/feature".to_owned(),
                ..valid.clone()
            },
            WorkflowIdentity {
                job: "build_unsigned".to_owned(),
                ..valid.clone()
            },
            WorkflowIdentity {
                run_attempt: 1,
                ..valid.clone()
            },
            WorkflowIdentity {
                workflow_ref: "attacker/.github/workflows/release-v4.yml@refs/heads/main"
                    .to_owned(),
                ..valid.clone()
            },
        ] {
            assert!(
                invalid
                    .validate_release_context(
                        "pumni/Sky-Auto-Player",
                        &"a".repeat(40),
                        &"a".repeat(40),
                        "123456",
                        2,
                        "sign_qualify",
                    )
                    .is_err()
            );
        }
    }
}
