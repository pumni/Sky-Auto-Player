use super::{
    ACCEPTANCE_FOCUS_RESTORE_GRACE_US, ACCEPTANCE_HOLD_FRAMES,
    ACCEPTANCE_INPUT_PATH_WARN_US, PhysicalExpectation, Scenario, ScenarioPlan,
    generation_stress_scenario_plan,
};
use sky_dispatch_core::model::{MAX_KEYS, OnsetChordInput};
use sky_dispatch_win32::input::{
    InstrumentKeyProfileSpec, PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalKey,
};
use sky_dispatch_win32::mmcss::PriorityMode;
use sky_player::adapter_support::build_onset_plan;
use sky_player::engine::{
    BackendConfig, DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, DispatchProfile, FocusOptions,
    NativeSessionOptions, PriorityOptions, TelemetryMode, TelemetryOptions, TimingOptions,
    WaitOptions,
};
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

pub(super) fn onset(
    source_action_index: u32,
    scheduled_us: u64,
    slots: &[usize],
) -> OnsetChordInput {
    let scan_codes = slots
        .iter()
        .map(|slot| PHYSICAL_INSTRUMENT_SCAN_CODES[*slot])
        .collect::<SmallVec<[u16; 4]>>();
    OnsetChordInput {
        source_action_index,
        scheduled_us,
        scan_codes,
    }
}

pub(crate) fn noncanonical_profile_spec() -> InstrumentKeyProfileSpec {
    let mut spec = InstrumentKeyProfileSpec::canonical();
    spec.keys[0] = PhysicalKey {
        scan_code: 0x02,
        extended: false,
    };
    spec
}

#[cfg(test)]
pub(super) fn scenario_plan(
    scenario: Scenario,
    timing_margin_us: u64,
) -> Result<ScenarioPlan, String> {
    scenario_plan_for_settings(scenario, timing_margin_us, 60, None)
}

#[cfg(test)]
pub(super) fn scenario_plan_with_sample_seconds(
    scenario: Scenario,
    timing_margin_us: u64,
    sample_seconds: Option<u8>,
) -> Result<ScenarioPlan, String> {
    scenario_plan_for_settings(scenario, timing_margin_us, 60, sample_seconds)
}

