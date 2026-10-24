use super::{
    CALIBRATION_DEFAULT_TIMEOUT_SECONDS, CALIBRATION_MIN_FULL_TIMEOUT_SECONDS,
    CALIBRATION_MIN_NATIVE_TOTAL_SECONDS, CALIBRATION_MIN_SINGLE_TIMEOUT_SECONDS,
    CALIBRATION_TIMING_MARGIN_GUARD_US, CalibrationRunError, CatalogLoadState,
    DRY_RUN_TERMINAL_NONE, DiagnosticsBackendHealth, DiagnosticsPublicationGate,
    DryRunTerminalRequest, MAX_DECISION_COUNT, MAX_NATIVE_EVENTS, MAX_PREPARED_PLANS,
    MaterializedTimingPolicy, NativeActivePlayback, NativeCalibrationService, NativeDesktopRuntime,
    NativeDiagnosticsSample, NativeEventHub, NativeEventSnapshotMetrics,
    NativePlaybackPresentation, NativePlaybackService, NativePlaybackStartRequest,
    NativePreparedInput, PLAYBACK_PRESENTATION_CRITICAL_CAPACITY, PlaybackActivation,
    PlaybackActiveStatusDto, PlaybackHandoffDecision, PlaybackPendingControl,
    PlaybackPresentationMetrics, PlaybackTerminalPublication, PlaybackTerminalStatusDto,
    SENDER_TRACE_CAPTURE_CAPACITY, SenderTraceState, TerminalReleaseEvidence, TerminalRetirement,
    TestSeams, WorkerQuiescenceProof, calibration_budget, compute_catalog_source_fingerprint,
    compute_catalog_source_static_fingerprint, decide_playback_handoff, diagnostics_backend_status,
    opaque_native_id, percentile_ms, physical_startup_failure, plan_fingerprint,
    population_sigma_ms, publish_calibration_cache, publish_diagnostics_snapshot_for_active,
    publish_final_diagnostics_snapshot_for_active, publish_playback_state,
    publish_retirement_barrier, publish_terminal_poll_result,
    recommended_calibrated_timing_margin_us, release_terminal_ownership, remove_oldest_snapshot,
    resolve_install_root, retain_prepared_capacity, safe_calibration_evidence,
    sender_sample_summary, sender_trace_export_json, settings_fingerprint,
    target_integrity_startup_failure, terminal_publication_after_worker, terminal_release_evidence,
    terminal_release_evidence_after_startup, timing_margin_recommendation,
    validate_playback_start_request,
};
#[cfg(feature = "tauri-test")]
use super::{
    EnginePollStatus, NativeDispatchSession, PlaybackPresentationStage,
    PlaybackPresentationTestHook, native_event_delivery_failure, playback_event_state,
    playback_health_from_snapshot, publish_playback_snapshot,
};
use crate::app_state::ActivityCoordinator;
use crate::commands::{
    CalibrationStartRequest, PlaybackConfigDto, PlaybackSessionState, PlaybackStartRequest,
};

#[cfg(feature = "tauri-test")]
type PlaybackPresentationTestGate = Arc<(Mutex<bool>, Condvar)>;
use crate::power_lifecycle::PowerLifecycleDiagnostics;
use crate::ui_events::{
    CalibrationMode, DiagnosticsBackendStatus, PlaybackEventState, PlaybackFocusState,
    PlaybackHealthState, PlaybackSnapshotPayload, UiEvent,
};
use serde_json::Value;
use sky_app_core::catalog::song_id_for_canonical_path;
use sky_app_core::settings::ApplicationSettings;
use sky_app_core::song::{
    Song, analyze_schedule_with_context, build_schedule_with_policy, parse_song_json,
};
use sky_native_adapters::{
    AppPaths, AppResources, CATALOG_CACHE_SCHEMA_VERSION, CatalogCache, CatalogCacheEntry,
    CatalogCacheSource, CatalogCacheSourceKind, CatalogCacheStore, load_calibration_resolution,
};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Barrier, Condvar, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::{Channel, InvokeResponseBody};

#[cfg(feature = "tauri-test")]
fn terminal_projection_session(
    supervisor_lease_timeout_us: u64,
    up_scheduled_us: u64,
    startup_ordering_hook: Option<Arc<sky_player::engine::StartupOrderingHook>>,
) -> Arc<NativeDispatchSession> {
    projection_session_with_down(
        supervisor_lease_timeout_us,
        100_000,
        up_scheduled_us,
        startup_ordering_hook,
    )
}

