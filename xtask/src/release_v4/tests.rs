use super::*;
use base64::Engine;
use std::ffi::OsString;
use std::process::Command;

static NEXT_TEST_TEMP_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn test_unique_temp(parent: &Path, prefix: &str) -> PathBuf {
    let sequence = NEXT_TEST_TEMP_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    parent.join(format!("{prefix}-{}-{sequence}", std::process::id()))
}

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run_test_tauri(root: &Path, args: Vec<OsString>) -> Result<()> {
    let mut command = Command::new("bun");
    command
        .arg("run")
        .arg("tauri")
        .args(args)
        .current_dir(root.join("apps/desktop"));
    for name in [
        "V4_TAURI_UPDATER_PRIVATE_KEY",
        "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
        "TAURI_SIGNING_PRIVATE_KEY",
        "TAURI_SIGNING_PRIVATE_KEY_PATH",
        "TAURI_SIGNING_PRIVATE_KEY_PASSWORD",
        "GH_TOKEN",
        "GITHUB_TOKEN",
    ] {
        command.env_remove(name);
    }
    let output = command.output()?;
    if !output.status.success() {
        return Err(format!("local Tauri test-key command failed with {}", output.status).into());
    }
    Ok(())
}

fn generate_test_signing_key(root: &Path, key_path: &Path) -> Result<PathBuf> {
    run_test_tauri(
        root,
        vec![
            "signer".into(),
            "generate".into(),
            "--ci".into(),
            "--password".into(),
            "".into(),
            "--force".into(),
            "-w".into(),
            key_path.as_os_str().to_owned(),
        ],
    )?;
    Ok(PathBuf::from(format!("{}.pub", key_path.display())))
}

fn sign_test_installer(
    root: &Path,
    key_path: &Path,
    version: &str,
    installer: &Path,
) -> Result<Vec<u8>> {
    run_test_tauri(
        root,
        vec![
            "signer".into(),
            "sign".into(),
            "--private-key-path".into(),
            key_path.as_os_str().to_owned(),
            "--app-version".into(),
            version.into(),
            installer.as_os_str().to_owned(),
        ],
    )?;
    let name = installer
        .file_name()
        .ok_or("test installer has no filename")?
        .to_string_lossy();
    Ok(fs::read(installer.with_file_name(format!("{name}.sig")))?)
}

fn rebind_test_signature(
    options: &Options,
    manifest: &mut CandidateManifest,
    signature_bytes: &[u8],
    test_key_id: &str,
) -> Result<()> {
    let signature_index = manifest
        .qualification_assets
        .iter()
        .position(|record| record.role == "updater-signature")
        .ok_or("test candidate signature record missing")?;
    let mut signature = manifest.qualification_assets[signature_index].clone();
    let signature_path = safe_state_asset_path(&options.state_root, &signature)?;
    fs::write(&signature_path, signature_bytes)?;
    signature.size = signature_bytes.len() as u64;
    signature.sha256 = sha256_hex(signature_bytes);
    manifest.qualification_assets[signature_index] = signature.clone();
    for record in &mut manifest.public_assets {
        if record.role == "updater-signature" {
            *record = signature.clone();
        }
    }

    for evidence_role in ["production-evidence", "qualification-evidence"] {
        let evidence_index = manifest
            .qualification_assets
            .iter()
            .position(|record| record.role == evidence_role)
            .ok_or("test candidate evidence record missing")?;
        let evidence_record = manifest.qualification_assets[evidence_index].clone();
        let evidence_path = safe_state_asset_path(&options.state_root, &evidence_record)?;
        let mut evidence: Value = read_json(&evidence_path)?;
        evidence["updater_signature"] = json!(signature.source_name);
        evidence["signature_size"] = json!(signature.size);
        evidence["updater_signature_sha256"] = json!(signature.sha256);
        if evidence_role == "production-evidence" {
            evidence["updater_key_id"] = json!(test_key_id);
        }
        write_json_atomic(&evidence_path, &evidence)?;
        let bytes = fs::read(&evidence_path)?;
        let mut updated_record = evidence_record;
        updated_record.size = bytes.len() as u64;
        updated_record.sha256 = sha256_hex(&bytes);
        manifest.qualification_assets[evidence_index] = updated_record;
    }
    Ok(())
}

fn tamper_test_signature(signature: &[u8]) -> Result<Vec<u8>> {
    let encoded = std::str::from_utf8(signature)?.trim();
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded)?;
    let decoded = String::from_utf8(decoded)?;
    let mut lines = decoded.lines().map(str::to_owned).collect::<Vec<_>>();
    let signature_line = lines
        .iter_mut()
        .find(|line| {
            !line.is_empty()
                && !line.starts_with("untrusted comment:")
                && !line.starts_with("trusted comment:")
        })
        .ok_or("test signature has no encoded signature payload")?;
    let mut bytes = signature_line.as_bytes().to_vec();
    bytes[0] = if bytes[0] == b'A' { b'B' } else { b'A' };
    *signature_line = String::from_utf8(bytes)?;
    Ok(base64::engine::general_purpose::STANDARD
        .encode(lines.join("\n"))
        .into_bytes())
}