pub(super) fn scenario_plan_for_settings(
    scenario: Scenario,
    timing_margin_us: u64,
    game_fps: u16,
    sample_seconds: Option<u8>,
) -> Result<ScenarioPlan, String> {
    if !matches!(game_fps, 60 | 120) {
        return Err("acceptance workload game FPS must be 60 or 120".to_string());
    }
    let hold_us = scenario_min_hold_us_for_fps(scenario, timing_margin_us, game_fps);
    let (chords, profile, expected_down_slots, expected_up_slots, allow_unpaired_cleanup_ups) =
        match scenario {
            Scenario::CanonicalSingle | Scenario::NoncanonicalProfile => (
                vec![onset(0, 50_000, &[0])],
                (scenario == Scenario::NoncanonicalProfile).then_some(noncanonical_profile_spec()),
                vec![0],
                vec![0],
                false,
            ),
            Scenario::CanonicalChord => (
                vec![onset(0, 50_000, &[0, 1])],
                None,
                vec![0, 1],
                vec![0, 1],
                false,
            ),
            Scenario::CanonicalMaxChord => (
                vec![onset(0, 50_000, &(0..MAX_KEYS).collect::<Vec<_>>())],
                None,
                (0..MAX_KEYS).collect(),
                (0..MAX_KEYS).collect(),
                false,
            ),
            Scenario::Hold => (
                vec![onset(0, 50_000, &[0])],
                None,
                vec![0],
                vec![0],
                false,
            ),
            Scenario::LongSingleSequence => {
                let gap_us = frame_period_us(game_fps);
                let mut chords = Vec::with_capacity(24);
                let mut down_slots = Vec::with_capacity(24);
                let mut up_slots = Vec::with_capacity(24);
                let mut timestamp_us = 50_000_u64;
                for index in 0..24_u32 {
                    chords.push(onset(index, timestamp_us, &[0]));
                    down_slots.push(0);
                    up_slots.push(0);
                    timestamp_us += hold_us + gap_us;
                }
                (chords, None, down_slots, up_slots, false)
            }
            Scenario::DenseAlternating => {
                let gap_us = frame_period_us(game_fps).saturating_add(timing_margin_us);
                let mut chords = Vec::with_capacity(16);
                let mut down_slots = Vec::with_capacity(16);
                let mut up_slots = Vec::with_capacity(16);
                let mut timestamp_us = 50_000_u64;
                for index in 0..16_u32 {
                    let slot = (index as usize) % 2;
                    chords.push(onset(index, timestamp_us, &[slot]));
                    down_slots.push(slot);
                    up_slots.push(slot);
                    timestamp_us += hold_us + gap_us;
                }
                (chords, None, down_slots, up_slots, false)
            }
            Scenario::ChordSweep => {
                let gap_us = 100_000;
                let mut chords = Vec::with_capacity(MAX_KEYS - 1);
                let mut down_slots = Vec::new();
                let mut up_slots = Vec::new();
                let mut timestamp_us = 50_000_u64;
                for (action_index, chord_size) in (2..=MAX_KEYS).enumerate() {
                    let slots = (0..chord_size).collect::<Vec<_>>();
                    chords.push(onset(action_index as u32, timestamp_us, &slots));
                    down_slots.extend(slots.iter().copied());
                    up_slots.extend(slots);
                    timestamp_us += hold_us + gap_us;
                }
                (chords, None, down_slots, up_slots, false)
            }
            Scenario::NearMinimumRetrigger => {
                let gap_us = frame_period_us(game_fps).saturating_add(timing_margin_us);
                let mut chords = Vec::with_capacity(4);
                let mut down_slots = Vec::with_capacity(4);
                let mut up_slots = Vec::with_capacity(4);
                let mut timestamp_us = 50_000_u64;
                for index in 0..4_u32 {
                    chords.push(onset(index, timestamp_us, &[0]));
                    down_slots.push(0);
                    up_slots.push(0);
                    timestamp_us += hold_us + gap_us;
                }
                (chords, None, down_slots, up_slots, false)
            }
            Scenario::RapidRetrigger => (
                vec![
                    onset(0, 50_000, &[0]),
                    onset(1, 110_000, &[0]),
                    onset(2, 170_000, &[0]),
                ],
                None,
                vec![0, 0, 0],
                vec![0, 0, 0],
                false,
            ),
            Scenario::ShortUpGapStress => {
                return generation_stress_scenario_plan(timing_margin_us);
            }
            Scenario::MixedUpDown => (
                vec![
                    onset(0, 50_000, &[0]),
                    onset(1, 50_000 + hold_us, &[1]),
                ],
                None,
                vec![0, 1],
                vec![0, 1],
                false,
            ),
            Scenario::AmbiguousPacket => (
                vec![
                    onset(0, 50_000, &[0]),
                    onset(1, 50_000 + hold_us, &[1]),
                ],
                None,
                vec![0, 1],
                vec![0, 0, 1],
                true,
            ),
            Scenario::PreflightUserHeld => (
                vec![onset(0, 50_000, &[0])],
                None,
                vec![],
                vec![],
                true,
            ),
            Scenario::ModifierHeldFinalBoundary => (
                vec![onset(0, 50_000, &[0])],
                None,
                vec![],
                vec![],
                true,
            ),
            Scenario::ModifierHeldAfterOwned => (
                vec![
                    onset(0, 50_000, &[0]),
                    onset(1, 50_000 + hold_us / 2, &[1]),
                ],
                None,
                vec![0],
                vec![0],
                false,
            ),
            Scenario::CleanupFullRelease => (
                vec![onset(0, 50_000, &(0..MAX_KEYS).collect::<Vec<_>>())],
                None,
                (0..MAX_KEYS).collect(),
                (0..MAX_KEYS).collect(),
                false,
            ),
            Scenario::FocusLoss => (
                vec![onset(0, 500_000, &[0]), onset(1, 3_000_000, &[1])],
                None,
                vec![0, 1],
                vec![0, 1],
                true,
            ),
            Scenario::TargetHwndChange
            | Scenario::OwnerMismatch
            | Scenario::OwnerQueryFailure
            | Scenario::OwnerProcessTermination => (
                vec![onset(0, 500_000, &[0])],
                None,
                vec![],
                vec![],
                true,
            ),
            Scenario::StopCleanup | Scenario::SkipCleanup => (
                vec![onset(0, 50_000, &[0])],
                None,
                vec![0],
                vec![0],
                false,
            ),
            Scenario::PauseResume => (
                vec![onset(0, 50_000, &[0]), onset(1, 500_000, &[1])],
                None,
                vec![0, 1],
                vec![0, 1],
                true,
            ),
            Scenario::PauseResumeActiveCleanup => (
                vec![onset(0, 50_000, &[0]), onset(1, 600_000, &[0])],
                None,
                vec![0, 0],
                vec![0, 0],
                true,
            ),
            Scenario::SuspendResume => (
                vec![onset(0, 50_000, &[0]), onset(1, 1_000_000, &[1])],
                None,
                vec![0, 1],
                vec![0, 0, 1],
                true,
            ),
            Scenario::SupervisorLeaseExpiry => (
                vec![onset(0, 5_000_000, &[0])],
                None,
                vec![],
                vec![],
                true,
            ),
            Scenario::LeaseAgeWatchdogDelayed => {
                let down = DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US + 500_000;
                (vec![onset(0, down, &[0])], None, vec![], vec![], true)
            }
            Scenario::TimingMarginSweep => {
                let down = 50_000;
                let next_down = down + hold_us;
                (
                    vec![onset(0, down, &[0]), onset(1, next_down, &[0])],
                    None,
                    vec![0, 0],
                    vec![0, 0],
                    false,
                )
            }
            Scenario::LongIdle => {
                let mut chords = Vec::with_capacity(4);
                let mut down_slots = Vec::with_capacity(4);
                let mut up_slots = Vec::with_capacity(4);
                let mut down_us = 50_000_u64;
                for index in 0..4_u32 {
                    let slot = if index % 2 == 0 { 0 } else { 1 };
                    chords.push(onset(index, down_us, &[slot]));
                    let up_us = down_us
                        .checked_add(hold_us)
                        .ok_or("long-idle schedule overflow")?;
                    down_slots.push(slot);
                    up_slots.push(slot);
                    let idle_us = if index % 2 == 0 { 250_000 } else { 2_000_000 };
                    down_us = up_us
                        .checked_add(idle_us)
                        .ok_or("long-idle schedule overflow")?;
                }
                (chords, None, down_slots, up_slots, false)
            }
            Scenario::HolInterleaved => (
                vec![
                    onset(0, 50_000, &[0]),
                    onset(1, 50_000 + hold_us + 1_000, &[1]),
                ],
                None,
                vec![0, 1],
                vec![0, 1],
                false,
            ),
        };
    let mut chords = chords;
    let mut expected_down_slots = expected_down_slots;
    let mut expected_up_slots = expected_up_slots;
    if let Some(sample_seconds) = sample_seconds {
        if !scenario.supports_timed_sample() {
            return Err(format!(
                "{scenario:?} does not support a repeated timed sample"
            ));
        }
        let first_scheduled_us = chords
            .first()
            .map(|chord| chord.scheduled_us)
            .ok_or_else(|| "timed sample scenario has no onset chords".to_string())?;
        let last_end_us = chords
            .last()
            .and_then(|chord| chord.scheduled_us.checked_add(hold_us))
            .ok_or_else(|| "timed sample scenario end overflow".to_string())?;
        let cycle_gap_us = if scenario == Scenario::LongIdle {
            2_000_000
        } else if scenario == Scenario::HolInterleaved {
            31_000
        } else {
            frame_period_us(game_fps).saturating_add(timing_margin_us)
        };
        let cycle_duration_us = last_end_us
            .checked_sub(first_scheduled_us)
            .and_then(|span| span.checked_add(cycle_gap_us))
            .filter(|duration| *duration > 0)
            .ok_or_else(|| "timed sample cycle duration is invalid".to_string())?;
        let required_duration_us = u64::from(sample_seconds)
            .saturating_mul(1_000_000)
            .saturating_add(cycle_gap_us);
        let repetitions = required_duration_us
            .saturating_add(cycle_duration_us - 1)
            / cycle_duration_us;
        let base_chords = chords.clone();
        let base_down_slots = expected_down_slots.clone();
        let base_up_slots = expected_up_slots.clone();
        for repetition in 1..repetitions {
            let schedule_offset_us = cycle_duration_us.saturating_mul(repetition);
            for chord in &base_chords {
                let mut repeated = chord.clone();
                repeated.source_action_index = u32::try_from(chords.len())
                    .map_err(|_| "timed sample exceeds source action index capacity".to_string())?;
                repeated.scheduled_us = repeated
                    .scheduled_us
                    .checked_add(schedule_offset_us)
                    .ok_or_else(|| "timed sample schedule overflow".to_string())?;
                chords.push(repeated);
            }
            expected_down_slots.extend_from_slice(&base_down_slots);
            expected_up_slots.extend_from_slice(&base_up_slots);
        }
    }
    let schedule = build_onset_plan(&chords, &PHYSICAL_INSTRUMENT_SCAN_CODES, hold_us)
        .map_err(|error| format!("scenario onset plan failed: {error}"))?;
    let expected_safety_up_slots = match scenario {
        Scenario::CleanupFullRelease => (0..MAX_KEYS).collect(),
        Scenario::StopCleanup
        | Scenario::SkipCleanup
        | Scenario::SuspendResume
        | Scenario::ModifierHeldAfterOwned => vec![0],
        Scenario::PauseResumeActiveCleanup => vec![0],
        Scenario::AmbiguousPacket => vec![0, 1],
        _ => Vec::new(),
    };
    let mut expected_authored_up_slots = expected_up_slots.clone();
    for slot in &expected_safety_up_slots {
        if let Some(index) = expected_authored_up_slots
            .iter()
            .position(|expected| expected == slot)
        {
            expected_authored_up_slots.remove(index);
        }
    }
    Ok(ScenarioPlan {
        schedule,
        profile,
        expected_down_slots,
        expected_up_slots,
        allow_unpaired_cleanup_ups,
        expected_authored_up_slots,
        expected_safety_up_slots,
    })
}

