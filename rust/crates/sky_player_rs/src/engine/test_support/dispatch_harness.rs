#![cfg(any(test, feature = "test-support"))]

//! Test harness for production dispatch paths (DownOnly, Mixed, UpOnly release).
//!
//! Provides `ProductionDispatchTestHarness` for deterministic zero-allocation
//! verification of production dispatch functions.

use crate::engine::config::{DispatchProfile, WorkerConfig};
use crate::engine::shared::SharedProgressClock;
use crate::engine::telemetry::{
    SharedMetrics, TelemetryCollector, TelemetryMode, WorkerMetricsLocal,
};
use crate::engine::worker::dispatch::PendingObservationQueue;
use crate::engine::worker::dispatch::observer::HoldForensics;
use crate::engine::worker::dispatch::{
    AuthoredPacketContext, DispatchStep, DownBoundaryAdmission, dispatch_authored_packet,
};
use crate::engine::worker::{
    DispatchHealthOptions, DispatchPath, NextDispatchPlan, PreparationCounts, TargetStamp,
    WaitBoundary, WaitBoundaryInput, WaitDeadline, WaitMutable, WaitResult, WaitSignals,
    WaitTiming, WorkerHealthState, WorkerResources, WorkerRuntime, WorkerSchedulingGuards,
    WorkerTimingState, dispatch_due_from_plan, plan_next_dispatch, plan_next_dispatch_projected,
    preflight_prepared_plan, wait_for_next_boundary,
};
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{RuntimeDispatchCoordinator, physical_packet_kind};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, PhysicalPacketKind};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{
    InstrumentPhysicalState, PHYSICAL_INSTRUMENT_SCAN_CODES, PacketRetryReason, PhysicalPacket,
    PlatformSendResult, PreparedPhysicalPacket, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus, TrackedKeyState,
};
use sky_dispatch_win32::wait::HybridWaiter;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
pub struct ProductionDispatchTestHarness {
    pub(crate) config: WorkerConfig,
    pub(crate) resources: WorkerResources,
    pub(crate) health: WorkerHealthState,
    pub(crate) timing: WorkerTimingState,
    pub(crate) runtime: WorkerRuntime,
    pub(crate) local_metrics: WorkerMetricsLocal,
    pub(crate) focus_active: AtomicBool,
    pub(crate) target_hwnd: AtomicIsize,
    pub(crate) target_generation: AtomicU64,
    pub(crate) quit_requested: AtomicBool,
    pub(crate) skip_requested: AtomicBool,
    pub(crate) panic_requested: AtomicBool,
    pub(crate) desired_pause: AtomicBool,
    pub(crate) supervisor_heartbeat_ticks: AtomicU64,
    pub(crate) metrics: SharedMetrics,
    pub(crate) progress_clock: SharedProgressClock,
    pub(crate) observer: PendingObservationQueue,
    pub(crate) hold_forensics: HoldForensics,
    pub(crate) interrupt: OwnedEvent,
    pub(crate) last_wait_result: Option<WaitResult>,
    effective_now_ticks: TimelineTicks,
}

