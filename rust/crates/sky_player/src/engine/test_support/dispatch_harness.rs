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
    StagedCandidate, StagedControlSignals, StagedDispatchContext, StagedDispatchOutcome,
    WorkerResources, WorkerRuntime, WorkerSchedulingGuards, WorkerTimingState,
    dispatch_staged_candidate, stage_candidate,
};
use sky_dispatch_core::accounting::GenerationAccounting;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::executor::{
    DispatchDecision, DispatchExecutor, DispatchTimingWindow, DownChordMetadata,
};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_core::validation::validate_min_hold_and_release_gap_feasibility;
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
pub const DISPATCH_RELEASE_GAP_US: u64 = DISPATCH_FRAME_US + DISPATCH_MARGIN_US;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchTimingConfigEvidence {
    pub frame_us: u64,
    pub frame_interval_ticks: u64,
    pub frame_base_hold_us: u64,
    pub timing_margin_us: u64,
    pub timing_margin_ticks: u64,
    pub effective_hold_us: u64,
    pub effective_hold_ticks: u64,
    pub min_release_gap_us: u64,
    pub min_release_gap_ticks: u64,
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
}

pub type CleanupCapture = Arc<Mutex<Vec<(Vec<u16>, bool)>>>;
pub type PacketCapture = Arc<Mutex<Vec<(PhysicalPacket, SendEvidence)>>>;

pub struct ProductionDispatchTestHarness {
    pub(crate) config: WorkerConfig,
    pub(crate) resources: WorkerResources,
    pub(crate) timing: WorkerTimingState,
    pub(crate) runtime: WorkerRuntime,
    pub(crate) local_metrics: WorkerMetricsLocal,
    pub(crate) focus_active: AtomicBool,
    pub(crate) target: Arc<SessionTarget>,
    pub(crate) quit_requested: AtomicBool,
    pub(crate) skip_requested: AtomicBool,
    pub(crate) panic_requested: AtomicBool,
    pub(crate) supervisor_expired: SupervisorLeaseState,
    pub(crate) desired_pause: AtomicBool,
    pub(super) system_power: Arc<SystemPowerState>,
    pub(crate) metrics: SharedMetrics,
    pub(crate) progress_clock: SharedProgressClock,
    pub(crate) observer: crate::engine::worker::dispatch::PendingObservationQueue,
    pub(crate) interrupt: Arc<OwnedEvent>,
    mock_sender_start_qpc: Arc<AtomicU64>,
}

impl ProductionDispatchTestHarness {
    pub fn create_harness(actions: &[KeyActionInput]) -> Self {
        Self::create(actions)
    }

