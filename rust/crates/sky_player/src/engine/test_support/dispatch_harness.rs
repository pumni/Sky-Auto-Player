#![cfg(any(test, feature = "test-support"))]

//! Deterministic test harness for the production DispatchExecutor and worker
//! staging/send path. It owns one compiled schedule and one live executor.

use crate::engine::SystemPowerState;
use crate::engine::config::WorkerConfig;
use crate::engine::shared::{SessionTarget, SharedProgressClock, SupervisorLeaseState};
use crate::engine::telemetry::{
    SharedMetrics, TelemetryCollector, TelemetryMode, WorkerMetricsLocal,
};
use crate::engine::worker::{
    DispatchAttemptContext, DispatchAttemptOutcome, DispatchCandidate, DispatchControlSignals,
    DispatchOutcome, WorkerResources, WorkerRuntime, WorkerSchedulingGuards, WorkerTimingState,
    dispatch_candidate, prepare_dispatch_candidate,
};
#[cfg(test)]
use crate::engine::worker::{FinalControlSignals, PreWaitAdmission, pre_wait_admission};
use sky_dispatch_core::accounting::GenerationAccounting;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::executor::{
    DispatchDecision, DispatchExecutor, DispatchTimingWindow, PreparedChord,
};
use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::plan::build_onset_plan;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, qpc_frequency_checked};
use sky_dispatch_win32::event::OwnedEvent;
use sky_dispatch_win32::input::{
    InstrumentPhysicalState, PHYSICAL_INSTRUMENT_SCAN_CODES, PacketRetryReason, PhysicalPacket,
    PlatformSendResult, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
    TrackedKeyState,
};
use sky_dispatch_win32::wait::HybridWaiter;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const DISPATCH_FRAME_US: u64 = 16_667;
pub const DISPATCH_MARGIN_US: u64 = 500;
pub const DISPATCH_EFFECTIVE_HOLD_US: u64 = DISPATCH_FRAME_US + DISPATCH_MARGIN_US;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchTimingConfigEvidence {
    pub frame_us: u64,
    pub frame_interval_ticks: u64,
    pub frame_base_hold_us: u64,
    pub timing_margin_us: u64,
    pub timing_margin_ticks: u64,
    pub effective_hold_us: u64,
    pub effective_hold_ticks: u64,
    pub lease_enabled: bool,
    pub lease_timeout_us: u64,
    pub lease_timeout_ticks: u64,
    pub native_admission_passed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct FinalizeEvidence {
    pub outcome_code: u8,
    pub attempted_mask: u16,
    pub release_obligation_mask: u16,
    pub active_mask: u16,
    pub possibly_active_mask: u16,
    pub failed_release_mask: u16,
    pub in_flight_mask: u16,
    pub activated: u64,
    pub released: u64,
    pub cancelled: u64,
    pub active_generations: u64,
    pub terminal_release_raw_up_syscall_count: u64,
}

pub type CleanupCapture = Arc<Mutex<Vec<(Vec<u16>, bool)>>>;
pub type PacketCapture = Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>;

pub struct ProductionDispatchTestHarness {
    pub(crate) config: WorkerConfig,
    pub(crate) resources: WorkerResources,
    pub(crate) timing: WorkerTimingState,
    pub(crate) runtime: WorkerRuntime,
    pub(crate) local_metrics: WorkerMetricsLocal,
    pub(crate) focus_active: Arc<AtomicBool>,
    pub(crate) target: Arc<SessionTarget>,
    pub(crate) quit_requested: Arc<AtomicBool>,
    pub(crate) skip_requested: Arc<AtomicBool>,
    pub(crate) panic_requested: AtomicBool,
    pub(crate) supervisor_expired: Arc<SupervisorLeaseState>,
    pub(crate) desired_pause: AtomicBool,
    pub(crate) system_power: Arc<SystemPowerState>,
    pub(crate) metrics: SharedMetrics,
    pub(crate) progress_clock: SharedProgressClock,
    pub(crate) observer: crate::engine::worker::dispatch::PendingObservationQueue,
    pub(crate) interrupt: Arc<OwnedEvent>,
    mock_sender_start_qpc: Arc<AtomicU64>,
}

impl ProductionDispatchTestHarness {
    pub fn create_harness(chords: &[OnsetChordInput]) -> Self {
        Self::create(chords)
    }

    pub fn new_down_only() -> Self {
        Self::create(&[chord(0, 0, &[0x15])])
    }

    pub fn new_down_chord(key_count: usize) -> Self {
        assert!((1..=PHYSICAL_INSTRUMENT_SCAN_CODES.len()).contains(&key_count));
        let scan_codes = &PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count];
        Self::create(&[chord(0, 0, scan_codes)])
    }

    pub fn new_mixed() -> Self {
        Self::create(&[
            chord(0, 0, &[0x15]),
            chord(1, DISPATCH_EFFECTIVE_HOLD_US, &[0x16]),
        ])
    }

    pub fn new_lifecycle_sequence_for_test(
        qpc_frequency_hz: u64,
        odd_frequency_vector: bool,
    ) -> Result<Self, String> {
        let second_onset = if odd_frequency_vector { 34_336 } else { 34_334 };
        let harness = Self::create_with_frequency(
            &[chord(0, 0, &[0x15]), chord(1, second_onset, &[0x15])],
            qpc_frequency_hz,
        )?;
        Ok(harness)
    }

    pub fn new_hol_sequence_for_test(qpc_frequency_hz: u64) -> Result<Self, String> {
        Self::create_with_frequency(
            &[chord(0, 0, &[0x15]), chord(1, 10_000, &[0x16])],
            qpc_frequency_hz,
        )
    }

    fn create(chords: &[OnsetChordInput]) -> Self {
        Self::create_with_frequency(chords, qpc_frequency_checked().expect("QPC frequency"))
            .expect("production executor fixture admission")
    }

    fn create_with_frequency(
        chords: &[OnsetChordInput],
        qpc_frequency_hz: u64,
    ) -> Result<Self, String> {
        let frequency = NonZeroU64::new(qpc_frequency_hz)
            .ok_or_else(|| "QPC frequency must be nonzero".to_string())?;
        let clock = QpcClock::from_frequency_hz(frequency);
        let mut scan_codes = chords
            .iter()
            .flat_map(|entry| entry.scan_codes.iter().copied())
            .collect::<Vec<_>>();
        scan_codes.sort_unstable();
        scan_codes.dedup();
        if scan_codes.is_empty() {
            scan_codes.push(PHYSICAL_INSTRUMENT_SCAN_CODES[0]);
        }

        let mut config = WorkerConfig::default();
        config.timing.frame_us = DISPATCH_FRAME_US;
        config.timing.frame_base_hold_us = DISPATCH_FRAME_US;
        config.timing.timing_margin_us = DISPATCH_MARGIN_US;
        config.timing.min_hold_us = DISPATCH_EFFECTIVE_HOLD_US;

        let plan = build_onset_plan(chords, &scan_codes, config.timing.min_hold_us)
            .map_err(|error| format!("production onset plan failed: {error}"))?;
        let prepared = sky_dispatch_core::executor::PreparedDispatchSchedule::prepare(
            &plan,
            clock.frequency_hz(),
            plan.compile_memory().caller_retained_capacity_bytes,
        )?;
        let scheduler = DispatchExecutor::from_prepared_schedule(prepared);

        let mut backend = TrackedKeyState::with_qpc_clock(clock);
        backend.set_test_emitters();
        backend.set_probe(|_, _| InstrumentPhysicalState::AllUp);
        backend.set_modifier_key_state_query_for_test(|_| 0);

        let test_epoch_margin = clock
            .duration_from_us(500_000)
            .map_err(|error| format!("test epoch margin conversion failed: {error:?}"))?;
        let epoch = clock
            .now()
            .map_err(|error| format!("test QPC sample failed: {error:?}"))?
            .checked_add_duration(test_epoch_margin)
            .map_err(|error| format!("test epoch overflow: {error}"))?;
        let playback = PlaybackClockState::new(epoch, DurationTicks::ZERO)
            .map_err(|error| format!("test playback clock: {error}"))?;
        let telemetry = TelemetryCollector::new(TelemetryMode::Ring, 64);
        let mut resources = WorkerResources {
            clock,
            waiter: HybridWaiter::new(),
            backend,
            scheduler,
            playback,
            telemetry: Arc::new(parking_lot::Mutex::new(telemetry)),
            scheduling: WorkerSchedulingGuards::create_test_guards(),
        };
        if let Some(proof) =
            sky_dispatch_win32::input::ReleaseOwnerProof::new_with_foreground_policy(
                1,
                0,
                None,
                None,
                config.focus.require_focus,
            )
        {
            let _ = resources.backend.bind_session_release_owner(proof);
        }
        let progress_clock = SharedProgressClock::default();
        progress_clock.publish(&resources.playback);

        let mut timing = WorkerTimingState::create_test_timing();
        timing.effective_spin_threshold_ticks = clock
            .duration_from_us(20_000)
            .map_err(|error| format!("test spin threshold conversion failed: {error:?}"))?;
        let target = Arc::new(SessionTarget::new(1, 0, config.focus.require_focus));
        let runtime =
            WorkerRuntime::create_test_runtime(Some(crate::engine::worker::TargetStamp {
                hwnd: 1,
                generation: 0,
            }));

        Ok(Self {
            config,
            resources,
            timing,
            runtime,
            local_metrics: WorkerMetricsLocal::default(),
            focus_active: Arc::new(AtomicBool::new(true)),
            target,
            quit_requested: Arc::new(AtomicBool::new(false)),
            skip_requested: Arc::new(AtomicBool::new(false)),
            panic_requested: AtomicBool::new(false),
            supervisor_expired: Arc::new(SupervisorLeaseState::new(QpcTicks::from_raw(1))),
            desired_pause: AtomicBool::new(false),
            system_power: Arc::new(SystemPowerState::default()),
            metrics: SharedMetrics::default(),
            progress_clock,
            observer: crate::engine::worker::dispatch::PendingObservationQueue::default(),
            interrupt: Arc::new(
                OwnedEvent::new_auto_reset()
                    .ok_or_else(|| "failed to create test interrupt event".to_string())?,
            ),
            mock_sender_start_qpc: Arc::new(AtomicU64::new(0)),
        })
    }

    pub fn evaluate_live_scheduler_for_test(
        &self,
        now: QpcTicks,
        epoch: QpcTicks,
    ) -> Result<DispatchDecision, String> {
        self.resources
            .scheduler
            .evaluate_at(now, epoch)
            .map_err(|error| format!("live scheduler QPC evaluation failed: {error}"))
    }

    pub fn current_executor_chord_for_test(&self) -> Option<PreparedChord> {
        self.resources.scheduler.current_chord()
    }

    pub fn prepare_candidate_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<PreparedChord>,
        at_offset_ticks: TimelineTicks,
    ) -> Result<Option<DispatchCandidate>, String> {
        self.prepare_candidate_with_observer_for_test(up_mask, down_chord, at_offset_ticks, false)
    }

    pub fn prepare_candidate_with_observer_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<PreparedChord>,
        at_offset_ticks: TimelineTicks,
        capture_up_slot_timings: bool,
    ) -> Result<Option<DispatchCandidate>, String> {
        let epoch = self.resources.playback.epoch;
        let requested_qpc = epoch
            .checked_add_duration(DurationTicks::from_raw(at_offset_ticks.as_u64()))
            .map_err(|error| format!("live dispatch target overflow: {error}"))?;
        let decision = self
            .resources
            .scheduler
            .evaluate_at(requested_qpc, epoch)
            .map_err(|error| format!("live dispatch scheduler evaluation failed: {error}"))?;
        let (actual_up_mask, actual_chord, timing_window, up_summary) = match decision {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
                up_summary,
            } => (up_mask, down_chord, timing_window, up_summary),
            decision => {
                return Err(format!(
                    "production executor did not offer Send at {requested_qpc:?}: {decision:?}"
                ));
            }
        };
        if actual_up_mask != up_mask
            || actual_chord.map(|chord| chord.key_mask) != down_chord.map(|chord| chord.key_mask)
        {
            return Err(format!(
                "prepared masks do not match the live executor decision: requested up={up_mask:#06x}, down={:?}; live up={actual_up_mask:#06x}, down={:?}",
                down_chord.map(|chord| chord.key_mask),
                actual_chord.map(|chord| chord.key_mask),
            ));
        }
        prepare_dispatch_candidate(
            actual_up_mask,
            actual_chord,
            timing_window,
            up_summary,
            capture_up_slot_timings,
            &self.target,
            &mut self.resources,
            &mut self.runtime,
        )
    }

    pub fn prepare_candidate_with_scheduler_window_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<PreparedChord>,
        timing_window: DispatchTimingWindow,
    ) -> Result<Option<DispatchCandidate>, String> {
        self.prepare_candidate_with_scheduler_window_and_observer_for_test(
            up_mask,
            down_chord,
            timing_window,
            false,
        )
    }

    pub fn prepare_candidate_with_scheduler_window_and_observer_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<PreparedChord>,
        timing_window: DispatchTimingWindow,
        capture_up_slot_timings: bool,
    ) -> Result<Option<DispatchCandidate>, String> {
        let up_summary = (up_mask != 0).then(|| {
            let timings = self.resources.scheduler.up_slot_timings(up_mask);
            let epoch = self.resources.playback.epoch;
            let mut authored_target_qpc: Option<QpcTicks> = None;
            let mut musical_up_not_before_qpc: Option<QpcTicks> = None;
            let mut hold_floor_mask = 0u16;
            for t in &timings {
                let nominal = epoch
                    .checked_add_duration(sky_dispatch_core::time::DurationTicks::from_raw(
                        t.nominal_up_offset_ticks.as_u64(),
                    ))
                    .expect("test nominal Up QPC");
                authored_target_qpc = Some(authored_target_qpc.map_or(nominal, |c| c.min(nominal)));
                musical_up_not_before_qpc = Some(
                    musical_up_not_before_qpc.map_or(t.release_not_before_qpc, |c| {
                        c.max(t.release_not_before_qpc)
                    }),
                );
                if t.release_not_before_qpc > nominal {
                    hold_floor_mask |= 1u16 << t.slot;
                }
            }
            sky_dispatch_core::executor::UpSlotTimingSummary {
                slot_count: timings.len() as u8,
                first_source_action_index: timings.first().map_or(0, |t| t.source_action_index),
                authored_target_qpc: authored_target_qpc.expect("non-empty up_mask"),
                musical_up_not_before_qpc: musical_up_not_before_qpc.expect("non-empty up_mask"),
                hold_floor_mask,
            }
        });
        prepare_dispatch_candidate(
            up_mask,
            down_chord,
            timing_window,
            up_summary,
            capture_up_slot_timings,
            &self.target,
            &mut self.resources,
            &mut self.runtime,
        )
    }

    pub fn candidate_timing_window_for_test(
        &self,
        candidate: &DispatchCandidate,
    ) -> DispatchTimingWindow {
        candidate.timing_window
    }

    /// Evaluate the live scheduler directly at `observed_wake_qpc` and prepare
    /// the resulting immediate packet on the stack (no speculative staging).
    pub fn evaluate_and_prepare_at_wake_for_test(
        &mut self,
        observed_wake_qpc: QpcTicks,
        capture_up_slot_timings: bool,
    ) -> Result<Option<DispatchCandidate>, String> {
        match self
            .resources
            .scheduler
            .evaluate_at(observed_wake_qpc, self.resources.playback.epoch)
            .map_err(|error| format!("post-wake scheduler evaluation failed: {error}"))?
        {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
                up_summary,
            } => prepare_dispatch_candidate(
                up_mask,
                down_chord,
                timing_window,
                up_summary,
                capture_up_slot_timings,
                &self.target,
                &mut self.resources,
                &mut self.runtime,
            ),
            DispatchDecision::Wait { .. } | DispatchDecision::Finished => Ok(None),
        }
    }

    pub fn dispatch_candidate_for_test(
        &mut self,
        candidate: DispatchCandidate,
        with_observer: bool,
    ) -> DispatchOutcome {
        self.dispatch_candidate_with_reason_for_test(candidate, with_observer)
            .public_outcome()
    }

    pub(crate) fn dispatch_candidate_with_reason_for_test(
        &mut self,
        candidate: DispatchCandidate,
        with_observer: bool,
    ) -> DispatchAttemptOutcome {
        let qpc_clock = self.resources.clock;
        let context = DispatchAttemptContext {
            target: &self.target,
            config: &self.config,
            control: DispatchControlSignals {
                focus_active: &self.focus_active,
                quit_requested: &self.quit_requested,
                skip_requested: &self.skip_requested,
                panic_requested: &self.panic_requested,
                desired_pause: &self.desired_pause,
                supervisor_expired: &self.supervisor_expired,
                system_power: &self.system_power,
            },
            timing: &self.timing,
            qpc_clock,
            progress_clock: &self.progress_clock,
            metrics: &self.metrics,
        };
        dispatch_candidate(
            candidate,
            context,
            &mut self.resources,
            &mut self.runtime,
            &mut self.local_metrics,
            with_observer.then_some(&self.observer),
        )
    }

    #[cfg(test)]
    pub(crate) fn pre_wait_admission_for_test(&mut self) -> PreWaitAdmission {
        pre_wait_admission(
            &self.target,
            self.config.focus.require_focus,
            &self.focus_active,
            FinalControlSignals {
                quit_requested: &self.quit_requested,
                skip_requested: &self.skip_requested,
                panic_requested: &self.panic_requested,
                desired_pause: &self.desired_pause,
                supervisor_expired: &self.supervisor_expired,
                system_power: Some(&self.system_power),
            },
            &mut self.runtime,
        )
    }

    pub fn attach_executor_for_test(&mut self) -> Result<(), String> {
        self.resources.scheduler.check_invariants()
    }

    pub fn reanchor_playback_clock_for_test(&mut self, epoch: QpcTicks) -> Result<(), String> {
        self.resources.playback.start_perf = epoch;
        self.resources.playback.epoch = epoch;
        self.progress_clock.publish(&self.resources.playback);
        Ok(())
    }

    pub fn commit_pause_for_test(
        &mut self,
        reason: sky_dispatch_core::clock::PauseReason,
        now: QpcTicks,
    ) -> Result<(), String> {
        self.resources
            .playback
            .enter_pause(reason, now)
            .map_err(|error| format!("test Pause control: {error}"))?;
        self.progress_clock.publish(&self.resources.playback);
        Ok(())
    }

    pub fn commit_resume_for_test(
        &mut self,
        reason: sky_dispatch_core::clock::PauseReason,
        now: QpcTicks,
    ) -> Result<QpcTicks, String> {
        self.resources
            .playback
            .exit_pause(reason, now)
            .map_err(|error| format!("test Resume control: {error}"))?;
        self.progress_clock.publish(&self.resources.playback);
        Ok(self.resources.playback.epoch)
    }

    pub fn is_paused_for_test(&self) -> bool {
        self.resources.playback.is_paused()
    }

    pub fn configure_mock_packet_sender_for_test(&mut self, latency_us: &[u64]) -> PacketCapture {
        let captured = Arc::new(Mutex::new(Vec::with_capacity(8)));
        let capture = Arc::clone(&captured);
        let sender_start = Arc::clone(&self.mock_sender_start_qpc);
        let latencies = Arc::new(latency_us.to_vec());
        let next_latency = Arc::new(AtomicUsize::new(0));
        let clock = self.resources.clock;
        self.resources.backend.set_packet_emitter(move |packet| {
            let index = next_latency.fetch_add(1, Ordering::Relaxed);
            let latency_us = latencies.get(index).copied().unwrap_or(0);
            let started_ticks = QpcTicks::from_raw(sender_start.load(Ordering::Acquire));
            let completed_ticks = started_ticks
                .checked_add_duration(
                    clock
                        .duration_from_us(latency_us)
                        .expect("mock transport latency conversion"),
                )
                .expect("mock transport completion arithmetic");
            let requested_mask = packet.up_mask | packet.down_mask;
            let evidence = SendEvidence {
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
            };
            capture
                .lock()
                .expect("packet capture lock")
                .push((packet, evidence));
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence,
            }
        });
        captured
    }

    pub fn configure_mock_cleanup_emitter_for_test(&mut self) -> CleanupCapture {
        let captured = Arc::new(Mutex::new(Vec::with_capacity(4)));
        let capture = Arc::clone(&captured);
        let clock = self.resources.clock;
        self.resources
            .backend
            .set_emitter(move |scan_codes, key_up| {
                capture
                    .lock()
                    .expect("cleanup capture lock")
                    .push((scan_codes.to_vec(), key_up));
                let now = clock.now().expect("cleanup QPC");
                PlatformSendResult {
                    requested: scan_codes.len() as u8,
                    inserted: scan_codes.len() as u8,
                    started_ticks: now,
                    completed_ticks: Some(now),
                    win32_error: 0,
                    timing_error: None,
                }
            });
        captured
    }

    pub fn set_mock_sender_start_qpc_for_test(&self, start_qpc: QpcTicks) {
        self.mock_sender_start_qpc
            .store(start_qpc.as_u64(), Ordering::Release);
    }

    pub fn set_modifier_key_state_query_for_test<F>(&mut self, query: F)
    where
        F: Fn(i32) -> i16 + Send + Sync + 'static,
    {
        self.resources
            .backend
            .set_modifier_key_state_query_for_test(query);
    }

    pub fn in_flight_mask_for_test(&self) -> u16 {
        self.resources.backend.in_flight_mask
    }

    pub fn set_pause_requested_for_test(&self, requested: bool) {
        self.desired_pause.store(requested, Ordering::Release);
        self.interrupt.signal();
    }

    pub fn generation_accounting_for_test(&self) -> GenerationAccounting {
        self.resources.scheduler.accounting()
    }

    pub fn release_obligation_mask_for_test(&self) -> u16 {
        self.resources.backend.release_obligation_mask()
    }

    pub fn reset_target_proof_counts_for_test(&self) {
        self.target.reset_proof_operation_counts_for_test();
    }

    pub fn target_proof_generation_revalidations_for_test(&self) -> u64 {
        let counts = self.target.proof_operation_counts_for_test();
        counts.generation_revalidations
    }

    pub fn target_proof_atomic_reads_for_test(&self) -> u64 {
        self.target.proof_operation_counts_for_test().atomic_reads
    }

    pub fn apply_scheduler_resumable_lifecycle_transition_for_test(
        &mut self,
    ) -> Result<(), String> {
        let expected_target = crate::engine::worker::load_target_stamp(&self.target);
        crate::engine::worker::apply_scheduler_resumable_lifecycle_transition_for_test(
            &mut self.resources.backend,
            &mut self.resources.scheduler,
            &self.target,
            expected_target,
            self.config.focus.require_focus,
            self.target.hwnd_for_safety_release(),
        )
    }

    pub fn process_command_control_for_test(&mut self) -> bool {
        crate::engine::worker::process_command_control_for_test(
            &mut self.runtime.terminal_trigger,
            &self.quit_requested,
            &self.skip_requested,
            &self.panic_requested,
            &self.supervisor_expired,
            &self.system_power,
        )
    }

    pub fn notify_system_power_for_test(&self, suspended: bool) -> bool {
        self.system_power.notify(suspended, &self.interrupt)
    }

    pub fn system_power_down_blocked_for_test(&self) -> bool {
        self.system_power.down_blocked()
    }

    pub fn set_preflight_physical_mask_probe_for_test<F>(&mut self, probe: F)
    where
        F: Fn() -> u16 + Send + Sync + 'static,
    {
        self.resources
            .backend
            .set_preflight_physical_mask_probe(probe);
    }

    pub fn set_post_focus_revalidation_race_hook<F>(&mut self, hook: F)
    where
        F: Fn(
                &AtomicBool,
                &SessionTarget,
                &AtomicBool,
                &AtomicBool,
                &AtomicBool,
                &AtomicBool,
                &SystemPowerState,
            ) + Send
            + Sync
            + 'static,
    {
        self.runtime.final_gate_post_focus_race_hook = Some(Arc::new(hook));
    }

    pub fn set_final_gate_before_syscall_hook<F>(&mut self, hook: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.runtime.final_gate_before_syscall_hook = Some(Arc::new(hook));
    }

    /// Enable deterministic foreground and retained owner identity for tests
    /// that exercise the production focus-required final gate.
    pub fn configure_focus_owner_for_test(&mut self, hwnd: isize, owner_pid: u32) {
        self.config.focus.require_focus = true;
        self.target.require_foreground_for_test();
        self.target.require_owner_identity();
        let (current_hwnd, generation) = self.target.load_stable().expect("test target snapshot");
        assert_eq!(current_hwnd, hwnd, "focus fixture HWND must be published");
        #[cfg(any(test, feature = "test-support"))]
        assert!(self.target.bind_owner_identity_authority(
            hwnd,
            generation,
            owner_pid,
            Arc::new(
                sky_dispatch_win32::focus::WindowIdentityAuthority::synthetic_for_test(
                    hwnd, owner_pid,
                )
            ),
        ));
        #[cfg(not(any(test, feature = "test-support")))]
        assert!(self.target.bind_owner_identity(hwnd, generation, owner_pid));
        let release_owner = self
            .target
            .try_release_owner_proof_for_generation(hwnd, generation)
            .expect("focus fixture session owner proof");
        self.resources
            .backend
            .forget_release_owner_proof_for_test(0xffff);
        self.resources
            .backend
            .bind_session_release_owner(release_owner)
            .expect("focus fixture binds one session release owner");
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(hwnd));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(owner_pid)));
        sky_dispatch_win32::focus::set_window_live_for_test(Some(hwnd));
    }

    pub fn invalidate_focus_owner_for_test(&self, hwnd: isize, owner_pid: u32) {
        self.target.invalidate_for_test();
        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(hwnd));
        sky_dispatch_win32::focus::set_foreground_owner_for_test(Some(Some(owner_pid)));
        sky_dispatch_win32::focus::set_window_live_for_test(Some(hwnd));
    }

    pub fn invalidate_bound_owner_for_test(&self) {
        self.target.invalidate_owner_identity();
    }

    pub fn dispatch_timing_config_for_test(&self) -> Result<DispatchTimingConfigEvidence, String> {
        let timing = self.config.timing;
        let frame_interval_ticks = self
            .resources
            .clock
            .duration_from_us(timing.frame_us)
            .map_err(|error| format!("frame conversion: {error:?}"))?;
        let timing_margin_ticks = self
            .resources
            .clock
            .duration_from_us(timing.timing_margin_us)
            .map_err(|error| format!("margin conversion: {error:?}"))?;
        let effective_hold_ticks = self
            .resources
            .clock
            .duration_from_us(timing.min_hold_us)
            .map_err(|error| format!("hold conversion: {error:?}"))?;
        let lease_timeout_ticks = self
            .resources
            .clock
            .duration_from_us(self.config.wait.supervisor_lease_timeout_us)
            .map_err(|error| format!("lease timeout conversion: {error:?}"))?;
        Ok(DispatchTimingConfigEvidence {
            frame_us: timing.frame_us,
            frame_interval_ticks: frame_interval_ticks.as_u64(),
            frame_base_hold_us: timing.frame_base_hold_us,
            timing_margin_us: timing.timing_margin_us,
            timing_margin_ticks: timing_margin_ticks.as_u64(),
            effective_hold_us: timing.min_hold_us,
            effective_hold_ticks: effective_hold_ticks.as_u64(),
            lease_enabled: self.config.wait.supervisor_lease_timeout_us > 0,
            lease_timeout_us: self.config.wait.supervisor_lease_timeout_us,
            lease_timeout_ticks: lease_timeout_ticks.as_u64(),
            native_admission_passed: true,
        })
    }

    pub fn qpc_now_for_test(&self) -> Result<QpcTicks, String> {
        self.resources
            .clock
            .now()
            .map_err(|error| format!("test QPC sample: {error:?}"))
    }

    pub fn qpc_duration_from_us_for_test(&self, us: u64) -> Result<DurationTicks, String> {
        self.resources
            .clock
            .duration_from_us(us)
            .map(|ticks| DurationTicks::from_raw(ticks.as_u64()))
            .map_err(|error| format!("test QPC conversion: {error:?}"))
    }

    pub fn finalize_for_test(self) -> FinalizeEvidence {
        let mut observation = crate::engine::worker::FinalizeTestObservation::default();
        let outcome_code = self.finalize_worker_for_test(false, &mut observation);
        FinalizeEvidence {
            outcome_code,
            attempted_mask: observation.attempted_mask,
            release_obligation_mask: observation.release_obligation_mask,
            active_mask: observation.active_mask,
            possibly_active_mask: observation.possibly_active_mask,
            failed_release_mask: observation.failed_release_mask,
            in_flight_mask: observation.in_flight_mask,
            activated: observation.generation_accounting.activated,
            released: observation.generation_accounting.released,
            cancelled: observation.generation_accounting.cancelled,
            active_generations: observation.generation_accounting.active,
            terminal_release_raw_up_syscall_count: observation
                .terminal_release_raw_up_syscall_count,
        }
    }

    pub(crate) fn finalize_worker_for_test(
        self,
        worker_panicked: bool,
        cleanup_observation: &mut crate::engine::worker::FinalizeTestObservation,
    ) -> u8 {
        let Self {
            resources,
            runtime,
            target,
            quit_requested,
            skip_requested,
            panic_requested,
            supervisor_expired,
            system_power,
            metrics,
            progress_clock,
            ..
        } = self;
        crate::engine::worker::finalize_worker_for_test(
            resources,
            &target,
            &skip_requested,
            &quit_requested,
            &panic_requested,
            &supervisor_expired,
            &system_power,
            &metrics,
            &progress_clock,
            worker_panicked,
            runtime.terminal_trigger,
            runtime.terminal_error,
            runtime.attempted_dispatch_anomaly,
            cleanup_observation,
        )
    }
}

fn chord(index: u32, scheduled_us: u64, scan_codes: &[u16]) -> OnsetChordInput {
    OnsetChordInput {
        source_action_index: index,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
    }
}

#[cfg(test)]
#[path = "dispatch_harness_tests.rs"]
mod dispatch_harness_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_harness_has_one_live_executor_owner() {
        let harness = ProductionDispatchTestHarness::new_down_only();
        assert_eq!(harness.resources.scheduler.down_chord_count(), 1);
        assert_eq!(harness.resources.scheduler.accounting().total, 1);
    }
}
