use super::super::{
    NativePreparedInput, analyze_schedule_with_context, build_schedule_with_policy,
};
use super::NativePlaybackService;
#[cfg(feature = "tauri-test")]
use super::{compile_dispatch_schedule_with_checkpoint, schedule_metadata_heap_capacity_bytes};
use crate::app_state::ActivityCoordinator;
use crate::commands::PlaybackConfigDto;
use crate::native_runtime::NativeEventHub;
use sky_app_core::song::parse_song_json;
use sky_app_core::timing::MaterializedTimingPolicy;
use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::Duration;

#[cfg(feature = "tauri-test")]
#[test]
fn valid_one_million_single_key_actions_fit_prepared_runtime_budget() {
    const ACTION_COUNT: usize = 1_000_000;
    let mut chords = Vec::with_capacity(ACTION_COUNT / 2);
    for index in 0..(ACTION_COUNT / 2) {
        let start = (index as u64) * 2;
        chords.push(sky_app_core::song::OnsetChord {
            at_us: start,
            scan_codes: vec![0x15],
        });
    }
    let schedule = sky_app_core::song::ScheduleMetadata {
        chords,
        hold_us: 1,
        source_duration_us: ACTION_COUNT as u64,
        playback_duration_us: ACTION_COUNT as u64,
        duration_us: ACTION_COUNT as u64,
        note_count: ACTION_COUNT / 2,
        deduplicated_note_count: ACTION_COUNT / 2,
        duplicate_note_count: 0,
        impossible_same_key_repeats: 0,
        max_polyphony: 1,
        shortest_same_key_interval_us: None,
        min_same_key_slack_us: None,
        same_key_repeat_count: 0,
        zero_slack_repeat_count: 0,
        recommended_hold_frames: None,
        recommended_tempo_scale: None,
        warnings: Vec::new(),
    };
    let source_capacity_bytes =
        schedule_metadata_heap_capacity_bytes(&schedule).expect("source schedule capacity");
    let qpc_frequency = std::num::NonZeroU64::new(1_000_000).expect("QPC frequency");

    let candidate_prepare_started = std::time::Instant::now();
    let compiled = compile_dispatch_schedule_with_checkpoint(
        &schedule,
        0,
        &std::sync::atomic::AtomicBool::new(false),
        |_| {},
    )
    .expect("candidate one-million-action Prepare compile");
    assert_eq!(compiled.generation_count(), (ACTION_COUNT / 2) as u64);
    let compile_memory = compiled.compile_memory();
    let schedule_memory = compiled
        .memory_metrics()
        .expect("compiled schedule capacity");
    let schedule_heap_bytes = schedule_memory.heap_payload_bytes;
    let schedule_capacity_bytes = schedule_heap_bytes
        .checked_add(schedule_memory.struct_bytes)
        .expect("compiled schedule total capacity");
    let prepared = sky_player::adapter_support::PreparedDispatchSchedule::prepare(
        &compiled,
        qpc_frequency,
        compile_memory.caller_retained_capacity_bytes,
    )
    .expect("one-million-action Prepare materialization");
    let prepared_memory = prepared.memory_metrics();
    let candidate_prepare_compile_and_materialize_us =
        candidate_prepare_started.elapsed().as_micros();
    drop(compiled);
    let candidate_start_started = std::time::Instant::now();
    let executor = sky_player::adapter_support::DispatchExecutor::from_prepared_schedule(prepared);
    let candidate_start_plan_move_ns = candidate_start_started.elapsed().as_nanos();
    let executor_memory = executor
        .try_memory_metrics()
        .expect("production executor capacity metrics");
    let retained_capacity_bytes = prepared_memory.overlapping_peak_capacity_bytes;
    let startup_peak_capacity_bytes = compile_memory
        .phase_peak_capacity_bytes
        .max(prepared_memory.overlapping_peak_capacity_bytes);
    let budget_bytes = sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES;
    assert!(executor_memory.chord_heap_bytes > 0);
    assert!(retained_capacity_bytes <= budget_bytes);
    assert!(startup_peak_capacity_bytes <= budget_bytes);
    eprintln!(
        "prepared_playback_evidence workload=valid_one_key chords={} source_schedule_capacity_bytes={source_capacity_bytes} prepare_compile_validate_plus_plan_materialization_us={candidate_prepare_compile_and_materialize_us} start_plan_move_and_session_state_ns={candidate_start_plan_move_ns} caller_retained_capacity_bytes={} compiler_input_capacity_bytes={} compiler_temporary_capacity_bytes={} compiled_schedule_capacity_bytes={schedule_capacity_bytes} prepared_plan_capacity_bytes={} runtime_executor_heap_bytes={} runtime_executor_struct_bytes={} runtime_executor_total_bytes={} prepare_overlapping_peak_capacity_bytes={retained_capacity_bytes} compiler_phase_peak_capacity_bytes={} startup_peak_capacity_bytes={startup_peak_capacity_bytes} policy_bytes={budget_bytes} timing_basis=wall_clock_single_offline_run_not_a_gate basis=compile_validate_and_prepare_plan_then_move_plan start_arm_timing=executor-construction-only native_process_setup_and_real_worker_arm=not_measured memory_basis=calculated_size_and_capacity complete_application_process_peak=not_measured windows_process_memory=not_measured",
        ACTION_COUNT / 2,
        compile_memory.caller_retained_capacity_bytes,
        compile_memory.input_capacity_bytes,
        compile_memory.temporary_capacity_bytes,
        prepared_memory.plan_capacity_bytes,
        executor_memory.chord_heap_bytes,
        executor_memory.struct_bytes,
        executor_memory.total_bytes,
        compile_memory.phase_peak_capacity_bytes,
    );
}

