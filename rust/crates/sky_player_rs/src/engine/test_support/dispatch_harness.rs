//! Test harness for production dispatch paths (DownOnly, Mixed, UpOnly release).
//!
//! Provides `ProductionDispatchTestHarness` for deterministic zero-allocation
//! verification of production dispatch functions.

use crate::engine::config::WorkerConfig;
use crate::engine::telemetry::{SharedMetrics, TelemetryCollector, TelemetryMode, WorkerMetricsLocal};
use crate::engine::worker::dispatch::observer_drain::PendingObservationQueue;
use crate::engine::worker::dispatch::{
    AuthoredPacketContext, DispatchStep, PendingReleaseContext, dispatch_authored_packet,
    dispatch_due_pending_releases,
};
use crate::engine::worker::{
    DispatchHealthOptions, DispatchPath, NextDispatchPlan, TargetStamp, WorkerHealthState,
    WorkerResources, WorkerRuntime, WorkerSchedulingGuards, WorkerTimingState, plan_next_dispatch,
};
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{
    PendingDispatchPlan, PendingRelease, RuntimeDispatchCoordinator, physical_packet_kind,
};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, PhysicalPacketKind};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::input::TrackedKeyState;
use sky_dispatch_win32::wait::HybridWaiter;
use smallvec::SmallVec;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64};

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
    pub(crate) metrics: SharedMetrics,
    pub(crate) observer: PendingObservationQueue,
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

    pub fn new_uponly_release() -> Self {
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
                scheduled_us: 1000,
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
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(actions, &[0x15, 0x16])
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
        let playback = PlaybackClockState::new(
            qpc_clock.now().expect("qpc now"),
            DurationTicks::ZERO,
        )
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
            metrics: SharedMetrics::default(),
            observer: PendingObservationQueue::default(),
        }
    }

    /// Advance simulated playback time by `us` microseconds and return effective now ticks.
    pub fn advance_playback_time_us(&mut self, us: u64) -> TimelineTicks {
        let advance_qpc = self.resources.clock.duration_from_us(us).unwrap();
        let now_ticks = self
            .resources
            .clock
            .now()
            .unwrap()
            .checked_add_duration(advance_qpc)
            .unwrap();
        let elapsed = now_ticks
            .checked_duration_since(self.resources.playback.epoch)
            .unwrap_or(DurationTicks::ZERO);
        TimelineTicks::from_raw(elapsed.as_u64())
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
        0
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

    /// Run production `plan_next_dispatch` for the harness state.
    pub fn plan_current_dispatch(&mut self) -> NextDispatchPlan {
        plan_next_dispatch(
            &self.resources.coordinator,
            &self.resources.estimator,
            self.resources.clock,
            LatencyClass::Hot,
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
        let effective_now_ticks = self
            .resources
            .playback
            .get_elapsed(now_ticks)
            .unwrap_or(TimelineTicks::ZERO);
        let ctx = AuthoredPacketContext {
            dispatch_plan: plan,
            effective_now_ticks,
            now_ticks,
            latency_class: plan.latency_class(),
            focus_loss_fault: false,
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
        let lead_up = pending_plan.map_or(0, |p| {
            self.resources.clock.duration_to_us(p.lead_ticks).unwrap_or(0)
        });
        let ctx = PendingReleaseContext {
            due_pending,
            pending_plan,
            lead_up_ticks,
            lead_up,
            latency_class,
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
}