#[cfg(test)]
pub(super) fn acceptance_min_hold_us(timing_margin_us: u64) -> u64 {
    acceptance_min_hold_us_for_fps(timing_margin_us, 60)
}

#[cfg(test)]
pub(super) fn acceptance_min_hold_us_for_fps(timing_margin_us: u64, game_fps: u16) -> u64 {
    ACCEPTANCE_HOLD_FRAMES * frame_period_us(game_fps) + timing_margin_us
}

pub(super) fn scenario_min_hold_us_for_fps(
    scenario: Scenario,
    timing_margin_us: u64,
    game_fps: u16,
) -> u64 {
    let frames = if scenario == Scenario::HolInterleaved {
        2
    } else {
        ACCEPTANCE_HOLD_FRAMES
    };
    frames * frame_period_us(game_fps) + timing_margin_us
}

const fn frame_period_us(game_fps: u16) -> u64 {
    1_000_000_u64.div_ceil(game_fps as u64)
}

#[cfg(test)]
pub(super) fn production_options(
    schedule: sky_dispatch_core::model::OnsetPlan,
    profile: Option<InstrumentKeyProfileSpec>,
    timing_margin_us: u64,
    scenario: Scenario,
    modifier_query_count: Arc<AtomicU64>,
) -> Result<NativeSessionOptions, String> {
    production_options_for_fps(
        schedule,
        profile,
        timing_margin_us,
        scenario,
        modifier_query_count,
        super::modifier_guard::ModifierHeldAfterOwnedLatch::default(),
        60,
    )
}