#[test]
fn spilled_chord_memory_accounting_counts_once_and_drops_temporary_inputs() {
    let wide_codes = vec![0x15, 0x16, 0x17, 0x18, 0x19, 0x23];
    assert!(wide_codes.len() > 4, "must spill SmallVec<[u16; 4]>");
    let schedule = sky_app_core::song::ScheduleMetadata {
        chords: vec![
            sky_app_core::song::OnsetChord {
                at_us: 0,
                scan_codes: wide_codes.clone(),
            },
            sky_app_core::song::OnsetChord {
                at_us: 50_000,
                scan_codes: wide_codes,
            },
        ],
        hold_us: 16_667,
        source_duration_us: 66_667,
        playback_duration_us: 66_667,
        duration_us: 66_667,
        note_count: 12,
        deduplicated_note_count: 12,
        duplicate_note_count: 0,
        impossible_same_key_repeats: 0,
        max_polyphony: 6,
        shortest_same_key_interval_us: Some(50_000),
        min_same_key_slack_us: Some(33_333),
        same_key_repeat_count: 6,
        zero_slack_repeat_count: 0,
        recommended_hold_frames: None,
        recommended_tempo_scale: None,
        warnings: Vec::new(),
    };
    let source_capacity_bytes =
        super::schedule_metadata_heap_capacity_bytes(&schedule).expect("source capacity");
    let external_retained = 4096usize;
    let cancelled = std::sync::atomic::AtomicBool::new(false);

    let compiled = super::compile_dispatch_schedule_with_checkpoint(
        &schedule,
        external_retained,
        &cancelled,
        |_| {},
    )
    .expect("spilled chord compile");
    let cm = compiled.compile_memory();

    // Caller retained excludes temporary OnsetChordInput headers and spilled SmallVec payloads.
    assert_eq!(
        cm.caller_retained_capacity_bytes,
        external_retained + source_capacity_bytes
    );
    // Core counts input Vec headers + spilled scan-code capacity once.
    let min_input_bytes = schedule.chords.len()
        * std::mem::size_of::<sky_player::adapter_support::OnsetChordInput>()
        + schedule.chords.len() * 6 * std::mem::size_of::<u16>();
    assert!(
        cm.input_capacity_bytes >= min_input_bytes,
        "input_capacity_bytes ({}) must include headers and spilled scan codes (>= {min_input_bytes})",
        cm.input_capacity_bytes
    );
    assert_eq!(
        cm.phase_peak_capacity_bytes,
        cm.caller_retained_capacity_bytes + cm.input_capacity_bytes + cm.schedule_capacity_bytes
    );

    // Near-budget boundary: exact remaining budget succeeds (would fail if spilled bytes were double-counted).
    let budget = sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES;
    let exact_external =
        budget - source_capacity_bytes - cm.input_capacity_bytes - cm.schedule_capacity_bytes;
    let at_limit = super::compile_dispatch_schedule_with_checkpoint(
        &schedule,
        exact_external,
        &cancelled,
        |_| {},
    )
    .expect("exact budget boundary must succeed without double-counting spilled scan codes");
    assert_eq!(at_limit.compile_memory().phase_peak_capacity_bytes, budget);
    assert_eq!(
        at_limit.compile_memory().caller_retained_capacity_bytes,
        exact_external + source_capacity_bytes
    );

    // Materialization after temporary inputs are dropped must not carry temporary input bytes
    // as retained caller storage; prepared chord storage fits in the remaining budget when
    // prepared_plan_bytes <= input_capacity_bytes.
    let qpc_hz = std::num::NonZeroU64::new(1_000_000).expect("qpc");
    let prepared = sky_player::adapter_support::PreparedDispatchSchedule::prepare(
        &compiled,
        qpc_hz,
        cm.caller_retained_capacity_bytes,
    )
    .expect("prepared materialization");
    let plan_memory = compiled.memory_metrics().expect("plan memory");
    let source_plan_bytes = plan_memory.heap_payload_bytes + plan_memory.struct_bytes;
    let materialize_overlap = cm.caller_retained_capacity_bytes
        + source_plan_bytes
        + prepared.memory_metrics().plan_capacity_bytes;
    assert_eq!(
        prepared.memory_metrics().overlapping_peak_capacity_bytes,
        materialize_overlap.max(cm.phase_peak_capacity_bytes)
    );

    // 1 byte over budget must fail cleanly.
    let over_limit = super::compile_dispatch_schedule_with_checkpoint(
        &schedule,
        exact_external + 1,
        &cancelled,
        |_| {},
    );
    assert!(over_limit.is_err(), "1 byte over budget must be rejected");

    // Overflow must fail cleanly via checked arithmetic without huge allocation.
    let overflow =
        super::compile_dispatch_schedule_with_checkpoint(&schedule, usize::MAX, &cancelled, |_| {});
    assert!(
        overflow
            .expect_err("usize::MAX must overflow")
            .contains("overflow")
    );
}

