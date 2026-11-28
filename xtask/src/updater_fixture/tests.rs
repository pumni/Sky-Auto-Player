use super::*;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn bridge_tauri_config() -> serde_json::Value {
    serde_json::json!({
        "identifier": UPDATER_PRODUCT_IDENTIFIER,
        "bundle": {
            "publisher": "pumni",
            "windows": {
                "nsis": {
                    "installMode": "currentUser",
                    "installerHooks": CURRENT_NSIS_INSTALLER_HOOK
                }
            }
        }
    })
}

fn temporary_config_root() -> std::path::PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "sky-bridge-tauri-config-test-{}-{nonce:x}",
        std::process::id()
    ))
}

#[test]
fn windows_install_location_parser_accepts_absolute_nsis_paths_with_optional_quotes() {
    for (raw, expected) in [
        (
            r"D:\Program Files\Sky Auto Player",
            r"D:\Program Files\Sky Auto Player",
        ),
        (
            r#""D:\Program Files\Sky Auto Player""#,
            r"D:\Program Files\Sky Auto Player",
        ),
        (
            r"\\?\D:\a\_temp\fixture\install",
            r"\\?\D:\a\_temp\fixture\install",
        ),
        (
            r#""\\?\D:\a\_temp\fixture\install""#,
            r"\\?\D:\a\_temp\fixture\install",
        ),
        (
            r"\\server\share\Sky Auto Player",
            r"\\server\share\Sky Auto Player",
        ),
        (
            r"\\?\UNC\server\share\Sky Auto Player",
            r"\\?\UNC\server\share\Sky Auto Player",
        ),
        ("D:/Apps/Sky", r"D:\Apps\Sky"),
    ] {
        assert_eq!(
            parse_windows_install_location(raw).expect("valid absolute Windows path"),
            expected,
            "path syntax: {raw:?}"
        );
    }
}

#[test]
fn windows_install_location_parser_rejects_relative_or_malformed_paths() {
    for raw in [
        "",
        " relative\\install",
        "D:\\Apps\\install ",
        "relative\\install",
        r"D:relative\install",
        r"\rooted\without\drive",
        "\"",
        r#""D:\install"#,
        r#"D:\install""#,
        r#""D:\bad"name""#,
        r#""D:\install" suffix"#,
        r"\\.\PhysicalDrive0",
        r"\\?\GLOBALROOT\Device\HarddiskVolume1",
        r"D:\Apps\..\install",
        r"D:\Apps\\install",
        "D:\\Apps\\install\t",
        "D:\\Apps\\install\n",
    ] {
        assert!(
            parse_windows_install_location(raw).is_err(),
            "malformed or non-absolute path must fail closed: {raw:?}"
        );
    }
}

#[cfg(windows)]
#[test]
fn registry_install_location_quotes_resolve_only_to_managed_fixture_root() {
    struct TestRoot(std::path::PathBuf);
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let root = temporary_config_root();
    std::fs::create_dir_all(&root).expect("create isolated InstallLocation test root");
    let _root = TestRoot(root.clone());
    let managed = root.join("managed");
    let sibling = root.join("sibling");
    std::fs::create_dir(&managed).expect("create managed fixture install root");
    std::fs::create_dir(&sibling).expect("create sibling path for negative comparison");
    let expected = std::fs::canonicalize(&managed).expect("canonicalize managed root");
    let sibling = std::fs::canonicalize(&sibling).expect("canonicalize sibling path");

    for raw in [
        expected.to_string_lossy().into_owned(),
        format!("\"{}\"", expected.display()),
    ] {
        assert_eq!(
            validate_registry_install_location(&raw, &expected)
                .expect("valid registry path resolves to the managed root"),
            expected
        );
    }
    assert!(
        validate_registry_install_location(&sibling.to_string_lossy(), &expected).is_err(),
        "an absolute sibling directory must not be accepted as the fixture install root"
    );
    for raw in [
        r"relative\install".to_owned(),
        format!("\"{}", expected.display()),
    ] {
        assert!(
            validate_registry_install_location(&raw, &expected).is_err(),
            "relative and malformed quoted registry paths must be rejected"
        );
    }
}

