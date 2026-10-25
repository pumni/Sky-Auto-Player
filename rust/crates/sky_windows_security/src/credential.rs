use crate::NativeBoundaryError;
use std::ffi::OsStr;
use zeroize::Zeroizing;

pub const CANONICAL_UPDATER_CREDENTIAL_TARGET: &str = "SkyAutoPlayer/V4UpdaterProduction";
pub const MAX_CREDENTIAL_BLOB: usize = 2560;
const TEST_TARGET_PREFIX: &str = "SkyAutoPlayer/V4UpdaterUnitTest_";

/// A fixed production target or a narrowly validated disposable test target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialTarget(String);

impl CredentialTarget {
    pub fn canonical() -> Self {
        Self(CANONICAL_UPDATER_CREDENTIAL_TARGET.to_owned())
    }

    /// Creates a non-production target whose suffix is exactly a 32 digit
    /// hexadecimal identifier. This seam cannot address the canonical target.
    pub fn test_target(target: impl Into<String>) -> Result<Self, NativeBoundaryError> {
        let target = target.into();
        let Some(suffix) = target.strip_prefix(TEST_TARGET_PREFIX) else {
            return Err(NativeBoundaryError::new(
                "credential test target must use the isolated unit-test prefix",
            ));
        };
        if suffix.len() != 32 || !suffix.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(NativeBoundaryError::new(
                "credential test target must end in a 32 digit hexadecimal identifier",
            ));
        }
        Ok(Self(target))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Windows Credential Manager owner for one constrained target.
#[derive(Debug, Clone)]
pub struct CredentialStore {
    target: CredentialTarget,
}

impl CredentialStore {
    pub fn new(target: CredentialTarget) -> Self {
        Self { target }
    }

    pub fn target(&self) -> &CredentialTarget {
        &self.target
    }

    pub fn has_nonempty(&self) -> Result<bool, NativeBoundaryError> {
        platform::has_nonempty(self.target.as_str())
    }

    pub fn is_session_persistent(&self) -> Result<bool, NativeBoundaryError> {
        platform::is_session_persistent(self.target.as_str())
    }

    pub fn read(&self) -> Result<Zeroizing<String>, NativeBoundaryError> {
        let secret = platform::read(self.target.as_str())?;
        if secret.is_empty() {
            return Err(NativeBoundaryError::new(
                "Windows Credential Manager credential blob is empty",
            ));
        }
        if secret.contains('\0') {
            return Err(NativeBoundaryError::new(
                "Windows Credential Manager credential contains a NUL character",
            ));
        }
        if secret.len() > MAX_CREDENTIAL_BLOB {
            return Err(NativeBoundaryError::new(
                "Windows Credential Manager credential blob exceeds the supported limit",
            ));
        }
        Ok(secret)
    }

    pub fn write_session(&self, secret: &str) -> Result<(), NativeBoundaryError> {
        if secret.is_empty() {
            return Err(NativeBoundaryError::new(
                "credential secret cannot be empty",
            ));
        }
        if secret.contains('\0') {
            return Err(NativeBoundaryError::new(
                "credential secret cannot contain a NUL character",
            ));
        }
        if secret.len() > MAX_CREDENTIAL_BLOB {
            return Err(NativeBoundaryError::new(
                "credential secret exceeds the Windows Credential Manager blob limit",
            ));
        }
        platform::write(self.target.as_str(), secret)
    }

    /// Deletes the credential and succeeds when it was already absent.
    pub fn delete(&self) -> Result<bool, NativeBoundaryError> {
        platform::delete(self.target.as_str())
    }
}

/// Minimal store contract used to test the one-shot lifecycle without any
/// Windows account or canonical credential state.
pub trait SessionCredentialOps {
    fn read(&self) -> Result<Zeroizing<String>, String>;
    fn delete(&self) -> Result<(), String>;
}

impl SessionCredentialOps for CredentialStore {
    fn read(&self) -> Result<Zeroizing<String>, String> {
        CredentialStore::read(self).map_err(|error| error.to_string())
    }

