#![cfg(any(test, feature = "test-support"))]

//! Test harness for production dispatch paths (DownOnly, Mixed, UpOnly release).
//!
//! Provides `ProductionDispatchTestHarness` for deterministic zero-allocation
//! verification of production dispatch functions.

use crate::engine::config::WorkerConfig;
use crate::engine::telemetry::{
    SharedMetrics, TelemetryCollector, TelemetryMode, WorkerMetricsLocal,
};
use crate::engine::worker::dispatch::PendingObservationQueue;
use crate::engine::worker::dispatch::{
    AuthoredPacketContext, DispatchStep, PendingReleaseContext, dispatch_authored_packet,
    dispatch_due_pending_releases,
};
use crate::engine::worker::{
    DispatchHealthOptions, DispatchPath, FrozenDispatchBudget, NextDispatchPlan, TargetStamp,
    WorkerHealthState, WorkerResources, WorkerRuntime, WorkerSchedulingGuards, WorkerTimingState,
    plan_next_dispatch,
};
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{
    PendingDispatchPlan, PendingRelease, RuntimeDispatchCoordinator, physical_packet_kind,
};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, PhysicalPacketKind};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::input::{PlatformSendResult, TrackedKeyState};
use sky_dispatch_win32::wait::HybridWaiter;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

