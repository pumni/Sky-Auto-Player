use super::{
    ACCEPTANCE_FRAME_US, ACCEPTANCE_FOCUS_RESTORE_GRACE_US, ACCEPTANCE_HOLD_FRAMES,
    ACCEPTANCE_INPUT_PATH_WARN_US, PhysicalExpectation, Scenario, ScenarioPlan,
    release_gap_scenario_plan,
};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, MAX_KEYS};
use sky_dispatch_win32::mmcss::PriorityMode;
use sky_dispatch_win32::input::{
    InstrumentKeyProfileSpec, PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalKey,
};
use sky_player::adapter_support::compile_runtime_intents;
use sky_player::engine::{
    BackendConfig, DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US, DispatchProfile, FocusOptions,
    NativeSessionOptions, PriorityOptions, TelemetryMode, TelemetryOptions, TimingOptions,
    WaitOptions,
};
use smallvec::SmallVec;
use std::sync::Arc;

pub(super) fn action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    slots: &[usize],
) -> KeyActionInput {
    let scan_codes = slots
        .iter()
        .map(|slot| PHYSICAL_INSTRUMENT_SCAN_CODES[*slot])
        .collect::<SmallVec<[u16; 4]>>();
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes,
        reason: Arc::<str>::from("rt-native-acceptance"),
    }
}

pub(crate) fn w4_profile_spec() -> InstrumentKeyProfileSpec {
    let mut spec = InstrumentKeyProfileSpec::canonical();
    spec.keys[0] = PhysicalKey {
        scan_code: 0x02,
        extended: false,
    };
    spec
}