    fn delete(&self) -> Result<(), String> {
        CredentialStore::delete(self)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialBoundaryError {
    action: String,
    cleanup: Option<String>,
}

impl std::fmt::Display for CredentialBoundaryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.action.is_empty(), self.cleanup.as_deref()) {
            (false, Some(cleanup)) => write!(
                formatter,
                "V4 updater credential boundary failure: {}; session credential cleanup also failed: {cleanup}",
                self.action
            ),
            (false, None) => write!(formatter, "{}", self.action),
            (true, Some(cleanup)) => write!(
                formatter,
                "V4 updater credential boundary failure: session credential cleanup failed: {cleanup}"
            ),
            (true, None) => formatter.write_str("V4 updater credential action failed"),
        }
    }
}

impl std::error::Error for CredentialBoundaryError {}

/// Reads the one-shot secret, executes the action, and always attempts
/// idempotent deletion. Secret text is redacted from both failure channels.
pub fn run_one_shot<T>(
    store: &impl SessionCredentialOps,
    action: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, CredentialBoundaryError> {
    let (secret, mut action_error) = match store.read() {
        Ok(secret) => (Some(secret), None),
        Err(error) => (None, Some(error)),
    };
    let mut result = None;
    if let Some(secret) = secret.as_deref() {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| action(secret))) {
            Ok(Ok(value)) => result = Some(value),
            Ok(Err(error)) => action_error = Some(redact_secret(&error, secret)),
            Err(_) => action_error = Some("credential action panicked".into()),
        }
    }

    let secret_value = secret.as_deref().map_or("", String::as_str);
    let cleanup_error = store
        .delete()
        .err()
        .map(|error| redact_secret(&error, secret_value));
    if action_error.is_some() || cleanup_error.is_some() {
        return Err(CredentialBoundaryError {
            action: action_error
                .map(|error| redact_secret(&error, secret_value))
                .unwrap_or_default(),
            cleanup: cleanup_error,
        });
    }
    result.ok_or_else(|| CredentialBoundaryError {
        action: "credential read failed".into(),
        cleanup: None,
    })
}

fn redact_secret(message: &str, secret: &str) -> String {
    if secret.is_empty() {
        message.to_owned()
    } else {
        message.replace(secret, "[REDACTED]")
    }
}