#[cfg(feature = "tauri-test")]
fn presentation_projection_session(
    supervisor_lease_timeout_us: u64,
    down_scheduled_us: u64,
    up_scheduled_us: u64,
) -> Arc<NativeDispatchSession> {
    projection_session_with_down(
        supervisor_lease_timeout_us,
        down_scheduled_us,
        up_scheduled_us,
        None,
    )
}

#[cfg(feature = "tauri-test")]
fn projection_session_with_down(
    supervisor_lease_timeout_us: u64,
    down_scheduled_us: u64,
    up_scheduled_us: u64,
    startup_ordering_hook: Option<Arc<sky_player::engine::StartupOrderingHook>>,
) -> Arc<NativeDispatchSession> {
    use sky_player::adapter_support::{
        ActionKind as DispatchActionKind, KeyActionInput, PriorityMode, compile_runtime_intents,
    };
    use sky_player::engine::{
        BackendConfig, DispatchProfile, FaultInjectionScript, FocusOptions, NativeSessionOptions,
        PriorityOptions, TelemetryMode, TelemetryOptions, TestWaitPolicy, TimingOptions,
        WaitOptions,
    };

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: DispatchActionKind::Down,
                scheduled_us: down_scheduled_us,
                scan_codes: smallvec::smallvec![0x15],
                reason: Arc::<str>::from("terminal-projection-down"),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: DispatchActionKind::Up,
                scheduled_us: up_scheduled_us,
                scan_codes: smallvec::smallvec![0x15],
                reason: Arc::<str>::from("terminal-projection-up"),
            },
        ],
        &sky_app_core::song::SKY_SCAN_CODES,
    )
    .expect("terminal projection schedule");
    let player = Arc::new(
        NativeDispatchSession::new_with_power_endpoint(
            NativeSessionOptions {
                schedule,
                #[cfg(feature = "tauri-test")]
                startup_calibration_placement:
                    sky_player::engine::StartupCalibrationPlacement::BeforeSchedulingGuards,
                #[cfg(feature = "tauri-test")]
                timing_observer_for_qualification: false,
                #[cfg(feature = "tauri-test")]
                modifier_key_state_query_for_test: None,
                backend: BackendConfig::Mock {
                    latency_base_us: 0,
                    latency_per_key_us: 0,
                    fault_script: FaultInjectionScript::none(),
                },
                profile: DispatchProfile::MockTest,
                timing: TimingOptions {
                    game_fps: 60,
                    min_hold_us: 17_467,
                    min_release_gap_us: 17_467,
                    frame_us: 16_667,
                    frame_base_hold_us: 16_667,
                    timing_margin_us: 800,
                    strict_timing: false,
                    strict_down_completion_late_us: 2_000,
                    strict_up_completion_late_us: 2_000,
                    input_path_warn_us: 300,
                },
                focus: FocusOptions {
                    require_focus: false,
                    focus_restore_grace_us: 100_000,
                },
                wait: WaitOptions {
                    enable_waitable_timer: true,
                    enable_event_wait: true,
                    supervisor_lease_timeout_us,
                    test_spin_threshold_us: None,
                    test_wait_policy: TestWaitPolicy::LegacyTestWideSpin,
                },
                telemetry: TelemetryOptions {
                    mode: TelemetryMode::Off,
                    capacity: 64,
                },
                priority: PriorityOptions {
                    mode: PriorityMode::Off,
                },
                instrument_key_profile: None,
                startup_ordering_hook,
                restore_race_hook: None,
                focus_pause_hook: None,
                timer_lifecycle_context: None,
                prepared_packet_ambiguity_mask: None,
                preflight_user_held_mask: None,
            },
            sky_player::engine::SystemPowerEndpoint::new()
                .expect("terminal projection power endpoint"),
        )
        .expect("terminal projection session"),
    );
    player.set_target_hwnd(123);
    player
        .bind_window_identity_authority(
            sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(123, 77),
        )
        .expect("bind test owner identity");
    player
}

