use super::*;
use crate::commands::UpdateInstallAckDto;
use crate::ui_events::{
    UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateSnapshotPayload, UpdateState,
};

fn test_snapshot(
    state: UpdateState,
    error_code: Option<UpdateErrorCode>,
    error_detail: Option<&str>,
    retry_action: UpdateRetryAction,
) -> UpdateSnapshotPayload {
    UpdateSnapshotPayload {
        revision: 1,
        state,
        current_version: "4.0.0-alpha.1".to_string(),
        available_version: Some("4.0.0-alpha.2".to_string()),
        channel: UpdateChannel::Stable,
        release_notes: None,
        published_at: None,
        error_code,
        error_detail: error_detail.map(|s| s.to_string()),
        retry_action,
        operation_id: None,
        progress: None,
    }
}

#[test]
fn accepted_install_waits_for_updater_lifecycle() {
    let ack = UpdateInstallAckDto { accepted: true };
    let result = packaged_update_install_ack_result(ack, None);
    assert_eq!(result, Ok(false));
}

#[test]
fn rejected_install_becomes_packaged_selftest_failure() {
    let ack = UpdateInstallAckDto { accepted: false };
    let snapshot = test_snapshot(
        UpdateState::Error,
        Some(UpdateErrorCode::DownloadFailed),
        Some("update download failed: signature verification failed"),
        UpdateRetryAction::Install,
    );
    let result = packaged_update_install_ack_result(ack, Some(&snapshot));
    let err = result.expect_err("rejected install must return Err");
    assert!(
        err.contains("DownloadFailed"),
        "error message should contain 'DownloadFailed': {err}"
    );
    assert!(
        err.contains("signature verification failed"),
        "error message should contain 'signature verification failed': {err}"
    );
}

#[test]
fn rejected_install_requires_error_state() {
    let ack = UpdateInstallAckDto { accepted: false };
    let snapshot = test_snapshot(UpdateState::Available, None, None, UpdateRetryAction::None);
    let result = packaged_update_install_ack_result(ack, Some(&snapshot));
    let err = result.expect_err("rejected install without Error state must return Err");
    assert!(
        err.contains("rejected without Error state"),
        "error message should contain 'rejected without Error state': {err}"
    );
}

#[test]
fn rejected_install_requires_typed_error() {
    let ack = UpdateInstallAckDto { accepted: false };
    let snapshot = test_snapshot(
        UpdateState::Error,
        None,
        Some("untyped error"),
        UpdateRetryAction::Check,
    );
    let result = packaged_update_install_ack_result(ack, Some(&snapshot));
    let err = result.expect_err("rejected install without typed error code must return Err");
    assert!(
        err.contains("without a typed native error code"),
        "error message should contain 'without a typed native error code': {err}"
    );
}

#[test]
fn rejected_install_without_snapshot_fails_closed() {
    let ack = UpdateInstallAckDto { accepted: false };
    let result = packaged_update_install_ack_result(ack, None);
    let err = result.expect_err("rejected install without snapshot must return Err");
    assert!(
        err.contains("without an authoritative native snapshot"),
        "error message should contain 'without an authoritative native snapshot': {err}"
    );
}