#[test]
fn bridge_tauri_config_persists_historical_identity_and_removes_nested_migration_hook() {
    struct TestRoot(std::path::PathBuf);
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let root = temporary_config_root();
    let _root = TestRoot(root.clone());
    let source_root = root.join("source");
    let config_path = source_root.join("apps/desktop/src-tauri/tauri.conf.json");
    std::fs::create_dir_all(config_path.parent().expect("config parent"))
        .expect("create disposable worktree config path");
    let mut config = bridge_tauri_config();
    std::fs::write(
        &config_path,
        serde_json::to_vec_pretty(&config).expect("serialize source config"),
    )
    .expect("write original disposable worktree config");
    prepare_bridge_tauri_config(&mut config, "github")
        .expect("prepare legacy bridge-only bundle identity");
    persist_bridge_tauri_config(&source_root, &config_path, &config, "github")
        .expect("persist and validate the isolated bridge config");

    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&config_path).expect("read persisted config"))
            .expect("persisted config remains valid JSON");
    validate_bridge_tauri_config(&persisted, "github")
        .expect("persisted config must match bridge metadata identity");
    assert_eq!(
        persisted["bundle"]["publisher"],
        serde_json::Value::String("github".to_owned())
    );
    assert_eq!(
        persisted["identifier"],
        serde_json::Value::String(UPDATER_PRODUCT_IDENTIFIER.to_owned())
    );
    assert!(
        persisted["bundle"]["windows"]["nsis"]
            .get("installerHooks")
            .is_none()
    );
    assert_eq!(
        persisted["bundle"]["windows"]["nsis"]["installMode"],
        "currentUser"
    );
    assert!(persisted["bundle"].get("installerHooks").is_none());

    let outside_config = root.join("outside-worktree-tauri.conf.json");
    let outside_bytes = br#"{"publisher":"must remain unchanged"}"#;
    std::fs::write(&outside_config, outside_bytes).expect("write outside sentinel config");
    assert!(persist_bridge_tauri_config(&source_root, &outside_config, &config, "github").is_err());
    assert_eq!(
        std::fs::read(&outside_config).expect("read outside sentinel config"),
        outside_bytes,
        "bridge config writer must not modify a file outside its temporary Git worktree"
    );
}

#[test]
fn bridge_tauri_config_preserves_current_candidate_migration_hook() {
    let mut config = bridge_tauri_config();
    prepare_bridge_tauri_config(&mut config, "pumni")
        .expect("prepare current candidate bundle identity");
    validate_bridge_tauri_config(&config, "pumni")
        .expect("candidate config retains the current nested migration hook");
    assert_eq!(config["bundle"]["publisher"], "pumni");
    assert_eq!(
        config["bundle"]["windows"]["nsis"]["installerHooks"],
        CURRENT_NSIS_INSTALLER_HOOK
    );
}

#[test]
fn bridge_tauri_config_rejects_wrong_hook_path_and_mismatched_manifest_identity() {
    let mut misplaced = bridge_tauri_config();
    let bundle = misplaced
        .get_mut("bundle")
        .and_then(serde_json::Value::as_object_mut)
        .expect("bundle object");
    bundle.insert(
        "installerHooks".to_owned(),
        serde_json::Value::String(CURRENT_NSIS_INSTALLER_HOOK.to_owned()),
    );
    bundle["windows"]["nsis"]
        .as_object_mut()
        .expect("nsis object")
        .remove("installerHooks");
    assert!(
        prepare_bridge_tauri_config(&mut misplaced, "github").is_err(),
        "a top-level hook must not stand in for bundle.windows.nsis.installerHooks"
    );

    let mut wrong_hook = bridge_tauri_config();
    wrong_hook["bundle"]["windows"]["nsis"]["installerHooks"] =
        serde_json::Value::String("wrong-hook.nsh".to_owned());
    assert!(
        prepare_bridge_tauri_config(&mut wrong_hook, "github").is_err(),
        "unknown migration hooks must fail closed"
    );

    let mut bridge_config = bridge_tauri_config();
    prepare_bridge_tauri_config(&mut bridge_config, "github").unwrap();
    assert!(
        validate_bridge_tauri_config(&bridge_config, "pumni").is_err(),
        "Tauri publisher must agree with the bridge manifest publisher"
    );
    assert!(
        validate_historical_bridge_manifest_identity("pumni", UPDATER_PRODUCT_IDENTIFIER).is_err()
    );
    assert!(validate_historical_bridge_manifest_identity("github", "wrong.identifier").is_err());
}