fn input(generation: u64, settings_fingerprint: &str) -> NativePreparedInput {
    let song = parse_song_json(
        br#"{"name":"Prepare publication race","songNotes":[{"time":0,"key":"Key0"}]}"#,
        "prepare-publication-race",
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
    let risk = analyze_schedule_with_context(
        &schedule,
        Some(&song.notes),
        config.hold_frames,
        config.tempo_scale,
    );
    NativePreparedInput {
        song_id: "prepare-publication-race".into(),
        generation,
        config,
        song,
        sheet_analysis: None,
        schedule,
        risk,
        timing_policy,
        settings_fingerprint: settings_fingerprint.into(),
        publication_fingerprint: settings_fingerprint.into(),
        source_path: PathBuf::new(),
        source_hash: String::new(),
        plan_identity: "test-plan".into(),
        plan_source: None,
        source_epoch: 0,
    }
}

fn recommendation_input(generation: u64, settings_fingerprint: &str) -> NativePreparedInput {
    let mut prepared = input(generation, settings_fingerprint);
    prepared.song_id = "a".repeat(32);
    prepared.risk.severity = "medium".into();
    prepared.risk.suggested_hold_frames = Some(1.25);
    prepared.risk.suggested_tempo_scale = Some(0.9);
    prepared
}

fn large_recommendation_input(generation: u64, settings_fingerprint: &str) -> NativePreparedInput {
    use sky_app_core::song::OnsetChord;

    const PAIR_COUNT: u64 = 25_001;
    let mut prepared = recommendation_input(generation, settings_fingerprint);
    let chords = (0..PAIR_COUNT)
        .map(|index| OnsetChord {
            at_us: index * 40_000,
            scan_codes: vec![0x15],
        })
        .collect::<Vec<_>>();
    let duration_us = chords.last().expect("last onset chord").at_us + prepared.schedule.hold_us;
    prepared.schedule.duration_us = duration_us;
    prepared.schedule.playback_duration_us = duration_us;
    prepared.schedule.source_duration_us = duration_us;
    prepared.schedule.chords = chords;
    prepared
}

#[cfg(feature = "tauri-test")]
fn prepared_mock_player_factory(
    hook: Arc<sky_player::engine::StartupOrderingHook>,
    send_calls: Arc<std::sync::atomic::AtomicU64>,
) -> impl Fn(
    sky_player::adapter_support::PreparedDispatchSchedule,
    &PlaybackConfigDto,
    &MaterializedTimingPolicy,
) -> Result<Arc<sky_player::engine::NativeDispatchSession>, String>
+ Send
+ Sync
+ 'static {
    move |prepared_schedule, config, policy| {
        use sky_player::engine::{
            BackendConfig, DispatchProfile, FaultInjectionScript, FocusOptions,
            NativeSessionOptions, PriorityOptions, TelemetryMode, TelemetryOptions, TestWaitPolicy,
            TimingOptions, WaitOptions,
        };

        let mut fault_script = FaultInjectionScript::none();
        fault_script.send_call_count = Some(Arc::clone(&send_calls));
        let options = NativeSessionOptions {
            schedule: prepared_schedule,
            #[cfg(feature = "tauri-test")]
            timing_observer_for_qualification: false,
            #[cfg(feature = "tauri-test")]
            modifier_key_state_query_for_test: None,
            backend: BackendConfig::Mock {
                latency_base_us: 0,
                latency_per_key_us: 0,
                fault_script,
            },
            profile: DispatchProfile::MockTest,
            timing: TimingOptions {
                game_fps: config.fps,
                min_hold_us: policy.min_hold_us,
                frame_us: policy.frame_us,
                frame_base_hold_us: policy.frame_base_hold_us,
                timing_margin_us: policy.timing_margin_us,
                strict_timing: false,
                strict_down_completion_late_us: 2_000,
                strict_up_completion_late_us: 2_000,
                input_path_warn_us: 300,
            },
            focus: FocusOptions {
                require_focus: false,
                focus_restore_grace_us: policy.focus_restore_grace_us,
            },
            wait: WaitOptions {
                enable_waitable_timer: true,
                enable_event_wait: true,
                supervisor_lease_timeout_us: 0,
                modifier_hold_timeout_us: sky_player::engine::DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
                test_spin_threshold_us: Some(20_000),
                test_wait_policy: TestWaitPolicy::LegacyTestWideSpin,
            },
            telemetry: TelemetryOptions {
                mode: TelemetryMode::Off,
                capacity: 64,
            },
            priority: PriorityOptions {
                mode: sky_dispatch_win32::mmcss::PriorityMode::Off,
            },
            instrument_key_profile: None,
            startup_ordering_hook: Some(Arc::clone(&hook)),
            timer_lifecycle_context: None,
            prepared_packet_ambiguity_mask: None,
            preflight_user_held_mask: None,
        };
        sky_player::engine::NativeDispatchSession::new(options).map(Arc::new)
    }
}