    pub fn new_down_only() -> Self {
        Self::create(&[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 10_000_000, &[0x15]),
        ])
    }

    pub fn new_down_chord(key_count: usize) -> Self {
        assert!((1..=PHYSICAL_INSTRUMENT_SCAN_CODES.len()).contains(&key_count));
        let scan_codes = &PHYSICAL_INSTRUMENT_SCAN_CODES[..key_count];
        Self::create(&[
            action(0, ActionKind::Down, 0, scan_codes),
            action(1, ActionKind::Up, 1_000_000, scan_codes),
        ])
    }

    pub fn new_mixed() -> Self {
        Self::create(&[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
            action(2, ActionKind::Down, 20_000, &[0x16]),
            action(3, ActionKind::Up, 40_000, &[0x16]),
        ])
    }

    pub fn new_lifecycle_sequence_for_test(
        qpc_frequency_hz: u64,
        odd_frequency_vector: bool,
    ) -> Result<Self, String> {
        let offsets = if odd_frequency_vector {
            [0, 17_168, 34_336, 51_504]
        } else {
            [0, 17_167, 34_334, 51_501]
        };
        let harness = Self::create_with_frequency(
            &[
                action(0, ActionKind::Down, offsets[0], &[0x15]),
                action(1, ActionKind::Up, offsets[1], &[0x15]),
                action(2, ActionKind::Down, offsets[2], &[0x15]),
                action(3, ActionKind::Up, offsets[3], &[0x15]),
            ],
            qpc_frequency_hz,
        )?;
        Ok(harness)
    }

    pub fn new_hol_sequence_for_test(qpc_frequency_hz: u64) -> Result<Self, String> {
        Self::create_with_frequency(
            &[
                action(0, ActionKind::Down, 0, &[0x15]),
                action(1, ActionKind::Down, 10_000, &[0x16]),
                action(2, ActionKind::Up, 30_000, &[0x15]),
                action(3, ActionKind::Up, 40_000, &[0x16]),
            ],
            qpc_frequency_hz,
        )
    }

    fn create(actions: &[KeyActionInput]) -> Self {
        Self::create_with_frequency(actions, qpc_frequency_checked().expect("QPC frequency"))
            .expect("production executor fixture admission")
    }

    fn create_with_frequency(
        actions: &[KeyActionInput],
        qpc_frequency_hz: u64,
    ) -> Result<Self, String> {
        let frequency = NonZeroU64::new(qpc_frequency_hz)
            .ok_or_else(|| "QPC frequency must be nonzero".to_string())?;
        let clock = QpcClock::from_frequency_hz(frequency);
        let mut scan_codes = actions
            .iter()
            .flat_map(|entry| entry.scan_codes.iter().copied())
            .collect::<Vec<_>>();
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let schedule = compile_runtime_intents(actions, &scan_codes)
            .map_err(|error| format!("production schedule compilation failed: {error}"))?;

        let mut config = WorkerConfig::default();
        config.timing.frame_us = DISPATCH_FRAME_US;
        config.timing.frame_base_hold_us = DISPATCH_FRAME_US;
        config.timing.timing_margin_us = DISPATCH_MARGIN_US;
        config.timing.min_hold_us = DISPATCH_EFFECTIVE_HOLD_US;
        config.timing.min_release_gap_us = DISPATCH_RELEASE_GAP_US;
        validate_min_hold_and_release_gap_feasibility(
            &schedule,
            config.timing.min_hold_us,
            config.timing.min_release_gap_us,
        )
        .map_err(|error| format!("production scheduler admission failed: {error}"))?;
        let min_hold_ticks = clock
            .duration_from_us(config.timing.min_hold_us)
            .map_err(|error| format!("minimum hold conversion failed: {error:?}"))?;
        let min_release_gap_ticks = clock
            .duration_from_us(config.timing.min_release_gap_us)
            .map_err(|error| format!("minimum release gap conversion failed: {error:?}"))?;
        let scheduler = DispatchExecutor::new(
            &schedule,
            DurationTicks::from_raw(min_hold_ticks.as_u64()),
            DurationTicks::from_raw(min_release_gap_ticks.as_u64()),
            |us| {
                clock
                    .timeline_from_us(us)
                    .map_err(|error| format!("{error:?}"))
            },
        )?;

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
        let resources = WorkerResources {
            clock,
            waiter: HybridWaiter::new(),
            backend,
            scheduler,
            playback,
            telemetry: Arc::new(parking_lot::Mutex::new(telemetry)),
            scheduling: WorkerSchedulingGuards::create_test_guards(),
        };
        let progress_clock = SharedProgressClock::default();
        progress_clock.publish(&resources.playback);

        let mut timing = WorkerTimingState::create_test_timing();
        timing.timing_margin_ticks = clock
            .duration_from_us(DISPATCH_MARGIN_US)
            .map_err(|error| format!("Timing Margin conversion failed: {error:?}"))?;
        timing.effective_spin_threshold_ticks = clock
            .duration_from_us(20_000)
            .map_err(|error| format!("test spin threshold conversion failed: {error:?}"))?;
        let target = Arc::new(SessionTarget::new(1, 0));
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
            focus_active: AtomicBool::new(true),
            target,
            quit_requested: AtomicBool::new(false),
            skip_requested: AtomicBool::new(false),
            panic_requested: AtomicBool::new(false),
            supervisor_expired: SupervisorLeaseState::new(QpcTicks::from_raw(1)),
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
    ) -> Result<DispatchDecision<'_>, String> {
        self.resources
            .scheduler
            .evaluate_at(now, epoch)
            .map_err(|error| format!("live scheduler QPC evaluation failed: {error}"))
    }

    pub fn current_executor_chord_for_test(&self) -> Option<DownChordMetadata> {
        self.resources.scheduler.current_chord().map(Into::into)
    }

    pub fn resume_barrier_qpc_for_test(&self) -> Option<QpcTicks> {
        self.resources.scheduler.resume_barrier_qpc()
    }

    pub fn stage_candidate_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<DownChordMetadata>,
        fallback_ticks: TimelineTicks,
    ) -> Result<Option<StagedCandidate>, String> {
        let epoch = self.resources.playback.epoch;
        let requested_qpc = epoch
            .checked_add_duration(DurationTicks::from_raw(fallback_ticks.as_u64()))
            .map_err(|error| format!("live staging target overflow: {error}"))?;
        let decision = self
            .resources
            .scheduler
            .evaluate_at(requested_qpc, epoch)
            .map_err(|error| format!("live staging scheduler evaluation failed: {error}"))?;
        let decision = match decision {
            DispatchDecision::Wait { deadline_qpc } if deadline_qpc > requested_qpc => self
                .resources
                .scheduler
                .evaluate_at(deadline_qpc, epoch)
                .map_err(|error| format!("live staging deadline evaluation failed: {error}"))?,
            decision => decision,
        };
        let (actual_up_mask, actual_chord, timing_window): (
            u16,
            Option<DownChordMetadata>,
            DispatchTimingWindow,
        ) = match decision {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => (up_mask, down_chord.map(Into::into), timing_window),
            decision => {
                return Err(format!(
                    "production executor did not offer Send: {decision:?}"
                ));
            }
        };
        if actual_up_mask != up_mask
            || actual_chord.map(|chord| chord.down_mask) != down_chord.map(|chord| chord.down_mask)
        {
            return Err(format!(
                "staged masks do not match the live executor decision: requested up={up_mask:#06x}, down={:?}; live up={actual_up_mask:#06x}, down={:?}",
                down_chord.map(|chord| chord.down_mask),
                actual_chord.map(|chord| chord.down_mask),
            ));
        }
        self.stage_candidate_with_scheduler_window_for_test(
            actual_up_mask,
            actual_chord,
            timing_window,
        )
    }

    pub fn stage_candidate_with_scheduler_window_for_test(
        &mut self,
        up_mask: u16,
        down_chord: Option<DownChordMetadata>,
        timing_window: DispatchTimingWindow,
    ) -> Result<Option<StagedCandidate>, String> {
        stage_candidate(
            up_mask,
            down_chord,
            timing_window,
            &self.target,
            &mut self.resources,
            &mut self.runtime,
        )
    }

    pub fn dispatch_staged_candidate_for_test(
        &mut self,
        candidate: StagedCandidate,
        with_observer: bool,
    ) -> StagedDispatchOutcome {
        let qpc_clock = self.resources.clock;
        let context = StagedDispatchContext {
            target: &self.target,
            config: &self.config,
            control: StagedControlSignals {
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
        dispatch_staged_candidate(
            candidate,
            context,
            &mut self.resources,
            &mut self.runtime,
            &mut self.local_metrics,
            with_observer.then_some(&self.observer),
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
        self.configure_mock_cleanup_emitter_with_completion_qpc_for_test()
            .0
    }

    pub fn configure_mock_cleanup_emitter_with_completion_qpc_for_test(
        &mut self,
    ) -> (CleanupCapture, Arc<AtomicU64>) {
        let captured = Arc::new(Mutex::new(Vec::with_capacity(4)));
        let capture = Arc::clone(&captured);
        let completion = Arc::new(AtomicU64::new(0));
        let completion_store = Arc::clone(&completion);
        let clock = self.resources.clock;
        self.resources
            .backend
            .set_emitter(move |scan_codes, key_up| {
                capture
                    .lock()
                    .expect("cleanup capture lock")
                    .push((scan_codes.to_vec(), key_up));
                let now = clock.now().expect("cleanup QPC");
                completion_store.store(now.as_u64(), Ordering::Release);
                PlatformSendResult {
                    requested: scan_codes.len() as u8,
                    inserted: scan_codes.len() as u8,
                    started_ticks: now,
                    completed_ticks: Some(now),
                    win32_error: 0,
                    timing_error: None,
                }
            });
        (captured, completion)
    }

    pub fn set_mock_sender_start_qpc_for_test(&self, start_qpc: QpcTicks) {
        self.mock_sender_start_qpc
            .store(start_qpc.as_u64(), Ordering::Release);
    }

    pub fn generation_accounting_for_test(&self) -> GenerationAccounting {
        self.resources.scheduler.accounting()
    }

    pub fn release_obligation_mask_for_test(&self) -> u16 {
        self.resources.backend.release_obligation_mask()
    }

    pub fn apply_scheduler_resumable_lifecycle_transition_for_test(
        &mut self,
    ) -> Result<(), String> {
        let clock = self.resources.clock;
        self.apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(|| {
            clock
                .now()
                .map_err(|error| format!("test cleanup QPC: {error:?}"))
        })
    }

    pub fn apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(
        &mut self,
        post_cleanup_qpc: impl FnOnce() -> Result<QpcTicks, String>,
    ) -> Result<(), String> {
        crate::engine::worker::apply_scheduler_resumable_lifecycle_transition_for_test(
            &mut self.resources.backend,
            &mut self.resources.scheduler,
            &mut self.runtime,
            self.target.hwnd_for_safety_release(),
            post_cleanup_qpc,
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
        let min_release_gap_ticks = self
            .resources
            .clock
            .duration_from_us(timing.min_release_gap_us)
            .map_err(|error| format!("release gap conversion: {error:?}"))?;
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
            min_release_gap_us: timing.min_release_gap_us,
            min_release_gap_ticks: min_release_gap_ticks.as_u64(),
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
            cleanup_observation,
        )
    }
}

fn action(index: u32, kind: ActionKind, scheduled_us: u64, scan_codes: &[u16]) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
        reason: Arc::from("production executor fixture"),
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
