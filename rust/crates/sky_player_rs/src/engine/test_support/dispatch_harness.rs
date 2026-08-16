#![cfg(any(test, feature = "test-support"))]

//! Test harness for production dispatch paths (DownOnly, Mixed, UpOnly release).
//!
//! Provides `ProductionDispatchTestHarness` for deterministic zero-allocation
//! verification of production dispatch functions.

use crate::engine::config::WorkerConfig;
use crate::engine::shared::SharedProgressClock;
use crate::engine::telemetry::{
    SharedMetrics, TelemetryCollector, TelemetryMode, WorkerMetricsLocal,
};
use crate::engine::worker::dispatch::PendingObservationQueue;
use crate::engine::worker::dispatch::{
    AuthoredPacketContext, DispatchStep, dispatch_authored_packet,
};
use crate::engine::worker::{
    DispatchHealthOptions, DispatchPath, NextDispatchPlan, TargetStamp, WaitBoundary,
    WaitBoundaryInput, WaitDeadline, WaitMutable, WaitResult, WaitSignals, WaitTiming,
    WorkerHealthState, WorkerResources, WorkerRuntime, WorkerSchedulingGuards, WorkerTimingState,
    dispatch_due_from_plan, plan_next_dispatch, plan_next_dispatch_projected,
    preflight_prepared_plan, wait_for_next_boundary,
};
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::coordinator::{RuntimeDispatchCoordinator, physical_packet_kind};
use sky_dispatch_core::model::{ActionKind, KeyActionInput, PhysicalPacketKind};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{
    PHYSICAL_INSTRUMENT_SCAN_CODES, PacketRetryReason, PlatformSendResult, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, TrackedKeyState,
};
use sky_dispatch_win32::wait::HybridWaiter;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

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
    pub(crate) interrupt: OwnedEvent,
    pub(crate) last_wait_result: Option<WaitResult>,
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

    /// A deferred release for key A shares an authored timestamp with an
    /// unrelated Down chord on keys B/C. Production must send B/C at their
    /// authored boundary and retain A as an independent pending release.
    pub fn new_deferred_release_with_unrelated_down() -> Self {
        Self::create_harness_with_min_hold(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 0,
                    scan_codes: vec![0x15].into(),
                    reason: "deferred-seed".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Up,
                    scheduled_us: 100,
                    scan_codes: vec![0x15].into(),
                    reason: "deferred-release".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Down,
                    scheduled_us: 100,
                    scan_codes: vec![0x16, 0x17].into(),
                    reason: "unrelated-chord".into(),
                },
            ],
            10_000,
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
        assert!(matches!(
            harness.dispatch_authored_with_plan(&plan),
            DispatchStep::Dispatched
        ));
        harness
    }

    /// Build a retrigger packet with `event_count` physical INPUT events at
    /// one deadline (half Up, half Down). Initial owners are dispatched during
    /// setup so the measured packet is genuinely Mixed.
    pub fn new_mixed_events(event_count: usize) -> Self {
        Self::new_mixed_events_with_gap(event_count, 1_000)
    }

    pub fn new_mixed_events_with_gap(event_count: usize, gap_us: u64) -> Self {
        assert!(
            event_count.is_multiple_of(2) && (2..=30).contains(&event_count),
            "mixed event count must be an even value in 2..=30"
        );
        let key_count = event_count / 2;
        let scan_codes = PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count].to_vec();
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
            scheduled_us: gap_us,
            scan_codes: scan_codes.clone().into(),
            reason: "bench-up".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: gap_us,
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
        Self::new_uponly_release_chord_with_gap(1, gap_us)
    }

    pub fn new_uponly_release_chord_with_gap(key_count: usize, gap_us: u64) -> Self {
        assert!((1..=15).contains(&key_count), "key count must be 1..=15");
        let scan_codes = PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count].to_vec();
        let mut harness = Self::create_harness(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: scan_codes.clone().into(),
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: gap_us,
                scan_codes: scan_codes.into(),
                reason: "up".into(),
            },
        ]);
        // Dispatch Down outside window
        let plan = harness.plan_current_dispatch();
        assert!(matches!(
            harness.dispatch_authored_with_plan(&plan),
            DispatchStep::Dispatched
        ));
        harness
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
        let waiter = HybridWaiter::new();
        let playback =
            PlaybackClockState::new(qpc_clock.now().expect("qpc now"), DurationTicks::ZERO)
                .expect("playback");
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

    pub fn last_wait_result(&self) -> Option<WaitResult> {
        self.last_wait_result
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
    pub fn set_deadline_wake_for_test(&mut self, ticks: QpcTicks) {
        self.runtime.set_deadline_wake_qpc_for_test(Some(ticks));
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
        self.align_epoch_to_selected_boundary_before_planning();
        let mut plan = plan_next_dispatch_projected(crate::engine::worker::PlanningInput {
            coordinator: &self.resources.coordinator,
            epoch_qpc: self.resources.playback.epoch,
            preparation_probe: &self.runtime.preparation_probe,
        })
        .expect("projected dispatch plan");
        preflight_prepared_plan(
            &mut plan,
            &mut self.resources.backend,
            &mut self.runtime,
            &self.target_hwnd,
            &self.target_generation,
        )
        .expect("preflight prepared projected plan");
        self.refresh_physical_target_for_test(&mut plan);
        plan
    }

    pub fn preparation_counts(&self) -> (u64, u64, u64, u64) {
        self.runtime.preparation_probe.counts()
    }

    pub fn set_force_preflight_failure(&mut self, flag: Arc<AtomicBool>) {
        self.resources.backend.set_force_preflight_failure(flag);
    }

    /// Run the production wait boundary and direct frozen-plan dispatch path.
    pub fn wait_and_dispatch_current_plan(
        &mut self,
        plan: &NextDispatchPlan,
    ) -> Result<DispatchStep, String> {
        let boundary = wait_for_next_boundary(WaitBoundaryInput {
            deadline: WaitDeadline {
                physical_target_qpc: plan.physical_target_qpc(),
                qpc_clock: self.resources.clock,
            },
            timing: WaitTiming {
                effective_spin_threshold_ticks: self.timing.effective_spin_threshold_ticks,
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
        let wait_result = match boundary {
            WaitBoundary::Due {
                wait_result: Some(wait_result),
                dispatch_qpc,
                ..
            } => {
                self.runtime
                    .set_deadline_wake_qpc_for_test(Some(dispatch_qpc));
                wait_result
            }
            WaitBoundary::Due {
                wait_result: None, ..
            } => {
                return Err(format!(
                    "benchmark deadline was already due without a blocking wait: deadline={:?}, effective_now={:?}",
                    plan.deadline_ticks(),
                    self.effective_now_ticks
                ));
            }
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
        self.last_wait_result = Some(wait_result);
        self.runtime
            .set_deadline_wake_qpc_for_test(wait_result.wake_qpc);
        let now_ticks = wait_result.wake_qpc.unwrap_or(
            self.resources
                .clock
                .now()
                .map_err(|error| format!("benchmark send QPC: {error:?}"))?,
        );
        let effective_now_ticks = self
            .resources
            .playback
            .get_elapsed_allow_pre_epoch(now_ticks, false)
            .map_err(|error| format!("benchmark timeline: {error}"))?;
        self.effective_now_ticks = effective_now_ticks;
        Ok(self.dispatch_plan_at(plan, effective_now_ticks, now_ticks))
    }
    fn align_epoch_to_deadline_for_test(&mut self, deadline: TimelineTicks, now_ticks: QpcTicks) {
        let duration = DurationTicks::from_raw(deadline.as_u64());
        let target = self
            .resources
            .playback
            .epoch
            .checked_add_duration(duration)
            .unwrap();
        if target > now_ticks {
            let raw = now_ticks.as_u64().checked_sub(deadline.as_u64()).unwrap();
            self.resources.playback.epoch = QpcTicks::from_raw(raw);
        }
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
            &mut self.observer,
        )
    }
    /// Invoke the production frozen-plan helper without the kernel wait.
    /// Tests use this to cover all structural plan states directly.
    pub fn dispatch_due_from_plan_for_test(&mut self, plan: &NextDispatchPlan) -> DispatchStep {
        let now_ticks = self.resources.clock.now().expect("qpc now");
        self.dispatch_plan_at(plan, self.effective_now_ticks, now_ticks)
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
        let now_ticks = self.resources.clock.now().expect("qpc now");
        let ctx = AuthoredPacketContext {
            dispatch_plan: plan,
            effective_now_ticks: self.effective_now_ticks,
            now_ticks,
            physical_target_qpc: plan.physical_target_qpc().expect("physical target QPC"),
            startup_target_selected: false,
            focus_loss_fault: false,
            supervisor_heartbeat_ticks: &self.supervisor_heartbeat_ticks,
            lease_timeout_ticks,
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
            &mut self.observer,
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