#[cfg(all(feature = "tauri-test", windows))]
#[test]
fn recommended_variant_starts_and_arms_its_prepared_plan() {
    use crate::commands::{PlaybackDecision, PlaybackDecisionAcceptanceDto, PlaybackStartRequest};
    use sky_player::engine::{StartupCheckpoint, StartupOrderingHook};

    let settings = sky_app_core::settings::ApplicationSettings::default();
    let fingerprint = super::settings_fingerprint(&settings).expect("settings fingerprint");
    let mut service = NativePlaybackService::new(ActivityCoordinator::default());
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::ArmedPublished, 1);
    let send_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    service.prepared_player_factory_for_test = Some(Arc::new(prepared_mock_player_factory(
        Arc::clone(&hook),
        Arc::clone(&send_calls),
    )));
    let compile_checkpoints = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let compile_checkpoints_hook = Arc::clone(&compile_checkpoints);
    *service
        .prepare_compile_test_hook
        .lock()
        .expect("Prepare compiler hook") = Some(Arc::new(move |action_count| {
        if action_count == 0 {
            compile_checkpoints_hook.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    }));

    let prepared = service
        .prepare(recommendation_input(1, &fingerprint))
        .expect("recommended physical Prepare");
    let recommendation = prepared
        .variants
        .iter()
        .find(|variant| variant.decision == PlaybackDecision::UseRecommended)
        .expect("recommended DTO variant");
    assert_eq!(recommendation.config.hold_frames, 1.25);
    assert_eq!(recommendation.config.tempo_scale, 0.9);
    let prepare_compile_count = compile_checkpoints.load(std::sync::atomic::Ordering::Acquire);
    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let started = service
        .start(
            PlaybackStartRequest {
                prepared_id: prepared.prepared_id.clone().expect("prepared identity"),
                decisions: vec![PlaybackDecisionAcceptanceDto {
                    decision: PlaybackDecision::UseRecommended,
                    accepted: true,
                }],
            },
            &settings,
            Arc::clone(&events),
        )
        .expect("selected recommended Start");
    assert_eq!(
        started.config.hold_frames,
        recommendation.config.hold_frames
    );
    assert_eq!(
        started.config.tempo_scale,
        recommendation.config.tempo_scale
    );
    assert_eq!(started.plan_fingerprint, recommendation.plan_fingerprint);
    assert_eq!(
        compile_checkpoints.load(std::sync::atomic::Ordering::Acquire),
        prepare_compile_count,
        "Start must not compile the selected variant again"
    );
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    assert_eq!(hook.prepared_schedule_executor_consumption_count(), 1);
    assert_eq!(send_calls.load(std::sync::atomic::Ordering::Acquire), 0);

    let player = service
        .active
        .lock()
        .expect("active playback")
        .as_ref()
        .and_then(|active| active.player.clone())
        .expect("prepared player attached to successful Start");
    player.quit().expect("quit at Armed checkpoint");
    hook.release_startup_checkpoint();
    assert!(player.join(Duration::from_secs(5)).expect("worker join"));
    assert!(service.shutdown(events));
    assert_eq!(send_calls.load(std::sync::atomic::Ordering::Acquire), 0);

    let mut dry_run_service = NativePlaybackService::new(ActivityCoordinator::default());
    dry_run_service.recommended_plan_caller_capacity_override_for_test =
        Some(sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES);
    let dry_run_prepared = dry_run_service
        .prepare(recommendation_input(1, &fingerprint))
        .expect("DryRun remains in the capacity-limited Prepare");
    let dry_run_variant = dry_run_prepared
        .variants
        .iter()
        .find(|variant| variant.decision == PlaybackDecision::DryRun)
        .expect("DryRun DTO variant");
    let dry_run_events = Arc::new(Mutex::new(NativeEventHub::default()));
    let dry_run_started = dry_run_service
        .start(
            PlaybackStartRequest {
                prepared_id: dry_run_prepared
                    .prepared_id
                    .clone()
                    .expect("DryRun prepared identity"),
                decisions: vec![PlaybackDecisionAcceptanceDto {
                    decision: PlaybackDecision::DryRun,
                    accepted: true,
                }],
            },
            &settings,
            Arc::clone(&dry_run_events),
        )
        .expect("DryRun selection remains startable");
    assert!(dry_run_started.config.dry_run);
    assert_eq!(
        dry_run_started.plan_fingerprint,
        dry_run_variant.plan_fingerprint
    );
    assert!(dry_run_service.shutdown(dry_run_events));
}

