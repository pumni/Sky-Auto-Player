use super::{
    DownloadProgressAccumulator, MAX_ARTIFACT_BYTES, bounded, first_verified_download,
    update_activity_error, verified_artifact_progress,
};
#[cfg(not(feature = "tauri-update-fixture"))]
use super::{
    V4_BETA_METADATA_ENDPOINT, V4_STABLE_METADATA_ENDPOINT, V4_TAURI_UPDATER_PUBLIC_KEY,
    V4_TAURI_UPDATER_PUBLIC_KEYS, metadata_endpoint, validate_official_metadata_endpoint,
};
use crate::app_state::ActivityReservationError;
#[cfg(not(feature = "tauri-update-fixture"))]
use crate::ui_events::UpdateChannel;

#[cfg(feature = "tauri-update-fixture")]
#[test]
fn fixture_new_only_mode_selects_only_the_last_supplied_root() {
    let keys = vec!["old-root".to_owned(), "new-root".to_owned()];
    assert_eq!(
        super::fixture_public_keys(&keys, true),
        vec![Some("new-root".to_owned())]
    );
    assert_eq!(
        super::fixture_public_keys(&keys, false),
        vec![Some("old-root".to_owned()), Some("new-root".to_owned())]
    );
}

#[cfg(feature = "tauri-update-fixture")]
#[test]
fn fixture_runtime_configuration_requires_a_loopback_port_and_public_keys() {
    let missing_port = super::fixture_runtime_config_from_args([
        super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
        "missing.pub".to_owned(),
    ]);
    assert!(missing_port.is_err());

    let missing_key = super::fixture_runtime_config_from_args([
        super::FIXTURE_PORT_ARG.to_owned(),
        "40000".to_owned(),
    ]);
    assert!(missing_key.is_err());
}

#[cfg(feature = "tauri-update-fixture")]
#[test]
fn fixture_runtime_configuration_rejects_invalid_port_and_private_key() {
    let root = std::env::temp_dir().join(format!(
        "sky-auto-player-fixture-key-{}",
        std::process::id()
    ));
    std::fs::write(&root, "PRIVATE KEY").expect("write fixture key");
    let invalid_port = super::fixture_runtime_config_from_args([
        super::FIXTURE_PORT_ARG.to_owned(),
        "0".to_owned(),
        super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
        root.to_string_lossy().into_owned(),
    ]);
    assert!(invalid_port.is_err());

    let private_key = super::fixture_runtime_config_from_args([
        super::FIXTURE_PORT_ARG.to_owned(),
        "40000".to_owned(),
        super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
        root.to_string_lossy().into_owned(),
    ]);
    assert!(private_key.is_err());
    let _ = std::fs::remove_file(root);
}

#[cfg(feature = "tauri-update-fixture")]
#[test]
fn fixture_runtime_configuration_reads_bounded_keys_and_selects_new_only() {
    let root = std::env::temp_dir().join(format!(
        "sky-auto-player-fixture-public-{}",
        std::process::id()
    ));
    std::fs::write(&root, "fixture-public-root").expect("write fixture key");
    let config = super::fixture_runtime_config_from_args([
        "--selftest-desktop-update".to_owned(),
        super::FIXTURE_PORT_ARG.to_owned(),
        "40000".to_owned(),
        super::FIXTURE_PUBLIC_KEY_ARG.to_owned(),
        root.to_string_lossy().into_owned(),
        super::FIXTURE_NEW_ONLY_ARG.to_owned(),
    ])
    .expect("valid fixture runtime configuration");
    assert_eq!(config.port, 40000);
    assert_eq!(config.public_keys, vec!["fixture-public-root"]);
    assert!(config.new_only);
    let _ = std::fs::remove_file(root);
}

#[cfg(not(feature = "tauri-update-fixture"))]
#[test]
fn production_metadata_endpoints_are_fixed_and_channel_isolated() {
    let stable = metadata_endpoint(UpdateChannel::Stable).unwrap();
    assert_eq!(stable.as_str(), V4_STABLE_METADATA_ENDPOINT);
    assert_eq!(stable.scheme(), "https");
    assert_eq!(stable.host_str(), Some("raw.githubusercontent.com"));

    let beta_err = metadata_endpoint(UpdateChannel::Beta).unwrap_err();
    assert!(beta_err.contains("channel_unavailable"));
    assert_eq!(
        V4_BETA_METADATA_ENDPOINT,
        "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json"
    );
}