#[allow(dead_code)]
impl ProductionDispatchTestHarness {
    pub fn new_down_only() -> Self {
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: vec![0x15].into(),
                reason: "up".into(),
            },
        ])
    }

    /// Build a DownOnly chord whose physical deadline is at 1 ms.
    pub fn new_down_chord(key_count: usize) -> Self {
        Self::new_down_chord_with_gap(key_count, 1_000)
    }

    pub fn new_down_chord_with_gap(key_count: usize, gap_us: u64) -> Self {
        assert!((1..=15).contains(&key_count), "key count must be 1..=15");
        let scan_codes = PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count].to_vec();
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: gap_us,
                scan_codes: scan_codes.clone().into(),
                reason: "bench-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: gap_us.saturating_mul(2),
                scan_codes: scan_codes.into(),
                reason: "bench-up".into(),
            },
        ])
    }

    pub fn new_mixed() -> Self {
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down1".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1000,
                scan_codes: vec![0x15].into(),
                reason: "up1".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: vec![0x16].into(),
                reason: "down2".into(),
            },
        ])
    }

    /// Two independent physical Down boundaries used by the deterministic
    /// pre-wait stall/backlog regression.
    pub fn new_two_down_boundaries() -> Self {
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "backlog-a-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: vec![0x16].into(),
                reason: "backlog-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 2_000,
                scan_codes: vec![0x15, 0x16].into(),
                reason: "backlog-cleanup".into(),
            },
        ])
    }

    /// Three overdue Down boundaries followed by a future Down. This keeps
    /// the no-catch-up proof separate from the two-boundary authorization
    /// race: every overdue Down must be committed missed before the future
    /// authored boundary can resume.
    pub fn new_three_overdue_then_future() -> Self {
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "stall-a-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: vec![0x16].into(),
                reason: "stall-b-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 2_000,
                scan_codes: vec![0x17].into(),
                reason: "stall-c-down".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Down,
                scheduled_us: 3_000,
                scan_codes: vec![0x18].into(),
                reason: "stall-d-down".into(),
            },
            KeyActionInput {
                source_action_index: 4,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: vec![0x19].into(),
                reason: "stall-e-future-down".into(),
            },
            KeyActionInput {
                source_action_index: 5,
                kind: ActionKind::Up,
                scheduled_us: 101_000,
                scan_codes: vec![0x15, 0x16, 0x17, 0x18, 0x19].into(),
                reason: "stall-cleanup".into(),
            },
        ])
    }

    /// A deferred release for key A shares an authored timestamp with an
    /// unrelated Down chord on keys B/C. Production must send B/C at their
    /// authored boundary and retain A as an independent pending release.
    pub fn new_deferred_release_with_unrelated_down() -> Self {
        // Leave enough slack for direct-dispatch tests to exercise ordering
        // without racing worker-startup wall clock. The overdue guard has
        // separate controlled-clock coverage.
        Self::create_harness_with_min_hold(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 100_000,
                    scan_codes: vec![0x15].into(),
                    reason: "deferred-seed".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 500_000,
                    scan_codes: vec![0x15].into(),
                    reason: "deferred-release".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 500_000,
                    scan_codes: vec![0x16, 0x17].into(),
                    reason: "unrelated-chord".into(),
                },
                KeyActionInput {
                    source_action_index: 3,
                    kind: ActionKind::Up,
                    scheduled_us: 1_000_000,
                    scan_codes: vec![0x16, 0x17].into(),
                    reason: "unrelated-cleanup".into(),
                },
            ],
            0,
        )
    }

    /// Build a healthy authored hold for lifecycle tests that explicitly seed
    /// a pending safety release after the Down has been committed.
    pub fn new_admissible_dynamic_pending_release() -> Self {
        Self::create_harness_with_min_hold(
            &[KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: vec![0x15].into(),
                reason: "dynamic-pending-down".into(),
            }],
            0,
        )
    }

    pub fn seed_pending_release_for_test(&mut self, scan_code: u16, due_us: u64) {
        let slot = PHYSICAL_INSTRUMENT_SCAN_CODES
            .iter()
            .position(|candidate| *candidate == scan_code)
            .expect("test scan code is an instrument key") as u8;
        let due_ticks = self
            .resources
            .clock
            .duration_from_us(due_us)
            .expect("pending release due conversion");
        self.resources
            .coordinator
            .seed_pending_release_for_test(slot, TimelineTicks::from_raw(due_ticks.as_u64()))
            .expect("seed pending release");
    }

    /// Build a pending release whose due boundary is shared with an authored
    /// metadata-only deferred Up.  The packet emitter uses deterministic QPC
    /// completion samples so the equality is a state-machine property rather
    /// than a wall-clock coincidence.
    pub fn new_pending_release_with_metadata_boundary() -> Self {
        let mut harness = Self::create_harness_with_min_hold(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 100_000,
                    scan_codes: vec![0x15].into(),
                    reason: "equal-boundary-a-down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 200_000,
                    scan_codes: vec![0x16].into(),
                    reason: "equal-boundary-b-down".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 220_000,
                    scan_codes: vec![0x16].into(),
                    reason: "equal-boundary-b-up".into(),
                },
            ],
            0,
        );
        harness.align_next_plan_to_future_for_test(500_000);
        let epoch = harness.resources.playback.epoch;
        let one_us = harness
            .resources
            .clock
            .duration_from_us(1)
            .expect("one microsecond");
        let base_completion_us = harness
            .resources
            .clock
            .duration_from_us(200_000)
            .expect("base completion boundary");
        let equal_boundary_us = harness
            .resources
            .clock
            .duration_from_us(220_000)
            .expect("equal completion boundary");
        let completion_clock = harness.resources.clock;
        let call_index = Arc::new(AtomicU64::new(0));
        let emitter_index = Arc::clone(&call_index);
        harness.resources.backend.set_packet_emitter(move |packet| {
            let index = emitter_index.fetch_add(1, Ordering::Relaxed);
            let completion_offset = match index {
                0 => base_completion_us,
                1 => base_completion_us
                    .checked_add(one_us)
                    .expect("base completion plus one microsecond"),
                _ => equal_boundary_us,
            };
            let boundary = epoch
                .checked_add_duration(completion_offset)
                .expect("deterministic completion boundary");
            let started = completion_clock.now().expect("completion QPC sample");
            let completed = boundary.max(started);
            let requested_mask = packet.up_mask | packet.down_mask;
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started),
                    completed_ticks: Some(completed),
                    timing_error: None,
                },
            }
        });
        harness
    }

    /// Build a schedule with a trailing stale Up boundary.  The stale batch
    /// is metadata-only and exercises the frozen commit path without relying
    /// on completion-derived hold deferral.
    pub fn new_stale_metadata_boundary() -> Self {
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "metadata-leading-stale".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 100_000,
                scan_codes: vec![0x15].into(),
                reason: "metadata-seed-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 200_000,
                scan_codes: vec![0x15].into(),
                reason: "metadata-seed-up".into(),
            },
            KeyActionInput {
                source_action_index: 3,
                kind: ActionKind::Up,
                scheduled_us: 300_000,
                scan_codes: vec![0x15].into(),
                reason: "metadata-trailing-stale".into(),
            },
        ])
    }

    /// Deterministic coalesced Mixed boundary: a seeded pending A Up and an
    /// authored B Down must be transported by one physical packet.
    pub fn new_coalesced_pending_release_with_unrelated_down() -> Self {
        Self::create_harness_with_min_hold(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "coalesced-a-down".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 21_000,
                    scan_codes: vec![0x16].into(),
                    reason: "coalesced-b-down".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 30_000,
                    scan_codes: vec![0x16].into(),
                    reason: "coalesced-b-up".into(),
                },
            ],
            1_000,
        )
    }

    /// Seed a Down observation while leaving a future Mixed packet for the
    /// planner.  This keeps the observer-drain replan test on the production
    /// authored path instead of testing only the boolean drain helper.
    pub fn new_down_then_mixed() -> Self {
        let mut harness = Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "seed-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: vec![0x15].into(),
                reason: "mixed-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 1_000,
                scan_codes: vec![0x15, 0x16].into(),
                reason: "mixed-down".into(),
            },
        ]);
        let plan = harness.plan_current_dispatch();
        let step = harness.dispatch_at_plan_target_for_test(&plan);
        assert!(
            matches!(step, DispatchStep::Dispatched),
            "initial harness dispatch failed: {step:?}"
        );
        harness
    }

    /// Build a mixed packet with `event_count` physical INPUT events at one
    /// deadline (half Up, half Down). The two directions use disjoint scan
    /// codes because a physical key cannot be requested in both directions.
    pub fn new_mixed_events(event_count: usize) -> Self {
        Self::new_mixed_events_with_gap(event_count, 1_000)
    }

    pub fn new_mixed_events_with_gap(event_count: usize, gap_us: u64) -> Self {
        Self::try_new_mixed_events_with_gap(event_count, gap_us)
            .expect("initial mixed benchmark dispatch")
    }

    pub fn try_new_mixed_events_with_gap(event_count: usize, gap_us: u64) -> Result<Self, String> {
        assert!(
            event_count.is_multiple_of(2) && (2..=15).contains(&event_count),
            "mixed event count must be an even value in 2..=15"
        );
        let key_count = event_count / 2;
        let up_scan_codes = PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count].to_vec();
        let down_scan_codes = PHYSICAL_INSTRUMENT_SCAN_CODES[key_count..event_count].to_vec();
        let mut actions = Vec::with_capacity(1 + event_count);
        actions.push(KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: up_scan_codes.clone().into(),
            reason: "bench-seed".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: gap_us,
            scan_codes: up_scan_codes.into(),
            reason: "bench-up".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: gap_us,
            scan_codes: down_scan_codes.into(),
            reason: "bench-down".into(),
        });
        let mut harness = Self::create_harness(&actions);
        harness.align_next_plan_to_benchmark_margin_for_test(gap_us);
        let plan = harness.plan_current_dispatch();
        let step = harness.dispatch_at_plan_target_for_test(&plan);
        match step {
            DispatchStep::Dispatched => Ok(harness),
            step => Err(format!("initial mixed benchmark step: {step:?}")),
        }
    }

    pub fn new_uponly_release() -> Self {
        Self::new_uponly_release_with_gap(100_000)
    }

    pub fn new_uponly_release_with_gap(gap_us: u64) -> Self {
        Self::new_uponly_release_chord_with_gap(1, gap_us)
    }

    pub fn new_uponly_release_chord_with_gap(key_count: usize, gap_us: u64) -> Self {
        Self::try_new_uponly_release_chord_with_gap(key_count, gap_us)
            .expect("initial UpOnly benchmark dispatch")
    }

    pub fn try_new_uponly_release_chord_with_gap(
        key_count: usize,
        gap_us: u64,
    ) -> Result<Self, String> {
        assert!((1..=15).contains(&key_count), "key count must be 1..=15");
        let scan_codes = PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count].to_vec();
        let mut harness = Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 200_000,
                scan_codes: scan_codes.clone().into(),
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 200_000 + gap_us,
                scan_codes: scan_codes.into(),
                reason: "up".into(),
            },
        ]);
        // Dispatch Down outside window
        harness.align_next_plan_to_benchmark_margin_for_test(gap_us);
        let plan = harness.plan_current_dispatch();
        let step = harness.dispatch_at_plan_target_for_test(&plan);
        match step {
            DispatchStep::Dispatched => Ok(harness),
            step => Err(format!("initial UpOnly benchmark step: {step:?}")),
        }
    }

    fn create_harness(actions: &[KeyActionInput]) -> Self {
        Self::create_harness_with_min_hold(actions, 0)
    }

    fn create_harness_with_min_hold(actions: &[KeyActionInput], min_hold_us: u64) -> Self {
        let mut scan_codes: Vec<u16> = actions
            .iter()
            .flat_map(|action| action.scan_codes.iter().copied())
            .collect();
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(actions, &scan_codes)
            .expect("schedule");
        let qpc_clock = QpcClock::initialize().expect("qpc_clock");
        let min_hold_ticks = qpc_clock
            .duration_from_us(min_hold_us)
            .map(|ticks| DurationTicks::from_raw(ticks.as_u64()))
            .expect("test min-hold conversion");
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            min_hold_us,
            min_hold_ticks,
            |us| {
                qpc_clock
                    .duration_from_us(us)
                    .map(|ticks| TimelineTicks::from_raw(ticks.as_u64()))
                    .map_err(|error| {
                        sky_dispatch_core::coordinator::CoordinatorError::TimeConversion(format!(
                            "{error:?}"
                        ))
                    })
            },
        )
        .expect("coordinator");
        let mut backend = TrackedKeyState::with_qpc_clock(qpc_clock);
        backend.set_test_emitters();
        backend.set_probe(|_, _| InstrumentPhysicalState::AllUp);
        let waiter = HybridWaiter::new();
        let test_margin = qpc_clock
            .duration_from_us(500_000)
            .expect("test epoch margin conversion");
        let test_epoch = qpc_clock
            .now()
            .expect("qpc now")
            .checked_add_duration(test_margin)
            .expect("test epoch margin overflow");
        let playback = PlaybackClockState::new(test_epoch, DurationTicks::ZERO).expect("playback");
        let telemetry = TelemetryCollector::new(TelemetryMode::Ring, 64);
        let scheduling = WorkerSchedulingGuards::create_test_guards();

        let resources = WorkerResources {
            clock: qpc_clock,
            waiter,
            backend,
            coordinator,
            playback,
            telemetry: Arc::new(parking_lot::Mutex::new(telemetry)),
            scheduling,
        };
        let progress_clock = SharedProgressClock::default();
        progress_clock.publish(&resources.playback);

        let health_options = DispatchHealthOptions::default();
        let health = WorkerHealthState::new(health_options);
        let mut timing = WorkerTimingState::create_test_timing();
        timing.down_late_grace_ticks = qpc_clock
            .duration_from_us(500)
            .expect("test down late-grace conversion");
        timing.effective_spin_threshold_ticks = qpc_clock
            .duration_from_us(20_000)
            .expect("test spin threshold conversion");

        Self {
            config: WorkerConfig::default(),
            resources,
            health,
            timing,
            runtime: WorkerRuntime::create_test_runtime(Some(TargetStamp {
                hwnd: 1,
                generation: 0,
            })),
            local_metrics: WorkerMetricsLocal::default(),
            focus_active: AtomicBool::new(true),
            target_hwnd: AtomicIsize::new(1),
            target_generation: AtomicU64::new(0),
            quit_requested: AtomicBool::new(false),
            skip_requested: AtomicBool::new(false),
            panic_requested: AtomicBool::new(false),
            desired_pause: AtomicBool::new(false),
            supervisor_heartbeat_ticks: AtomicU64::new(0),
            metrics: SharedMetrics::default(),
            progress_clock,
            observer: PendingObservationQueue::default(),
            hold_forensics: HoldForensics::default(),
            interrupt: OwnedEvent::new_auto_reset().expect("test interrupt event"),
            last_wait_result: None,
            effective_now_ticks: TimelineTicks::ZERO,
        }
    }

    /// Configure the real HybridWaiter and tick-domain spin threshold used by
    /// the production wait boundary. This is benchmark-only setup; it does
    /// not alter the production worker configuration.
    pub fn configure_wait_policy(
        &mut self,
        waitable_timer_enabled: bool,
        event_wait_enabled: bool,
        spin_threshold_us: u64,
    ) -> Result<(), String> {
        self.resources.waiter =
            HybridWaiter::with_options(waitable_timer_enabled, event_wait_enabled);
        self.timing.effective_spin_threshold_ticks = self
            .resources
            .clock
            .duration_from_us(spin_threshold_us)
            .map_err(|error| format!("benchmark spin threshold conversion: {error:?}"))?;
        Ok(())
    }

    /// Enable the test-only observer profile so timing benchmarks can report
    /// the post-SendInput ready boundary without changing production policy.
    pub fn enable_dispatch_ready_timing_for_benchmark(&mut self) {
        self.config.profile = DispatchProfile::MockTest;
    }

    pub fn last_wait_result(&self) -> Option<WaitResult> {
        self.last_wait_result
    }

    pub fn last_wait_spin_us(&self) -> Result<u64, String> {
        let Some(wait_result) = self.last_wait_result else {
            // A target that was already due takes the production overdue
            // path and performs no blocking wait or precision spin.
            return Ok(0);
        };
        let spin_ticks = wait_result.spin_ticks;
        self.resources
            .clock
            .duration_to_us(spin_ticks)
            .map_err(|error| format!("benchmark spin-duration conversion: {error:?}"))
    }
    /// Advance simulated playback time by `us` microseconds and return effective now ticks.
    pub fn advance_playback_time_us(&mut self, us: u64) -> TimelineTicks {
        let advance_qpc = self.resources.clock.duration_from_us(us).unwrap();
        self.effective_now_ticks = self
            .effective_now_ticks
            .checked_add_duration(advance_qpc)
            .unwrap();
        self.effective_now_ticks
    }
    /// Return the deterministic effective playback time used by test setup.
    pub fn current_effective_time(&self) -> TimelineTicks {
        self.effective_now_ticks
    }
    pub fn set_effective_time_for_test(&mut self, ticks: TimelineTicks) {
        self.effective_now_ticks = ticks;
    }

    /// Test-only clock setup: place the next authored boundary a fixed margin
    /// into the future before the plan is frozen.  This removes harness
    /// startup jitter without changing an already-frozen target.
    pub fn align_next_plan_to_future_for_test(&mut self, margin_us: u64) {
        let authored = self
            .resources
            .coordinator
            .prepare_current_authored_frame()
            .expect("authored frame")
            .map(|frame| frame.authored_ticks);
        let pending = self.resources.coordinator.earliest_pending_release_ticks();
        let Some(deadline) = (match (authored, pending) {
            (Some(authored), Some(pending)) => Some(authored.min(pending)),
            (Some(authored), None) => Some(authored),
            (None, Some(pending)) => Some(pending),
            (None, None) => None,
        }) else {
            return;
        };
        let now = self.resources.clock.now().expect("qpc now");
        let margin = self
            .resources
            .clock
            .duration_from_us(margin_us)
            .expect("test epoch margin conversion");
        // Keep direct frozen-plan sends out of the host's ordinary scheduler
        // jitter window. This changes only the test epoch before planning;
        // it never rewrites a plan after it has been frozen.
        let epoch = now
            .as_u64()
            .saturating_sub(deadline.as_u64())
            .saturating_add(margin.as_u64().max(2_000_000));
        self.resources.playback.epoch = QpcTicks::from_raw(epoch);
    }

    /// Benchmark-only alignment that keeps the same frozen absolute-target
    /// semantics while avoiding the long startup margin used by unit tests.
    pub fn align_next_plan_to_benchmark_margin_for_test(&mut self, margin_us: u64) {
        let authored = self
            .resources
            .coordinator
            .prepare_current_authored_frame()
            .expect("authored frame")
            .map(|frame| frame.authored_ticks);
        let pending = self.resources.coordinator.earliest_pending_release_ticks();
        let Some(deadline) = (match (authored, pending) {
            (Some(authored), Some(pending)) => Some(authored.min(pending)),
            (Some(authored), None) => Some(authored),
            (None, Some(pending)) => Some(pending),
            (None, None) => None,
        }) else {
            return;
        };
        let now = self.resources.clock.now().expect("qpc now");
        let margin = self
            .resources
            .clock
            .duration_from_us(margin_us)
            .expect("benchmark margin conversion");
        let epoch = now
            .as_u64()
            .saturating_sub(deadline.as_u64())
            .saturating_add(margin.as_u64());
        // A zero benchmark margin is the production-boundary mode. Keep the
        // frozen target just inside the Down grace window so the legacy and
        // fused senders sample immediately instead of spending milliseconds
        // spinning on an occasionally future target.
        let epoch = if margin_us == 0 {
            let past_boundary = self
                .resources
                .clock
                .duration_from_us(100)
                .expect("benchmark past-boundary conversion");
            epoch.saturating_sub(past_boundary.as_u64())
        } else {
            epoch
        };
        self.resources.playback.epoch = QpcTicks::from_raw(epoch);
    }

    pub fn set_deadline_wake_for_test(&mut self, ticks: QpcTicks) {
        self.runtime.set_deadline_wake_qpc_for_test(Some(ticks));
    }

    pub fn set_deadline_wake_for_plan_for_test(&mut self, plan: &NextDispatchPlan) {
        let target = plan.physical_target_qpc().expect("physical plan target");
        self.runtime
            .set_deadline_wait_evidence_for_test(Some(target), Some(target));
    }
    /// Query whether coordinator has active generation for `scan_code`.
    pub fn has_active_generation(&self, scan_code: u16) -> bool {
        let slot = match scan_code {
            0x15 => 0,
            0x16 => 1,
            _ => return false,
        };
        (self.resources.coordinator.active_mask & (1 << slot)) != 0
    }

    /// Query coordinator chord integrity lost count.
    pub fn chord_integrity_lost_count(&self) -> u64 {
        self.runtime.chord_integrity_lost_count()
    }

    pub fn backend_active_mask(&self) -> u16 {
        self.resources.backend.active_mask
    }

    pub fn backend_possibly_active_mask(&self) -> u16 {
        self.resources.backend.possibly_active_mask
    }

    /// Exercise the same verified-release/cancel seam used by manual pause
    /// and focus suspension.  The harness keeps this call explicit so tests
    /// cannot accidentally replace the production cleanup path with a direct
    /// coordinator mutation.
    pub fn suspend_live_input_for_test(&mut self) -> Result<Vec<u64>, String> {
        super::super::worker::suspend_live_input(
            &mut self.resources.backend,
            &mut self.resources.coordinator,
            self.target_hwnd.load(Ordering::Acquire),
        )
    }

    /// Number of full-instrument cleanup operations performed by terminal
    /// cleanup. This is a test-support observation of the backend, not a
    /// replacement for the production cleanup call.
    pub fn full_instrument_release_calls(&self) -> u64 {
        self.resources.backend.full_instrument_release_calls
    }

    pub fn configure_send_counter(&mut self) -> Arc<AtomicU64> {
        let calls = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&calls);
        let packet_counter = Arc::clone(&calls);
        let clock = self.resources.clock;
        self.resources
            .backend
            .set_emitter(move |scan_codes, _key_up| {
                counter.fetch_add(1, Ordering::SeqCst);
                let now = clock.now().expect("test QPC");
                PlatformSendResult {
                    requested: scan_codes.len() as u8,
                    inserted: scan_codes.len() as u8,
                    started_ticks: now,
                    completed_ticks: Some(now),
                    win32_error: 0,
                    timing_error: None,
                }
            });
        let packet_clock = self.resources.clock;
        self.resources.backend.set_packet_emitter(move |packet| {
            packet_counter.fetch_add(1, Ordering::SeqCst);
            let now = packet_clock.now().expect("test QPC");
            let requested_mask = packet.up_mask | packet.down_mask;
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(now),
                    completed_ticks: Some(now),
                    timing_error: None,
                },
            }
        });
        calls
    }

    /// Capture the exact directional packet masks presented to the production
    /// packet emitter.  This is intentionally a test-support seam: assertions
    /// can distinguish an authored Down chord from a deferred Up or a
    /// coalesced Mixed transaction without inferring packet identity from a
    /// final coordinator snapshot.
    pub fn configure_packet_capture(&mut self) -> Arc<Mutex<Vec<PhysicalPacket>>> {
        let packets = Arc::new(Mutex::new(Vec::with_capacity(32)));
        let captured = Arc::clone(&packets);
        let clock = self.resources.clock;
        self.resources.backend.set_packet_emitter(move |packet| {
            captured.lock().expect("packet capture lock").push(packet);
            let now = clock.now().expect("test QPC");
            let requested_mask = packet.up_mask | packet.down_mask;
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(now),
                    completed_ticks: Some(now),
                    timing_error: None,
                },
            }
        });
        packets
    }

    pub fn configure_deadline_missed_packet_sender(&mut self) {
        let clock = self.resources.clock;
        self.resources.backend.set_packet_emitter(move |packet| {
            let now = clock.now().expect("test QPC");
            SendTransactionOutcome {
                status: SendTransactionStatus::DeadlineMissedBeforeSend,
                evidence: SendEvidence {
                    requested_mask: packet.up_mask | packet.down_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(now),
                    completed_ticks: None,
                    timing_error: None,
                },
            }
        });
    }
    /// Run production `plan_next_dispatch` for the harness state.
    pub fn plan_current_dispatch(&mut self) -> NextDispatchPlan {
        self.align_epoch_to_selected_boundary_before_planning();
        let mut plan = plan_next_dispatch(
            &self.resources.coordinator,
            self.resources.playback.epoch,
            self.resources.clock,
            &self.config.timing,
            &self.runtime.preparation_probe,
        )
        .expect("plan_next_dispatch");
        preflight_prepared_plan(
            &mut plan,
            &mut self.resources.backend,
            &mut self.runtime,
            &self.target_hwnd,
            &self.target_generation,
        )
        .expect("preflight prepared dispatch plan");
        self.refresh_physical_target_for_test(&mut plan);
        plan
    }

    pub fn plan_current_dispatch_projected(&mut self) -> NextDispatchPlan {
        let mut plan = NextDispatchPlan::default();
        self.plan_current_dispatch_projected_into(&mut plan);
        plan
    }

    /// Build the projected plan directly into caller-owned storage so the
    /// benchmark uses the same ABI shape as the production worker.
    pub fn plan_current_dispatch_projected_into(&mut self, plan: &mut NextDispatchPlan) {
        self.align_epoch_to_selected_boundary_before_planning();
        plan_next_dispatch_projected(
            crate::engine::worker::PlanningInput {
                coordinator: &self.resources.coordinator,
                epoch_qpc: self.resources.playback.epoch,
                preparation_probe: &self.runtime.preparation_probe,
            },
            plan,
        )
        .expect("projected dispatch plan");
        preflight_prepared_plan(
            plan,
            &mut self.resources.backend,
            &mut self.runtime,
            &self.target_hwnd,
            &self.target_generation,
        )
        .expect("preflight prepared projected plan");
        self.refresh_physical_target_for_test(plan);
    }

    pub fn preparation_counts(&self) -> PreparationCounts {
        self.runtime.preparation_probe.counts()
    }

    pub fn reset_preparation_counts_for_test(&mut self) {
        self.runtime.preparation_probe.reset();
    }

    pub fn set_force_preflight_failure(&mut self, flag: Arc<AtomicBool>) {
        self.resources.backend.set_force_preflight_failure(flag);
    }

    /// Inject a deterministic mutation immediately after worker target
    /// crossing and before the final control/target/focus gate. This is a
    /// runtime integration seam, not a production synchronization path.
    pub fn set_final_gate_race_hook<F>(&mut self, hook: F)
    where
        F: Fn(
                &AtomicBool,
                &AtomicIsize,
                &AtomicU64,
                &AtomicBool,
                &AtomicBool,
                &AtomicBool,
                &AtomicBool,
            ) + Send
            + Sync
            + 'static,
    {
        self.runtime.final_gate_race_hook = Some(Arc::new(hook));
    }

    /// Inject a deterministic mutation after fresh foreground proof and
    /// before the cheap final atomic revalidation. This is test-only evidence
    /// for the post-focus TOCTOU seam.
    pub fn set_post_focus_revalidation_race_hook<F>(&mut self, hook: F)
    where
        F: Fn(
                &AtomicBool,
                &AtomicIsize,
                &AtomicU64,
                &AtomicBool,
                &AtomicBool,
                &AtomicBool,
                &AtomicBool,
            ) + Send
            + Sync
            + 'static,
    {
        self.runtime.final_gate_post_focus_race_hook = Some(Arc::new(hook));
    }

    /// Run the production wait boundary and direct frozen-plan dispatch path.
    pub fn wait_and_dispatch_current_plan(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> Result<DispatchStep, String> {
        let boundary = wait_for_next_boundary(WaitBoundaryInput {
            deadline: WaitDeadline {
                physical_target_qpc: plan.physical_target_qpc(),
                spin_threshold_ticks: if matches!(plan, NextDispatchPlan::Physical(_)) {
                    self.timing.effective_spin_threshold_ticks
                } else {
                    DurationTicks::ZERO
                },
                qpc_clock: self.resources.clock,
            },
            timing: WaitTiming {
                lease_timeout_ticks: self.timing.lease_timeout_ticks,
                supervisor_heartbeat_ticks: &self.supervisor_heartbeat_ticks,
            },
            signals: WaitSignals {
                waiter: &self.resources.waiter,
                interrupt: &self.interrupt,
            },
            mutable: WaitMutable {
                local_metrics: &mut self.local_metrics,
                force_full_cleanup: &mut self.runtime.force_full_cleanup,
                terminal_error: &mut self.runtime.terminal_error,
            },
        });
        let (wait_result, dispatch_qpc) = match boundary {
            WaitBoundary::Due {
                wait_result: Some(wait_result),
                dispatch_qpc,
                ..
            } => {
                self.runtime.set_deadline_wait_evidence_for_test(
                    Some(dispatch_qpc),
                    plan.physical_target_qpc(),
                );
                (Some(wait_result), dispatch_qpc)
            }
            WaitBoundary::Due {
                wait_result: None,
                dispatch_qpc,
                ..
            } => (None, dispatch_qpc),
            WaitBoundary::Replan { .. } => {
                return Err("benchmark wait unexpectedly required replan".to_string());
            }
            WaitBoundary::Exit => {
                return Err(self
                    .runtime
                    .terminal_error
                    .take()
                    .unwrap_or_else(|| "benchmark wait exited".to_string()));
            }
        };
        self.last_wait_result = wait_result;
        self.runtime.set_deadline_wait_evidence_for_test(
            wait_result.and_then(|result| result.wake_qpc),
            plan.physical_target_qpc(),
        );
        let now_ticks = wait_result
            .and_then(|result| result.wake_qpc)
            .unwrap_or(dispatch_qpc);
        let effective_now_ticks = self
            .resources
            .playback
            .get_elapsed_allow_pre_epoch(now_ticks, true)
            .map_err(|error| format!("benchmark timeline: {error}"))?;
        self.effective_now_ticks = effective_now_ticks;
        let step = self.dispatch_plan_at_with_sender_option(
            plan,
            effective_now_ticks,
            now_ticks,
            true,
            Some(now_ticks),
            None,
            false,
        );
        Ok(step)
    }
    fn align_epoch_to_deadline_for_test(&mut self, deadline: TimelineTicks, now_ticks: QpcTicks) {
        if self.runtime.down_boundary_state.awaiting_future() {
            return;
        }
        let current_target = self
            .resources
            .playback
            .epoch
            .checked_add_duration(DurationTicks::from_raw(deadline.as_u64()))
            .expect("test target arithmetic");
        if current_target > now_ticks {
            return;
        }
        // Direct harness dispatch must exercise the same pre-deadline
        // admission window as the worker. Keep the frozen target at least
        // two seconds in the future; this is test-only epoch setup for a
        // non-controlled QPC host. Tests that need an overdue boundary set
        // that condition explicitly through their dedicated race helper.
        let margin = self
            .resources
            .clock
            .duration_from_us(500_000)
            .expect("test dispatch margin conversion");
        let raw = now_ticks
            .as_u64()
            .saturating_sub(deadline.as_u64())
            .checked_add(margin.as_u64().max(2_000_000))
            .expect("test epoch margin overflow");
        self.resources.playback.epoch = QpcTicks::from_raw(raw);
    }
    fn align_epoch_to_selected_boundary_before_planning(&mut self) {
        let authored = self
            .resources
            .coordinator
            .prepare_current_authored_frame()
            .expect("authored frame")
            .map(|frame| frame.authored_ticks);
        let pending = self.resources.coordinator.earliest_pending_release_ticks();
        let selected_deadline = match (authored, pending) {
            (Some(authored), Some(pending)) => Some(authored.min(pending)),
            (Some(authored), None) => Some(authored),
            (None, Some(pending)) => Some(pending),
            (None, None) => None,
        }
        .filter(|deadline| self.effective_now_ticks >= *deadline);
        if let Some(deadline) = selected_deadline {
            self.align_epoch_to_deadline_for_test(
                deadline,
                self.resources.clock.now().expect("qpc now"),
            );
        }
    }
    fn refresh_physical_target_for_test(&self, _plan: &mut NextDispatchPlan) {
        // The production plan owns one frozen absolute QPC target. Tests may
        // align the synthetic epoch, but must not reconstruct that target.
    }
    fn dispatch_plan_at(
        &mut self,
        plan: &NextDispatchPlan,
        effective_now_ticks: TimelineTicks,
        now_ticks: QpcTicks,
        allow_pre_deadline: bool,
        test_physical_target_qpc: Option<QpcTicks>,
    ) -> DispatchStep {
        self.dispatch_plan_at_with_sender_option(
            plan,
            effective_now_ticks,
            now_ticks,
            allow_pre_deadline,
            None,
            test_physical_target_qpc,
            test_physical_target_qpc.is_some(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_plan_at_with_sender_option(
        &mut self,
        plan: &NextDispatchPlan,
        effective_now_ticks: TimelineTicks,
        now_ticks: QpcTicks,
        allow_pre_deadline: bool,
        boundary_crossing_qpc: Option<QpcTicks>,
        test_physical_target_qpc: Option<QpcTicks>,
        test_inject_sender_start: bool,
    ) -> DispatchStep {
        self.effective_now_ticks = effective_now_ticks;
        dispatch_due_from_plan(
            plan,
            effective_now_ticks,
            now_ticks,
            false,
            &self.config,
            &mut self.resources,
            &mut self.health,
            &self.timing,
            &mut self.runtime,
            &mut self.local_metrics,
            &self.focus_active,
            &self.target_hwnd,
            &self.target_generation,
            &self.quit_requested,
            &self.skip_requested,
            &self.panic_requested,
            &self.desired_pause,
            &self.supervisor_heartbeat_ticks,
            self.timing.lease_timeout_ticks,
            &self.progress_clock,
            Some(&self.observer),
            boundary_crossing_qpc,
            allow_pre_deadline,
            test_physical_target_qpc,
            test_inject_sender_start,
        )
    }
    /// Invoke the production frozen-plan helper without the kernel wait.
    /// Tests use this to cover all structural plan states directly.
    pub fn dispatch_due_from_plan_for_test(&mut self, plan: &NextDispatchPlan) -> DispatchStep {
        let now_ticks = self.resources.clock.now().expect("qpc now");
        let allow_pre_deadline = !self.runtime.down_boundary_state.awaiting_future();
        let boundary_crossing_qpc = if allow_pre_deadline {
            // The old direct test seam performed the now-removed inner wait;
            // represent its completed crossing with the frozen target.
            plan.physical_target_qpc()
        } else {
            plan.physical_target_qpc()
                .filter(|target| *target <= now_ticks)
                .map(|_| now_ticks)
        };
        let dispatch_now_ticks = boundary_crossing_qpc.unwrap_or(now_ticks);
        self.dispatch_plan_at_with_sender_option(
            plan,
            self.effective_now_ticks,
            dispatch_now_ticks,
            allow_pre_deadline,
            boundary_crossing_qpc,
            None,
            boundary_crossing_qpc.is_some(),
        )
    }

    /// Invoke a frozen plan at its exact synthetic QPC boundary.  This keeps
    /// metadata-only commit allocation tests independent of waiter setup and
    /// wall-clock timer state.
    pub fn dispatch_at_plan_target_for_test(&mut self, plan: &NextDispatchPlan) -> DispatchStep {
        let target = plan
            .physical_target_qpc()
            .expect("plan target required for synthetic boundary");
        let deadline = plan
            .deadline_ticks()
            .expect("plan deadline required for synthetic boundary");
        self.runtime
            .set_deadline_wait_evidence_for_test(Some(target), Some(target));
        // Use the frozen target as a test-controlled exact-boundary sample;
        // this never re-anchors or rewrites the plan after it is frozen.
        self.dispatch_plan_at(plan, deadline, target, true, Some(target))
    }

    pub fn physical_target_qpc_for_test(&self, plan: &NextDispatchPlan) -> Option<QpcTicks> {
        plan.physical_target_qpc()
    }

    pub fn send_phase_a_packet_for_test(
        &mut self,
        packet: PhysicalPacket,
    ) -> (QpcTicks, SendTransactionOutcome) {
        let prepared = PreparedPhysicalPacket::try_new(packet).expect("prepared benchmark packet");
        self.send_prepared_phase_a_packet_for_test(&prepared)
    }

    pub fn send_prepared_phase_a_packet_for_test(
        &mut self,
        prepared: &PreparedPhysicalPacket,
    ) -> (QpcTicks, SendTransactionOutcome) {
        let packet = prepared.packet();
        let target = self.resources.clock.now().expect("benchmark sender QPC");
        let latest_allowed_down_qpc = (packet.down_mask != 0).then(|| {
            target
                .checked_add_duration(self.timing.down_late_grace_ticks)
                .expect("benchmark Down cutoff")
        });
        let outcome = self.resources.backend.send_phase_a_benchmark_boundary(
            prepared,
            self.resources.clock,
            target,
            latest_allowed_down_qpc,
            target,
        );
        (target, outcome)
    }

    /// Invoke the frozen Phase-A sender boundary with a deterministic first
    /// sample one QPC tick after the target and a deterministic completion
    /// delay. This keeps baseline and candidate A/B runs on the same sender
    /// path without waiting on wall-clock time.
    pub fn dispatch_at_phase_a_benchmark_boundary_for_test(
        &mut self,
        plan: &NextDispatchPlan,
        completion_delay_us: u64,
    ) -> DispatchStep {
        let target = plan
            .physical_target_qpc()
            .expect("plan target required for Phase-A benchmark boundary");
        let benchmark_now = target
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("Phase-A benchmark boundary arithmetic");
        let deadline = plan
            .deadline_ticks()
            .expect("plan deadline required for Phase-A benchmark boundary");
        let completion_delay = self
            .resources
            .clock
            .duration_from_us(completion_delay_us)
            .expect("Phase-A benchmark completion delay conversion");
        let completed_ticks = benchmark_now
            .checked_add_duration(completion_delay)
            .expect("Phase-A benchmark completion boundary arithmetic");
        self.resources.backend.set_packet_emitter(move |packet| {
            let requested_mask = packet.up_mask | packet.down_mask;
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(benchmark_now),
                    completed_ticks: Some(completed_ticks),
                    timing_error: None,
                },
            }
        });
        self.runtime
            .set_deadline_wait_evidence_for_test(Some(target), Some(target));
        self.dispatch_plan_at(plan, deadline, benchmark_now, true, Some(target))
    }

    /// Invoke the coordinator dispatch boundary at a frozen crossing. The
    /// test transport records its own immediate QPC sample for sender timing;
    /// production builds take that sample inside the native sender.
    pub fn dispatch_at_phase_a_production_boundary_for_test(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> DispatchStep {
        let target = plan
            .physical_target_qpc()
            .expect("plan target required for Phase-A production boundary");
        let deadline = plan
            .deadline_ticks()
            .expect("plan deadline required for Phase-A production boundary");
        let clock = self.resources.clock;
        self.resources.backend.set_packet_emitter(move |packet| {
            let requested_mask = packet.up_mask | packet.down_mask;
            let started_ticks = clock.now().expect("production-boundary pre-call QPC");
            let completed_ticks = clock.now().expect("production-boundary completion QPC");
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask: 0,
                    first_inserted: packet.event_count(),
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks: Some(completed_ticks),
                    timing_error: None,
                },
            }
        });
        self.runtime
            .set_deadline_wait_evidence_for_test(Some(target), Some(target));
        self.dispatch_plan_at_with_sender_option(
            plan,
            deadline,
            target,
            true,
            None,
            Some(target),
            false,
        )
    }

    /// Inject the exact waiter-entry race for a still-frozen physical plan:
    /// it was future when classified, the worker stalled before the waiter's
    /// first QPC read, and the waiter therefore returned `Due(None)` at an
    /// already-overdue target.  No wall-clock sleep or replan is involved.
    pub fn dispatch_same_frozen_plan_after_due_without_wait_for_test(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> DispatchStep {
        let target = plan
            .physical_target_qpc()
            .expect("waiter-entry race requires a physical target");
        let overdue_now = target
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("overdue test target arithmetic");
        self.runtime.record_due_without_wait_for_test();
        self.dispatch_plan_at_with_sender_option(
            plan,
            plan.deadline_ticks().expect("physical deadline"),
            overdue_now,
            false,
            None,
            None,
            true,
        )
    }

    /// Inject a known backlog while strict diagnostic mode also observes a
    /// lateness greater than the Down grace.  The boundary must retain the
    /// Backlog classification; strict mode changes the terminal behavior,
    /// not the reason that the future authorization was missed.
    pub fn dispatch_known_backlog_with_strict_lateness_for_test(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> DispatchStep {
        let view = plan
            .physical()
            .expect("strict backlog test requires a physical plan");
        let target = plan
            .physical_target_qpc()
            .expect("strict backlog test requires a physical target");
        let lateness = self
            .timing
            .down_late_grace_ticks
            .checked_add(DurationTicks::from_raw(1))
            .expect("strict backlog lateness arithmetic");
        let effective_now_ticks = TimelineTicks::from_raw(
            view.authored_view
                .authored_batch_scheduled_ticks
                .as_u64()
                .saturating_add(lateness.as_u64()),
        );
        let overdue_now = target
            .checked_add_duration(lateness)
            .expect("strict backlog QPC lateness arithmetic");
        self.config.timing.strict_timing = true;
        self.runtime.record_due_without_wait_for_test();
        self.dispatch_plan_at_with_sender_option(
            plan,
            effective_now_ticks,
            overdue_now,
            false,
            None,
            None,
            false,
        )
    }

    /// Inject an authorized boundary whose trusted pre-call sample is beyond
    /// the session Down late-grace cutoff. The sender must make zero Down calls;
    /// Production recovery then commits the boundary as missed.
    pub fn dispatch_same_frozen_plan_after_hard_late_for_test(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> DispatchStep {
        self.dispatch_same_frozen_plan_at_lateness_for_test(
            plan,
            self.timing
                .down_late_grace_ticks
                .checked_add(DurationTicks::from_raw(1))
                .expect("hard-late test lateness arithmetic"),
        )
    }

    /// Inject an authorized boundary with an exact deterministic pre-call
    /// lateness measured in QPC ticks. The sender must preserve equality at
    /// the grace cutoff and reject the first tick beyond it.
    pub fn dispatch_same_frozen_plan_at_lateness_for_test(
        &mut self,
        plan: &NextDispatchPlan,
        lateness: DurationTicks,
    ) -> DispatchStep {
        let target = plan
            .physical_target_qpc()
            .expect("hard-late race requires a physical target");
        let overdue_now = target
            .checked_add_duration(lateness)
            .expect("hard-late test target arithmetic");
        self.runtime.set_deadline_wait_evidence_for_test(None, None);
        self.dispatch_plan_at(
            plan,
            plan.deadline_ticks().expect("physical deadline"),
            overdue_now,
            false,
            Some(target),
        )
    }

    pub fn dispatch_with_strict_admission_late_for_test(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> DispatchStep {
        let view = plan
            .physical()
            .expect("strict admission requires physical plan");
        let target = plan
            .physical_target_qpc()
            .expect("strict admission requires physical target");
        let late = self
            .timing
            .down_late_grace_ticks
            .checked_add(DurationTicks::from_raw(1))
            .expect("strict admission lateness arithmetic");
        let effective_now_ticks = TimelineTicks::from_raw(
            view.authored_view
                .authored_batch_scheduled_ticks
                .as_u64()
                .saturating_add(late.as_u64()),
        );
        let now_ticks = target
            .checked_add_duration(late)
            .expect("strict admission QPC lateness arithmetic");
        self.config.timing.strict_timing = true;
        self.dispatch_plan_at_with_sender_option(
            plan,
            effective_now_ticks,
            now_ticks,
            false,
            None,
            Some(target),
            false,
        )
    }
    /// Query the current authored packet path without mutating state.
    pub fn current_authored_path(&self) -> Option<DispatchPath> {
        let (up_mask, down_mask) = self.resources.coordinator.next_authored_packet_masks()?;
        let up_count = up_mask.count_ones() as usize;
        let down_count = down_mask.count_ones() as usize;
        match physical_packet_kind(up_mask, down_mask) {
            Ok(PhysicalPacketKind::UpOnly) => Some(DispatchPath::UpOnly { up_count }),
            Ok(PhysicalPacketKind::DownOnly) => Some(DispatchPath::DownOnly { down_count }),
            Ok(PhysicalPacketKind::Mixed) => Some(DispatchPath::Mixed {
                up_count,
                down_count,
            }),
            Err(_) => None,
        }
    }
    /// Dispatch authored packet using an explicit production `NextDispatchPlan`.
    pub fn dispatch_authored_with_plan(&mut self, plan: &NextDispatchPlan) -> DispatchStep {
        self.dispatch_authored_with_plan_and_lease(plan, DurationTicks::ZERO)
    }
    pub fn dispatch_authored_with_plan_and_lease(
        &mut self,
        plan: &NextDispatchPlan,
        lease_timeout_ticks: DurationTicks,
    ) -> DispatchStep {
        let physical_target_qpc = plan.physical_target_qpc().expect("physical target QPC");
        // This direct helper represents the old inner wait with a synthetic
        // exact-boundary sample. The production worker supplies the real
        // crossing from its single wait before entering this function.
        let now_ticks = physical_target_qpc;
        let ctx = AuthoredPacketContext {
            dispatch_plan: plan,
            effective_now_ticks: self.effective_now_ticks,
            now_ticks,
            physical_target_qpc,
            down_admission: DownBoundaryAdmission::Normal,
            focus_loss_fault: false,
            supervisor_heartbeat_ticks: &self.supervisor_heartbeat_ticks,
            lease_timeout_ticks,
            // This helper dispatches a frozen plan at its synthetic exact
            // boundary; the worker loop normally supplies this sample from
            // its single wait.
            boundary_crossing_qpc: Some(physical_target_qpc),
            test_direct_boundary: false,
            test_inject_sender_start: true,
        };
        dispatch_authored_packet(
            ctx,
            &self.config,
            &mut self.resources,
            &mut self.health,
            &self.timing,
            &mut self.runtime,
            &mut self.local_metrics,
            &self.focus_active,
            &self.target_hwnd,
            &self.target_generation,
            &self.quit_requested,
            &self.skip_requested,
            &self.panic_requested,
            &self.desired_pause,
            &self.progress_clock,
            Some(&self.observer),
        )
    }
    pub fn drain_observer(&mut self) -> Result<Option<u64>, DispatchStep> {
        super::super::worker::drain_one_observer(
            &mut self.observer,
            &mut self.health,
            &mut self.local_metrics,
            &self.metrics,
            &mut self.resources.telemetry.lock(),
            self.resources.clock,
            sky_dispatch_win32::clock::QpcTicks::ZERO,
            &mut self.timing,
            &mut self.hold_forensics,
        )
    }
    pub fn pop_observation(
        &mut self,
    ) -> Option<super::super::worker::dispatch::DispatchObservation> {
        self.observer.pop_front()
    }
    pub fn pending_observation_count(&self) -> usize {
        self.observer.len()
    }
}