#[cfg(windows)]
fn bridge_test_roots() -> (PathBuf, PathBuf, PathBuf) {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let base = std::env::temp_dir().join(format!(
        "sky-bridge-cleanup-test-{}-{nonce:x}",
        std::process::id()
    ));
    let runner_temp = base.join("runner-temp");
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&runner_temp).expect("create synthetic runner temp");
    std::fs::create_dir_all(&workspace).expect("create synthetic workspace");
    (base, runner_temp, workspace)
}

#[cfg(windows)]
#[test]
fn git_worktree_add_and_remove_accepts_verified_ordinary_windows_paths() {
    struct TestRoot(PathBuf);
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let (base, runner_temp, workspace) = bridge_test_roots();
    let _root = TestRoot(base.clone());
    let repository = workspace.join("source repository");
    std::fs::create_dir(&repository).expect("create isolated synthetic Git repository");
    let repository = std::fs::canonicalize(&repository).expect("canonicalize source repo");
    let repository_cli = verified_ordinary_windows_path(&repository)
        .expect("source repository must have an ordinary Git-compatible path");

    let init = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(&repository_cli)
        .output()
        .expect("run isolated git init");
    assert!(init.status.success(), "git init failed: {init:?}");
    for (key, value) in [
        ("user.name", "Sky Auto Player fixture"),
        ("user.email", "fixture@example.invalid"),
        ("commit.gpgsign", "false"),
        ("core.autocrlf", "false"),
    ] {
        let configured = std::process::Command::new("git")
            .args(["config", key, value])
            .current_dir(&repository_cli)
            .output()
            .expect("configure isolated synthetic Git repository");
        assert!(
            configured.status.success(),
            "git config failed: {configured:?}"
        );
    }
    std::fs::write(
        repository.join("fixture.txt"),
        b"synthetic worktree fixture\n",
    )
    .expect("write synthetic Git source file");
    let add_source = std::process::Command::new("git")
        .args(["add", "--", "fixture.txt"])
        .current_dir(&repository_cli)
        .output()
        .expect("stage synthetic Git source file");
    assert!(
        add_source.status.success(),
        "git add failed: {add_source:?}"
    );
    let commit = std::process::Command::new("git")
        .args(["commit", "--quiet", "--no-gpg-sign", "-m", "fixture"])
        .current_dir(&repository_cli)
        .output()
        .expect("commit synthetic Git source file");
    assert!(commit.status.success(), "git commit failed: {commit:?}");
    let head = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(&repository_cli)
        .output()
        .expect("read synthetic Git HEAD");
    assert!(head.status.success(), "git rev-parse failed: {head:?}");
    let source_sha = String::from_utf8(head.stdout)
        .expect("synthetic Git SHA is UTF-8")
        .trim()
        .to_owned();

    let fixture_root = runner_temp.join("bridge fixture with spaces");
    std::fs::create_dir(&fixture_root).expect("create synthetic bridge root");
    let canonical_fixture_root =
        std::fs::canonicalize(&fixture_root).expect("canonicalize synthetic bridge root");
    let ordinary_fixture_root = verified_ordinary_windows_path(&canonical_fixture_root)
        .expect("bridge root must normalize to a verified ordinary Git path");
    let cargo_target = ordinary_fixture_root.join("target");
    let cargo_target_text = cargo_target
        .to_str()
        .expect("synthetic Cargo target path is UTF-8");
    assert!(
        !cargo_target_text.starts_with(r"\\?\"),
        "Cargo target paths must not retain the Windows extended-length prefix"
    );
    assert_eq!(
        std::fs::canonicalize(cargo_target.parent().unwrap())
            .expect("normalize Cargo target parent"),
        canonical_fixture_root
    );
    let worktree = ordinary_fixture_root.join("source worktree");
    let worktree_text = worktree
        .to_str()
        .expect("synthetic Git worktree path is UTF-8")
        .to_owned();
    assert!(
        !worktree_text.starts_with(r"\\?\"),
        "Git CLI path must not retain the Windows extended-length prefix"
    );

    let added = std::process::Command::new("git")
        .args(["worktree", "add", "--detach"])
        .arg(&worktree_text)
        .arg(&source_sha)
        .current_dir(&repository_cli)
        .output()
        .expect("add isolated synthetic Git worktree");
    assert!(
        added.status.success(),
        "git worktree add failed: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(
        std::fs::read(worktree.join("fixture.txt")).expect("read synthetic worktree file"),
        b"synthetic worktree fixture\n"
    );

    // A generated dependency tree can exceed Windows MAX_PATH even though
    // the worktree root itself is an ordinary, verified Git path.
    let long_untracked_path = std::fs::canonicalize(&worktree)
        .expect("canonicalize synthetic worktree")
        .join("node_modules")
        .join("nested-dependency-".repeat(8))
        .join("generated-files-".repeat(8))
        .join("artifact.js");
    assert!(
        long_untracked_path.to_string_lossy().len() > 260,
        "fixture must exercise a path longer than Windows MAX_PATH"
    );
    std::fs::create_dir_all(long_untracked_path.parent().unwrap())
        .expect("create long untracked dependency path");
    std::fs::write(&long_untracked_path, b"generated dependency fixture\n")
        .expect("write long untracked dependency fixture");

    let removed = std::process::Command::new("git")
        .args(["-c", "core.longpaths=true", "worktree", "remove", "--force"])
        .arg(&worktree_text)
        .current_dir(&repository_cli)
        .output()
        .expect("remove isolated synthetic Git worktree");
    assert!(
        removed.status.success(),
        "git worktree remove failed: {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(!worktree.exists(), "synthetic Git worktree must be removed");
    let remaining = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&repository_cli)
        .output()
        .expect("inspect isolated Git worktree registrations");
    assert!(
        remaining.status.success(),
        "git worktree list failed: {remaining:?}"
    );
    assert_eq!(
        String::from_utf8_lossy(&remaining.stdout)
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        1,
        "only the synthetic repository's primary worktree may remain"
    );
}

#[cfg(windows)]
#[test]
#[ignore = "requires the pinned Bun desktop dependencies; invoked by updater_contract CI"]
fn tauri_signer_pair_is_verified_by_rust_and_rejects_malformed_or_wrong_pair() {
    struct TestRoot(PathBuf);
    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sky-tauri-signer-pair-test-{}-{nonce:x}",
        std::process::id()
    ));
    std::fs::create_dir(&root).expect("create disposable signer test root");
    let _root = TestRoot(root.clone());
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let first_key = root.join("first.key");
    let second_key = root.join("second.key");
    generate_signing_key(&repository_root, &first_key).expect("generate first Tauri key pair");
    generate_signing_key(&repository_root, &second_key).expect("generate second Tauri key pair");

    let payload = b"disposable Tauri signer verification fixture\n";
    let payload_path = root.join("payload.bin");
    std::fs::write(&payload_path, payload).expect("write harmless signer challenge");
    sign_with_key(&repository_root, &first_key, &payload_path)
        .expect("Tauri signer must sign the harmless challenge");
    let signature = read_bounded_text(
        &with_sig_extension(&payload_path),
        MAX_FIXTURE_SIGNATURE_BYTES,
    )
    .expect("read bounded Tauri signature");
    let first_public = read_bounded_text(&with_pub_extension(&first_key), 4096)
        .expect("read first Tauri public key");
    let second_public = read_bounded_text(&with_pub_extension(&second_key), 4096)
        .expect("read second Tauri public key");

    crate::updater_trust::verify_signature_text_with_public_key(payload, &signature, &first_public)
        .expect("Rust Minisign verifier must accept the actual Tauri signer output");
    assert!(
        crate::updater_trust::verify_signature_text_with_public_key(
            payload,
            &signature,
            "malformed public key",
        )
        .is_err(),
        "malformed public keys must fail closed"
    );
    assert!(
        crate::updater_trust::verify_signature_text_with_public_key(
            payload,
            "malformed signature",
            &first_public,
        )
        .is_err(),
        "malformed signatures must fail closed"
    );
    assert!(
        crate::updater_trust::verify_signature_text_with_public_key(
            payload,
            &signature,
            &second_public,
        )
        .is_err(),
        "a different Tauri key pair must not validate the signature"
    );
}