#[cfg(feature = "tauri-test")]
fn terminal_projection_active(player: Arc<NativeDispatchSession>) -> NativeActivePlayback {
    NativeActivePlayback {
        activity: ActivityCoordinator::default(),
        session_id: "a".repeat(32),
        prepared_id: "b".repeat(32),
        song_id: "c".repeat(32),
        title: "Terminal projection fixture".into(),
        total_us: 5_100_000,
        config: PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: false,
        },
        timing_policy: MaterializedTimingPolicy::from_user_margin(60, 1.0, 800)
            .expect("terminal projection timing policy"),
        timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
            recommended_timing_margin_us: Some(800),
            qualified: false,
            source: "default_fallback".into(),
        },
        plan_fingerprint: "d".repeat(64),
        physical: true,
        activity_lease: Mutex::new(None),
        target_hwnd: None,
        state: Mutex::new(PlaybackSessionState::Playing),
        pending: Mutex::new(None),
        player: Some(player),
        power_lifecycle: Arc::new(PowerLifecycleDiagnostics::default()),
        power_request: Mutex::new(None),
        suspend_resume_registration: Mutex::new(None),
        started_at: Instant::now(),
        paused_since: Mutex::new(None),
        paused_total: Mutex::new(Duration::ZERO),
        stop_requested: AtomicBool::new(false),
        dry_run_terminal_request: AtomicU8::new(DRY_RUN_TERMINAL_NONE),
        done: AtomicBool::new(false),
        worker_quiescent: AtomicBool::new(false),
        release_verified: AtomicBool::new(false),
        sequence: AtomicU64::new(0),
        event_delivery_failure: Arc::new(AtomicBool::new(false)),
        presentation: OnceLock::new(),
    }
}

#[cfg(feature = "tauri-test")]
fn tracked_test_active(
    player: Arc<NativeDispatchSession>,
    activity: ActivityCoordinator,
) -> NativeActivePlayback {
    let mut active = terminal_projection_active(player);
    active.activity = activity.clone();
    active.total_us = 30_100_000;
    active.state = Mutex::new(PlaybackSessionState::Starting);
    active.activity_lease = Mutex::new(Some(
        activity
            .reserve_playback(&active.session_id)
            .expect("mock physical activity lease"),
    ));
    active
}

#[cfg(test)]
fn attach_test_presentation(
    mut active: NativeActivePlayback,
    events: &Arc<Mutex<NativeEventHub>>,
    diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    #[cfg(feature = "tauri-test")] test_hook: Option<PlaybackPresentationTestHook>,
) -> Arc<NativeActivePlayback> {
    let failure_flag = events.lock().expect("test event hub").failure_flag();
    active.event_delivery_failure = failure_flag;
    let active = Arc::new(active);
    if let Some(player) = &active.player {
        events
            .lock()
            .expect("test event hub")
            .register_playback_player(Arc::downgrade(player))
            .expect("register test presentation player");
    }
    let presentation = NativePlaybackPresentation::start(
        &active,
        Arc::clone(events),
        diagnostics_gate,
        #[cfg(feature = "tauri-test")]
        test_hook,
    )
    .expect("test presentation consumer");
    assert!(active.presentation.set(presentation).is_ok());
    active
}

#[cfg(test)]
fn attach_shared_test_presentation(
    mut active: Arc<NativeActivePlayback>,
    events: &Arc<Mutex<NativeEventHub>>,
    diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    #[cfg(feature = "tauri-test")] test_hook: Option<PlaybackPresentationTestHook>,
) -> Arc<NativeActivePlayback> {
    let failure_flag = events.lock().expect("test event hub").failure_flag();
    Arc::get_mut(&mut active)
        .expect("shared test active playback must have one owner before presentation")
        .event_delivery_failure = failure_flag;
    if let Some(player) = &active.player {
        events
            .lock()
            .expect("test event hub")
            .register_playback_player(Arc::downgrade(player))
            .expect("register shared test presentation player");
    }
    let presentation = NativePlaybackPresentation::start(
        &active,
        Arc::clone(events),
        diagnostics_gate,
        #[cfg(feature = "tauri-test")]
        test_hook,
    )
    .expect("shared test presentation consumer");
    assert!(active.presentation.set(presentation).is_ok());
    active
}

#[cfg(feature = "tauri-test")]
fn blocked_presentation_hook(
    blocked_stage: PlaybackPresentationStage,
) -> (
    PlaybackPresentationTestHook,
    mpsc::Receiver<()>,
    PlaybackPresentationTestGate,
) {
    let (entered_sender, entered_receiver) = mpsc::channel();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let hook_gate = Arc::clone(&gate);
    let entered = Arc::new(AtomicBool::new(false));
    let hook_entered = Arc::clone(&entered);
    let hook: PlaybackPresentationTestHook = Arc::new(move |stage| {
        if stage == blocked_stage && !hook_entered.swap(true, Ordering::AcqRel) {
            entered_sender
                .send(())
                .expect("presentation hook entered receiver");
            let (released, changed) = &*hook_gate;
            let mut released = released.lock().expect("presentation hook gate");
            while !*released {
                released = changed.wait(released).expect("presentation hook wait");
            }
        }
    });
    (hook, entered_receiver, gate)
}