#[test]
#[ignore = "manual read-only GET integration against the immutable public v4.2.0 assets"]
fn public_v4_2_0_asset_pair_downloads_through_production_adapter() -> Result<()> {
    if ["GH_TOKEN", "GITHUB_TOKEN", "GH_ENTERPRISE_TOKEN"]
        .iter()
        .any(|name| env::var_os(name).is_some())
    {
        return Err("manual public asset check requires token environment variables unset".into());
    }
    let mut transport = GhTransport::new();
    let release = required_api(
        &mut transport,
        "GET",
        "repos/pumni/Sky-Auto-Player/releases/tags/v4.2.0",
        None,
    )?;
    if release["tag_name"] != "v4.2.0" || release["draft"] != false || release["immutable"] != true
    {
        return Err("public v4.2.0 release is not the expected immutable published release".into());
    }
    let expected = [
        (
            "Sky.Auto.Player_4.2.0_x64-setup.exe",
            "Sky Auto Player_4.2.0_x64-setup.exe",
            4_459_233,
            "a6048bd8dc8691798ed2ec7078dd66abe57a5cedfd4d086eedb10cd185751105",
        ),
        (
            "Sky.Auto.Player_4.2.0_x64-setup.exe.sig",
            "Sky Auto Player_4.2.0_x64-setup.exe.sig",
            448,
            "2f1603eb7448339a4a3bd43ec892298a1d82421a38d5bf72ceb0bc4984d678ce",
        ),
    ]
    .into_iter()
    .map(
        |(release_name, source_name, size, sha256)| CandidateRecord {
            name: release_name.to_owned(),
            release_name: release_name.to_owned(),
            source_name: source_name.to_owned(),
            role: if release_name.ends_with(".sig") {
                "updater-signature"
            } else {
                "installer"
            }
            .to_owned(),
            size,
            sha256: sha256.to_owned(),
            state_path: "public-read-only-check".to_owned(),
        },
    )
    .collect::<Vec<_>>();
    verify_public_assets(&mut transport, &release, &expected, true)?;

    let verification_root = test_unique_temp(&env::temp_dir(), "sky-release-v4-2-0-public-proof");
    fs::create_dir(&verification_root)?;
    let _verification_cleanup = TestDirectory(verification_root.clone());
    let assets = release["assets"]
        .as_array()
        .ok_or("public v4.2.0 release omitted its assets array")?;
    for record in &expected {
        let asset = assets
            .iter()
            .find(|asset| asset["name"] == record.release_name)
            .ok_or("public v4.2.0 asset disappeared during signature verification")?;
        let endpoint = asset["url"]
            .as_str()
            .ok_or("public v4.2.0 asset omitted its API URL")?;
        let path = verification_root.join(&record.source_name);
        transport.download(asset_api_endpoint(endpoint)?, &path)?;
        if fs::metadata(&path)?.len() != record.size || crate::hash::sha256(&path)? != record.sha256
        {
            return Err(format!(
                "production-adapter v4.2.0 readback differs for {}",
                record.release_name
            )
            .into());
        }
    }
    crate::updater_trust::verify_updater_signature_for_version(
        &verification_root.join(&expected[0].source_name),
        &verification_root.join(&expected[1].source_name),
        "4.2.0",
    )?;
    fs::remove_dir_all(&verification_root)?;
    if verification_root.exists() {
        return Err("public v4.2.0 verification files were not cleaned up".into());
    }
    Ok(())
}