fn response(port: u16, path: &str) -> HttpResponse {
    http_get_loopback(&format!("http://127.0.0.1:{port}{path}")).unwrap()
}

#[test]
fn loopback_fixture_serves_exact_bytes_and_records_status_length_and_digest() {
    let manifest = br#"{"version":"4.2.0","platforms":{"windows-x86_64":{"signature":"exact-signature","url":"http://127.0.0.1:42000/candidate/update.exe"}}}"#.to_vec();
    let installer = b"fixed NSIS candidate bytes\0\xff".to_vec();
    let mut server = FixtureServer::start(manifest.clone(), installer.clone()).unwrap();
    let manifest_response = response(server.port(), "/stable");
    assert_eq!(manifest_response.status, 200);
    assert_eq!(manifest_response.content_type, "application/json");
    assert_eq!(manifest_response.content_length, manifest.len() as u64);
    assert_eq!(manifest_response.body, manifest);
    let candidate_response = response(server.port(), "/candidate/update.exe");
    assert_eq!(candidate_response.status, 200);
    assert_eq!(candidate_response.content_type, "application/octet-stream");
    assert_eq!(candidate_response.content_length, installer.len() as u64);
    assert_eq!(candidate_response.body, installer);
    let missing = response(server.port(), "/unapproved");
    assert_eq!(missing.status, 404);
    server.stop().unwrap();
    let requests = server.requests().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].body_sha256, sha256_bytes(&manifest));
    assert_eq!(requests[1].body_sha256, sha256_bytes(&installer));
    assert_eq!(requests[2].status_code, 404);
    assert_eq!(requests[2].content_type, "text/plain");
}