pub(super) fn production_options_for_fps(
    schedule: sky_dispatch_core::model::OnsetPlan,
    profile: Option<InstrumentKeyProfileSpec>,
    timing_margin_us: u64,
    scenario: Scenario,
    modifier_query_count: Arc<AtomicU64>,
    after_owned_latch: super::modifier_guard::ModifierHeldAfterOwnedLatch,
    game_fps: u16,
) -> Result<NativeSessionOptions, String> {
    let frame_us = frame_period_us(game_fps);
    let timing = TimingOptions {
        game_fps,
        min_hold_us: scenario_min_hold_us_for_fps(scenario, timing_margin_us, game_fps),
        frame_us,
        frame_base_hold_us: if scenario == Scenario::HolInterleaved {
            2 * frame_us
        } else {
            frame_us
        },
        timing_margin_us,
        strict_timing: false,
        strict_down_completion_late_us: 2_000,
        strict_up_completion_late_us: 2_000,
        input_path_warn_us: ACCEPTANCE_INPUT_PATH_WARN_US,
    };
    let qpc_frequency_hz = sky_player::adapter_support::qpc_frequency_hz()
        .map_err(|error| format!("QPC frequency failed during acceptance Prepare: {error:?}"))?;
    let qpc_frequency_hz = std::num::NonZeroU64::new(qpc_frequency_hz)
        .ok_or_else(|| "QPC frequency was zero during acceptance Prepare".to_string())?;
    let prepared_schedule = sky_dispatch_core::executor::PreparedDispatchSchedule::prepare(
        &schedule,
        qpc_frequency_hz,
        schedule.compile_memory.caller_retained_capacity_bytes,
    )
    .map_err(|error| format!("native acceptance schedule Prepare failed: {error}"))?;
    let modifier_key_state_query_for_test =
        super::modifier_guard::query_for_scenario(
            scenario,
            Arc::clone(&modifier_query_count),
            after_owned_latch,
        );
    Ok(NativeSessionOptions {
        schedule: prepared_schedule,
        backend: BackendConfig::Production,
        profile: DispatchProfile::Production,
        timing,
        focus: FocusOptions {
            require_focus: true,
            focus_restore_grace_us: ACCEPTANCE_FOCUS_RESTORE_GRACE_US,
        },
        wait: WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
            modifier_hold_timeout_us: sky_player::engine::DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
            #[cfg(feature = "test-support")]
            test_spin_threshold_us: None,
            #[cfg(feature = "test-support")]
            test_wait_policy: sky_player::engine::TestWaitPolicy::ProductionCalibrated,
        },
        telemetry: TelemetryOptions {
            mode: TelemetryMode::Ring,
            capacity: 1_024,
        },
        priority: PriorityOptions {
            mode: PriorityMode::Auto,
        },
        instrument_key_profile: profile,
        #[cfg(feature = "test-support")]
        startup_calibration_placement:
            sky_player::engine::StartupCalibrationPlacement::BeforeSchedulingGuards,
        #[cfg(feature = "test-support")]
        timing_observer_for_qualification: false,
        #[cfg(feature = "test-support")]
        startup_ordering_hook: None,
        #[cfg(feature = "test-support")]
        focus_pause_hook: None,
        #[cfg(feature = "test-support")]
        timer_lifecycle_context: None,
        #[cfg(feature = "test-support")]
        prepared_packet_ambiguity_mask: (scenario == Scenario::AmbiguousPacket).then_some(0b11),
        #[cfg(feature = "test-support")]
        preflight_user_held_mask: (scenario == Scenario::PreflightUserHeld).then_some(0b01),
        #[cfg(feature = "test-support")]
        modifier_key_state_query_for_test,
    })
}

pub(super) fn expected_physical_keys(
    profile: Option<&InstrumentKeyProfileSpec>,
    slots: &[usize],
) -> Vec<PhysicalExpectation> {
    slots
        .iter()
        .map(|slot| {
            let key = profile
                .map(|profile| profile.keys[*slot])
                .unwrap_or(PhysicalKey {
                    scan_code: PHYSICAL_INSTRUMENT_SCAN_CODES[*slot],
                    extended: false,
                });
            PhysicalExpectation {
                scan_code: key.scan_code,
                extended: key.extended,
            }
        })
        .collect()
}