pub(super) fn scenario_plan(
    scenario: Scenario,
    timing_margin_us: u64,
) -> Result<ScenarioPlan, String> {
    let (actions, profile, expected_down_slots, expected_up_slots, allow_unpaired_cleanup_ups) =
        match scenario {
            Scenario::CanonicalSingle | Scenario::W4Noncanonical => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 80_000, &[0]),
                ],
                (scenario == Scenario::W4Noncanonical).then_some(w4_profile_spec()),
                vec![0],
                vec![0],
                false,
            ),
            Scenario::CanonicalChord => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0, 1]),
                    action(1, ActionKind::Up, 90_000, &[0, 1]),
                ],
                None,
                vec![0, 1],
                vec![0, 1],
                false,
            ),
            Scenario::CanonicalMaxChord => (
                vec![
                    action(
                        0,
                        ActionKind::Down,
                        50_000,
                        &(0..MAX_KEYS).collect::<Vec<_>>(),
                    ),
                    action(
                        1,
                        ActionKind::Up,
                        90_000,
                        &(0..MAX_KEYS).collect::<Vec<_>>(),
                    ),
                ],
                None,
                (0..MAX_KEYS).collect(),
                (0..MAX_KEYS).collect(),
                false,
            ),
            Scenario::Hold => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 500_000, &[0]),
                ],
                None,
                vec![0],
                vec![0],
                false,
            ),
            Scenario::LongSingleSequence => {
                let hold_us = acceptance_min_hold_us(timing_margin_us);
                let gap_us = acceptance_min_release_gap_us(timing_margin_us);
                let mut actions = Vec::with_capacity(48);
                let mut down_slots = Vec::with_capacity(24);
                let mut up_slots = Vec::with_capacity(24);
                let mut timestamp_us = 50_000_u64;
                for index in 0..24_u32 {
                    actions.push(action(index * 2, ActionKind::Down, timestamp_us, &[0]));
                    actions.push(action(
                        index * 2 + 1,
                        ActionKind::Up,
                        timestamp_us + hold_us,
                        &[0],
                    ));
                    down_slots.push(0);
                    up_slots.push(0);
                    timestamp_us += hold_us + gap_us;
                }
                (actions, None, down_slots, up_slots, false)
            }
            Scenario::DenseAlternating => {
                let hold_us = acceptance_min_hold_us(timing_margin_us);
                let gap_us = acceptance_min_release_gap_us(timing_margin_us);
                let mut actions = Vec::with_capacity(32);
                let mut down_slots = Vec::with_capacity(16);
                let mut up_slots = Vec::with_capacity(16);
                let mut timestamp_us = 50_000_u64;
                for index in 0..16_u32 {
                    let slot = (index as usize) % 2;
                    actions.push(action(index * 2, ActionKind::Down, timestamp_us, &[slot]));
                    actions.push(action(
                        index * 2 + 1,
                        ActionKind::Up,
                        timestamp_us + hold_us,
                        &[slot],
                    ));
                    down_slots.push(slot);
                    up_slots.push(slot);
                    timestamp_us += hold_us + gap_us;
                }
                (actions, None, down_slots, up_slots, false)
            }
            Scenario::ChordSweep => {
                let hold_us = acceptance_min_hold_us(timing_margin_us);
                // Keep the chord-size sweep physically observable on the
                // receive-only Windows sink. Dense minimum-geometry timing is
                // covered independently by DenseAlternating.
                let gap_us = 100_000;
                let mut actions = Vec::with_capacity((MAX_KEYS - 1) * 2);
                let mut down_slots = Vec::new();
                let mut up_slots = Vec::new();
                let mut timestamp_us = 50_000_u64;
                let mut action_index = 0_u32;
                for chord_size in 2..=MAX_KEYS {
                    let slots = (0..chord_size).collect::<Vec<_>>();
                    actions.push(action(action_index, ActionKind::Down, timestamp_us, &slots));
                    actions.push(action(
                        action_index + 1,
                        ActionKind::Up,
                        timestamp_us + hold_us,
                        &slots,
                    ));
                    down_slots.extend(slots.iter().copied());
                    up_slots.extend(slots);
                    timestamp_us += hold_us + gap_us;
                    action_index += 2;
                }
                (actions, None, down_slots, up_slots, false)
            }
            Scenario::NearMinimumRetrigger => {
                let hold_us = acceptance_min_hold_us(timing_margin_us);
                let gap_us = acceptance_min_release_gap_us(timing_margin_us);
                let mut actions = Vec::with_capacity(8);
                let mut down_slots = Vec::with_capacity(4);
                let mut up_slots = Vec::with_capacity(4);
                let mut timestamp_us = 50_000_u64;
                for index in 0..4_u32 {
                    actions.push(action(index * 2, ActionKind::Down, timestamp_us, &[0]));
                    actions.push(action(
                        index * 2 + 1,
                        ActionKind::Up,
                        timestamp_us + hold_us,
                        &[0],
                    ));
                    down_slots.push(0);
                    up_slots.push(0);
                    timestamp_us += hold_us + gap_us;
                }
                (actions, None, down_slots, up_slots, false)
            }
            Scenario::RapidRetrigger => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 80_000, &[0]),
                    action(2, ActionKind::Down, 110_000, &[0]),
                    action(3, ActionKind::Up, 140_000, &[0]),
                    action(4, ActionKind::Down, 170_000, &[0]),
                    action(5, ActionKind::Up, 200_000, &[0]),
                ],
                None,
                vec![0, 0, 0],
                vec![0, 0, 0],
                false,
            ),
            Scenario::ReleaseGapStress => return release_gap_scenario_plan(timing_margin_us),
            Scenario::MixedUpDown => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 100_000, &[0]),
                    action(2, ActionKind::Down, 100_000, &[1]),
                    action(3, ActionKind::Up, 150_000, &[1]),
                ],
                None,
                vec![0, 1],
                vec![0, 1],
                false,
            ),
            Scenario::CleanupFullRelease => (
                vec![
                    action(0, ActionKind::Down, 50_000, &(0..MAX_KEYS).collect::<Vec<_>>()),
                    action(1, ActionKind::Up, 10_000_000, &(0..MAX_KEYS).collect::<Vec<_>>()),
                ],
                None,
                (0..MAX_KEYS).collect(),
                (0..MAX_KEYS).collect(),
                false,
            ),
            Scenario::FocusLoss => (
                vec![
                    action(0, ActionKind::Down, 500_000, &[0]),
                    action(1, ActionKind::Up, 600_000, &[0]),
                    action(2, ActionKind::Down, 3_000_000, &[1]),
                    action(3, ActionKind::Up, 3_100_000, &[1]),
                ],
                None,
                vec![0, 1],
                vec![0].into_iter().chain(0..MAX_KEYS).chain([1]).collect(),
                true,
            ),
            Scenario::TargetHwndChange => (
                vec![
                    action(0, ActionKind::Down, 500_000, &[0]),
                    action(1, ActionKind::Up, 600_000, &[0]),
                ],
                None,
                vec![],
                (0..MAX_KEYS).collect(),
                true,
            ),
            Scenario::StopCleanup | Scenario::SkipCleanup => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 10_000_000, &[0]),
                ],
                None,
                vec![0],
                vec![0],
                false,
            ),
            Scenario::PauseResume => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 120_000, &[0]),
                    action(2, ActionKind::Down, 500_000, &[1]),
                    action(3, ActionKind::Up, 570_000, &[1]),
                ],
                None,
                vec![0, 1],
                vec![0, 1].into_iter().chain(0..MAX_KEYS).collect(),
                true,
            ),
            Scenario::SuspendResume => (
                vec![
                    action(0, ActionKind::Down, 50_000, &[0]),
                    action(1, ActionKind::Up, 500_000, &[0]),
                    action(2, ActionKind::Down, 1_000_000, &[1]),
                    action(3, ActionKind::Up, 1_100_000, &[1]),
                ],
                None,
                vec![0, 1],
                (0..MAX_KEYS).chain([0, 1]).collect(),
                true,
            ),
            Scenario::SupervisorLeaseExpiry => (
                vec![
                    action(0, ActionKind::Down, 5_000_000, &[0]),
                    action(1, ActionKind::Up, 5_100_000, &[0]),
                ],
                None,
                vec![],
                (0..MAX_KEYS).collect(),
                true,
            ),
            Scenario::TimingMarginSweep => {
                let down = 50_000;
                let up = down + acceptance_min_hold_us(timing_margin_us);
                let next_down = up + acceptance_min_release_gap_us(timing_margin_us);
                let next_up = next_down + acceptance_min_hold_us(timing_margin_us);
                (
                    vec![
                        action(0, ActionKind::Down, down, &[0]),
                        action(1, ActionKind::Up, up, &[0]),
                        action(2, ActionKind::Down, next_down, &[0]),
                        action(3, ActionKind::Up, next_up, &[0]),
                    ],
                    None,
                    vec![0, 0],
                    vec![0, 0],
                    false,
                )
            }
        };
    let schedule = compile_runtime_intents(&actions, &PHYSICAL_INSTRUMENT_SCAN_CODES)
        .map_err(|error| format!("scenario schedule compilation failed: {error}"))?;
    Ok(ScenarioPlan {
        schedule,
        profile,
        expected_down_slots,
        expected_up_slots,
        allow_unpaired_cleanup_ups,
    })
}