#[cfg(all(feature = "tauri-test", windows))]
#[test]
fn recommendation_capacity_failure_keeps_only_runnable_proceed_and_dry_run() {
    use crate::commands::{PlaybackDecision, PlaybackDecisionAcceptanceDto, PlaybackStartRequest};
    use sky_player::engine::{StartupCheckpoint, StartupOrderingHook};

    let settings = sky_app_core::settings::ApplicationSettings::default();
    let fingerprint = super::settings_fingerprint(&settings).expect("settings fingerprint");
    let mut service = NativePlaybackService::new(ActivityCoordinator::default());
    service.recommended_plan_caller_capacity_override_for_test =
        Some(sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES);
    let recommendation_materialization_attempted =
        Arc::new(std::sync::atomic::AtomicBool::new(false));
    let recommendation_materialization_attempted_hook =
        Arc::clone(&recommendation_materialization_attempted);
    *service
        .recommendation_materialization_test_hook
        .lock()
        .expect("recommendation materialization hook") = Some(Arc::new(move || {
        recommendation_materialization_attempted_hook
            .store(true, std::sync::atomic::Ordering::Release);
    }));
    let hook = Arc::new(StartupOrderingHook::default());
    hook.arm_startup_checkpoint(StartupCheckpoint::ArmedPublished, 1);
    let send_calls = Arc::new(std::sync::atomic::AtomicU64::new(0));
    service.prepared_player_factory_for_test = Some(Arc::new(prepared_mock_player_factory(
        Arc::clone(&hook),
        Arc::clone(&send_calls),
    )));

    let prepared = service
        .prepare(recommendation_input(1, &fingerprint))
        .expect("capacity-limited optional recommendation leaves Prepare valid");
    assert!(recommendation_materialization_attempted.load(std::sync::atomic::Ordering::Acquire));
    let prepared_id = prepared.prepared_id.clone().expect("prepared identity");
    assert_eq!(
        prepared
            .variants
            .iter()
            .map(|variant| variant.decision)
            .collect::<Vec<_>>(),
        vec![PlaybackDecision::Proceed, PlaybackDecision::DryRun]
    );
    assert_eq!(
        prepared
            .decisions
            .iter()
            .map(|decision| decision.decision)
            .collect::<Vec<_>>(),
        vec![PlaybackDecision::Proceed, PlaybackDecision::DryRun]
    );
    assert!(
        prepared
            .variants
            .iter()
            .all(|variant| variant.decision != PlaybackDecision::UseRecommended)
    );
    assert!(
        prepared.variants.iter().any(|variant| {
            variant.decision == PlaybackDecision::DryRun && variant.config.dry_run
        })
    );

    let cache = service.prepared.lock().expect("prepared cache");
    let record = cache
        .iter()
        .find(|(id, _)| id == &prepared_id)
        .map(|(_, record)| record)
        .expect("published Proceed record");
    assert_eq!(
        record
            .variants
            .keys()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        std::collections::HashSet::from([PlaybackDecision::Proceed, PlaybackDecision::DryRun])
    );
    assert!(
        record
            .variants
            .get(&PlaybackDecision::Proceed)
            .and_then(|variant| variant.prepared_schedule.as_ref())
            .is_some()
    );
    assert!(
        record
            .variants
            .get(&PlaybackDecision::DryRun)
            .is_some_and(|variant| variant.config.dry_run && variant.prepared_schedule.is_none())
    );
    assert!(
        record.retained_capacity_bytes <= sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES
    );
    assert!(
        super::prepared_cache_retained_capacity_bytes(&cache, None).unwrap()
            <= sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES
    );
    drop(cache);

    let events = Arc::new(Mutex::new(NativeEventHub::default()));
    let started = service
        .start(
            PlaybackStartRequest {
                prepared_id,
                decisions: vec![PlaybackDecisionAcceptanceDto {
                    decision: PlaybackDecision::Proceed,
                    accepted: true,
                }],
            },
            &settings,
            Arc::clone(&events),
        )
        .expect("Proceed remains runnable after optional recommendation omission");
    assert_eq!(started.config.hold_frames, 1.0);
    assert_eq!(started.config.tempo_scale, 1.0);
    assert!(hook.wait_for_startup_checkpoint(Duration::from_secs(5)));
    assert_eq!(hook.prepared_schedule_executor_consumption_count(), 1);
    let player = service
        .active
        .lock()
        .expect("active playback")
        .as_ref()
        .and_then(|active| active.player.clone())
        .expect("Proceed session attached");
    player.quit().expect("quit at Armed checkpoint");
    hook.release_startup_checkpoint();
    assert!(player.join(Duration::from_secs(5)).expect("worker join"));
    assert!(service.shutdown(events));
    assert_eq!(send_calls.load(std::sync::atomic::Ordering::Acquire), 0);
}

#[cfg(feature = "tauri-test")]
#[test]
fn recommended_advisory_uses_canonical_generation_data_for_each_variant() {
    use crate::commands::{PlaybackDecision, TimingAdvisoryStatus};

    let settings = sky_app_core::settings::ApplicationSettings::default();
    let fingerprint = super::settings_fingerprint(&settings).expect("settings fingerprint");
    let prepared = large_recommendation_input(1, &fingerprint);
    assert_eq!(prepared.schedule.chords.len(), 25_001);
    let recommended_policy =
        MaterializedTimingPolicy::from_user_margin(60, 1.25, 800).expect("recommended policy");
    let recommended_schedule =
        super::recommended_schedule(&prepared.song, None, 0.9, &recommended_policy)
            .expect("recommended schedule");
    assert_eq!(recommended_schedule.chords.len(), 1);
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let result = service
        .prepare(prepared)
        .expect("large Proceed and small optional recommendation Prepare");
    let proceed = result
        .variants
        .iter()
        .find(|variant| variant.decision == PlaybackDecision::Proceed)
        .expect("Proceed DTO");
    let proceed_advisory = proceed.timing_advisory.as_ref().expect("Proceed advisory");
    assert_eq!(proceed_advisory.status, TimingAdvisoryStatus::Analyzed);

    let recommended = result
        .variants
        .iter()
        .find(|variant| variant.decision == PlaybackDecision::UseRecommended)
        .expect("recommended DTO remains publishable");
    let recommended_advisory = recommended
        .timing_advisory
        .as_ref()
        .expect("recommended advisory");
    assert_eq!(recommended_advisory.status, TimingAdvisoryStatus::Analyzed);
    assert_eq!(
        recommended_advisory.plan_fingerprint,
        recommended.plan_fingerprint
    );
    assert_eq!(recommended.config.hold_frames, 1.25);
    assert_eq!(recommended.config.tempo_scale, 0.9);
}