#[test]
#[ignore = "hermetic fake-transport proof using disposable local Tauri test keys"]
fn test_key_signature_is_verified_after_draft_readback_before_patch() -> Result<()> {
    let (root, options, context, manifest) = fixture()?;
    let _state_cleanup = TestDirectory(options.state_root.clone());
    let signing_root = test_unique_temp(&env::temp_dir(), "sky-release-publication-test-key");
    fs::create_dir(&signing_root)?;
    let _signing_cleanup = TestDirectory(signing_root.clone());
    let key_a = signing_root.join("publication-a.key");
    let key_b = signing_root.join("publication-b.key");
    let public_a = generate_test_signing_key(&root, &key_a)?;
    let public_b = generate_test_signing_key(&root, &key_b)?;
    let key_a_id =
        crate::updater_trust::extract_key_id_from_public_key(&fs::read_to_string(&public_a)?)?;
    let key_b_id =
        crate::updater_trust::extract_key_id_from_public_key(&fs::read_to_string(&public_b)?)?;
    if key_a_id == crate::updater_trust::canonical_key_id()?
        || key_b_id == crate::updater_trust::canonical_key_id()?
        || key_a_id == key_b_id
    {
        return Err("disposable publication test keys did not have isolated identities".into());
    }

    let installer_record = manifest
        .public_assets
        .iter()
        .find(|record| record.role == "installer")
        .ok_or("test candidate installer record missing")?;
    let installer_path = safe_state_asset_path(&options.state_root, installer_record)?;
    let correct_signature = sign_test_installer(&root, &key_a, &context.version, &installer_path)?;
    let wrong_key_signature =
        sign_test_installer(&root, &key_b, &context.version, &installer_path)?;
    let wrong_version = if context.version == "0.0.0" {
        "0.0.1"
    } else {
        "0.0.0"
    };
    let wrong_version_signature =
        sign_test_installer(&root, &key_a, wrong_version, &installer_path)?;
    let wrong_file = signing_root.join("wrong-file.exe");
    fs::copy(&installer_path, &wrong_file)?;
    let wrong_file_signature = sign_test_installer(&root, &key_a, &context.version, &wrong_file)?;
    let tampered_signature = tamper_test_signature(&correct_signature)?;
    let installer_sha256 = installer_record.sha256.clone();

    struct Scenario {
        label: &'static str,
        signature: Vec<u8>,
        evidence_key_id: String,
        verification_key: PathBuf,
        expected_error: Option<&'static str>,
    }
    let scenarios = [
        Scenario {
            label: "correct signature",
            signature: correct_signature,
            evidence_key_id: key_a_id.clone(),
            verification_key: public_a.clone(),
            expected_error: None,
        },
        Scenario {
            label: "wrong key",
            signature: wrong_key_signature,
            evidence_key_id: key_b_id,
            verification_key: public_a.clone(),
            expected_error: Some("signature does not match the supplied test public key"),
        },
        Scenario {
            label: "wrong trusted filename",
            signature: wrong_file_signature,
            evidence_key_id: key_a_id.clone(),
            verification_key: public_a.clone(),
            expected_error: Some("filename does not match the candidate"),
        },
        Scenario {
            label: "wrong trusted version",
            signature: wrong_version_signature,
            evidence_key_id: key_a_id.clone(),
            verification_key: public_a.clone(),
            expected_error: Some("version does not match the candidate"),
        },
        Scenario {
            label: "tampered signature",
            signature: tampered_signature,
            evidence_key_id: key_a_id,
            verification_key: public_a,
            expected_error: Some("signature"),
        },
    ];

    for scenario in scenarios {
        let mut case_manifest = manifest.clone();
        rebind_test_signature(
            &options,
            &mut case_manifest,
            &scenario.signature,
            &scenario.evidence_key_id,
        )?;
        let latest = fake_release(11, "v4.1.4", "b".repeat(40), false, false, true);
        prepare_publish_fixture(&options, &context, &case_manifest, &latest)?;
        let mut github = FakeTransport::new();
        github.latest = latest;
        let expected_key_id = scenario.evidence_key_id;
        let verification_key = scenario.verification_key;
        let expected_sha256 = installer_sha256.clone();
        let source_sha = context.source_sha.clone();
        let result = publish_release_with_signature_verifier(
            &root,
            &options,
            &mut github,
            &expected_key_id,
            |installer, signature, version| {
                crate::updater_trust::verify_test_updater_signature(
                    installer,
                    signature,
                    &verification_key,
                    version,
                    &expected_sha256,
                    &source_sha,
                    None,
                )
            },
        );
        let patch_count = github
            .events
            .iter()
            .filter(|event| event.as_str() == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
            .count();
        if let Some(expected_error) = scenario.expected_error {
            let error = result.expect_err("invalid test signature must stop publication");
            assert!(
                error.to_string().contains(expected_error),
                "{} returned an unexpected verification error: {error}",
                scenario.label
            );
            assert_eq!(
                patch_count, 0,
                "{} issued a publication PATCH",
                scenario.label
            );
            assert_eq!(github.deletions, [12], "owned failed draft is cleaned up");
            let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
            assert_eq!(state.phase, Phase::Ready, "{} phase", scenario.label);
            assert!(
                !state.qualified_after_download,
                "{} proof flag",
                scenario.label
            );
            assert_eq!(state.release_id, None, "{} release ID", scenario.label);
        } else {
            result?;
            assert_eq!(patch_count, 1);
            assert_eq!(
                github
                    .events
                    .iter()
                    .filter(|event| event.starts_with("DOWNLOAD "))
                    .count(),
                2,
                "installer and sidecar are read back before publication"
            );
            let patch = github
                .events
                .iter()
                .position(|event| event == "PATCH repos/pumni/Sky-Auto-Player/releases/12")
                .ok_or("valid test-key publication PATCH missing")?;
            assert!(
                github
                    .events
                    .iter()
                    .enumerate()
                    .filter(|(_, event)| event.starts_with("DOWNLOAD "))
                    .all(|(index, _)| index < patch)
            );
            let state: ReleaseState = read_json(&options.state_root.join("release-state.json"))?;
            assert_eq!(state.phase, Phase::PublishedPendingMetadata);
            assert!(state.qualified_after_download);
            assert_eq!(state.release_id, Some(12));
            assert!(github.deletions.is_empty());
        }
    }
    fs::remove_dir_all(&signing_root)?;
    fs::remove_dir_all(&options.state_root)?;
    if signing_root.exists() || options.state_root.exists() {
        return Err("disposable publication test material was not cleaned up".into());
    }
    Ok(())
}

#[test]
fn candidate_verification_dispatcher_requires_the_exact_workflow_job() {
    assert_eq!(expected_job_for_command("verify-candidate"), Some("attest"));
    assert_eq!(
        expected_job_for_command("probe-verify-candidate"),
        Some("metadata_probe")
    );
    assert_eq!(
        validate_dispatch_job("probe-verify-candidate", "metadata_probe").unwrap(),
        "metadata_probe"
    );
    assert!(validate_dispatch_job("probe-verify-candidate", "attest").is_err());
    assert!(validate_dispatch_job("verify-candidate", "metadata_probe").is_err());
    assert!(validate_dispatch_job("probe-verify-candidate", "*").is_err());
    assert!(validate_dispatch_job("unknown", "metadata_probe").is_err());
}

#[test]
fn sign_candidate_entrypoint_does_not_leak_updater_secrets_to_git_child() {
    const CHILD_MODE: &str = "SKY_XTASK_RELEASE_GIT_CHILD_TEST";
    const RUNNER_TEMP: &str = "SKY_XTASK_RELEASE_GIT_CHILD_RUNNER_TEMP";
    const SOURCE_SHA: &str = "SKY_XTASK_RELEASE_GIT_CHILD_SOURCE_SHA";
    const CAPTURE_PATH: &str = "SKY_XTASK_RELEASE_GIT_CHILD_CAPTURE";
    const COMMAND: &str = "SKY_XTASK_RELEASE_GIT_CHILD_COMMAND";

    if env::var_os(CHILD_MODE).is_some() {
        let root = repo::root();
        let source_sha = env::var(SOURCE_SHA).expect("test source SHA");
        let runner_temp = PathBuf::from(env::var_os(RUNNER_TEMP).expect("test runner temp"));
        let state_root = runner_temp.join("sky-v4-release-123456-1");
        fs::create_dir_all(&state_root).expect("create release state root");
        let version = repo::project_version(&root).expect("project version");
        let parsed = version::parse(&version).expect("parse project version");
        let channel = if parsed.pre.is_empty() {
            "stable"
        } else {
            "beta"
        };
        let tag = format!("v{version}");
        let context = Context {
            schema_version: 2,
            repository: REPOSITORY.to_owned(),
            version,
            channel: channel.to_owned(),
            tag: tag.clone(),
            release_notes_path: format!("docs/releases/{tag}.md"),
            source_sha: source_sha.clone(),
            workflow_sha: source_sha.clone(),
            run_id: "123456".to_owned(),
            run_attempt: 1,
            created_at: "2026-10-27T00:00:00Z".to_owned(),
        };
        write_json_atomic(&state_root.join("release-context.json"), &context)
            .expect("write release context");
        let command = env::var(COMMAND).expect("test release command");
        let actual_job = env::var("GITHUB_JOB").expect("test GitHub job identity");
        let args = [
            command.clone(),
            "--state-root".to_owned(),
            state_root.to_string_lossy().into_owned(),
            "--workflow-sha".to_owned(),
            source_sha,
        ];
        let error = run(&root, &args).expect_err("dispatcher fixture must stop before mutation");
        let expected_error = match (command.as_str(), actual_job.as_str()) {
            ("sign-candidate", "sign_qualify") => "unsigned handoff manifest",
            ("verify-candidate", "attest") | ("probe-verify-candidate", "metadata_probe") => {
                "required release state file is missing"
            }
            ("verify-candidate", _) => "canonical workflow job attest",
            ("probe-verify-candidate", _) => "canonical workflow job metadata_probe",
            _ => panic!("unexpected dispatcher fixture command or job"),
        };
        assert!(
            error.to_string().contains(expected_error),
            "dispatcher `{command}` under `{actual_job}` stopped unexpectedly: {error}"
        );
        return;
    }

    let test_root = env::temp_dir().join(format!(
        "sky-release-git-child-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&test_root).expect("create test root");
    struct RemoveTestRoot(PathBuf);
    impl Drop for RemoveTestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _remove_root = RemoveTestRoot(test_root.clone());
    let fake_git_dir = test_root.join("fake-git");
    fs::create_dir(&fake_git_dir).expect("create fake git directory");
    let capture_path = test_root.join("git-child-environments.txt");
    let source_sha = repo::git_head(&repo::root(), false).expect("read current source SHA");
    let fake_source = format!(
        r#"
use std::{{env, fs::OpenOptions, io::Write}};
const CAPTURE: &str = {capture_path:?};
const HEAD: &str = {source_sha:?};
fn main() {{
    let mut file = OpenOptions::new().create(true).append(true).open(CAPTURE).unwrap();
    writeln!(file, "--- child ---").unwrap();
    let mut values = env::vars().collect::<Vec<_>>();
    values.sort_by(|left, right| left.0.cmp(&right.0));
    for (name, value) in values {{ writeln!(file, "{{name}}={{value}}").unwrap(); }}
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("rev-parse") {{ println!("{{HEAD}}"); }}
}}
"#
    );
    let fake_source_path = test_root.join("fake-git.rs");
    fs::write(&fake_source_path, fake_source).expect("write fake git source");
    let fake_git_path = fake_git_dir.join(if cfg!(windows) { "git.exe" } else { "git" });
    let compile = Command::new("rustc")
        .arg("--edition=2021")
        .arg(&fake_source_path)
        .arg("-o")
        .arg(&fake_git_path)
        .output()
        .expect("compile fake git child");
    assert!(
        compile.status.success(),
        "fake git compilation failed: {}",
        String::from_utf8_lossy(&compile.stderr)
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&fake_git_path)
            .expect("fake git metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&fake_git_path, permissions).expect("make fake git executable");
    }
    let mut search_path = vec![fake_git_dir.clone()];
    search_path.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let path = env::join_paths(search_path).expect("join fake git search path");
    let runner_temp = test_root.join("runner-temp");
    fs::create_dir(&runner_temp).expect("create test runner temp");
    let executable = env::current_exe().expect("test executable path");
    for (command, job) in [
        ("sign-candidate", "sign_qualify"),
        ("verify-candidate", "attest"),
        ("probe-verify-candidate", "metadata_probe"),
        ("probe-verify-candidate", "attest"),
        ("verify-candidate", "metadata_probe"),
    ] {
        let child = Command::new(&executable)
                .args([
                    "--exact",
                    "release_v4::tests::sign_candidate_entrypoint_does_not_leak_updater_secrets_to_git_child",
                    "--nocapture",
                ])
                .env(CHILD_MODE, "1")
                .env("RUNNER_TEMP", &runner_temp)
                .env(RUNNER_TEMP, &runner_temp)
                .env(SOURCE_SHA, &source_sha)
                .env(CAPTURE_PATH, &capture_path)
                .env(COMMAND, command)
                .env("PATH", &path)
                .env("GITHUB_SHA", &source_sha)
                .env("GITHUB_REPOSITORY", REPOSITORY)
                .env("GITHUB_RUN_ID", "123456")
                .env("GITHUB_RUN_ATTEMPT", "1")
                .env("GITHUB_EVENT_NAME", "workflow_dispatch")
                .env("GITHUB_REF", "refs/heads/main")
                .env("GITHUB_WORKFLOW_REF", RELEASE_WORKFLOW_REF)
                .env("GITHUB_JOB", job)
                .env("V4_TAURI_UPDATER_PRIVATE_KEY", "synthetic-private-key")
                .env(
                    "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD",
                    "synthetic-updater-passphrase",
                )
                .env("GITHUB_TOKEN", "synthetic-github-token")
                .env("GH_TOKEN", "synthetic-gh-token")
                .env("NPM_TOKEN", "synthetic-package-token")
                .output()
                .expect("run release dispatcher entrypoint test child");
        assert!(
            child.status.success(),
            "dispatcher regression for `{command}` / `{job}` failed: {}",
            String::from_utf8_lossy(&child.stderr)
        );
    }
    let observed = fs::read_to_string(&capture_path).expect("captured fake Git environments");
    assert!(observed.matches("--- child ---").count() >= 20);
    for forbidden in [
        "V4_TAURI_UPDATER_PRIVATE_KEY=",
        "V4_TAURI_UPDATER_PRIVATE_KEY_PASSWORD=",
        "GITHUB_TOKEN=",
        "GH_TOKEN=",
        "NPM_TOKEN=",
        "synthetic-private-key",
        "synthetic-updater-passphrase",
        "synthetic-github-token",
        "synthetic-gh-token",
        "synthetic-package-token",
    ] {
        assert!(
            !observed.contains(forbidden),
            "fake Git child observed a forbidden signing secret or token"
        );
    }
}

#[test]
fn metadata_recovery_auth_routes_reads_and_one_stable_put_to_separate_tokens() {
    let transport = GhTransport::for_metadata_recovery(
        "synthetic-read-only-token".to_owned(),
        Some("synthetic-metadata-write-token".to_owned()),
    )
    .unwrap();
    for endpoint in [
        "repos/pumni/Sky-Auto-Player/actions/runs/7001",
        "repos/pumni/Sky-Auto-Player/actions/runs/7001/attempts/1/jobs",
        "repos/pumni/Sky-Auto-Player/releases/latest",
        "repos/pumni/Sky-Auto-Player/releases/assets/42",
    ] {
        let selected = transport
            .selected_recovery_token("GET", endpoint)
            .unwrap()
            .expect("recovery reads require an explicit credential");
        assert_eq!(selected.class, RecoveryTokenClass::ReadOnly);
        assert_eq!(selected.value, "synthetic-read-only-token");
    }
    let stable_put = transport
        .selected_recovery_token(
            "PUT",
            "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json",
        )
        .unwrap()
        .expect("the canonical stable metadata write selects the App token");
    assert_eq!(stable_put.class, RecoveryTokenClass::MetadataWrite);
    assert_eq!(stable_put.value, "synthetic-metadata-write-token");
    for (method, endpoint) in [
        (
            "PUT",
            "repos/pumni/Sky-Auto-Player/contents/channels/beta/latest.json",
        ),
        (
            "PUT",
            "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json?ref=main",
        ),
        (
            "POST",
            "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json",
        ),
        ("DELETE", "repos/pumni/Sky-Auto-Player/releases/42"),
    ] {
        assert!(
            transport.selected_recovery_token(method, endpoint).is_err(),
            "recovery must reject {method} {endpoint}"
        );
    }
    let read_only =
        GhTransport::for_metadata_recovery("synthetic-read-only-token".to_owned(), None).unwrap();
    assert!(
        read_only
            .selected_recovery_token(
                "PUT",
                "repos/pumni/Sky-Auto-Player/contents/channels/stable/latest.json",
            )
            .is_err(),
        "a read-only plan cannot synthesize metadata write authority"
    );
}

#[test]
fn malformed_github_api_json_fails_closed_with_endpoint_context() {
    assert_eq!(
        decode_api_response("DELETE", "repos/example/releases/1", b"").unwrap(),
        None
    );
    let error = decode_api_response("GET", "repos/example/releases/latest", b"{not-json")
        .expect_err("malformed API JSON must be rejected");
    assert!(error.to_string().contains("repos/example/releases/latest"));
    assert!(
        decode_api_response("GET", "repos/example", br#"{"ok":true}"#)
            .unwrap()
            .is_some()
    );
}

#[test]
fn release_upload_authentication_is_opaque_but_rejects_header_injection() {
    // GitHub's 2026 installation-token format can contain JWT dots and
    // punctuation; an old A-Za-z0-9_- regex rejected production credentials.
    let token = "ghs_12345_header.payload+with/slash~and=padding-underscore_";
    assert_eq!(
        release_upload_authorization_config(token).unwrap(),
        format!("header = \"Authorization: Bearer {token}\"\n")
    );
    let long_token = "x".repeat(9000);
    assert!(release_upload_authorization_config(&long_token).is_ok());
    assert_eq!(
        release_upload_authorization_config("quoted\"back\\slash").unwrap(),
        "header = \"Authorization: Bearer quoted\\\"back\\\\slash\"\n"
    );
    for invalid in [
        "",
        "token\rBad: yes",
        "token\nBad: yes",
        "token\tbad",
        "token\0bad",
    ] {
        assert!(
            release_upload_authorization_config(invalid).is_err(),
            "control characters must be rejected without echoing credentials"
        );
    }
}

#[test]
fn real_curl_parses_private_opaque_token_config_and_sends_exact_binary() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    // Localhost-only harmless fixture. Never mint tokens or mutate GitHub.
    let token = "ghs_12345_header.payload+with/slash~and=padding-underscore_\"quoted\"\\backslash";
    let auth_path = test_unique_temp(&env::temp_dir(), "sky-v4-test-auth");
    let binary_path = test_unique_temp(&env::temp_dir(), "sky-v4-test-binary");
    let contents = b"exact-binary\0payload\xFF\xFE\x00special";
    let auth_config = release_upload_authorization_config(token).unwrap();
    fs::write(&binary_path, contents).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local fixture");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        let started = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                            // curl's configured connect timeout is 20 seconds; leave room for
                            // Windows process startup and runner scheduling.
                            && started.elapsed() < Duration::from_secs(30) =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    panic!("local upload fixture timed out waiting for curl: {error}")
                }
                Err(error) => panic!("local upload fixture accept failed: {error}"),
            }
        };
        stream
            .set_nonblocking(false)
            .expect("set accepted upload fixture stream blocking");
        stream
            .set_read_timeout(Some(Duration::from_secs(7)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let received = stream.read(&mut chunk).expect("read fixture request");
            assert!(received > 0, "curl closed fixture request early");
            request.extend_from_slice(&chunk[..received]);
            let header_end = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .map(|position| position + 4);
            if let Some(header_end) = header_end
                && request.len() >= header_end + contents.len()
            {
                break;
            }
            assert!(request.len() < 16 * 1024, "unexpected fixture request size");
        }
        stream
            .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        request
    });

    let url = canonical_upload_url(
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets{?name,label}",
    )
    .unwrap();
    let mut args = release_upload_curl_args(&url, &binary_path, &auth_path);
    // Redirect ONLY this isolated, non-production unit test to a loopback
    // HTTP fixture; production continues to require canonical HTTPS.
    let proto = args.iter().position(|arg| arg == "--proto").unwrap() + 1;
    args[proto] = "=http".into();
    let url_index = args.iter().position(|arg| arg == "--url").unwrap() + 1;
    args[url_index] = format!("http://127.0.0.1:{port}/fixture");

    let result = execute_release_upload_curl(&args, &auth_path, &auth_config);
    let cleanup_binary = fs::remove_file(&binary_path);
    let server_result = server.join();
    cleanup_binary.unwrap();
    let request = match (result, server_result) {
        (Ok(()), Ok(request)) => request,
        (Err(curl_error), Err(_)) => {
            panic!("fixture curl failed: {curl_error:?}; local fixture thread also failed")
        }
        (Err(curl_error), Ok(_)) => {
            panic!("fixture curl failed after the local fixture received a request: {curl_error:?}")
        }
        (Ok(()), Err(_)) => panic!("local fixture thread failed after curl returned success"),
    };
    assert!(
        !auth_path.exists(),
        "auth config must be removed deterministically"
    );
    let header_end = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    assert_eq!(&request[header_end..], contents);
    let headers = String::from_utf8_lossy(&request[..header_end]);
    assert!(headers.starts_with("POST /fixture HTTP/1.1\r\n"));
    let authorization = headers
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        .expect("curl config must supply the private header");
    assert_eq!(
        authorization.trim_end_matches('\r'),
        format!("Authorization: Bearer {token}")
    );
}