/// A nonempty ambient password is untrusted caller state. The release action
/// must receive its password only from this one-shot credential boundary.
pub fn reject_ambient_password(value: Option<&OsStr>) -> Result<(), NativeBoundaryError> {
    if value.is_some_and(|value| !value.is_empty()) {
        Err(NativeBoundaryError::new(
            "ambient TAURI_SIGNING_PRIVATE_KEY_PASSWORD is forbidden",
        ))
    } else {
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::win32;

    pub(super) fn has_nonempty(target: &str) -> Result<bool, NativeBoundaryError> {
        win32::credential_has_nonempty(target)
    }
    pub(super) fn is_session_persistent(target: &str) -> Result<bool, NativeBoundaryError> {
        win32::credential_is_session_persistent(target)
    }
    pub(super) fn read(target: &str) -> Result<Zeroizing<String>, NativeBoundaryError> {
        win32::credential_read(target)
    }
    pub(super) fn write(target: &str, secret: &str) -> Result<(), NativeBoundaryError> {
        win32::credential_write(target, secret)
    }
    pub(super) fn delete(target: &str) -> Result<bool, NativeBoundaryError> {
        win32::credential_delete(target)
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    fn unsupported<T>() -> Result<T, NativeBoundaryError> {
        Err(NativeBoundaryError::new(
            "Windows Credential Manager is only available on Windows",
        ))
    }
    pub(super) fn has_nonempty(_target: &str) -> Result<bool, NativeBoundaryError> {
        unsupported()
    }
    pub(super) fn is_session_persistent(_target: &str) -> Result<bool, NativeBoundaryError> {
        unsupported()
    }
    pub(super) fn read(_target: &str) -> Result<Zeroizing<String>, NativeBoundaryError> {
        unsupported()
    }
    pub(super) fn write(_target: &str, _secret: &str) -> Result<(), NativeBoundaryError> {
        unsupported()
    }
    pub(super) fn delete(_target: &str) -> Result<bool, NativeBoundaryError> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FakeStore {
        read_result: Result<Zeroizing<String>, String>,
        delete_result: Result<(), String>,
        deleted: Cell<bool>,
    }

    impl SessionCredentialOps for FakeStore {
        fn read(&self) -> Result<Zeroizing<String>, String> {
            self.read_result.clone()
        }
        fn delete(&self) -> Result<(), String> {
            self.deleted.set(true);
            self.delete_result.clone()
        }
    }

    #[test]
    fn target_test_seam_cannot_select_canonical_or_arbitrary_entries() {
        let unique = format!("{TEST_TARGET_PREFIX}0123456789abcdef0123456789abcdef");
        assert!(CredentialTarget::test_target(unique).is_ok());
        assert!(CredentialTarget::test_target(CANONICAL_UPDATER_CREDENTIAL_TARGET).is_err());
        assert!(
            CredentialTarget::test_target(format!("{TEST_TARGET_PREFIX}../canonical")).is_err()
        );
    }

    #[test]
    fn one_shot_absent_or_empty_read_fails_closed_and_still_deletes() {
        let store = FakeStore {
            read_result: Err("credential is absent".into()),
            delete_result: Ok(()),
            deleted: Cell::new(false),
        };
        let error = run_one_shot(&store, |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains("absent"));
        assert!(store.deleted.get());

        let empty_store = FakeStore {
            read_result: Ok(Zeroizing::new(String::new())),
            delete_result: Ok(()),
            deleted: Cell::new(false),
        };
        // Empty blobs are rejected by the Windows backend before this seam;
        // this verifies an empty fake value cannot be treated as a valid key.
        let error = run_one_shot(&empty_store, |secret| {
            if secret.is_empty() {
                Err("empty credential".into())
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert!(error.to_string().contains("empty credential"));
        assert!(empty_store.deleted.get());
    }

    #[test]
    fn one_shot_runs_action_and_reports_only_sanitized_failure() {
        let secret = "V4_TEST_SENTINEL_DO_NOT_LOG";
        let store = FakeStore {
            read_result: Ok(Zeroizing::new(secret.into())),
            delete_result: Ok(()),
            deleted: Cell::new(false),
        };
        let error = run_one_shot(&store, |value| -> Result<(), String> {
            assert_eq!(value, secret);
            Err(format!("action failed while printing {value}"))
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("[REDACTED]"));
        assert!(!error.contains(secret));
        assert!(store.deleted.get());
    }

    #[test]
    fn one_shot_keeps_the_caller_password_environment_unchanged() {
        let store = FakeStore {
            read_result: Ok(Zeroizing::new("sentinel".into())),
            delete_result: Ok(()),
            deleted: Cell::new(false),
        };
        let before = std::env::var_os("TAURI_SIGNING_PRIVATE_KEY_PASSWORD");
        run_one_shot(&store, |_| Ok(())).unwrap();
        assert_eq!(
            std::env::var_os("TAURI_SIGNING_PRIVATE_KEY_PASSWORD"),
            before
        );
    }

    #[test]
    fn one_shot_deletes_after_action_panic() {
        let store = FakeStore {
            read_result: Ok(Zeroizing::new("sentinel".into())),
            delete_result: Ok(()),
            deleted: Cell::new(false),
        };
        let error = run_one_shot(&store, |_| -> Result<(), String> {
            panic!("controlled panic")
        })
        .unwrap_err();
        assert!(error.to_string().contains("action panicked"));
        assert!(store.deleted.get());
    }

    #[test]
    fn one_shot_surfaces_both_action_and_cleanup_failures_and_redacts_secret() {
        let secret = "V4_TEST_SENTINEL_DO_NOT_LOG";
        let store = FakeStore {
            read_result: Ok(Zeroizing::new(secret.into())),
            delete_result: Err("delete failed".into()),
            deleted: Cell::new(false),
        };
        let error = run_one_shot(&store, |_| -> Result<(), String> {
            Err("action failed".into())
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("action failed"));
        assert!(error.contains("cleanup also failed"));
        assert!(!error.contains(secret));
        assert!(store.deleted.get());
    }

    #[test]
    fn one_shot_fails_when_only_credential_deletion_fails() {
        let store = FakeStore {
            read_result: Ok(Zeroizing::new("sentinel".into())),
            delete_result: Err("delete failed".into()),
            deleted: Cell::new(false),
        };
        let error = run_one_shot(&store, |_| Ok(())).unwrap_err();
        assert!(error.to_string().contains("cleanup failed"));
        assert!(error.to_string().contains("delete failed"));
        assert!(store.deleted.get());
    }

    #[test]
    fn nonempty_ambient_password_is_rejected() {
        assert!(reject_ambient_password(Some(OsStr::new("ambient"))).is_err());
        assert!(reject_ambient_password(Some(OsStr::new(""))).is_ok());
        assert!(reject_ambient_password(None).is_ok());
    }
}
