use super::*;

/// The hosted proof creates a disposable encrypted Tauri v2.12.3 key and
/// exercises the PRODUCTION adapter, materialization, real signer CLI,
/// and signature verification against the disposable public root.
/// It never accesses the protected production root or any real credentials.
#[test]
#[ignore = "requires pinned Tauri CLI and Bun on the hosted Windows proof runner"]
fn hosted_disposable_tauri_private_key_roundtrip_through_adapter() {
    use std::ffi::OsStr;

    let root = crate::repo::root();
    let runner_temp =
        PathBuf::from(env::var_os("RUNNER_TEMP").expect("hosted runner temp is required"));
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = runner_temp.join(format!(
        "sky-v4-signer-adapter-proof-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&directory).expect("create isolated disposable test directory");
    let test_key_path = directory.join("disposable.key");
    let _cleanup = TemporarySigningKey {
        directory: directory.clone(),
        path: test_key_path.clone(),
    };

    const TEST_PASSWORD: &str = "throwaway-hosted-adapter-only";
    const TEST_VERSION: &str = "4.3.0";
    let desktop = root.join("desktop");
    let generation = process::capture_owned_timeout_runtime_only(
        OsStr::new("bun"),
        &[
            "run".to_owned(),
            "tauri".to_owned(),
            "signer".to_owned(),
            "generate".to_owned(),
            "--ci".to_owned(),
            "--password".to_owned(),
            TEST_PASSWORD.to_owned(),
            "--write-keys".to_owned(),
            test_key_path.to_string_lossy().into_owned(),
        ],
        &desktop,
        &[],
        Duration::from_secs(120),
    )
    .expect("execute disposable Tauri key generation");
    assert!(
        generation.status.success(),
        "disposable Tauri key generation failed"
    );

    let original = Zeroizing::new(
        fs::read_to_string(&test_key_path).expect("read Tauri-generated disposable key file"),
    );
    let secrets = SigningSecrets::from_values(original.to_string(), TEST_PASSWORD.to_owned())
        .expect("production adapter accepts generated Tauri secret envelope");
    assert!(
        original.trim_end_matches(['\r', '\n']) == secrets.private_key.as_str(),
        "adapter must preserve Tauri's canonical encoded key file"
    );

    let decoded = Zeroizing::new(
        STANDARD
            .decode(original.trim_end_matches(['\r', '\n']))
            .expect("decode disposable outer Base64 layer"),
    );
    let inner = std::str::from_utf8(&decoded).expect("generated key inner text is UTF-8");
    assert!(
        inner.lines().next() == Some(TAURI_RSIGN_HEADER),
        "pinned Tauri private key must have rsign encrypted header"
    );
    let from_raw = SigningSecrets::from_values(inner.to_owned(), TEST_PASSWORD.to_owned())
        .expect("inner raw text is normalized into the canonical Tauri envelope");
    assert!(
        from_raw.private_key == secrets.private_key,
        "both transports must materialize identical encoded bytes"
    );

    let temporary_key =
        materialize_key(&secrets, "9876543210", 1).expect("materialize under runner temp");
    assert!(
        fs::read_to_string(&temporary_key.path).expect("read isolated materialized test key")
            == secrets.private_key.as_str(),
        "temporary key file must contain outer Base64 for the Tauri CLI"
    );

    let installer = directory.join(crate::release_metadata::canonical_installer_name(
        TEST_VERSION,
    ));
    fs::write(
        &installer,
        b"MZ disposable Tauri signing adapter proof fixture",
    )
    .expect("create disposable signing fixture");
    let signed = process::capture_owned_timeout_runtime_only(
        OsStr::new("bun"),
        &[
            "run".to_owned(),
            "tauri".to_owned(),
            "signer".to_owned(),
            "sign".to_owned(),
            "--private-key-path".to_owned(),
            temporary_key.path.to_string_lossy().into_owned(),
            "--app-version".to_owned(),
            TEST_VERSION.to_owned(),
            installer.to_string_lossy().into_owned(),
        ],
        &desktop,
        &[
            (
                "TAURI_SIGNING_PRIVATE_KEY_PATH".to_owned(),
                temporary_key.path.to_string_lossy().into_owned(),
            ),
            (
                "TAURI_SIGNING_PRIVATE_KEY_PASSWORD".to_owned(),
                TEST_PASSWORD.to_owned(),
            ),
        ],
        Duration::from_secs(120),
    )
    .expect("execute disposable standalone Tauri signer");
    assert!(
        signed.status.success(),
        "real Tauri signing rejected normalized test key"
    );

    let signature = PathBuf::from(format!("{}.sig", installer.display()));
    let public_key = PathBuf::from(format!("{}.pub", test_key_path.display()));
    let source_sha = crate::repo::git_head(&root, true).expect("exact proof source SHA");
    updater_trust::verify_test_updater_signature(
        &installer,
        &signature,
        &public_key,
        TEST_VERSION,
        &hash::sha256(&installer).expect("hash exact disposable fixture"),
        &source_sha,
        None,
    )
    .expect("verify disposable signature and version with independent disposable root");
}
