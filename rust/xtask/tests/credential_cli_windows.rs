#![cfg(windows)]

use sky_windows_security::credential::{CredentialStore, CredentialTarget};
use std::io::Write;
use std::process::{Command, Stdio};

struct IsolatedTarget {
    name: String,
    store: CredentialStore,
}

impl Drop for IsolatedTarget {
    fn drop(&mut self) {
        let _ = self.store.delete();
    }
}

#[test]
fn stdin_provisioning_read_status_delete_and_output_redaction_use_isolated_target() {
    let suffix = format!(
        "{:032x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let target = format!("SkyAutoPlayer/V4UpdaterUnitTest_{suffix}");
    let isolated = IsolatedTarget {
        store: CredentialStore::new(
            CredentialTarget::test_target(target.clone()).expect("isolated test target"),
        ),
        name: target,
    };
    let secret = format!("BENIGN_RUST_CREDENTIAL_SENTINEL_{suffix}");
    let binary = env!("CARGO_BIN_EXE_xtask");

    let mut writer = Command::new(binary)
        .args([
            "credential",
            "write-stdin",
            "--test-target",
            isolated.name.as_str(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("xtask credential writer must start");
    writer
        .stdin
        .take()
        .expect("writer stdin must be piped")
        .write_all(secret.as_bytes())
        .expect("test password must reach Rust only through stdin");
    let write_result = writer
        .wait_with_output()
        .expect("credential writer must finish");
    assert!(
        write_result.status.success(),
        "writer failed: {:?}",
        write_result.stderr
    );
    assert!(!String::from_utf8_lossy(&write_result.stdout).contains(&secret));
    assert!(!String::from_utf8_lossy(&write_result.stderr).contains(&secret));

    let status = Command::new(binary)
        .args([
            "credential",
            "status",
            "--test-target",
            isolated.name.as_str(),
        ])
        .output()
        .expect("status command must start");
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("PRESENT_NONEMPTY"));
    assert!(!String::from_utf8_lossy(&status.stdout).contains(&secret));

    let read = Command::new(binary)
        .args([
            "credential",
            "read-check",
            "--test-target",
            isolated.name.as_str(),
        ])
        .output()
        .expect("read check must start");
    assert!(read.status.success());
    assert!(!String::from_utf8_lossy(&read.stdout).contains(&secret));
    assert!(!String::from_utf8_lossy(&read.stderr).contains(&secret));

    let delete = Command::new(binary)
        .args([
            "credential",
            "delete",
            "--test-target",
            isolated.name.as_str(),
        ])
        .output()
        .expect("delete command must start");
    assert!(delete.status.success());
    assert!(!String::from_utf8_lossy(&delete.stdout).contains(&secret));
    assert!(!String::from_utf8_lossy(&delete.stderr).contains(&secret));

    let duplicate_delete = Command::new(binary)
        .args([
            "credential",
            "delete",
            "--test-target",
            isolated.name.as_str(),
        ])
        .output()
        .expect("duplicate delete command must start");
    assert!(duplicate_delete.status.success());
}

#[test]
fn actual_cli_action_rejects_ambient_password_and_deletes_after_child_failure() {
    let suffix = format!(
        "{:032x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let target = format!("SkyAutoPlayer/V4UpdaterUnitTest_{suffix}");
    let isolated = IsolatedTarget {
        store: CredentialStore::new(
            CredentialTarget::test_target(target.clone()).expect("isolated test target"),
        ),
        name: target,
    };
    let secret = format!("BENIGN_RUST_ACTION_SECRET_{suffix}");
    isolated
        .store
        .write_session(&secret)
        .expect("write synthetic session credential");
    let key_path = std::env::temp_dir().join(format!(
        "sky-updater-key-intentionally-missing-{suffix}.key"
    ));
    let binary = env!("CARGO_BIN_EXE_xtask");
    let ambient = format!("BENIGN_AMBIENT_PASSWORD_{suffix}");

    let mut rejected_ambient = Command::new(binary);
    rejected_ambient
        .args([
            "credential",
            "run-action",
            "--action",
            "verify-private-key",
            "--key-path",
        ])
        .arg(&key_path)
        .args(["--test-target", isolated.name.as_str()])
        .env("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", &ambient)
        .env_remove("TAURI_SIGNING_PRIVATE_KEY")
        .env_remove("TAURI_SIGNING_PRIVATE_KEY_PATH");
    let rejected_ambient = rejected_ambient
        .output()
        .expect("ambient-password rejection command must start");
    assert!(!rejected_ambient.status.success());
    let rejected_stdout = String::from_utf8_lossy(&rejected_ambient.stdout);
    let rejected_stderr = String::from_utf8_lossy(&rejected_ambient.stderr);
    assert!(!rejected_stdout.contains(&secret));
    assert!(!rejected_stderr.contains(&secret));
    assert!(!rejected_stdout.contains(&ambient));
    assert!(!rejected_stderr.contains(&ambient));
    assert!(
        isolated.store.has_nonempty().unwrap(),
        "ambient-password rejection must not consume the isolated credential"
    );

    let mut action = Command::new(binary);
    action
        .args([
            "credential",
            "run-action",
            "--action",
            "verify-private-key",
            "--key-path",
        ])
        .arg(&key_path)
        .args(["--test-target", isolated.name.as_str()])
        .env_remove("TAURI_SIGNING_PRIVATE_KEY_PASSWORD")
        .env_remove("TAURI_SIGNING_PRIVATE_KEY")
        .env_remove("TAURI_SIGNING_PRIVATE_KEY_PATH");
    let action = action
        .output()
        .expect("synthetic private-key verification action must start");
    assert!(
        !action.status.success(),
        "a missing synthetic key must fail closed"
    );
    let stdout = String::from_utf8_lossy(&action.stdout);
    let stderr = String::from_utf8_lossy(&action.stderr);
    assert!(!stdout.contains(&secret));
    assert!(!stderr.contains(&secret));
    assert!(
        !isolated.store.has_nonempty().unwrap(),
        "failed child action must still delete the isolated session credential"
    );
}