pub struct ProductionDispatchTestHarness {
    pub(crate) config: WorkerConfig,
    pub(crate) resources: WorkerResources,
    pub(crate) health: WorkerHealthState,
    pub(crate) timing: WorkerTimingState,
    pub(crate) runtime: WorkerRuntime,
    pub(crate) local_metrics: WorkerMetricsLocal,
    pub(crate) last_published_error: Option<String>,
    pub(crate) secondary_errors: Vec<String>,
    pub(crate) focus_active: AtomicBool,
    pub(crate) target_hwnd: AtomicIsize,
    pub(crate) target_generation: AtomicU64,
    pub(crate) quit_requested: AtomicBool,
    pub(crate) skip_requested: AtomicBool,
    pub(crate) panic_requested: AtomicBool,
    pub(crate) desired_pause: AtomicBool,
    pub(crate) supervisor_heartbeat_ticks: AtomicU64,
    pub(crate) pending_budget: FrozenDispatchBudget,
    pub(crate) metrics: SharedMetrics,
    pub(crate) observer: PendingObservationQueue,
    effective_now_ticks: TimelineTicks,
}

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

    /// Build a DownOnly chord whose physical deadline is at 10 ms.
    pub fn new_down_chord(key_count: usize) -> Self {
        assert!((1..=15).contains(&key_count), "key count must be 1..=15");
        let scan_codes: Vec<u16> = (0..key_count)
            .map(|index| 0x15u16.saturating_add(index as u16))
            .collect();
        Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 10_000,
                scan_codes: scan_codes.clone().into(),
                reason: "bench-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
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

    /// Build a retrigger packet with `event_count` physical INPUT events at
    /// one deadline (half Up, half Down). Initial owners are dispatched during
    /// setup so the measured packet is genuinely Mixed.
    pub fn new_mixed_events(event_count: usize) -> Self {
        assert!(
            event_count.is_multiple_of(2) && (2..=30).contains(&event_count),
            "mixed event count must be an even value in 2..=30"
        );
        let key_count = event_count / 2;
        let scan_codes: Vec<u16> = (0..key_count)
            .map(|index| 0x15u16.saturating_add(index as u16))
            .collect();
        let mut actions = Vec::with_capacity(1 + event_count);
        actions.push(KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: scan_codes.clone().into(),
            reason: "bench-seed".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 10_000,
            scan_codes: scan_codes.clone().into(),
            reason: "bench-up".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: scan_codes.into(),
            reason: "bench-down".into(),
        });
        let mut harness = Self::create_harness(&actions);
        let plan = harness.plan_current_dispatch();
        assert!(matches!(
            harness.dispatch_authored_with_plan(&plan),
            DispatchStep::Dispatched
        ));
        harness
    }

    pub fn new_uponly_release() -> Self {
        Self::new_uponly_release_with_gap(1_000)
    }

    pub fn new_uponly_release_with_gap(gap_us: u64) -> Self {
        let mut harness = Self::create_harness(&[
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
                scheduled_us: gap_us,
                scan_codes: vec![0x15].into(),
                reason: "up".into(),
            },
        ]);
        // Dispatch Down outside window
        let plan = harness.plan_current_dispatch();
        harness.dispatch_authored_with_plan(&plan);
        harness
    }

    fn create_harness(actions: &[KeyActionInput]) -> Self {
        let mut scan_codes: Vec<u16> = actions
            .iter()
            .flat_map(|action| action.scan_codes.iter().copied())
            .collect();
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(actions, &scan_codes)
            .expect("schedule");
        let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            0,
            DurationTicks::ZERO,
            0,
            DurationTicks::ZERO,
            |us| Ok(TimelineTicks::from_raw(us)),
        )
        .expect("coordinator");
        let qpc_clock = QpcClock::initialize().expect("qpc_clock");
        let mut backend = TrackedKeyState::with_qpc_clock(qpc_clock);
        backend.set_test_emitters();
        let waiter = HybridWaiter::new();
        let playback =
            PlaybackClockState::new(qpc_clock.now().expect("qpc now"), DurationTicks::ZERO)
                .expect("playback");
        let estimator = SendLatencyEstimator::default();
        let telemetry = TelemetryCollector::new(TelemetryMode::Ring, 64);
        let scheduling = WorkerSchedulingGuards::create_test_guards();

        let resources = WorkerResources {
            clock: qpc_clock,
            waiter,
            backend,
            coordinator,
            playback,
            estimator,
            telemetry,
            scheduling,
        };

        let health_options = DispatchHealthOptions::default();
        let health = WorkerHealthState::new(health_options);
        let timing = WorkerTimingState::create_test_timing();

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
            last_published_error: None,
            secondary_errors: Vec::new(),
            focus_active: AtomicBool::new(true),
            target_hwnd: AtomicIsize::new(1),
            target_generation: AtomicU64::new(0),
            quit_requested: AtomicBool::new(false),
            skip_requested: AtomicBool::new(false),
            panic_requested: AtomicBool::new(false),
            desired_pause: AtomicBool::new(false),
            supervisor_heartbeat_ticks: AtomicU64::new(0),
            pending_budget: crate::engine::worker::health::build_dispatch_budget(
                &SendLatencyEstimator::default(),
                DispatchPath::UpOnly { up_count: 1 },
                LatencyClass::Hot,
                health_options,
                false,
            ),
            metrics: SharedMetrics::default(),
            observer: PendingObservationQueue::default(),
            effective_now_ticks: TimelineTicks::ZERO,
        }
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

    /// Number of full-instrument cleanup operations performed by production
    /// release recovery. This is a test-support observation of the backend,
    /// not a replacement for the production cleanup call.
    pub fn full_instrument_release_calls(&self) -> u64 {
        self.resources.backend.full_instrument_release_calls
    }

    /// Make every production pending-release send fail while retaining a
    /// real sender seam. The counter is incremented inside the emitter that
    /// production invokes, so tests can distinguish physical cleanup from a
    /// helper-only assertion.
    pub fn configure_persistent_release_failure(&mut self, calls: Arc<AtomicU64>) {
        let clock = self.resources.clock;
        self.resources
            .backend
            .set_emitter(move |scan_codes, key_up| {
                let now = clock.now().expect("test QPC");
                if key_up {
                    calls.fetch_add(1, Ordering::SeqCst);
                    PlatformSendResult {
                        requested: scan_codes.len() as u8,
                        inserted: 0,
                        started_ticks: now,
                        completed_ticks: Some(now),
                        win32_error: 1460,
                        timing_error: None,
                    }
                } else {
                    PlatformSendResult {
                        requested: scan_codes.len() as u8,
                        inserted: scan_codes.len() as u8,
                        started_ticks: now,
                        completed_ticks: Some(now),
                        win32_error: 0,
                        timing_error: None,
                    }
                }
            });
    }

    /// Pop due pending releases using plan.
    pub fn pop_due_pending_for_plan(
        &mut self,
        effective_now: TimelineTicks,
        plan: &NextDispatchPlan,
    ) -> SmallVec<[PendingRelease; 15]> {
        let pending = plan.pending().expect("pending release plan");
        self.resources
            .coordinator
            .pop_due_pending_ticks(effective_now, pending)
            .expect("pop due pending")
    }

    /// Create a real coordinator-owned pending release during test setup.
    /// The measurement must exercise `plan_next_dispatch` and
    /// `dispatch_due_pending_releases`, not the authored Up packet path.
    pub fn seed_pending_release_for_test(&mut self) {
        let prepared = self
            .resources
            .coordinator
            .prepare_next_due_authored(self.effective_now_ticks, DurationTicks::ZERO)
            .expect("prepare pending-release request")
            .expect("authored release request");
        self.resources
            .coordinator
            .commit_up_request(prepared)
            .expect("commit pending-release request");
    }

    /// Run production `plan_next_dispatch` for the harness state.
    pub fn plan_current_dispatch(&mut self) -> NextDispatchPlan {
        self.plan_current_dispatch_class(LatencyClass::Hot)
    }

    pub fn plan_current_dispatch_class(&mut self, latency_class: LatencyClass) -> NextDispatchPlan {
        plan_next_dispatch(
            &self.resources.coordinator,
            &self.resources.estimator,
            self.resources.clock,
            latency_class,
            &self.config.timing,
            self.config.estimator.enable_adaptive_lead,
        )
        .expect("plan_next_dispatch")
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
            Err(_) => Some(DispatchPath::DownOnly {
                down_count: self.resources.coordinator.next_authored_polyphony().max(1),
            }),
        }
    }

    /// Dispatch authored packet using an explicit production `NextDispatchPlan`.
    pub fn dispatch_authored_with_plan(&mut self, plan: &NextDispatchPlan) -> DispatchStep {
        let now_ticks = self.resources.clock.now().expect("qpc now");
        let ctx = AuthoredPacketContext {
            dispatch_plan: plan,
            effective_now_ticks: self.effective_now_ticks,
            now_ticks,
            latency_class: plan.latency_class(),
            focus_loss_fault: false,
            supervisor_heartbeat_ticks: &self.supervisor_heartbeat_ticks,
            lease_timeout_ticks: DurationTicks::ZERO,
        };
        dispatch_authored_packet(
            ctx,
            &self.config,
            &mut self.resources,
            &mut self.health,
            &self.timing,
            &mut self.runtime,
            &mut self.local_metrics,
            &mut self.last_published_error,
            &self.focus_active,
            &self.target_hwnd,
            &self.target_generation,
            &self.quit_requested,
            &self.skip_requested,
            &self.panic_requested,
            &self.desired_pause,
            &self.metrics,
            &mut self.observer,
        )
    }

    /// Dispatch pending releases using explicit `due_pending` and `pending_plan`.
    pub fn dispatch_pending_release_with_plan(
        &mut self,
        due_pending: SmallVec<[PendingRelease; 15]>,
        pending_plan: Option<&PendingDispatchPlan>,
        latency_class: LatencyClass,
    ) -> DispatchStep {
        let lead_up_ticks = pending_plan.map_or(DurationTicks::ZERO, |p| p.lead_ticks);
        let ctx = PendingReleaseContext {
            due_pending,
            pending_plan,
            lead_up_ticks,
            latency_class,
            frozen_budget: self.pending_budget,
            quit_requested: &self.quit_requested,
            skip_requested: &self.skip_requested,
            panic_requested: &self.panic_requested,
            desired_pause: &self.desired_pause,
            supervisor_heartbeat_ticks: &self.supervisor_heartbeat_ticks,
            lease_timeout_ticks: DurationTicks::ZERO,
            observer: &mut self.observer,
        };
        dispatch_due_pending_releases(
            ctx,
            &self.config,
            &mut self.resources,
            &mut self.health,
            &self.timing,
            &mut self.runtime,
            &mut self.local_metrics,
            &mut self.secondary_errors,
            &self.target_hwnd,
        )
    }

    pub fn drain_observer(&mut self) -> Result<u64, DispatchStep> {
        super::super::worker::drain_one_observer(
            &mut self.observer,
            &self.config,
            &mut self.health,
            &mut self.local_metrics,
            &mut self.last_published_error,
            &self.metrics,
            &mut self.resources.backend,
            &mut self.resources.estimator,
            &mut self.resources.telemetry,
            self.resources.clock,
            0,
            &mut self.timing,
        )
    }

    pub fn pop_observation(
        &mut self,
    ) -> Option<super::super::worker::dispatch::DispatchObservation> {
        self.observer.pop_front()
    }
}