fn loopback_curl_response_fixture(response_status: &[u8]) -> (Result<()>, PathBuf, PathBuf) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::{Duration, Instant};

    let token = "ghs_12345_fixture_token";
    let auth_path = test_unique_temp(&env::temp_dir(), "sky-v4-test-auth");
    let binary_path = test_unique_temp(&env::temp_dir(), "sky-v4-test-binary");
    let auth_config = release_upload_authorization_config(token).unwrap();
    fs::write(&binary_path, b"fixture-binary-data").unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local fixture");
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();

    let response_bytes = response_status.to_vec();
    let server = std::thread::spawn(move || {
        let started = Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                            // Match the production curl connect budget with scheduling slack.
                            && started.elapsed() < Duration::from_secs(30) =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    panic!("local upload fixture timed out waiting for curl: {error}")
                }
                Err(error) => panic!("local upload fixture accept failed: {error}"),
            }
        };
        stream
            .set_nonblocking(false)
            .expect("set accepted upload fixture stream blocking");
        stream
            .set_read_timeout(Some(Duration::from_secs(7)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let received = stream
                .read(&mut chunk)
                .expect("read loopback upload fixture request");
            assert_ne!(
                received, 0,
                "curl closed before sending the complete fixture upload request"
            );
            request.extend_from_slice(&chunk[..received]);
            let header_end = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .map(|position| position + 4);
            if let Some(header_end) = header_end
                && request.len() >= header_end + b"fixture-binary-data".len()
            {
                assert_eq!(
                    &request[header_end..],
                    b"fixture-binary-data",
                    "curl sent an unexpected fixture upload body"
                );
                break;
            }
            assert!(
                request.len() < 16 * 1024,
                "fixture upload request exceeded bound before complete body"
            );
        }
        if !response_bytes.is_empty() {
            stream
                .write_all(&response_bytes)
                .expect("write loopback upload fixture response");
            stream
                .flush()
                .expect("flush loopback upload fixture response");
            stream
                .shutdown(std::net::Shutdown::Write)
                .expect("finish loopback upload fixture response");
            let mut drain = [0_u8; 1024];
            while let Ok(n) = stream.read(&mut drain) {
                if n == 0 {
                    break;
                }
            }
        }
    });

    let url = canonical_upload_url(
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets{?name,label}",
    )
    .unwrap();
    let mut args = release_upload_curl_args(&url, &binary_path, &auth_path);
    let proto = args.iter().position(|arg| arg == "--proto").unwrap() + 1;
    args[proto] = "=http".into();
    let url_index = args.iter().position(|arg| arg == "--url").unwrap() + 1;
    args[url_index] = format!("http://127.0.0.1:{port}/fixture");

    let result = execute_release_upload_curl(&args, &auth_path, &auth_config);
    server.join().expect("local fixture thread");
    (result, auth_path, binary_path)
}

