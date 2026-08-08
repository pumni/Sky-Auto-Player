//! Test harness for production dispatch paths (DownOnly, Mixed, UpOnly release).
//!
//! Provides `ProductionDispatchTestHarness` for deterministic zero-allocation
//! verification of production dispatch functions.

use super::super::super::{LatencyClass, TimelineTicks};
use super::super::planning::NextDispatchPlan;
use super::super::startup::WorkerSchedulingGuards;
use super::{
    AuthoredPacketContext, DispatchStep, PendingObservationQueue, PendingReleaseContext,
    dispatch_authored_packet, dispatch_due_pending_releases,
};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};

pub struct ProductionDispatchTestHarness {
    pub(crate) config: super::super::WorkerConfig,
    pub(crate) resources: super::super::WorkerResources,
    pub(crate) health: super::super::WorkerHealthState,
    pub(crate) timing: super::super::WorkerTimingState,
    pub(crate) runtime: super::super::WorkerRuntime,
    pub(crate) local_metrics: super::super::WorkerMetricsLocal,
    pub(crate) last_published_error: Option<String>,
    pub(crate) secondary_errors: Vec<String>,
    pub(crate) focus_active: std::sync::atomic::AtomicBool,
    pub(crate) target_hwnd: std::sync::atomic::AtomicIsize,
    pub(crate) target_generation: std::sync::atomic::AtomicU64,
    pub(crate) quit_requested: std::sync::atomic::AtomicBool,
    pub(crate) skip_requested: std::sync::atomic::AtomicBool,
    pub(crate) panic_requested: std::sync::atomic::AtomicBool,
    pub(crate) desired_pause: std::sync::atomic::AtomicBool,
    pub(crate) metrics: super::super::super::SharedMetrics,
    pub(crate) observer: PendingObservationQueue,
}

