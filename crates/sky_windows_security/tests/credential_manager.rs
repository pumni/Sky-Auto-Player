#![cfg(windows)]

use sky_windows_security::credential::{CredentialStore, CredentialTarget};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TARGET: AtomicU64 = AtomicU64::new(1);

struct IsolatedCredential(CredentialStore);

impl IsolatedCredential {
    fn new() -> Self {
        let suffix = format!("{:032x}", NEXT_TARGET.fetch_add(1, Ordering::Relaxed));
        let target =
            CredentialTarget::test_target(format!("SkyAutoPlayer/V4UpdaterUnitTest_{suffix}"))
                .expect("test target is constrained to its isolated namespace");
        let store = CredentialStore::new(target);
        let _ = store.delete();
        Self(store)
    }
}

impl Drop for IsolatedCredential {
    fn drop(&mut self) {
        let _ = self.0.delete();
    }
}

#[test]
fn session_credential_round_trips_and_delete_is_idempotent() {
    let fixture = IsolatedCredential::new();
    let secret = "benign credential manager regression sentinel";
    assert!(!fixture.0.has_nonempty().unwrap());
    fixture.0.write_session(secret).unwrap();
    assert!(fixture.0.has_nonempty().unwrap());
    assert!(fixture.0.is_session_persistent().unwrap());
    assert_eq!(fixture.0.read().unwrap().as_str(), secret);
    assert!(fixture.0.delete().unwrap());
    assert!(!fixture.0.delete().unwrap());
    assert!(!fixture.0.has_nonempty().unwrap());
}

#[test]
fn empty_oversized_and_nul_secrets_fail_before_credential_write() {
    let fixture = IsolatedCredential::new();
    assert!(fixture.0.write_session("").is_err());
    assert!(fixture.0.write_session("\0").is_err());
    assert!(fixture.0.write_session(&"x".repeat(2561)).is_err());
    assert!(!fixture.0.has_nonempty().unwrap());
}