#[test]
fn real_curl_upload_fails_closed_on_http_errors_and_redirects() {
    for (code, response) in [
            (
                Some(401),
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
            ),
            (
                Some(403),
                b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
            ),
            (
                Some(422),
                b"HTTP/1.1 422 Unprocessable Entity\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
            ),
            (
                Some(500),
                b"HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
            ),
            (
                Some(503),
                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
            ),
            (
                Some(302),
                b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9999/redirect\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .as_slice(),
            ),
            (None, b"".as_slice()),
        ] {
            let (result, auth_path, binary_path) = loopback_curl_response_fixture(response);
            let _ = fs::remove_file(&binary_path);
            assert!(result.is_err(), "expected error for response {code:?}");
            assert!(!auth_path.exists(), "auth config must be removed on error: {code:?}");
            if let Some(expected_code) = code {
                let failure = result
                    .unwrap_err()
                    .downcast::<HttpFailure>()
                    .expect("HttpFailure");
                assert_eq!(
                    failure.status,
                    Some(expected_code),
                    "sanitized curl error: {}",
                    failure.detail
                );
            }
        }
}

#[test]
fn real_curl_production_args_refuse_cleartext_http() {
    let auth_path = test_unique_temp(&env::temp_dir(), "sky-v4-test-auth");
    let binary_path = test_unique_temp(&env::temp_dir(), "sky-v4-test-binary");
    fs::write(&binary_path, b"test-data").unwrap();
    let url = canonical_upload_url(
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets{?name,label}",
    )
    .unwrap();
    let mut args = release_upload_curl_args(&url, &binary_path, &auth_path);
    // Change url to http:// WITHOUT changing --proto =https
    let url_index = args.iter().position(|arg| arg == "--url").unwrap() + 1;
    args[url_index] = "http://127.0.0.1:9/fixture".into();

    let result = execute_release_upload_curl(
        &args,
        &auth_path,
        &release_upload_authorization_config("dummy_token").unwrap(),
    );
    let _ = fs::remove_file(&binary_path);
    assert!(
        result.is_err(),
        "curl must reject http protocol under --proto =https"
    );
    assert!(
        !auth_path.exists(),
        "auth config must be removed on protocol failure"
    );
}