#[cfg(feature = "tauri-test")]
#[test]
fn cancellation_during_recommendation_materialization_fails_whole_prepare() {
    let settings = sky_app_core::settings::ApplicationSettings::default();
    let fingerprint = super::settings_fingerprint(&settings).expect("settings fingerprint");
    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let shutdown_service = Arc::clone(&service);
    *service
        .recommendation_materialization_test_hook
        .lock()
        .expect("recommendation materialization hook") = Some(Arc::new(move || {
        assert!(shutdown_service.shutdown(Arc::new(Mutex::new(NativeEventHub::default()))));
    }));

    let error = service
        .prepare(large_recommendation_input(1, &fingerprint))
        .expect_err("cancellation must reject whole Prepare");
    assert!(error.contains("cancelled") || error.contains("superseded"));
    assert!(service.prepared.lock().expect("prepared cache").is_empty());
}

#[cfg(feature = "tauri-test")]
#[test]
fn supersession_during_recommendation_materialization_fails_old_prepare() {
    let settings = sky_app_core::settings::ApplicationSettings::default();
    let fingerprint = super::settings_fingerprint(&settings).expect("settings fingerprint");
    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let superseding_service = Arc::clone(&service);
    let superseding_id = Arc::new(Mutex::new(None::<String>));
    let superseding_id_for_hook = Arc::clone(&superseding_id);
    let superseding_fingerprint = fingerprint.clone();
    *service
        .recommendation_materialization_test_hook
        .lock()
        .expect("recommendation materialization hook") = Some(Arc::new(move || {
        let superseding = superseding_service
            .prepare(input(1, &superseding_fingerprint))
            .expect("new Prepare supersedes old request");
        *superseding_id_for_hook
            .lock()
            .expect("superseding prepared identity") = superseding.prepared_id;
    }));

    let error = service
        .prepare(large_recommendation_input(1, &fingerprint))
        .expect_err("superseded request must not publish partial variants");
    assert!(error.contains("superseded") || error.contains("cancelled"));
    let superseding_id = superseding_id
        .lock()
        .expect("superseding prepared identity")
        .clone()
        .expect("superseding Prepare published");
    let cache = service.prepared.lock().expect("prepared cache");
    assert_eq!(cache.len(), 1);
    assert_eq!(
        cache.front().map(|(id, _)| id.as_str()),
        Some(superseding_id.as_str())
    );
}

#[test]
fn prepare_publishes_a_plan_that_moves_into_fresh_runtime_state() {
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let compile_checkpoints = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let compile_checkpoints_hook = Arc::clone(&compile_checkpoints);
    *service
        .prepare_compile_test_hook
        .lock()
        .expect("Prepare compiler hook") = Some(Arc::new(move |action_count| {
        if action_count == 0 {
            compile_checkpoints_hook.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }
    }));
    let prepared_input = input(1, "prepared-runtime");
    let source_chord_count = prepared_input.schedule.chords.len();
    let prepare_started = std::time::Instant::now();
    let prepared = service.prepare(prepared_input).expect("physical Prepare");
    let prepare_elapsed_us = prepare_started.elapsed().as_micros();
    let runtime_schedule_compile_count =
        compile_checkpoints.load(std::sync::atomic::Ordering::Acquire);
    assert!(runtime_schedule_compile_count > 0);
    let prepared_id = prepared.prepared_id.expect("prepared identity");
    let mut cache = service.prepared.lock().expect("prepared cache");
    let record = cache
        .iter_mut()
        .find(|(id, _)| id == &prepared_id)
        .map(|(_, record)| record)
        .expect("published prepared record");
    let variant = record
        .variants
        .get_mut(&crate::commands::PlaybackDecision::Proceed)
        .expect("physical proceed variant");
    let plan = variant
        .prepared_schedule
        .take()
        .expect("Prepare materializes the physical dispatch plan");
    let metrics = plan.memory_metrics();
    assert!(plan.has_down());
    assert_eq!(plan.generation_count(), 1);
    assert!(metrics.plan_capacity_bytes > 0);
    assert!(
        metrics.overlapping_peak_capacity_bytes
            <= sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES
    );
    drop(cache);
    let start_runtime_started = std::time::Instant::now();
    let executor = sky_player::adapter_support::DispatchExecutor::from_prepared_schedule(plan);
    let start_runtime_move_ns = start_runtime_started.elapsed().as_nanos();
    assert_eq!(executor.active_mask(), 0);
    assert_eq!(executor.total_chords(), 1);
    eprintln!(
        "prepared_playback_evidence workload=short source_chords={source_chord_count} runtime_schedule_compile_count={runtime_schedule_compile_count} desktop_prepare_wall_us={prepare_elapsed_us} prepared_executor_move_and_state_init_ns={start_runtime_move_ns} prepared_plan_capacity_bytes={} prepare_overlapping_peak_capacity_bytes={} timing_basis=wall_clock_single_offline_run_not_a_gate basis=desktop_prepare_and_prepared_executor_move start_arm_timing=executor-construction-only native_process_setup_and_real_worker_arm=not_measured memory_basis=calculated_size_and_capacity disposition=valid_physical_plan_prepared_and_consumed",
        metrics.plan_capacity_bytes, metrics.overlapping_peak_capacity_bytes,
    );
}