pub(super) fn acceptance_min_hold_us(timing_margin_us: u64) -> u64 {
    ACCEPTANCE_HOLD_FRAMES * ACCEPTANCE_FRAME_US + timing_margin_us
}

pub(super) fn acceptance_min_release_gap_us(timing_margin_us: u64) -> u64 {
    ACCEPTANCE_FRAME_US + timing_margin_us
}

pub(super) fn production_options(
    schedule: sky_dispatch_core::model::RuntimeSchedule,
    profile: Option<InstrumentKeyProfileSpec>,
    timing_margin_us: u64,
) -> NativeSessionOptions {
    NativeSessionOptions {
        schedule,
        backend: BackendConfig::Production,
        profile: DispatchProfile::Production,
        timing: TimingOptions {
            game_fps: 60,
            min_hold_us: acceptance_min_hold_us(timing_margin_us),
            min_release_gap_us: acceptance_min_release_gap_us(timing_margin_us),
            frame_us: ACCEPTANCE_FRAME_US,
            frame_base_hold_us: ACCEPTANCE_FRAME_US,
            timing_margin_us,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: ACCEPTANCE_INPUT_PATH_WARN_US,
        },
        focus: FocusOptions {
            require_focus: true,
            focus_restore_grace_us: ACCEPTANCE_FOCUS_RESTORE_GRACE_US,
        },
        wait: WaitOptions {
            enable_waitable_timer: true,
            enable_event_wait: true,
            supervisor_lease_timeout_us: DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
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
        startup_ordering_hook: None,
        #[cfg(feature = "test-support")]
        restore_race_hook: None,
        #[cfg(feature = "test-support")]
        focus_pause_hook: None,
        #[cfg(feature = "test-support")]
        timer_lifecycle_context: None,
    }
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