#[test]
fn safe_cli_error_redacts_credentials_and_bounds_output() {
    let secret = "ghs_test_secret_redaction_token_12345";
    let dirty = format!("curl failed: Authorization: Bearer {secret} occurred");
    let sanitized = safe_cli_error_with_secrets(&dirty, &[secret]);
    assert!(!sanitized.contains(secret), "secret token must be redacted");
    assert!(sanitized.contains("[REDACTED]"));

    let oversized = "x".repeat(1000);
    assert_eq!(safe_cli_error_with_secrets(&oversized, &[]).len(), 600);
}

#[test]
fn request_with_retry_policy_never_blind_retries_post() {
    let mut attempts = 0;
    let result: Result<()> = request_with_retry_policy(
        "POST",
        "/repos/pumni/Sky-Auto-Player/releases/123/assets",
        || {
            attempts += 1;
            Err("network timeout during POST".into())
        },
        |_| panic!("POST must never sleep or retry blindly"),
    );
    assert!(result.is_err());
    assert_eq!(attempts, 1, "POST must be executed exactly once");
    let error = result.unwrap_err();
    assert!(error.is::<MutationOutcomeUnknown>());
    assert!(
        error
            .to_string()
            .contains("POST /repos/pumni/Sky-Auto-Player/releases/123/assets")
    );

    let mut get_attempts = 0;
    let get_result = request_with_retry_policy(
        "GET",
        "/repos/pumni/Sky-Auto-Player/releases/123",
        || {
            get_attempts += 1;
            if get_attempts < 3 {
                Err(Box::new(HttpFailure {
                    status: Some(503),
                    detail: "service unavailable".into(),
                }))
            } else {
                Ok("success")
            }
        },
        |_| {},
    );
    assert_eq!(get_result.unwrap(), "success");
    assert_eq!(get_attempts, 3, "GET may retry on transient errors");
}