#[test]
fn generation_advisory_does_not_block_large_physical_preparation() {
    use sky_app_core::song::OnsetChord;

    const PAIR_COUNT: u64 = 25_001;
    let service = NativePlaybackService::new(ActivityCoordinator::default());
    let mut prepared = input(1, "advisory-over-limit");
    let chords = (0..PAIR_COUNT)
        .map(|index| OnsetChord {
            at_us: index * 40_000,
            scan_codes: vec![0x15],
        })
        .collect::<Vec<_>>();
    let duration_us = chords.last().expect("last chord").at_us + prepared.schedule.hold_us;
    prepared.schedule.duration_us = duration_us;
    prepared.schedule.playback_duration_us = duration_us;
    prepared.schedule.source_duration_us = duration_us;
    prepared.schedule.chords = chords;

    let result = service
        .prepare(prepared)
        .expect("advisory limit does not reject a valid physical plan");
    let prepared_id = result.prepared_id.expect("prepared identity");
    let variant_dto = result
        .variants
        .iter()
        .find(|variant| variant.decision == crate::commands::PlaybackDecision::Proceed)
        .expect("physical Proceed variant");
    assert_eq!(
        variant_dto
            .timing_advisory
            .as_ref()
            .expect("advisory result")
            .status,
        crate::commands::TimingAdvisoryStatus::Analyzed
    );
    let cache = service.prepared.lock().expect("prepared cache");
    let record = cache
        .iter()
        .find(|(id, _)| id == &prepared_id)
        .map(|(_, record)| record)
        .expect("valid plan remains cached");
    let plan = record
        .variants
        .get(&crate::commands::PlaybackDecision::Proceed)
        .and_then(|variant| variant.prepared_schedule.as_ref())
        .expect("valid physical variant remains available");
    assert_eq!(plan.generation_count(), PAIR_COUNT);
}

#[test]
fn native_schedule_compilation_honors_cancelled_request() {
    let prepared = input(1, "cancelled");
    let cancelled = std::sync::atomic::AtomicBool::new(true);
    let error = super::compile_dispatch_schedule(&prepared.schedule, &cancelled).unwrap_err();
    assert!(error.contains("cancelled"));
    cancelled.store(false, std::sync::atomic::Ordering::Release);
    assert!(super::compile_dispatch_schedule(&prepared.schedule, &cancelled).is_ok());
}

#[test]
fn shutdown_cancels_an_in_flight_prepare_compiler_and_releases_activity() {
    use sky_app_core::song::OnsetChord;

    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let mut prepared = input(1, "settings-a");
    let chords = (0..300u64)
        .map(|index| OnsetChord {
            at_us: index * 100_000,
            scan_codes: vec![0x15],
        })
        .collect::<Vec<_>>();
    prepared.schedule.duration_us =
        chords.last().expect("last chord").at_us + prepared.schedule.hold_us;
    prepared.schedule.playback_duration_us = prepared.schedule.duration_us;
    prepared.schedule.source_duration_us = prepared.schedule.duration_us;
    prepared.schedule.chords = chords;
    let (compiler_entered_tx, compiler_entered_rx) = mpsc::channel();
    let (compiler_release_tx, compiler_release_rx) = mpsc::channel();
    let compiler_release_rx = Mutex::new(compiler_release_rx);
    *service
        .prepare_compile_test_hook
        .lock()
        .expect("Prepare compiler hook") = Some(Arc::new(move |action_count| {
        if action_count == 256 {
            compiler_entered_tx
                .send(action_count)
                .expect("Prepare test is waiting at the compiler checkpoint");
            compiler_release_rx
                .lock()
                .expect("Prepare compiler release lock")
                .recv()
                .expect("Prepare test releases the compiler checkpoint");
        }
    }));

    let prepare_service = Arc::clone(&service);
    let prepare_thread = thread::spawn(move || prepare_service.prepare(prepared));

    match compiler_entered_rx.recv_timeout(Duration::from_secs(10)) {
        Ok(256) => {}
        Ok(action_count) => panic!("unexpected compiler checkpoint {action_count}"),
        Err(error) => {
            let prepare_result = prepare_thread.join().expect("Prepare thread");
            panic!(
                "Prepare compiler did not reach the in-flight checkpoint: {error}; result={prepare_result:?}"
            );
        }
    }
    assert!(service.shutdown(Arc::new(Mutex::new(NativeEventHub::default()))));
    compiler_release_tx
        .send(())
        .expect("Prepare compiler is paused at its in-flight checkpoint");

    let error = prepare_thread
        .join()
        .expect("Prepare thread")
        .expect_err("shutdown cancels Prepare compilation");
    assert!(error.contains("cancelled"));
    assert!(service.prepared.lock().expect("prepared cache").is_empty());
    assert!(!service.activity.physical_release_is_quarantined());
    let lease = service
        .activity
        .reserve_playback("post-prepare-cancellation")
        .expect("Prepare cancellation leaves physical activity available");
    drop(lease);
}