#[cfg(not(feature = "tauri-update-fixture"))]
#[test]
fn production_metadata_allowlist_rejects_unapproved_endpoints() {
    let rejected = [
        (
            "http://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json",
            UpdateChannel::Stable,
        ),
        (
            "https://evil.example/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json",
            UpdateChannel::Stable,
        ),
        (
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/main/channels/stable/latest.json",
            UpdateChannel::Stable,
        ),
        (
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/beta/latest.json",
            UpdateChannel::Stable,
        ),
        (
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json?redirect=1",
            UpdateChannel::Stable,
        ),
        (
            "https://raw.githubusercontent.com/pumni/Sky-Auto-Player/release-metadata/channels/stable/latest.json/",
            UpdateChannel::Stable,
        ),
    ];

    for (raw, channel) in rejected {
        let endpoint = url::Url::parse(raw).unwrap();
        assert!(
            validate_official_metadata_endpoint(&endpoint, channel).is_err(),
            "endpoint should be rejected: {raw}"
        );
    }
}

#[cfg(not(feature = "tauri-update-fixture"))]
#[test]
fn production_updater_trust_root_is_independent_and_bounded() {
    assert_eq!(V4_TAURI_UPDATER_PUBLIC_KEYS, &[V4_TAURI_UPDATER_PUBLIC_KEY]);
    assert!(!V4_TAURI_UPDATER_PUBLIC_KEY.is_empty());
    assert!(V4_TAURI_UPDATER_PUBLIC_KEY.len() <= 4096);
    assert!(!V4_TAURI_UPDATER_PUBLIC_KEY.contains("PRIVATE KEY"));
    assert!(
        V4_TAURI_UPDATER_PUBLIC_KEY
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    );
}

#[test]
fn production_error_is_bounded_and_does_not_name_a_release_namespace() {
    let message = bounded("x".repeat(8_000));
    assert_eq!(message.len(), 4096);
    assert!(
        update_activity_error(ActivityReservationError::PhysicalPlaybackActive)
            .contains("playback_active")
    );
}

#[test]
fn download_progress_accumulates_chunk_lengths() {
    let mut accumulator = DownloadProgressAccumulator::default();
    let completed = [16_000_usize, 16_000, 8_000]
        .into_iter()
        .map(|chunk_length| {
            accumulator
                .record_chunk(chunk_length, Some(40_000))
                .expect("valid chunk should produce progress")
                .completed
        })
        .collect::<Vec<_>>();
    assert_eq!(completed, [16_000, 32_000, 40_000]);
}

#[test]
fn download_progress_never_decreases_within_an_attempt() {
    let mut accumulator = DownloadProgressAccumulator::default();
    let mut previous = 0;
    for chunk_length in [16_000_usize, 16_000, 8_000, 1_000] {
        let progress = accumulator
            .record_chunk(chunk_length, Some(41_000))
            .expect("valid chunk should produce progress");
        assert!(progress.completed >= previous);
        previous = progress.completed;
    }
}

#[test]
fn download_progress_stays_within_a_valid_total() {
    let mut accumulator = DownloadProgressAccumulator::default();
    for chunk_length in [16_000_usize, 16_000, 8_000] {
        let progress = accumulator
            .record_chunk(chunk_length, Some(40_000))
            .expect("valid chunk should produce progress");
        assert!(progress.completed <= progress.total.expect("total is valid"));
    }
}