#[cfg(feature = "tauri-test")]
fn release_test_gate(gate: &PlaybackPresentationTestGate) {
    let (released, changed) = &**gate;
    *released.lock().expect("test gate") = true;
    changed.notify_all();
}

#[cfg(test)]
fn wait_for_playback_done(active: &NativeActivePlayback, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while !active.done.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    active.done.load(Ordering::Acquire)
}

fn catalog_generation(runtime: &NativeDesktopRuntime) -> u64 {
    runtime
        .dispatch(
            "catalog.search",
            serde_json::json!({"query":"","offset":0,"limit":1}),
        )
        .expect("catalog search")["generation"]
        .as_u64()
        .expect("catalog generation")
}

fn user_cache_entry(path: &Path, title: &str, file_size: u64) -> CatalogCacheEntry {
    CatalogCacheEntry {
        source_id: "user".into(),
        kind: CatalogCacheSourceKind::User,
        canonical_path: fs::canonicalize(path)
            .expect("canonical cache path")
            .to_string_lossy()
            .into_owned(),
        song_id: song_id_for_canonical_path(
            &fs::canonicalize(path)
                .expect("canonical cache ID path")
                .to_string_lossy(),
        ),
        title: title.into(),
        file_size,
        modified_unix_ms: 1,
    }
}

fn write_catalog_cache(root: &Path, entries: Vec<CatalogCacheEntry>) {
    let song_ids = entries.iter().map(|entry| entry.song_id.clone()).collect();
    let resources = AppResources::from_resource_dir(root);
    let static_fingerprint =
        compute_catalog_source_static_fingerprint(&resources, &root.join("songs"));
    CatalogCacheStore::new(root.join("cache/catalog-index.json"))
        .save(&CatalogCache {
            schema_version: CATALOG_CACHE_SCHEMA_VERSION,
            cache_generation: 1,
            source_fingerprint: compute_catalog_source_fingerprint(&static_fingerprint, &[]),
            sources: vec![CatalogCacheSource {
                source_id: "user".into(),
                kind: CatalogCacheSourceKind::User,
                import_kind: None,
                display_name: "User library".into(),
                available: true,
                song_ids,
            }],
            entries,
        })
        .expect("write catalog cache");
}

fn active_for_control(
    state: PlaybackSessionState,
    pending: Option<PlaybackPendingControl>,
) -> Arc<NativeActivePlayback> {
    active_for_control_with_physical(state, pending, false)
}

fn active_for_control_with_physical(
    state: PlaybackSessionState,
    pending: Option<PlaybackPendingControl>,
    physical: bool,
) -> Arc<NativeActivePlayback> {
    active_for_control_with_player(
        state,
        pending,
        physical,
        ActivityCoordinator::default(),
        None,
    )
}

fn active_for_control_with_player(
    state: PlaybackSessionState,
    pending: Option<PlaybackPendingControl>,
    physical: bool,
    activity: ActivityCoordinator,
    player: Option<Arc<NativeDispatchSession>>,
) -> Arc<NativeActivePlayback> {
    Arc::new(NativeActivePlayback {
        activity,
        session_id: "a".repeat(32),
        prepared_id: "b".repeat(32),
        song_id: "c".repeat(32),
        title: "Fixture".into(),
        total_us: 1_000_000,
        config: PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: true,
        },
        timing_policy: MaterializedTimingPolicy::from_user_margin(60, 1.0, 800)
            .expect("fixture timing policy"),
        timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
            recommended_timing_margin_us: Some(800),
            qualified: false,
            source: "default_fallback".into(),
        },
        plan_fingerprint: "d".repeat(64),
        physical,
        activity_lease: Mutex::new(None),
        target_hwnd: None,
        state: Mutex::new(state),
        pending: Mutex::new(pending),
        player,
        power_lifecycle: Arc::new(PowerLifecycleDiagnostics::default()),
        power_request: Mutex::new(None),
        suspend_resume_registration: Mutex::new(None),
        started_at: Instant::now(),
        paused_since: Mutex::new(None),
        paused_total: Mutex::new(Duration::ZERO),
        stop_requested: std::sync::atomic::AtomicBool::new(false),
        dry_run_terminal_request: AtomicU8::new(DRY_RUN_TERMINAL_NONE),
        done: std::sync::atomic::AtomicBool::new(false),
        worker_quiescent: std::sync::atomic::AtomicBool::new(false),
        release_verified: std::sync::atomic::AtomicBool::new(false),
        sequence: std::sync::atomic::AtomicU64::new(0),
        event_delivery_failure: Arc::new(AtomicBool::new(false)),
        presentation: OnceLock::new(),
    })
}