#[test]
fn loopback_client_rejects_truncated_response_bodies() {
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let responder = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut chunk = [0_u8; 256];
        while request.len() < 2048 && !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
        }
        assert!(
            request.windows(4).any(|window| window == b"\r\n\r\n"),
            "synthetic HTTP client must finish its request before the server responds"
        );
        stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\nConnection: close\r\n\r\nshort")
                .unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
    });
    let error =
        http_get_loopback(&format!("http://127.0.0.1:{}/stable", address.port())).unwrap_err();
    assert!(error.to_string().contains("body length mismatch"));
    responder.join().unwrap();
}

#[test]
fn fixture_server_teardown_is_bounded_for_a_stalled_partial_request() {
    let mut server = FixtureServer::start(b"manifest".to_vec(), b"candidate".to_vec()).unwrap();
    let mut stalled = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
    stalled
        .write_all(b"GET /stable HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    let started = Instant::now();
    let stop = server.stop();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        stop.is_ok(),
        "stop must cancel the partial request cleanly: {stop:?}"
    );
    assert!(
        server.thread.is_none(),
        "stop must reap the listener thread"
    );
    drop(stalled);
}

#[test]
fn fixture_response_writer_observes_an_absolute_deadline() {
    struct SlowWriter {
        writes: usize,
    }
    impl Write for SlowWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.writes += 1;
            thread::sleep(Duration::from_millis(20));
            Ok(bytes.len().min(1))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut writer = SlowWriter { writes: 0 };
    let started = Instant::now();
    let deadline = started + Duration::from_millis(55);
    let error =
        write_all_before_deadline(&mut writer, b"response", deadline, |_, _| Ok(())).unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_millis(500));
    assert!(
        writer.writes <= 4,
        "writer must stop at the absolute deadline"
    );
}