#[test]
fn download_progress_accumulates_without_a_content_length() {
    let mut accumulator = DownloadProgressAccumulator::default();
    let progress = [16_000_usize, 16_000, 8_000]
        .into_iter()
        .map(|chunk_length| {
            accumulator
                .record_chunk(chunk_length, None)
                .expect("chunk without total should produce progress")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        progress
            .iter()
            .map(|value| value.completed)
            .collect::<Vec<_>>(),
        [16_000, 32_000, 40_000]
    );
    assert!(progress.iter().all(|value| value.total.is_none()));
}

#[test]
fn download_progress_fails_closed_at_the_artifact_bound() {
    let mut accumulator = DownloadProgressAccumulator::default();
    assert!(
        accumulator
            .record_chunk(MAX_ARTIFACT_BYTES as usize, None)
            .is_ok()
    );
    assert!(accumulator.record_chunk(1, None).is_err());

    let mut invalid_total = DownloadProgressAccumulator::default();
    let progress = invalid_total
        .record_chunk(1, Some(MAX_ARTIFACT_BYTES + 1))
        .expect("oversized content length is treated as unknown");
    assert_eq!(progress.total, None);

    let mut overflowed = DownloadProgressAccumulator {
        downloaded: u64::MAX,
    };
    assert!(overflowed.record_chunk(1, None).is_err());
}

#[test]
fn final_ready_and_installing_progress_use_exact_verified_artifact_length() {
    let bytes = vec![0_u8; 40_000];
    let ready = verified_artifact_progress(&bytes, "Update is ready to install");
    let installing = verified_artifact_progress(&bytes, "Installing update and restarting");
    assert_eq!(ready.completed, bytes.len() as u64);
    assert_eq!(ready.total, Some(bytes.len() as u64));
    assert_eq!(installing.completed, bytes.len() as u64);
    assert_eq!(installing.total, Some(bytes.len() as u64));
}

#[test]
fn production_fixture_manifest_is_tauri_consumable() {
    let manifest = serde_json::json!({
        "version": "4.0.0-rc.2",
        "notes": "Deterministic bridge rotation candidate.",
        "pub_date": "2026-09-04T00:00:00Z",
        "platforms": {
            "windows-x86_64": {
                "signature": "fixture-signature",
                "url": "http://127.0.0.1:40000/candidate/update.exe"
            }
        }
    });
    let release: tauri_plugin_updater::RemoteRelease =
        serde_json::from_value(manifest).expect("fixture manifest must match Tauri schema");
    assert_eq!(release.version.to_string(), "4.0.0-rc.2");
    assert_eq!(
        release
            .download_url("windows-x86_64")
            .expect("production platform must be present")
            .as_str(),
        "http://127.0.0.1:40000/candidate/update.exe"
    );
    assert_eq!(
        release
            .signature("windows-x86_64")
            .expect("production signature must be present"),
        "fixture-signature"
    );
}

#[test]
fn update_installation_has_a_distinct_playback_policy_error() {
    let message = update_activity_error(ActivityReservationError::PhysicalPlaybackActive);
    assert_eq!(
        message,
        "playback_active: update installation cannot run during physical playback"
    );
}

#[test]
fn runtime_rotation_download_falls_through_to_the_root_that_verifies_bytes() {
    let mut attempts = Vec::new();
    let (selected, bytes) = first_verified_download(vec!["old-root", "new-root"], |root| {
        attempts.push(*root);
        if *root == "old-root" {
            Err("signature mismatch".into())
        } else {
            Ok(b"new-root-signed-update".to_vec())
        }
    })
    .unwrap();
    assert_eq!(attempts, ["old-root", "new-root"]);
    assert_eq!(selected, "new-root");
    assert_eq!(bytes, b"new-root-signed-update");
}

#[test]
fn runtime_rotation_download_fails_closed_when_no_root_verifies_bytes() {
    let result = first_verified_download(vec!["old-root", "new-root"], |_| {
        Err::<Vec<u8>, _>("signature mismatch".into())
    });
    assert_eq!(result.unwrap_err(), "signature mismatch");
}

#[test]
fn classify_check_error_maps_known_classes() {
    use crate::ui_events::UpdateErrorCode;
    assert_eq!(
        super::classify_check_error("channel_unavailable: beta update channel is not supported"),
        UpdateErrorCode::ChannelUnavailable
    );
    assert_eq!(
        super::classify_check_error("connection refused while fetching"),
        UpdateErrorCode::CheckFailed
    );
}

#[test]
fn native_update_state_default_has_zero_revision_and_idle() {
    use crate::ui_events::{UpdateRetryAction, UpdateState};
    let state = super::NativeUpdateState::default();
    assert_eq!(state.revision, 0);
    assert_eq!(state.state, UpdateState::Idle);
    assert_eq!(state.retry_action, UpdateRetryAction::None);
    assert!(state.candidate.is_none());
    assert!(state.progress.is_none());
}

#[test]
fn check_disposition_checks_on_each_launch_and_respects_auto_check() {
    use crate::ui_events::{UpdateCheckDisposition, UpdateCheckOrigin};
    use sky_app_core::settings::UpdatePreferences;

    let mut prefs = UpdatePreferences::default();
    let now = 1_700_000_000;

    assert_eq!(
        super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
        UpdateCheckDisposition::Performed
    );

    prefs.auto_check = false;
    assert_eq!(
        super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
        UpdateCheckDisposition::Disabled
    );
    assert_eq!(
        super::check_disposition(UpdateCheckOrigin::Manual, &prefs, now),
        UpdateCheckDisposition::Performed
    );

    // Previous launches' successes must not hide a newly published version.
    prefs.auto_check = true;
    prefs.last_check_ts = now - 100;
    assert_eq!(
        super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
        UpdateCheckDisposition::Performed
    );

    // Even a recent failed check must be retried on the next app launch.
    prefs.last_error_ts = now - 20;
    assert_eq!(
        super::check_disposition(UpdateCheckOrigin::Background, &prefs, now),
        UpdateCheckDisposition::Performed
    );
}

#[test]
fn native_update_state_transitions_maintain_monotonic_revisions_and_invariants() {
    use super::{NativeUpdateCandidate, NativeUpdateState, StateTransition};
    use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

    let mut state = NativeUpdateState::default();
    assert_eq!(state.revision, 0);
    assert_eq!(state.snapshot().revision, 0);

    // 1. Idle -> Checking
    let (rev1, snap1) = state.apply_transition(StateTransition {
        state: UpdateState::Checking,
        channel: UpdateChannel::Stable,
        ..Default::default()
    });
    assert_eq!(rev1, 1);
    assert_eq!(snap1.revision, 1);
    assert_eq!(snap1.state, UpdateState::Checking);
    assert_eq!(state.snapshot(), snap1);

    // 2. Checking -> Available
    let candidate = NativeUpdateCandidate {
        version: "4.1.0".to_string(),
        channel: UpdateChannel::Stable,
        release_notes: Some("Notes".to_string()),
        published_at: Some("2026-09-01T00:00:00Z".to_string()),
    };
    let (rev2, snap2) = state.apply_transition(StateTransition {
        state: UpdateState::Available,
        channel: UpdateChannel::Stable,
        candidate: Some(candidate.clone()),
        ..Default::default()
    });
    assert_eq!(rev2, 2);
    assert_eq!(snap2.revision, 2);
    assert_eq!(snap2.state, UpdateState::Available);
    assert_eq!(snap2.available_version.as_deref(), Some("4.1.0"));
    assert_eq!(snap2.release_notes.as_deref(), Some("Notes"));
    assert_eq!(state.snapshot(), snap2);

    // 3. Available -> Current (candidate cleared)
    let (rev3, snap3) = state.apply_transition(StateTransition {
        state: UpdateState::Current,
        channel: UpdateChannel::Stable,
        ..Default::default()
    });
    assert_eq!(rev3, 3);
    assert_eq!(snap3.revision, 3);
    assert_eq!(snap3.state, UpdateState::Current);
    assert!(snap3.available_version.is_none());
    assert_eq!(state.snapshot(), snap3);

    // 4. Checking error -> error fields set, candidate None, retry Check
    let (rev4, snap4) = state.apply_transition(StateTransition {
        state: UpdateState::Error,
        channel: UpdateChannel::Stable,
        error_code: Some(UpdateErrorCode::CheckFailed),
        error_detail: Some("network error".to_string()),
        retry_action: UpdateRetryAction::Check,
        ..Default::default()
    });
    assert_eq!(rev4, 4);
    assert_eq!(snap4.revision, 4);
    assert_eq!(snap4.state, UpdateState::Error);
    assert_eq!(snap4.error_code, Some(UpdateErrorCode::CheckFailed));
    assert_eq!(snap4.retry_action, UpdateRetryAction::Check);
    assert!(snap4.available_version.is_none());
    assert_eq!(state.snapshot(), snap4);

    // 5. Available -> Install error (e.g. download failure): candidate preserved, retry Install
    let _ = state.apply_transition(StateTransition {
        state: UpdateState::Available,
        channel: UpdateChannel::Stable,
        candidate: Some(candidate.clone()),
        ..Default::default()
    });
    let (rev6, snap6) = state.apply_transition(StateTransition {
        state: UpdateState::Error,
        channel: UpdateChannel::Stable,
        candidate: Some(candidate.clone()),
        error_code: Some(UpdateErrorCode::DownloadFailed),
        error_detail: Some("download failed".to_string()),
        retry_action: UpdateRetryAction::Install,
        ..Default::default()
    });
    assert_eq!(rev6, 6);
    assert_eq!(snap6.revision, 6);
    assert_eq!(snap6.state, UpdateState::Error);
    assert_eq!(snap6.error_code, Some(UpdateErrorCode::DownloadFailed));
    assert_eq!(snap6.retry_action, UpdateRetryAction::Install);
    assert_eq!(snap6.available_version.as_deref(), Some("4.1.0"));
    assert_eq!(state.snapshot(), snap6);

    // 6. Stale update error: candidate cleared, retry Check
    let (rev7, snap7) = state.apply_transition(StateTransition {
        state: UpdateState::Error,
        channel: UpdateChannel::Stable,
        error_code: Some(UpdateErrorCode::StaleUpdate),
        error_detail: Some("stale update".to_string()),
        retry_action: UpdateRetryAction::Check,
        ..Default::default()
    });
    assert_eq!(rev7, 7);
    assert_eq!(snap7.revision, 7);
    assert_eq!(snap7.state, UpdateState::Error);
    assert_eq!(snap7.retry_action, UpdateRetryAction::Check);
    assert!(snap7.available_version.is_none());
    assert_eq!(state.snapshot(), snap7);

    // 7. Reset to Idle: candidate, error, progress cleared
    let (rev8, snap8) = state.apply_transition(StateTransition {
        state: UpdateState::Idle,
        channel: UpdateChannel::Stable,
        ..Default::default()
    });
    assert_eq!(rev8, 8);
    assert_eq!(snap8.revision, 8);
    assert_eq!(snap8.state, UpdateState::Idle);
    assert!(snap8.available_version.is_none());
    assert!(snap8.error_code.is_none());
    assert!(snap8.error_detail.is_none());
    assert!(snap8.progress.is_none());
    assert_eq!(snap8.retry_action, UpdateRetryAction::None);
    assert_eq!(state.snapshot(), snap8);
}

#[cfg(feature = "tauri-test")]
#[test]
fn check_persistence_failure_produces_state_persistence_failed_and_performed_ack() {
    use super::UpdateService;
    use crate::app_state::ActivityCoordinator;
    use crate::ui_events::{
        UiEvent, UpdateChannel, UpdateCheckDisposition, UpdateErrorCode, UpdateRetryAction,
        UpdateState,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    let app = tauri::test::mock_builder()
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    let activity = ActivityCoordinator::default();
    let service = UpdateService::new(app.handle().clone(), activity);

    let event_count = AtomicUsize::new(0);
    let oversized_error = "disk write error (simulated): ".to_string() + &"x".repeat(8192);
    let ack = service
        .handle_check_persistence_failure(UpdateChannel::Stable, &oversized_error, &|event| {
            event_count.fetch_add(1, Ordering::SeqCst);
            if let UiEvent::UpdateChanged { v, payload } = event {
                assert_eq!(v, 1);
                assert_eq!(payload.state, UpdateState::Error);
                assert_eq!(
                    payload.error_code,
                    Some(UpdateErrorCode::StatePersistenceFailed)
                );
                assert_eq!(payload.retry_action, UpdateRetryAction::Check);
                let detail = payload.error_detail.unwrap();
                assert!(detail.contains("disk write error"));
                assert!(detail.len() <= 4096);
            } else {
                panic!("unexpected event: {event:?}");
            }
            Ok(())
        })
        .expect("handled persistence failure");

    assert_eq!(ack.disposition, UpdateCheckDisposition::Performed);
    assert_eq!(event_count.load(Ordering::SeqCst), 1);

    let snapshot = service.current_snapshot();
    assert_eq!(snapshot.state, UpdateState::Error);
    assert_eq!(
        snapshot.error_code,
        Some(UpdateErrorCode::StatePersistenceFailed)
    );
    assert_eq!(snapshot.retry_action, UpdateRetryAction::Check);
    assert!(snapshot.error_detail.as_ref().unwrap().len() <= 4096);
}

#[test]
fn native_update_state_bounds_oversized_error_detail_at_transition_boundary() {
    use super::{NativeUpdateState, StateTransition};
    use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

    let oversized = "x".repeat(8_192);
    let mut state = NativeUpdateState::default();

    let (_, snapshot) = state.apply_transition(StateTransition {
        state: UpdateState::Error,
        channel: UpdateChannel::Stable,
        error_code: Some(UpdateErrorCode::DownloadFailed),
        error_detail: Some(oversized),
        retry_action: UpdateRetryAction::Install,
        ..Default::default()
    });

    let detail = snapshot
        .error_detail
        .as_deref()
        .expect("error detail must be present");

    assert_eq!(detail.chars().count(), 4096);
    assert_eq!(detail, "x".repeat(4096));

    assert_eq!(snapshot.error_code, Some(UpdateErrorCode::DownloadFailed));
    assert_eq!(snapshot.retry_action, UpdateRetryAction::Install);

    let stored = state.snapshot();

    assert_eq!(stored.error_detail, snapshot.error_detail);
    assert_eq!(
        stored
            .error_detail
            .as_deref()
            .expect("stored detail")
            .chars()
            .count(),
        4096
    );
}

#[test]
fn native_update_state_preserves_short_error_detail_exactly() {
    use super::{NativeUpdateState, StateTransition};
    use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

    let detail = "update install failed: signature verification failed";
    let mut state = NativeUpdateState::default();

    let (_, snapshot) = state.apply_transition(StateTransition {
        state: UpdateState::Error,
        channel: UpdateChannel::Stable,
        error_code: Some(UpdateErrorCode::InstallFailed),
        error_detail: Some(detail.to_string()),
        retry_action: UpdateRetryAction::Install,
        ..Default::default()
    });

    assert_eq!(snapshot.error_detail.as_deref(), Some(detail));
    assert_eq!(state.snapshot().error_detail.as_deref(), Some(detail));
    assert_eq!(snapshot.error_code, Some(UpdateErrorCode::InstallFailed));
    assert_eq!(snapshot.retry_action, UpdateRetryAction::Install);
}

#[test]
fn native_update_state_bounds_error_detail_by_characters() {
    use super::{NativeUpdateState, StateTransition};
    use crate::ui_events::{UpdateChannel, UpdateErrorCode, UpdateRetryAction, UpdateState};

    let oversized = "é".repeat(5_000);
    let mut state = NativeUpdateState::default();

    let (_, snapshot) = state.apply_transition(StateTransition {
        state: UpdateState::Error,
        channel: UpdateChannel::Stable,
        error_code: Some(UpdateErrorCode::DownloadFailed),
        error_detail: Some(oversized),
        retry_action: UpdateRetryAction::Install,
        ..Default::default()
    });

    let detail = snapshot.error_detail.as_deref().unwrap();

    assert_eq!(detail.chars().count(), 4096);
    assert_eq!(detail, "é".repeat(4096));

    let stored = state.snapshot();
    assert_eq!(stored.error_detail, snapshot.error_detail);
    assert_eq!(
        stored
            .error_detail
            .as_deref()
            .expect("stored detail")
            .chars()
            .count(),
        4096
    );
}