fn prepare_physical_failure_plan(
    service: &NativePlaybackService,
    settings: &ApplicationSettings,
    song_id: &str,
) -> String {
    let song = parse_song_json(
        br#"{"name":"Target failure fixture","songNotes":[{"time":0,"key":"Key0"}]}"#,
        song_id,
    )
    .expect("fixture song");
    let config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 1.0,
        fps: 60,
        dry_run: false,
    };
    let timing_policy = MaterializedTimingPolicy::from_user_margin(
        config.fps,
        config.hold_frames,
        config.timing_margin_us,
    )
    .expect("fixture timing policy");
    let schedule = build_schedule_with_policy(&song, config.tempo_scale, &timing_policy)
        .expect("fixture schedule");
    let risk = super::analyze_schedule_with_context(
        &schedule,
        Some(&song.notes),
        config.hold_frames,
        config.tempo_scale,
    );
    let prepared = service
        .prepare(super::NativePreparedInput {
            song_id: song_id.into(),
            generation: 1,
            config,
            song,
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            settings_fingerprint: settings_fingerprint(settings).expect("settings fingerprint"),
            source_epoch: 0,
        })
        .expect("physical fixture plan");
    assert_eq!(
        prepared.admission,
        crate::commands::PlaybackAdmission::Ready
    );
    prepared.prepared_id.expect("prepared ID")
}

fn prepare_dry_run_plan(
    service: &NativePlaybackService,
    settings: &ApplicationSettings,
    song_id: &str,
) -> String {
    let song = parse_song_json(
        br#"{"name":"Dry-run lifecycle fixture","songNotes":[{"time":0,"key":"Key0"}]}"#,
        song_id,
    )
    .expect("fixture song");
    let config = PlaybackConfigDto {
        hold_frames: 1.0,
        timing_margin_us: 800,
        tempo_scale: 1.0,
        fps: 60,
        dry_run: true,
    };
    let timing_policy = MaterializedTimingPolicy::from_user_margin(
        config.fps,
        config.hold_frames,
        config.timing_margin_us,
    )
    .expect("fixture timing policy");
    let schedule = build_schedule_with_policy(&song, config.tempo_scale, &timing_policy)
        .expect("fixture schedule");
    let risk = super::analyze_schedule_with_context(
        &schedule,
        Some(&song.notes),
        config.hold_frames,
        config.tempo_scale,
    );
    let prepared = service
        .prepare(super::NativePreparedInput {
            song_id: song_id.into(),
            generation: 1,
            config,
            song,
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation: crate::commands::TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            settings_fingerprint: settings_fingerprint(settings).expect("settings fingerprint"),
            source_epoch: 0,
        })
        .expect("dry-run fixture plan");
    prepared.prepared_id.expect("prepared ID")
}