#[test]
fn shutdown_after_prepare_materialization_before_publication_discards_plan() {
    let activity = ActivityCoordinator::default();
    let service = Arc::new(NativePlaybackService::new(activity.clone()));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *service
        .prepare_before_publication_test_hook
        .lock()
        .expect("publication hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let prepare_service = Arc::clone(&service);
    let prepare_thread = thread::spawn(move || prepare_service.prepare(input(1, "settings-a")));
    entered.wait();
    assert!(service.shutdown(Arc::new(Mutex::new(NativeEventHub::default()))));
    release.wait();

    let error = prepare_thread
        .join()
        .expect("Prepare thread")
        .expect_err("shutdown wins before Prepare publication");
    assert!(error.contains("superseded"));
    assert!(service.prepared.lock().expect("prepared cache").is_empty());
    assert!(!service.activity.physical_release_is_quarantined());
    assert!(service.active.lock().expect("active slot").is_none());
    let lease = activity
        .reserve_playback("after-cancelled-prepare")
        .expect("cancelled Prepare leaves activity available");
    drop(lease);
}

#[test]
fn newer_prepare_cancels_old_analysis_without_clearing_new_owner() {
    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let (started, started_rx) = mpsc::channel();
    let (continue_old, continue_old_rx) = mpsc::channel();
    let old_service = Arc::clone(&service);
    let old_request = thread::spawn(move || {
        let cancellation = old_service
            .begin_preparation()
            .expect("first Prepare cancellation token");
        started
            .send(Arc::clone(&cancellation))
            .expect("publish token");
        continue_old_rx.recv().expect("release first request");
        let was_cancelled = cancellation.load(std::sync::atomic::Ordering::Acquire);
        old_service.finish_preparation(&cancellation);
        was_cancelled
    });

    let old_token = started_rx.recv().expect("first Prepare started");
    let new_token = service
        .begin_preparation()
        .expect("newer Prepare cancellation token");
    assert!(old_token.load(std::sync::atomic::Ordering::Acquire));
    assert!(!new_token.load(std::sync::atomic::Ordering::Acquire));

    continue_old.send(()).expect("resume old request");
    assert!(old_request.join().expect("first Prepare thread"));
    let active = service
        .prepare_publication
        .lock()
        .expect("Prepare cancellation owner lock");
    assert!(
        active
            .active_request
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &new_token))
    );
    drop(active);
    service.finish_preparation(&new_token);
    assert!(
        service
            .prepare_publication
            .lock()
            .expect("Prepare cancellation owner lock")
            .active_request
            .is_none()
    );
}

#[test]
fn superseding_request_before_cache_commit_cannot_publish_or_return_old_plan() {
    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *service
        .prepare_before_publication_test_hook
        .lock()
        .expect("publication hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let old_service = Arc::clone(&service);
    let old = thread::spawn(move || old_service.prepare(input(1, "settings-a")));
    entered.wait();

    // The old request has completed model work and is paused immediately
    // before the atomic final check plus cache insertion.
    let latest = service
        .prepare(input(1, "settings-a"))
        .expect("newer request publishes");
    release.wait();

    let old_result = old.join().expect("old Prepare thread");
    assert!(
        old_result
            .expect_err("superseded request must not return an accepted plan")
            .contains("superseded")
    );
    let prepared_id = latest.prepared_id.expect("latest prepared identity");
    let cache = service.prepared.lock().expect("prepared cache");
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, prepared_id);
}

#[test]
fn settings_invalidation_while_model_is_queued_cancels_stale_publication() {
    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *service
        .prepare_before_model_test_hook
        .lock()
        .expect("model hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let old_service = Arc::clone(&service);
    let old = thread::spawn(move || old_service.prepare(input(1, "settings-a")));
    entered.wait();
    service.invalidate_settings();
    release.wait();

    assert!(
        old.join()
            .expect("old Prepare thread")
            .expect_err("settings invalidation rejects in-flight Prepare")
            .contains("superseded")
    );
    assert!(service.prepared.lock().expect("prepared cache").is_empty());

    let latest = service
        .prepare(input(1, "settings-a"))
        .expect("request after invalidation remains usable");
    let prepared_id = latest.prepared_id.expect("latest prepared identity");
    assert_eq!(service.prepared.lock().expect("prepared cache").len(), 1);
    assert!(service.prepared.lock().expect("prepared cache")[0].0 == prepared_id);
}

#[test]
fn catalog_generation_invalidation_cancels_outstanding_prepare_and_keeps_new_generation() {
    let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let entered_hook = Arc::clone(&entered);
    let release_hook = Arc::clone(&release);
    *service
        .prepare_before_model_test_hook
        .lock()
        .expect("model hook") = Some(Arc::new(move || {
        entered_hook.wait();
        release_hook.wait();
    }));

    let old_service = Arc::clone(&service);
    let old = thread::spawn(move || old_service.prepare(input(1, "settings-a")));
    entered.wait();
    service.invalidate_catalog(2);
    release.wait();

    assert!(
        old.join()
            .expect("old Prepare thread")
            .expect_err("catalog reload rejects old-generation Prepare")
            .contains("superseded")
    );
    assert!(service.prepared.lock().expect("prepared cache").is_empty());

    let latest = service
        .prepare(input(2, "settings-a"))
        .expect("new-generation request remains usable");
    let prepared_id = latest.prepared_id.expect("latest prepared identity");
    let cache = service.prepared.lock().expect("prepared cache");
    assert_eq!(cache.len(), 1);
    assert_eq!(cache[0].0, prepared_id);
    assert_eq!(cache[0].1.generation, 2);
}