impl ProductionDispatchTestHarness {
    pub fn new_down_only() -> Self {
        Self::create_harness(&[
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 0,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 1,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: vec![0x15].into(),
                reason: "up".into(),
            },
        ])
    }

    pub fn new_mixed() -> Self {
        Self::create_harness(&[
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 0,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down1".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 1,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 1000,
                scan_codes: vec![0x15].into(),
                reason: "up1".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 2,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: vec![0x16].into(),
                reason: "down2".into(),
            },
        ])
    }

    pub fn new_uponly_release() -> (
        Self,
        smallvec::SmallVec<[sky_dispatch_core::coordinator::PendingRelease; 15]>,
    ) {
        let mut harness = Self::create_harness(&[
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 0,
                kind: sky_dispatch_core::model::ActionKind::Down,
                scheduled_us: 0,
                scan_codes: vec![0x15].into(),
                reason: "down".into(),
            },
            sky_dispatch_core::model::KeyActionInput {
                source_action_index: 1,
                kind: sky_dispatch_core::model::ActionKind::Up,
                scheduled_us: 1000,
                scan_codes: vec![0x15].into(),
                reason: "up".into(),
            },
        ]);
        // Dispatch Down outside window
        let plan = NextDispatchPlan::default();
        let ctx = AuthoredPacketContext {
            dispatch_plan: &plan,
            effective_now_ticks: TimelineTicks::ZERO,
            now_ticks: QpcTicks::from_raw(0),
            latency_class: LatencyClass::Hot,
            focus_loss_fault: false,
        };
        dispatch_authored_packet(
            ctx,
            &harness.config,
            &mut harness.resources,
            &mut harness.health,
            &harness.timing,
            &mut harness.runtime,
            &mut harness.local_metrics,
            &mut harness.last_published_error,
            &harness.focus_active,
            &harness.target_hwnd,
            &harness.target_generation,
            &harness.quit_requested,
            &harness.skip_requested,
            &harness.panic_requested,
            &harness.desired_pause,
            &harness.metrics,
            &mut harness.observer,
        );
        // Pop Up batch outside window
        let prepared = harness
            .resources
            .coordinator
            .prepare_next_due_authored(
                TimelineTicks::from_raw(10_000),
                sky_dispatch_win32::clock::DurationTicks::ZERO,
            )
            .expect("prepare authored")
            .expect("due batch");
        let (due_pending, _) = harness
            .resources
            .coordinator
            .commit_up_request(prepared)
            .expect("commit up request");
        (harness, due_pending)
    }

    fn create_harness(actions: &[sky_dispatch_core::model::KeyActionInput]) -> Self {
        let schedule = sky_dispatch_core::compile::compile_runtime_intents(actions, &[0x15, 0x16])
            .expect("schedule");
        let coordinator =
            sky_dispatch_core::coordinator::RuntimeDispatchCoordinator::try_new_ticks(
                schedule,
                0,
                sky_dispatch_win32::clock::DurationTicks::ZERO,
                0,
                sky_dispatch_win32::clock::DurationTicks::ZERO,
                |us| Ok(TimelineTicks::from_raw(us)),
            )
            .expect("coordinator");
        let qpc_clock = QpcClock::initialize().expect("qpc_clock");
        let mut backend = sky_dispatch_win32::input::TrackedKeyState::with_qpc_clock(qpc_clock);
        backend.set_test_emitters();
        let waiter = sky_dispatch_win32::wait::HybridWaiter::new();
        let playback = sky_dispatch_core::clock::PlaybackClockState::new(
            qpc_clock.now().expect("qpc now"),
            sky_dispatch_win32::clock::DurationTicks::ZERO,
        )
        .expect("playback");
        let estimator = sky_dispatch_core::estimator::SendLatencyEstimator::default();
        let telemetry = super::super::telemetry::TelemetryCollector::new(
            super::super::telemetry::TelemetryMode::Ring,
            64,
        );
        let scheduling = WorkerSchedulingGuards::create_test_guards();

        let resources = super::super::WorkerResources {
            clock: qpc_clock,
            waiter,
            backend,
            coordinator,
            playback,
            estimator,
            telemetry,
            scheduling,
        };

        let health_options = super::super::health::DispatchHealthOptions::default();
        let health = super::super::WorkerHealthState::new(health_options);
        let timing = super::super::WorkerTimingState::create_test_timing();

        Self {
            config: super::super::WorkerConfig::default(),
            resources,
            health,
            timing,
            runtime: super::super::WorkerRuntime {
                verified_target: Some(super::super::admission::TargetStamp {
                    hwnd: 1,
                    generation: 0,
                }),
                ..super::super::WorkerRuntime::default()
            },
            local_metrics: super::super::WorkerMetricsLocal::default(),
            last_published_error: None,
            secondary_errors: Vec::new(),
            focus_active: std::sync::atomic::AtomicBool::new(true),
            target_hwnd: std::sync::atomic::AtomicIsize::new(1),
            target_generation: std::sync::atomic::AtomicU64::new(0),
            quit_requested: std::sync::atomic::AtomicBool::new(false),
            skip_requested: std::sync::atomic::AtomicBool::new(false),
            panic_requested: std::sync::atomic::AtomicBool::new(false),
            desired_pause: std::sync::atomic::AtomicBool::new(false),
            metrics: super::super::super::SharedMetrics::default(),
            observer: PendingObservationQueue::default(),
        }
    }

    pub fn dispatch_authored_packet_path(&mut self) -> DispatchStep {
        let plan = NextDispatchPlan::default();
        let ctx = AuthoredPacketContext {
            dispatch_plan: &plan,
            effective_now_ticks: TimelineTicks::ZERO,
            now_ticks: QpcTicks::from_raw(0),
            latency_class: LatencyClass::Hot,
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

    pub fn dispatch_pending_releases_path(
        &mut self,
        due_pending: smallvec::SmallVec<[sky_dispatch_core::coordinator::PendingRelease; 15]>,
    ) -> DispatchStep {
        let ctx = PendingReleaseContext {
            due_pending,
            pending_plan: None,
            lead_up_ticks: sky_dispatch_win32::clock::DurationTicks::ZERO,
            lead_up: 0,
            latency_class: LatencyClass::Hot,
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