#[test]
fn fixture_server_shutdown_interrupts_a_slow_response_reader_and_reaps_thread() {
    let body = vec![0x5a; 32 * 1024 * 1024];
    let mut server = FixtureServer::start(b"manifest".to_vec(), body).unwrap();
    let mut slow_reader = TcpStream::connect((Ipv4Addr::LOCALHOST, server.port())).unwrap();
    slow_reader
        .write_all(b"GET /candidate/update.exe HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .unwrap();

    let request_deadline = Instant::now() + Duration::from_secs(3);
    while server.requests().unwrap().is_empty() && Instant::now() < request_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(server.requests().unwrap().len(), 1);
    thread::sleep(Duration::from_millis(75));

    let started = Instant::now();
    server.stop().unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(
        server.thread.is_none(),
        "stop must positively reap the server thread"
    );
    drop(slow_reader);
}

#[test]
fn server_switches_only_manifest_payload_and_request_limit_is_bounded() {
    let mut server = FixtureServer::start(b"manifest A".to_vec(), b"candidate".to_vec()).unwrap();
    let first = response(server.port(), "/stable");
    assert_eq!(first.body, b"manifest A");
    server.set_manifest(b"manifest B".to_vec()).unwrap();
    let second = response(server.port(), "/stable");
    assert_eq!(second.body, b"manifest B");
    assert_eq!(
        response(server.port(), "/candidate/update.exe").body,
        b"candidate"
    );
    server.stop().unwrap();
    let evidence = server.requests().unwrap();
    assert!(
        evidence
            .iter()
            .any(|entry| entry.body_sha256 == sha256_bytes(b"manifest A"))
    );
    assert!(
        evidence
            .iter()
            .any(|entry| entry.body_sha256 == sha256_bytes(b"manifest B"))
    );
}

#[test]
fn manifest_contract_fails_closed_on_version_url_signature_and_json_errors() {
    let valid = make_manifest(
        "4.2.0",
        "signature",
        "http://127.0.0.1:42000/candidate/update.exe",
        "test",
    )
    .unwrap();
    validate_manifest_bytes(
        &valid,
        "4.2.0",
        "http://127.0.0.1:42000/candidate/update.exe",
    )
    .unwrap();
    assert!(
        validate_manifest_bytes(
            &valid,
            "4.2.1",
            "http://127.0.0.1:42000/candidate/update.exe"
        )
        .is_err()
    );
    assert!(validate_manifest_bytes(&valid, "4.2.0", "http://127.0.0.1:42000/other").is_err());
    assert!(validate_manifest_bytes(b"not json", "4.2.0", "url").is_err());
    assert!(make_manifest("4.2.0", "", "url", "test").is_err());
}

#[test]
fn updater_options_reject_unknown_duplicate_and_incomplete_values() {
    assert!(parse_options(&["--bad".to_owned(), "x".to_owned()]).is_err());
    assert!(parse_options(&["--candidate-root".to_owned()]).is_err());
    assert!(
        parse_options(&[
            "--source-sha".to_owned(),
            "a".repeat(40),
            "--source-sha".to_owned(),
            "b".repeat(40),
        ])
        .is_err()
    );
    assert!(
        parse_options(&[
            "--allow-hosted-disposable-runner".to_owned(),
            "--allow-protected-release-runner".to_owned(),
        ])
        .is_err()
    );
}

#[cfg(not(windows))]
#[test]
fn actual_updater_installer_command_fails_closed_off_windows() {
    let result = run(Path::new("."), &["run".to_owned()]);
    assert!(result.unwrap_err().to_string().contains("Windows-only"));
}

#[test]
fn exact_loopback_client_refuses_public_and_proxyable_destinations() {
    assert!(http_get_loopback("http://example.com/stable").is_err());
    assert!(http_get_loopback("https://127.0.0.1:42000/stable").is_err());
}

#[cfg(windows)]
#[test]
fn bridge_build_cleanup_requires_direct_runner_temp_and_exact_owner_sentinel() {
    let (base, runner_temp, workspace) = bridge_test_roots();
    let root = runner_temp.join("owned-build-root");
    std::fs::create_dir(&root).expect("create synthetic bridge root");
    std::fs::write(
        root.join(BRIDGE_BUILD_OWNER_SENTINEL),
        BRIDGE_BUILD_OWNER_BYTES,
    )
    .expect("write synthetic ownership sentinel");
    std::fs::create_dir(root.join("target")).expect("create synthetic build output");
    remove_owned_bridge_build_root(&root, &runner_temp, &workspace)
        .expect("remove owned synthetic build root");
    assert!(!root.exists());

    let tampered = runner_temp.join("tampered-build-root");
    std::fs::create_dir(&tampered).expect("create tampered root");
    std::fs::write(tampered.join(BRIDGE_BUILD_OWNER_SENTINEL), b"foreign owner")
        .expect("write foreign sentinel");
    assert!(remove_owned_bridge_build_root(&tampered, &runner_temp, &workspace).is_err());
    assert!(
        tampered.exists(),
        "invalid ownership proof must retain the tree"
    );

    let outside = workspace.join("outside-build-root");
    std::fs::create_dir(&outside).expect("create out-of-scope root");
    std::fs::write(
        outside.join(BRIDGE_BUILD_OWNER_SENTINEL),
        BRIDGE_BUILD_OWNER_BYTES,
    )
    .expect("write out-of-scope sentinel");
    assert!(remove_owned_bridge_build_root(&outside, &runner_temp, &workspace).is_err());
    assert!(
        outside.exists(),
        "out-of-scope build roots must be retained"
    );
    std::fs::remove_dir_all(base).expect("remove only synthetic build-test roots");
}

#[cfg(windows)]
#[test]
fn bridge_build_entry_rollback_accepts_only_a_partial_created_sentinel() {
    let (base, runner_temp, workspace) = bridge_test_roots();
    let root = runner_temp.join("partial-build-root");
    std::fs::create_dir(&root).expect("create partial root");
    std::fs::write(
        root.join(BRIDGE_BUILD_OWNER_SENTINEL),
        &BRIDGE_BUILD_OWNER_BYTES[..9],
    )
    .expect("write partial ownership sentinel");
    remove_incomplete_bridge_build_root(&root, &runner_temp, &workspace, true)
        .expect("rollback partial owner sentinel");
    assert!(!root.exists());

    let foreign = runner_temp.join("foreign-partial-root");
    std::fs::create_dir(&foreign).expect("create foreign partial root");
    std::fs::write(foreign.join(BRIDGE_BUILD_OWNER_SENTINEL), b"wrong")
        .expect("write foreign sentinel");
    assert!(remove_incomplete_bridge_build_root(&foreign, &runner_temp, &workspace, true).is_err());
    assert!(foreign.exists());
    std::fs::remove_dir_all(base).expect("remove only synthetic rollback-test roots");
}

#[cfg(windows)]
#[test]
fn fixture_manifest_and_http_response_bytes_have_no_implicit_encoding() {
    let signature = "untrusted comment: fixture\nSGVsbG8=";
    let manifest = make_manifest(
        "4.2.0",
        signature,
        "http://127.0.0.1:42000/candidate/update.exe",
        "notes",
    )
    .unwrap();
    assert_eq!(manifest.last(), Some(&b'\n'));
    validate_manifest_bytes(
        &manifest,
        "4.2.0",
        "http://127.0.0.1:42000/candidate/update.exe",
    )
    .unwrap();
}