#[cfg(feature = "tauri-test")]
fn native_handoff_test_player(
    hook: Arc<sky_player::engine::StartupOrderingHook>,
    capture: Arc<Mutex<Vec<sky_player::engine::MockSendInputCapture>>>,
    mut fault_script: sky_player::engine::FaultInjectionScript,
) -> Arc<NativeDispatchSession> {
    use sky_player::adapter_support::{
        ActionKind as DispatchActionKind, KeyActionInput, compile_runtime_intents,
    };
    use sky_player::engine::{
        BackendConfig, DispatchProfile, FocusOptions, NativeSessionOptions, PriorityOptions,
        SystemPowerEndpoint, TelemetryMode, TelemetryOptions, TestWaitPolicy, TimingOptions,
        WaitOptions,
    };

    let schedule = compile_runtime_intents(
        &[
            KeyActionInput {
                source_action_index: 0,
                kind: DispatchActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: Arc::<str>::from("terminal-handoff-test-down"),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: DispatchActionKind::Up,
                scheduled_us: 100_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: Arc::<str>::from("terminal-handoff-test-up"),
            },
        ],
        &sky_app_core::song::SKY_SCAN_CODES,
    )
    .expect("handoff test schedule");
    fault_script.send_input_capture = Some(capture);
    Arc::new(
        NativeDispatchSession::new_with_power_endpoint(
            NativeSessionOptions {
                schedule,
                #[cfg(feature = "tauri-test")]
                startup_calibration_placement:
                    sky_player::engine::StartupCalibrationPlacement::BeforeSchedulingGuards,
                #[cfg(feature = "tauri-test")]
                timing_observer_for_qualification: false,
                modifier_key_state_query_for_test: None,
                backend: BackendConfig::Mock {
                    latency_base_us: 0,
                    latency_per_key_us: 0,
                    fault_script,
                },
                profile: DispatchProfile::MockTest,
                timing: TimingOptions {
                    game_fps: 60,
                    min_hold_us: 17_467,
                    min_release_gap_us: 17_467,
                    frame_us: 16_667,
                    frame_base_hold_us: 16_667,
                    timing_margin_us: 800,
                    strict_timing: false,
                    strict_down_completion_late_us: 2_000,
                    strict_up_completion_late_us: 2_000,
                    input_path_warn_us: 300,
                },
                focus: FocusOptions {
                    require_focus: false,
                    focus_restore_grace_us: 100_000,
                },
                wait: WaitOptions {
                    enable_waitable_timer: true,
                    enable_event_wait: true,
                    supervisor_lease_timeout_us: 0,
                    test_spin_threshold_us: None,
                    test_wait_policy: TestWaitPolicy::LegacyTestWideSpin,
                },
                telemetry: TelemetryOptions {
                    mode: TelemetryMode::Ring,
                    capacity: 64,
                },
                priority: PriorityOptions {
                    mode: sky_player::adapter_support::PriorityMode::Off,
                },
                instrument_key_profile: None,
                startup_ordering_hook: Some(Arc::clone(&hook)),
                restore_race_hook: None,
                focus_pause_hook: None,
                timer_lifecycle_context: None,
                prepared_packet_ambiguity_mask: None,
                preflight_user_held_mask: None,
            },
            SystemPowerEndpoint::new().expect("isolated test power endpoint"),
        )
        .expect("mock native handoff session"),
    )
}

fn playback_trace(events: &Arc<Mutex<NativeEventHub>>) -> Vec<String> {
    events
        .lock()
        .expect("event hub")
        .buffered_for_test()
        .iter()
        .filter_map(|event| match event {
            UiEvent::PlaybackStateChanged { payload, .. } => {
                Some(format!("state:{}", payload.state.as_str()))
            }
            UiEvent::PlaybackFinished { payload, .. } => {
                Some(format!("finished:{}", payload.outcome))
            }
            UiEvent::PlaybackFailed { payload, .. } => Some(format!("failed:{}", payload.code)),
            _ => None,
        })
        .collect()
}

fn quick_calibration_request() -> CalibrationStartRequest {
    CalibrationStartRequest {
        mode: CalibrationMode::Quick,
        class_name: None,
        polyphony: None,
        samples: None,
        timeout_seconds: None,
    }
}

fn calibration_finished_count(events: &Arc<Mutex<NativeEventHub>>) -> usize {
    events
        .lock()
        .expect("event hub")
        .buffered_for_test()
        .iter()
        .filter(|event| matches!(event, UiEvent::CalibrationFinished { .. }))
        .count()
}

fn snapshot(session_id: &str, seq: u64) -> UiEvent {
    UiEvent::PlaybackSnapshot {
        v: 1,
        payload: PlaybackSnapshotPayload {
            session_id: session_id.into(),
            seq,
            state: PlaybackEventState::Playing,
            song_id: "0123456789abcdef0123456789abcdef".into(),
            title: "demo".into(),
            current_us: seq,
            total_us: 100,
            pre_roll_remaining_us: 0,
            focus_state: PlaybackFocusState::Focused,
            health: PlaybackHealthState::Healthy,
            input_path_degraded: false,
            message: None,
        },
    }
}

#[path = "tests/calibration.rs"]
mod calibration;
#[path = "tests/catalog.rs"]
mod catalog;
#[path = "tests/event_delivery.rs"]
mod event_delivery;
#[path = "tests/playback_integration.rs"]
mod playback_integration;