#[test]
fn reconciliation_identifies_exact_asset_and_refuses_mismatches() {
    let (_root, _options, context, manifest) = fixture().unwrap();
    let record = &manifest.public_assets[0];
    let mut github = FakeTransport::new();
    let mut draft = fake_release(12, &context.tag, &context.source_sha, true, false, false);
    let marker = format!(
        "<!-- v4-release-tx: {} -->",
        serde_json::to_string(&json!({
            "repository": context.repository,
            "run_id": context.run_id,
            "source_sha": context.source_sha,
            "run_attempt": context.run_attempt,
            "version": context.version,
            "tag": context.tag
        }))
        .unwrap()
    );
    draft["body"] = json!(format!("Release notes\n{marker}"));

    // 1. Exact matching asset -> Ok(true)
    draft["assets"] = json!([{
        "name": record.release_name,
        "size": record.size,
        "state": "uploaded",
        "digest": format!("sha256:{}", record.sha256),
    }]);
    github.releases.insert(12, draft.clone());
    assert!(reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap());

    // 2. No matching asset -> Ok(false)
    draft["assets"] = json!([]);
    github.releases.insert(12, draft.clone());
    assert!(!reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap());

    // 3. Duplicate matching assets -> Err
    draft["assets"] = json!([
        {
            "name": record.release_name,
            "size": record.size,
            "state": "uploaded",
            "digest": format!("sha256:{}", record.sha256),
        },
        {
            "name": record.release_name,
            "size": record.size,
            "state": "uploaded",
            "digest": format!("sha256:{}", record.sha256),
        }
    ]);
    github.releases.insert(12, draft.clone());
    let err = reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap_err();
    assert!(err.to_string().contains("duplicate"));

    // 4. Size mismatch -> Err
    draft["assets"] = json!([{
        "name": record.release_name,
        "size": record.size + 1,
        "state": "uploaded",
        "digest": format!("sha256:{}", record.sha256),
    }]);
    github.releases.insert(12, draft.clone());
    let err = reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap_err();
    assert!(err.to_string().contains("exact asset validation failed"));

    // 5. Digest mismatch -> Err
    draft["assets"] = json!([{
        "name": record.release_name,
        "size": record.size,
        "state": "uploaded",
        "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    }]);
    github.releases.insert(12, draft.clone());
    let err = reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap_err();
    assert!(err.to_string().contains("exact asset validation failed"));

    // 6. Digest missing sha256 prefix -> Err
    draft["assets"] = json!([{
        "name": record.release_name,
        "size": record.size,
        "state": "uploaded",
        "digest": record.sha256,
    }]);
    github.releases.insert(12, draft.clone());
    let err = reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap_err();
    assert!(err.to_string().contains("exact asset validation failed"));

    // 7. State not "uploaded" -> Err
    draft["assets"] = json!([{
        "name": record.release_name,
        "size": record.size,
        "state": "uploading",
        "digest": format!("sha256:{}", record.sha256),
    }]);
    github.releases.insert(12, draft.clone());
    let err = reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap_err();
    assert!(err.to_string().contains("exact asset validation failed"));

    // 8. Draft is not draft (draft=false) -> Err
    draft["draft"] = json!(false);
    github.releases.insert(12, draft.clone());
    let err = reconcile_uncertain_upload(&mut github, 12, &context, record).unwrap_err();
    assert!(
        err.to_string()
            .contains("exact draft identity failed validation")
    );
}

#[test]
fn release_asset_upload_transport_targets_real_upload_host_without_exposing_token() {
    let mut url = canonical_upload_url(
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets{?name,label}",
    )
    .expect("canonical upload URL");
    url.query_pairs_mut()
        .append_pair("name", "Sky.Auto.Player_4.3.0_x64-setup.exe");
    let args = release_upload_curl_args(
        &url,
        Path::new("/tmp/installer.exe"),
        Path::new("/tmp/private-auth.cfg"),
    );
    assert!(args.windows(2).any(|pair| pair == ["--url", url.as_str()]));
    assert!(url.as_str().starts_with(
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets?name="
    ));
    for forbidden in ["api.uploads.github.com", "--hostname", "ghs_dummy_secret"] {
        assert!(!args.iter().any(|arg| arg.contains(forbidden)));
    }
    for expected in [
        ["--request", "POST"],
        ["--data-binary", "@/tmp/installer.exe"],
        ["--config", "/tmp/private-auth.cfg"],
        ["--write-out", "%{http_code}"],
        ["--proto", "=https"],
        ["--max-redirs", "0"],
    ] {
        assert!(args.windows(2).any(|pair| pair == expected));
    }
    assert!(!args.iter().any(|arg| arg == "--location"));
}

#[test]
fn release_upload_url_is_bound_to_canonical_numeric_release_route() {
    let endpoint = canonical_upload_url(
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets{?name,label}",
    )
    .expect("canonical GitHub upload template must be accepted");
    assert_eq!(
        endpoint.as_str(),
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets"
    );

    for endpoint in [
        "http://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets",
        "https://uploads.github.test/repos/pumni/Sky-Auto-Player/releases/123/assets",
        "https://uploads.github.com/repos/other/Sky-Auto-Player/releases/123/assets",
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/latest/assets",
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/0/assets",
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets/456",
        "https://uploads.github.com:443/repos/pumni/Sky-Auto-Player/releases/123/assets",
        "https://uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets?name=other.exe",
        "https://user@uploads.github.com/repos/pumni/Sky-Auto-Player/releases/123/assets",
    ] {
        assert!(
            canonical_upload_url(endpoint).is_err(),
            "noncanonical upload endpoint must be rejected: {endpoint}"
        );
    }
}

#[test]
fn mock_release_success_and_reconciliation() {
    mock_publish_succeeds().unwrap();
    mock_draft_readback_failures_never_patch().unwrap();
    mock_pr_merge_sha_does_not_break_release_or_owned_cleanup().unwrap();
    mock_draft_post_timeout_reconciles_exact_marker().unwrap();
    mock_patch_timeout_reconciles_exact_release_id().unwrap();
    mock_complete_transaction_and_raw_metadata().unwrap();
    mock_metadata_promotion_is_idempotent_after_committed_timeout().unwrap();
    mock_metadata_promotion_rejects_changed_latest().unwrap();
    mock_publication_identity_and_asset_inventory_fail_closed().unwrap();
    metadata_token_probe_rejects_wrong_repository_state().unwrap();
}
#[test]
fn mock_failure_cleanup_and_immutability() {
    mock_conflicting_draft_is_not_deleted().unwrap();
    mock_digest_mismatch_cleans_owned_draft().unwrap();
    mock_partial_upload_cleans_owned_draft().unwrap();
    mock_unknown_publication_never_cleans_up().unwrap();
    mock_latest_policy_failure_is_post_publication().unwrap();
    mock_stale_evidence_identity_fails_closed().unwrap();
    mock_manifest_path_traversal_fails_closed().unwrap();
    mock_release_doctor_detects_metadata_endpoint_divergence().unwrap();
    doctor_state_input_rejects_malformed_and_oversized_files().unwrap();
    retry_policy_retries_only_transient_responses().unwrap();
}
#[test]
fn doctor_and_timestamp_negative_contracts() {
    marker_parser_rejects_duplicates_and_multiple_markers().unwrap();
    workflow_run_identity_fails_closed().unwrap();
    timestamp_normalization_is_utc_and_strict().unwrap();
}
