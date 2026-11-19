use super::super::shared::{SessionTarget, SupervisorLeaseState, SystemPowerState};
use super::super::{
    DurationTicks, QpcError, TimelineTicks, publish_lifecycle_metrics,
    try_publish_metrics_with_update,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use super::admission::verify_final_down_admission;
use super::admission::{
    FinalControlAdmission, MusicalPacketKind, final_control_precheck, verify_final_packet_admission,
};
use super::{
    CommandControl, CommandControlInput, CommandControlRuntime, CommandControlSignals,
    FinalControlSignals, FinalDownAdmissionResult, TargetStamp, WaitBoundary, WaitBoundaryInput,
    WaitDeadline, WaitMutable, WaitSignals, Worker, ensure_preflight_for_target, enter_focus_pause,
    focus_matches, focus_matches_hwnd, load_target_stamp, process_command_control,
    publish_backend_counters, publish_backend_metrics, record_wait_failure,
    target_stamp_still_current, wait_for_next_boundary,
};
#[cfg(any(test, feature = "test-support"))]
use crate::engine::test_support::PacketSenderAttemptEvidence;
use sky_dispatch_core::clock::PauseReason;
use smallvec::SmallVec;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

fn publish_down_observation(
    observer: &crate::engine::worker::dispatch::observer::PendingObservationQueue,
    observation: crate::engine::worker::dispatch::observation::DispatchObservation,
    up_slot_count: usize,
    local_metrics: &mut super::WorkerMetricsLocal,
) {
    local_metrics.up_slot_records_expected = local_metrics
        .up_slot_records_expected
        .saturating_add(up_slot_count as u64);
    let up_slot_storage_present = match &observation {
        crate::engine::worker::dispatch::observation::DispatchObservation::Down(observation) => {
            observation.up_slots.is_some()
        }
        _ => up_slot_count == 0,
    };
    if !observer.push(observation, &mut local_metrics.observer_dropped_samples) {
        if up_slot_storage_present {
            local_metrics.observer_dropped_up_slot_records = local_metrics
                .observer_dropped_up_slot_records
                .saturating_add(up_slot_count as u64);
        } else {
            local_metrics.observer_dropped_up_slot_pool_records = local_metrics
                .observer_dropped_up_slot_pool_records
                .saturating_add(up_slot_count as u64);
        }
    }
}

fn account_unobserved_up_slots(
    local_metrics: &mut super::WorkerMetricsLocal,
    up_slot_count: usize,
) {
    let count = up_slot_count as u64;
    local_metrics.up_slot_records_expected =
        local_metrics.up_slot_records_expected.saturating_add(count);
    local_metrics.up_slot_records_not_captured_by_mode = local_metrics
        .up_slot_records_not_captured_by_mode
        .saturating_add(count);
}

fn paused_focus_grace_deadline(
    focus_pause_active: bool,
    focus_active: bool,
    target_available: bool,
    restored_at: Option<sky_dispatch_win32::clock::QpcTicks>,
    grace_ticks: DurationTicks,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    if !focus_pause_active || !focus_active || !target_available {
        return Ok(None);
    }
    restored_at
        .map(|start| {
            start
                .checked_add_duration(grace_ticks)
                .map_err(|error| format!("focus grace deadline overflow: {error}"))
        })
        .transpose()
}

#[allow(clippy::too_many_arguments)]
fn paused_wait_state_is_stable(
    desired_pause: &AtomicBool,
    expected_desired_pause: bool,
    focus_active: &AtomicBool,
    expected_focus_active: bool,
    require_focus: bool,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &SupervisorLeaseState,
    system_power: &SystemPowerState,
    target: &SessionTarget,
    expected_target: Option<(isize, u64)>,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    expected_wake_generation: u64,
) -> bool {
    desired_pause.load(Ordering::Acquire) == expected_desired_pause
        && focus_matches(require_focus, focus_active) == expected_focus_active
        && !quit_requested.load(Ordering::Acquire)
        && !skip_requested.load(Ordering::Acquire)
        && !panic_requested.load(Ordering::Acquire)
        && !supervisor_expired.is_expired()
        && !system_power.down_blocked()
        && target.load_stable() == expected_target
        && interrupt.signal_generation() == expected_wake_generation
}

#[allow(clippy::too_many_arguments)]
fn wait_for_blocked_resume_replan(
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
    control_signals: FinalControlSignals<'_>,
    expected_desired_pause: bool,
    focus_active: &AtomicBool,
    expected_focus_active: bool,
    require_focus: bool,
    target: &SessionTarget,
    expected_target: Option<TargetStamp>,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
    waiter: &sky_dispatch_win32::wait::HybridWaiter,
    interrupt: &sky_dispatch_win32::event::OwnedEvent,
    local_metrics: &mut super::WorkerMetricsLocal,
    terminal_error: &mut Option<String>,
) -> bool {
    let Some(system_power) = control_signals.system_power else {
        *terminal_error = Some("resume replan is missing system power state".to_string());
        return false;
    };
    let wake_generation = interrupt.signal_generation();
    if !paused_wait_state_is_stable(
        control_signals.desired_pause,
        expected_desired_pause,
        focus_active,
        expected_focus_active,
        require_focus,
        control_signals.quit_requested,
        control_signals.skip_requested,
        control_signals.panic_requested,
        control_signals.supervisor_expired,
        system_power,
        target,
        expected_target.map(|stamp| (stamp.hwnd, stamp.generation)),
        interrupt,
        wake_generation,
    ) {
        return true;
    }

    match wait_after_unstable_target(
        now_qpc,
        now_qpc,
        qpc_clock,
        waiter,
        interrupt,
        local_metrics,
        terminal_error,
    ) {
        WaitBoundary::Due { .. } => true,
        WaitBoundary::Interrupted(_, _, _) => {
            let _ = interrupt.try_take();
            true
        }
        WaitBoundary::Exit => false,
    }
}

pub(crate) fn publish_live_metrics_after_dispatch(
    local: &mut super::WorkerMetricsLocal,
    backend: &sky_dispatch_win32::input::TrackedKeyState,
    shared: &super::SharedMetrics,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
) {
    let Ok(now_us) = qpc_clock.duration_to_us(DurationTicks::from_raw(now_qpc.as_u64())) else {
        return;
    };
    let _ = try_publish_metrics_with_update(local, shared, qpc_clock, now_us, false, |local| {
        publish_backend_counters(backend, local)
    });
}

#[inline]
pub(crate) const fn preroll_manual_pause_cancels(
    manual_pause: bool,
    musical_physical_attempt_started: bool,
) -> bool {
    manual_pause && !musical_physical_attempt_started
}

#[cfg(test)]
fn suspend_live_input_for_dispatch(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    scheduler: &mut sky_dispatch_core::executor::DispatchExecutor,
    cleanup_target: super::cleanup::CleanupTarget<'_>,
    #[cfg(any(test, feature = "test-support"))] runtime: &super::WorkerRuntime,
    post_cleanup_qpc: impl FnOnce() -> Result<sky_dispatch_win32::clock::QpcTicks, String>,
) -> Result<(), String> {
    suspend_live_input_for_dispatch_with_control(
        backend,
        scheduler,
        cleanup_target,
        #[cfg(any(test, feature = "test-support"))]
        runtime,
        || false,
        post_cleanup_qpc,
    )
}

fn suspend_live_input_for_dispatch_with_control(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    scheduler: &mut sky_dispatch_core::executor::DispatchExecutor,
    cleanup_target: super::cleanup::CleanupTarget<'_>,
    #[cfg(any(test, feature = "test-support"))] runtime: &super::WorkerRuntime,
    terminal_requested: impl Fn() -> bool,
    post_cleanup_qpc: impl FnOnce() -> Result<sky_dispatch_win32::clock::QpcTicks, String>,
) -> Result<(), String> {
    let session_release_owner = backend.session_release_owner_proof().cloned();
    let mut drain_control = || {
        let stamp = cleanup_target.expected_target.ok_or_else(|| {
            "target generation unavailable while draining tracked safety obligations".to_string()
        })?;
        let release_owner = session_release_owner.as_ref().ok_or_else(|| {
            "session release owner proof is missing while draining tracked obligations".to_string()
        })?;
        super::admission::safety_cleanup_drain_decision(
            cleanup_target.target,
            stamp,
            release_owner,
            cleanup_target.require_owner_identity,
            terminal_requested(),
        )
    };
    super::cleanup::suspend_scheduler_input(
        backend,
        scheduler,
        cleanup_target,
        &mut drain_control,
        || {
            let cleanup_qpc = post_cleanup_qpc()?;
            #[cfg(any(test, feature = "test-support"))]
            if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
                hook.record_resume_cleanup_qpc(cleanup_qpc.as_u64());
            }
            Ok(cleanup_qpc)
        },
    )
    .map(|_| ())
}

#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_scheduler_resumable_lifecycle_transition_for_test(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    scheduler: &mut sky_dispatch_core::executor::DispatchExecutor,
    runtime: &mut super::WorkerRuntime,
    target: &SessionTarget,
    expected_target: Option<TargetStamp>,
    require_owner_identity: bool,
    target_hwnd: isize,
    post_cleanup_qpc: impl FnOnce() -> Result<sky_dispatch_win32::clock::QpcTicks, String>,
) -> Result<(), String> {
    super::cleanup::suspend_scheduler_input(
        backend,
        scheduler,
        super::cleanup::CleanupTarget {
            verification_hwnd: target_hwnd,
            ..super::cleanup::CleanupTarget::for_session(
                target,
                expected_target,
                require_owner_identity,
            )
        },
        || Ok(sky_dispatch_win32::input::CleanupDrainDecision::default()),
        || {
            let cleanup_qpc = post_cleanup_qpc()?;
            if let Some(hook) = runtime.startup_ordering_hook.as_ref() {
                hook.record_resume_cleanup_qpc(cleanup_qpc.as_u64());
            }
            Ok(cleanup_qpc)
        },
    )
    .map(|_| ())
}

#[derive(Debug)]
pub struct StagedCandidate {
    pub(crate) up_mask: u16,
    pub(crate) down_chord: Option<sky_dispatch_core::executor::DownChordMetadata>,
    pub(crate) packet_masks: sky_dispatch_win32::input::PhysicalPacket,
    pub(crate) prepared_packet: sky_dispatch_win32::input::PreparedPhysicalPacket,
    pub(crate) authored_target_ticks: TimelineTicks,
    pub(crate) authored_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    pub(crate) down_authored_target_ticks: Option<TimelineTicks>,
    pub(crate) down_authored_target_qpc: Option<sky_dispatch_win32::clock::QpcTicks>,
    pub(crate) timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    /// Stable HWND/generation snapshot captured during staging. Down-bearing
    /// packets also pass physical-key preflight against this exact stamp.
    pub(crate) target_stamp: Option<TargetStamp>,
    pub(crate) source_action_index: u32,
    pub(crate) trace_kind: u8,
    pub(crate) up_slot_timing_summary: Option<sky_dispatch_core::executor::UpSlotTimingSummary>,
    pub(crate) up_slot_timings: Option<SmallVec<[sky_dispatch_core::executor::UpSlotTiming; 15]>>,
}

fn proven_raw_zero_down_only(
    candidate: &StagedCandidate,
    outcome: sky_dispatch_win32::input::SendTransactionOutcome,
) -> bool {
    let packet = candidate.packet_masks;
    let evidence = outcome.evidence;
    let Some(started) = evidence.started_ticks else {
        return false;
    };
    let Some(completed) = evidence.completed_ticks else {
        return false;
    };

    candidate.up_mask == 0
        && packet.up_mask == 0
        && packet.down_mask != 0
        && candidate.down_chord.is_some_and(|chord| {
            chord.source_action_index == candidate.source_action_index
                && chord.down_mask == packet.down_mask
        })
        && packet.event_count() == packet.down_mask.count_ones() as u8
        && outcome.status == sky_dispatch_win32::input::SendTransactionStatus::ZeroProgress
        && evidence.requested_mask == packet.down_mask
        && evidence.confirmed_mask == 0
        && evidence.skipped_mask == 0
        && evidence.first_inserted == 0
        && evidence.attempts == 1
        && evidence.zero_progress_retries == 0
        && matches!(
            evidence.retry_reason,
            sky_dispatch_win32::input::PacketRetryReason::None
                | sky_dispatch_win32::input::PacketRetryReason::ZeroProgress
        )
        && evidence.first_win32_error == evidence.last_win32_error
        && evidence.timing_error.is_none()
        && completed.checked_duration_since(started).is_ok()
}

fn down_authored_target(
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
    down_chord: Option<sky_dispatch_core::executor::DownChordMetadata>,
) -> Result<Option<(TimelineTicks, sky_dispatch_win32::clock::QpcTicks)>, String> {
    down_chord
        .map(|chord| {
            let target_qpc = epoch_qpc
                .checked_add_duration(DurationTicks::from_raw(chord.scheduled_ticks.as_u64()))
                .map_err(|error| format!("Down authored target QPC overflow: {error}"))?;
            Ok((chord.scheduled_ticks, target_qpc))
        })
        .transpose()
}

fn source_action_target(
    packet_ticks: TimelineTicks,
    packet_qpc: sky_dispatch_win32::clock::QpcTicks,
    down_target: Option<(TimelineTicks, sky_dispatch_win32::clock::QpcTicks)>,
) -> (TimelineTicks, sky_dispatch_win32::clock::QpcTicks) {
    down_target.unwrap_or((packet_ticks, packet_qpc))
}

include!("dispatch_loop_scheduler.rs");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StagedDispatchOutcome {
    Dispatched,
    Replan,
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DispatchAttemptOutcome {
    Dispatched,
    ModifierHeld,
    Replan,
    Exit,
}

impl DispatchAttemptOutcome {
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn public_outcome(self) -> StagedDispatchOutcome {
        match self {
            Self::Dispatched => StagedDispatchOutcome::Dispatched,
            Self::ModifierHeld => StagedDispatchOutcome::Replan,
            Self::Replan => StagedDispatchOutcome::Replan,
            Self::Exit => StagedDispatchOutcome::Exit,
        }
    }
}

pub(crate) struct StagedControlSignals<'a> {
    pub(crate) focus_active: &'a AtomicBool,
    pub(crate) quit_requested: &'a AtomicBool,
    pub(crate) skip_requested: &'a AtomicBool,
    pub(crate) panic_requested: &'a AtomicBool,
    pub(crate) desired_pause: &'a AtomicBool,
    pub(crate) supervisor_expired: &'a SupervisorLeaseState,
    pub(crate) system_power: &'a SystemPowerState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DispatchReplanAction {
    ContinueImmediately,
    WaitForModifier,
    Exit,
}

struct DispatchReplanWaitContext<'a> {
    target: &'a SessionTarget,
    control_signals: FinalControlSignals<'a>,
    modifier_held: bool,
}

/// Apply the worker-loop policy after final packet admission requests replan.
/// A target invalidation is terminal. Modifier-only rejection waits are used
/// only after any independent due Up fallback has been dispatched.
fn dispatch_replan_action(context: DispatchReplanWaitContext<'_>) -> DispatchReplanAction {
    let controls_allow_wait = matches!(
        final_control_precheck(context.control_signals),
        FinalControlAdmission::Allowed
    );
    if !controls_allow_wait {
        return DispatchReplanAction::ContinueImmediately;
    }
    if !context.target.is_valid() {
        return DispatchReplanAction::Exit;
    }
    if context.modifier_held {
        return DispatchReplanAction::WaitForModifier;
    }
    DispatchReplanAction::ContinueImmediately
}

pub(crate) struct StagedDispatchContext<'a> {
    pub(crate) target: &'a SessionTarget,
    pub(crate) config: &'a super::WorkerConfig,
    pub(crate) control: StagedControlSignals<'a>,
    pub(crate) timing: &'a super::WorkerTimingState,
    pub(crate) qpc_clock: sky_dispatch_win32::clock::QpcClock,
    pub(crate) progress_clock: &'a crate::engine::shared::SharedProgressClock,
    pub(crate) metrics: &'a super::SharedMetrics,
}

pub(crate) fn stage_candidate(
    up_mask: u16,
    down_chord: Option<sky_dispatch_core::executor::DownChordMetadata>,
    timing_window: sky_dispatch_core::executor::DispatchTimingWindow,
    capture_up_slot_timings: bool,
    target: &SessionTarget,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
) -> Result<Option<StagedCandidate>, String> {
    let down_mask = down_chord.map_or(0, |c| c.down_mask);
    let has_down = down_mask != 0;
    let packet_masks = sky_dispatch_win32::input::PhysicalPacket::new(up_mask, down_mask);

    let target_stamp = if has_down {
        let Some(target_stamp) = load_target_stamp(target) else {
            runtime.verified_target = None;
            return Ok(None);
        };
        if let Err(error) = ensure_preflight_for_target(
            &resources.backend,
            target_stamp,
            &mut runtime.verified_target,
        ) {
            runtime.verified_target = None;
            return Err(format!(
                "instrument key preflight failed before SendInput: {error}"
            ));
        }
        if !target.is_valid() {
            runtime.verified_target = None;
            return Ok(None);
        }
        Some(target_stamp)
    } else if up_mask != 0 {
        let Some(target_stamp) = load_target_stamp(target) else {
            runtime.verified_target = None;
            return Ok(None);
        };
        Some(target_stamp)
    } else {
        None
    };

    let up_slot_timing_summary = resources
        .scheduler
        .up_slot_timing_summary(up_mask, resources.playback.epoch)
        .map_err(|error| format!("Up slot timing summary arithmetic failure: {error}"))?;
    let up_slot_timings = capture_up_slot_timings.then(|| {
        if up_mask != 0 {
            resources.scheduler.up_slot_timings(up_mask)
        } else {
            SmallVec::new()
        }
    });
    if let Some(timings) = &up_slot_timings
        && timings.len()
            != usize::from(up_slot_timing_summary.map_or(0, |summary| summary.slot_count))
    {
        return Err("Up slot timing summary disagrees with observer attribution".to_string());
    }

    let authored_target_qpc = timing_window.authored_target_qpc;
    let authored_target_ticks = authored_target_qpc
        .checked_duration_since(resources.playback.epoch)
        .map_err(|error| format!("authored target precedes current playback epoch: {error}"))?
        .as_u64();
    let authored_target_ticks = TimelineTicks::from_raw(authored_target_ticks);
    let down_target = down_authored_target(resources.playback.epoch, down_chord)?;

    let prepared_packet =
        match sky_dispatch_win32::input::PreparedPhysicalPacket::try_new_with_profile(
            packet_masks,
            resources.backend.instrument_key_profile(),
        ) {
            Ok(p) => p,
            Err(err) => return Err(format!("failed to prepare physical packet: {err}")),
        };
    let trace_kind = match (up_mask != 0, down_mask != 0) {
        (true, true) => crate::engine::TRACE_KIND_MIXED,
        (false, true) => crate::engine::TRACE_KIND_DOWN,
        _ => crate::engine::TRACE_KIND_UP,
    };
    let source_action_index = if has_down {
        down_chord.map_or(0, |c| c.source_action_index)
    } else if let Some(summary) = up_slot_timing_summary {
        summary.first_source_action_index
    } else {
        0
    };

    Ok(Some(StagedCandidate {
        up_mask,
        down_chord,
        packet_masks,
        prepared_packet,
        authored_target_ticks,
        authored_target_qpc,
        down_authored_target_ticks: down_target.map(|target| target.0),
        down_authored_target_qpc: down_target.map(|target| target.1),
        timing_window,
        target_stamp,
        source_action_index,
        trace_kind,
        up_slot_timing_summary,
        up_slot_timings,
    }))
}

pub(crate) fn dispatch_staged_candidate(
    candidate: StagedCandidate,
    ctx: StagedDispatchContext<'_>,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
    local_metrics: &mut super::WorkerMetricsLocal,
    observer: Option<&super::dispatch::PendingObservationQueue>,
) -> DispatchAttemptOutcome {
    let target = ctx.target;
    let config = ctx.config;
    let focus_active = ctx.control.focus_active;
    let quit_requested = ctx.control.quit_requested;
    let skip_requested = ctx.control.skip_requested;
    let panic_requested = ctx.control.panic_requested;
    let desired_pause = ctx.control.desired_pause;
    let supervisor_expired = ctx.control.supervisor_expired;
    let system_power = ctx.control.system_power;
    let timing = ctx.timing;
    let qpc_clock = ctx.qpc_clock;
    let progress_clock = ctx.progress_clock;
    let metrics = ctx.metrics;
    let has_down = candidate.packet_masks.down_mask != 0;

    #[cfg(any(test, feature = "test-support"))]
    if has_down && let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.record_down_window(
            candidate.timing_window.authored_target_qpc.as_u64(),
            candidate.timing_window.packet_not_before_qpc.as_u64(),
        );
        hook.visit_startup_checkpoint(super::super::config::StartupCheckpoint::FirstDownAdmission);
        hook.mark_first_physical_send_started();
        let source_idx = candidate
            .down_chord
            .as_ref()
            .map_or(0, |c| c.source_action_index);
        hook.maybe_inject_hol_stall(candidate.packet_masks.down_mask, source_idx, qpc_clock);
    }

    if candidate.packet_masks.up_mask != 0 && candidate.up_slot_timing_summary.is_none() {
        runtime.terminal_error =
            Some("missing Up slot timing summary in staged candidate".to_string());
        return DispatchAttemptOutcome::Exit;
    }
    if observer.is_some()
        && candidate.packet_masks.up_mask != 0
        && candidate.up_slot_timings.is_none()
    {
        runtime.terminal_error =
            Some("observer Up-slot attribution missing from staged candidate".to_string());
        return DispatchAttemptOutcome::Exit;
    }

    let control_signals = FinalControlSignals {
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        supervisor_expired,
        system_power: Some(system_power),
    };
    let expected_target = candidate
        .target_stamp
        .expect("musical packet target stamp is captured during staging");
    let up_origin = if candidate.packet_masks.up_mask != 0 {
        match resources
            .backend
            .release_owner_proof_for_mask(candidate.packet_masks.up_mask)
        {
            Ok(Some(proof)) => Some(proof),
            Ok(None) => {
                runtime.terminal_error = Some(format!(
                    "musical Up owner proof is missing for mask 0x{:04x}; release obligation retained",
                    candidate.packet_masks.up_mask
                ));
                return DispatchAttemptOutcome::Exit;
            }
            Err(error) => {
                runtime.terminal_error = Some(format!(
                    "musical Up owner proof is missing or conflicted for mask 0x{:04x}; release obligation retained: {error}",
                    candidate.packet_masks.up_mask
                ));
                return DispatchAttemptOutcome::Exit;
            }
        }
    } else {
        None
    };

    let packet_kind = MusicalPacketKind::from_masks(
        candidate.packet_masks.down_mask,
        candidate.packet_masks.up_mask,
    )
    .expect("musical packet has non-zero mask");

    if !packet_kind.has_down() {
        match final_control_precheck(control_signals) {
            FinalControlAdmission::Allowed => {}
            FinalControlAdmission::PauseRequested => return DispatchAttemptOutcome::Replan,
            FinalControlAdmission::SystemSuspendRequested => {
                runtime.terminal_trigger = Some(super::control::TerminalTrigger::SystemSuspend);
                return DispatchAttemptOutcome::Exit;
            }
            FinalControlAdmission::PanicRequested
            | FinalControlAdmission::QuitRequested
            | FinalControlAdmission::SkipRequested => return DispatchAttemptOutcome::Exit,
        }
    }

    let admission_result = verify_final_packet_admission(
        target,
        expected_target,
        config.focus.require_focus,
        focus_active,
        control_signals,
        timing.lease_timeout_ticks,
        qpc_clock,
        progress_clock,
        &resources.backend,
        &mut resources.playback,
        runtime,
        local_metrics,
        packet_kind,
    );

    match admission_result {
        FinalDownAdmissionResult::Allowed => {}
        FinalDownAdmissionResult::ModifierHeld => {
            if packet_kind.has_up() {
                match stage_up_only_for_mixed_replan(
                    &candidate,
                    SchedulerStageAdmission {
                        target,
                        require_focus: config.focus.require_focus,
                        focus_active,
                        capture_up_slot_timings: observer.is_some(),
                        control_signals,
                    },
                    resources,
                    runtime,
                ) {
                    Ok(Some(up_only)) => {
                        let fallback_ctx = StagedDispatchContext {
                            target,
                            config,
                            control: StagedControlSignals {
                                focus_active,
                                quit_requested,
                                skip_requested,
                                panic_requested,
                                desired_pause,
                                supervisor_expired,
                                system_power,
                            },
                            timing,
                            qpc_clock,
                            progress_clock,
                            metrics,
                        };
                        return dispatch_staged_candidate(
                            up_only,
                            fallback_ctx,
                            resources,
                            runtime,
                            local_metrics,
                            observer,
                        );
                    }
                    Ok(None) => return DispatchAttemptOutcome::ModifierHeld,
                    Err(error) => {
                        runtime.terminal_error = Some(error);
                        return DispatchAttemptOutcome::Exit;
                    }
                }
            } else {
                return DispatchAttemptOutcome::ModifierHeld;
            }
        }
        FinalDownAdmissionResult::Replan => {
            if packet_kind.has_up() && packet_kind.has_down() {
                match stage_up_only_for_mixed_replan(
                    &candidate,
                    SchedulerStageAdmission {
                        target,
                        require_focus: config.focus.require_focus,
                        focus_active,
                        capture_up_slot_timings: observer.is_some(),
                        control_signals,
                    },
                    resources,
                    runtime,
                ) {
                    Ok(Some(up_only)) => {
                        let fallback_ctx = StagedDispatchContext {
                            target,
                            config,
                            control: StagedControlSignals {
                                focus_active,
                                quit_requested,
                                skip_requested,
                                panic_requested,
                                desired_pause,
                                supervisor_expired,
                                system_power,
                            },
                            timing,
                            qpc_clock,
                            progress_clock,
                            metrics,
                        };
                        return dispatch_staged_candidate(
                            up_only,
                            fallback_ctx,
                            resources,
                            runtime,
                            local_metrics,
                            observer,
                        );
                    }
                    Ok(None) => return DispatchAttemptOutcome::Replan,
                    Err(error) => {
                        runtime.terminal_error = Some(error);
                        return DispatchAttemptOutcome::Exit;
                    }
                }
            } else {
                return DispatchAttemptOutcome::Replan;
            }
        }
        FinalDownAdmissionResult::Terminal(err) => {
            runtime.terminal_error = Some(gate_rejection_record(&candidate, &err));
            return DispatchAttemptOutcome::Exit;
        }
    }

    if let Some(up_origin) = up_origin.as_ref()
        && !resources
            .backend
            .session_release_owner_proof()
            .is_some_and(|release_owner| up_origin.same_owner(release_owner))
    {
        runtime.terminal_error = Some(format!(
            "musical Up origin owner differs from current packet owner; up_mask=0x{:04x}; release obligation retained",
            candidate.up_mask
        ));
        return DispatchAttemptOutcome::Exit;
    }

    let previous_sendinput_completion_qpc = runtime.last_physical_send_completion_qpc;
    let previous_sender_completion_known = runtime.last_physical_send_completion_qpc.is_some();

    let physical_packet = sky_dispatch_win32::input::PhysicalPacket::new(
        candidate.packet_masks.up_mask,
        candidate.packet_masks.down_mask,
    );
    if resources.backend.session_release_owner_proof().is_none() {
        runtime.terminal_error = Some(
            "session release owner proof is missing at the prepared packet sender".to_string(),
        );
        return DispatchAttemptOutcome::Exit;
    }
    let send_result = resources
        .backend
        .send_prepared_physical_packet_at_final_boundary_deferred_commit_for_target(
            &candidate.prepared_packet,
            expected_target.hwnd,
            expected_target.generation,
        );
    let complete_transport_evidence = send_result.has_complete_evidence_for(physical_packet);
    let valid_sender_timing_evidence = send_result.has_valid_sender_timing_evidence();
    let healthy_completion_evidence = complete_transport_evidence && valid_sender_timing_evidence;

    #[cfg(any(test, feature = "test-support"))]
    if send_result.evidence.attempts != 0
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.record_packet_sender_attempt(PacketSenderAttemptEvidence {
            has_down,
            has_up: candidate.packet_masks.up_mask != 0,
            sender_start_qpc_ticks: send_result
                .evidence
                .started_ticks
                .map(|ticks| ticks.as_u64()),
            authored_target_qpc_ticks: candidate.timing_window.authored_target_qpc.as_u64(),
            packet_not_before_qpc_ticks: candidate.timing_window.packet_not_before_qpc.as_u64(),
            wake_qpc_ticks: runtime
                .last_dispatch_deadline_wake_qpc
                .map(|ticks| ticks.as_u64()),
        });
    }

    local_metrics.observe_precision_packet_result(
        crate::engine::telemetry::metrics::PrecisionPacketTimingObservation {
            sendinput_attempts: send_result.evidence.attempts,
            healthy_completion: healthy_completion_evidence,
            authored_target_qpc: candidate.timing_window.authored_target_qpc.as_u64(),
            packet_not_before_qpc: candidate.timing_window.packet_not_before_qpc.as_u64(),
            pre_call_qpc: send_result
                .evidence
                .started_ticks
                .map(|ticks| ticks.as_u64()),
            sendinput_completion_qpc: send_result
                .evidence
                .completed_ticks
                .map(|ticks| ticks.as_u64()),
            previous_sendinput_completion_qpc: previous_sendinput_completion_qpc
                .map(|ticks| ticks.as_u64()),
            previous_sender_completion_known,
            transport_anomaly: !healthy_completion_evidence
                || send_result.evidence.first_win32_error.is_some()
                || send_result.evidence.last_win32_error.is_some()
                || send_result.evidence.skipped_mask != 0
                || send_result.evidence.attempts != 1
                || send_result.evidence.started_ticks.is_none()
                || send_result.evidence.completed_ticks.is_none()
                || send_result.evidence.timing_error.is_some(),
        },
    );

    if has_down && send_result.evidence.attempts != 0 {
        // Crossing the syscall boundary consumes this authored onset even if
        // the sender cannot prove count or timing completion.
        runtime.musical_physical_attempt_started = true;
    }

    if system_power.system_suspend_terminal() {
        runtime.terminal_trigger = Some(super::control::TerminalTrigger::SystemSuspend);
        return DispatchAttemptOutcome::Exit;
    }
    if !healthy_completion_evidence {
        runtime
            .terminal_trigger
            .get_or_insert(super::control::TerminalTrigger::SenderAnomaly);
    }
    if send_result.evidence.attempts == 0 {
        runtime.terminal_error = Some(format!(
            "SendInput rejected before syscall; authored onset remains unattempted: {}",
            sender_failure_record(&candidate, &send_result),
        ));
        return DispatchAttemptOutcome::Exit;
    }

    if !healthy_completion_evidence {
        let anomaly_record = sender_failure_record(&candidate, &send_result);
        runtime
            .attempted_dispatch_anomalies
            .record(format!("post-call attempted anomaly: {anomaly_record}"));
        // A prior successful sender timestamp is not evidence about this
        // anomalous syscall and must not be attributed to the next packet.
        runtime.last_physical_send_completion_qpc = None;

        let Some(expected_target) = candidate.target_stamp else {
            runtime.verified_target = None;
            runtime.terminal_error = Some(format!(
                "post-call anomaly has no retained target stamp; release remains pending: {anomaly_record}"
            ));
            return DispatchAttemptOutcome::Exit;
        };
        if proven_raw_zero_down_only(&candidate, send_result) {
            let chord = candidate
                .down_chord
                .expect("proven raw-zero Down retains its authored chord identity");
            if let Err(error) = resources
                .scheduler
                .consume_attempted_down_after_proven_raw_zero(
                    chord.source_action_index,
                    chord.down_mask,
                )
            {
                let _ = suspend_live_input_for_dispatch_with_control(
                    &mut resources.backend,
                    &mut resources.scheduler,
                    super::cleanup::CleanupTarget::for_session(
                        target,
                        Some(expected_target),
                        config.focus.require_focus,
                    ),
                    #[cfg(any(test, feature = "test-support"))]
                    runtime,
                    || true,
                    || qpc_clock.now().map_err(|error| format!("{error:?}")),
                );
                runtime.terminal_error = Some(format!(
                    "proven-zero Down could not consume its attempted scheduler identity ({error}); tracked cleanup was requested: {anomaly_record}"
                ));
                return DispatchAttemptOutcome::Exit;
            }
            if let Err(error) = resources
                .backend
                .clear_proven_raw_zero_down_attempt(chord.down_mask)
            {
                let _ = suspend_live_input_for_dispatch_with_control(
                    &mut resources.backend,
                    &mut resources.scheduler,
                    super::cleanup::CleanupTarget::for_session(
                        target,
                        Some(expected_target),
                        config.focus.require_focus,
                    ),
                    #[cfg(any(test, feature = "test-support"))]
                    runtime,
                    || true,
                    || qpc_clock.now().map_err(|error| format!("{error:?}")),
                );
                runtime.terminal_error = Some(format!(
                    "proven-zero Down journal did not match its raw packet ({error}); tracked cleanup was requested: {anomaly_record}"
                ));
                return DispatchAttemptOutcome::Exit;
            }
            if resources.backend.release_obligation_mask() != 0
                && let Err(error) = suspend_live_input_for_dispatch_with_control(
                    &mut resources.backend,
                    &mut resources.scheduler,
                    super::cleanup::CleanupTarget::for_session(
                        target,
                        Some(expected_target),
                        config.focus.require_focus,
                    ),
                    #[cfg(any(test, feature = "test-support"))]
                    runtime,
                    || true,
                    || qpc_clock.now().map_err(|error| format!("{error:?}")),
                )
            {
                runtime.verified_target = None;
                runtime.terminal_error = Some(format!(
                    "proven-zero Down preserved prior obligations but cleanup failed ({error}): {anomaly_record}"
                ));
                return DispatchAttemptOutcome::Exit;
            }
            runtime.terminal_error = Some(format!(
                "musical playback terminated after proven raw-zero Down; no new release obligation was created: {anomaly_record}"
            ));
            return DispatchAttemptOutcome::Exit;
        }
        if resources.backend.release_obligation_mask() != 0
            && let Err(error) = suspend_live_input_for_dispatch_with_control(
                &mut resources.backend,
                &mut resources.scheduler,
                super::cleanup::CleanupTarget::for_session(
                    target,
                    Some(expected_target),
                    config.focus.require_focus,
                ),
                #[cfg(any(test, feature = "test-support"))]
                runtime,
                || true,
                || qpc_clock.now().map_err(|error| format!("{error:?}")),
            )
        {
            runtime.verified_target = None;
            runtime.terminal_error = Some(format!(
                "post-call anomaly tracked safety cleanup failed or was withheld ({error}): {anomaly_record}"
            ));
            return DispatchAttemptOutcome::Exit;
        }
        runtime
            .production_forensics
            .observe_lifecycle(super::dispatch::observation::ObserverLifecycle::ResetAll);

        if has_down {
            let chord = candidate
                .down_chord
                .expect("Down-bearing packet retains its authored chord identity");
            if let Err(error) = resources
                .scheduler
                .consume_attempted_down_after_verified_cleanup(
                    chord.source_action_index,
                    chord.down_mask,
                )
            {
                runtime.terminal_error = Some(format!(
                    "verified cleanup could not consume attempted Down ({error}): {anomaly_record}"
                ));
                return DispatchAttemptOutcome::Exit;
            }
        }

        runtime.terminal_error = Some(format!(
            "musical playback terminated after first sender anomaly: {anomaly_record}"
        ));
        return DispatchAttemptOutcome::Exit;
    }

    let Some(completion_qpc) = send_result.evidence.completed_ticks else {
        runtime.terminal_error = Some(format!(
            "healthy SendInput evidence lost its completion QPC: {}",
            sender_failure_record(&candidate, &send_result),
        ));
        return DispatchAttemptOutcome::Exit;
    };

    #[cfg(any(test, feature = "test-support"))]
    if candidate.up_mask != 0
        && candidate.packet_masks.down_mask == 0
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.wait_after_successful_up();
    }

    #[cfg(any(test, feature = "test-support"))]
    if has_down && let Some(hook) = runtime.startup_ordering_hook.as_ref() {
        hook.visit_startup_checkpoint(super::super::config::StartupCheckpoint::FirstDownCommitted);
    }
    let (source_action_target_ticks, source_action_target_qpc) = source_action_target(
        candidate.authored_target_ticks,
        candidate.authored_target_qpc,
        candidate
            .down_authored_target_ticks
            .zip(candidate.down_authored_target_qpc),
    );
    if let Some(started_qpc) = send_result.evidence.started_ticks {
        let _ = super::health::record_sendinput_pre_call_lateness(
            source_action_target_qpc,
            started_qpc,
            timing,
            local_metrics,
        );
    }
    runtime.last_physical_send_completion_qpc = Some(completion_qpc);

    let confirmed_up_mask = send_result.evidence.confirmed_mask & candidate.up_mask;
    let confirmed_down_mask =
        send_result.evidence.confirmed_mask & candidate.packet_masks.down_mask;

    if let Err(error) = resources.scheduler.on_send_completed_at(
        confirmed_up_mask,
        confirmed_down_mask,
        completion_qpc,
    ) {
        runtime.terminal_error = Some(format!("scheduler completion commit failed: {error}"));
        return DispatchAttemptOutcome::Exit;
    }
    if let Err(error) = resources
        .backend
        .commit_physical_packet(candidate.packet_masks.up_mask | candidate.packet_masks.down_mask)
    {
        runtime.terminal_error = Some(format!("transport journal commit failed: {error}"));
        return DispatchAttemptOutcome::Exit;
    }

    let pre_call_qpc = send_result.evidence.started_ticks;
    runtime.defer_production_packet_result(
        candidate.packet_masks,
        candidate.source_action_index,
        source_action_target_qpc,
        pre_call_qpc,
        completion_qpc,
        send_result.status,
    );

    if let Some(observer) = observer {
        let up_slot_count = usize::from(
            candidate
                .up_slot_timing_summary
                .map_or(0, |summary| summary.slot_count),
        );
        let up_slot_timings = candidate.up_slot_timings.as_deref().unwrap_or_default();
        let mut up_slots = if up_slot_count == 0 {
            None
        } else {
            observer.acquire_up_slots()
        };
        if up_slot_count != 0 && up_slots.is_none() {
            local_metrics.observer_dropped_samples =
                local_metrics.observer_dropped_samples.saturating_add(1);
        }
        if let Some(storage) = up_slots.as_mut() {
            for (slot, timing) in storage
                .slots
                .as_mut()
                .expect("owned slot storage")
                .iter_mut()
                .zip(up_slot_timings)
            {
                *slot = *timing;
            }
        }
        let wake_qpc = super::dispatch::take_deadline_wake_qpc(runtime);
        let precision_handoff = super::dispatch::precision_handoff(runtime, wake_qpc, None, None);
        let observation = crate::engine::worker::dispatch::observation::DownObservation {
            wait_sequence: runtime
                .pending_wait_observation
                .map_or(0, |wait| wait.sequence),
            wait_spin_ticks: runtime
                .pending_wait_observation
                .map_or(0, |wait| wait.spin_ticks.as_u64()),
            up_slots,
            up_slot_count: up_slot_count as u8,
            up_pre_call_available: send_result.evidence.started_ticks.is_some(),
            epoch_qpc: resources.playback.epoch,
            allow_pre_epoch_startup_dispatch: runtime.allow_pre_epoch_startup_dispatch,
            physical_target_qpc: candidate.authored_target_qpc,
            physical_timing_window: candidate.timing_window,
            final_policy_qpc: None,
            pre_call_qpc,
            sendinput_completion_qpc: completion_qpc,
            previous_sendinput_completion_qpc,
            previous_sender_completion_known,
            dispatch_ready_qpc: None,
            wake_qpc,
            precision_handoff,
            requested_packet: candidate.packet_masks,
            confirmed_mask: send_result.evidence.confirmed_mask,
            skipped_mask: send_result.evidence.skipped_mask,
            trace: crate::engine::worker::dispatch::observation::DownTraceObservation {
                event_index: candidate.source_action_index,
                compiled_packet_index: candidate.down_chord.map(|c| c.chord_index as u64),
                trace_kind: candidate.trace_kind,
                result_status: send_result.status,
                send_attempts: send_result.evidence.attempts,
                retry_reason: send_result.evidence.retry_reason,
                chord_integrity_lost: false,
                last_win32_error: send_result.evidence.last_win32_error.unwrap_or(0),
                authored_ticks: source_action_target_ticks,
                effective_deadline_ticks: source_action_target_ticks,
            },
        };
        publish_down_observation(
            observer,
            crate::engine::worker::dispatch::observation::DispatchObservation::Down(observation),
            up_slot_count,
            local_metrics,
        );
    } else if let Some(summary) = candidate.up_slot_timing_summary {
        account_unobserved_up_slots(local_metrics, usize::from(summary.slot_count));
    }
    super::dispatch::publish_pending_wait_observation(runtime, observer, local_metrics);

    if metrics.live_diagnostics_enabled.load(Ordering::Relaxed) {
        publish_live_metrics_after_dispatch(
            local_metrics,
            &resources.backend,
            metrics,
            qpc_clock,
            completion_qpc,
        );
    }

    DispatchAttemptOutcome::Dispatched
}

pub(super) fn dispatch(worker: &mut Worker<'_>) -> Result<(), Box<dyn Any + Send>> {
    let shared = worker.shared;
    let core = &mut worker.core;
    let config = &worker.config;
    let interrupt = &shared.commands.interrupt;
    let desired_pause = &shared.commands.desired_pause;
    let quit_requested = &shared.commands.quit_requested;
    let skip_requested = &shared.commands.skip_requested;
    let panic_requested = &shared.commands.panic_requested;
    let supervisor_expired = &shared.commands.supervisor_expired;
    let focus_active = &shared.commands.focus_active;
    let system_power = &shared.commands.system_power;
    let target = &shared.target;
    let metrics = &shared.publication.metrics;
    #[cfg(any(test, feature = "test-support"))]
    let command_timing = &shared.commands.command_timing;

    let timing = *core.timing.as_ref().expect("worker timing initialized");
    let qpc_clock = core
        .resources
        .as_ref()
        .expect("worker resources initialized")
        .clock;

    // Every QPC query after admission is part of the worker's correctness
    // boundary. A failed query is terminal and must take the cleanup path;
    // it must never become timestamp zero or a best-effort continuation.
    macro_rules! qpc_us_or_terminal {
        () => {{
            match qpc_clock.now().and_then(|ticks| {
                qpc_clock
                    .duration_to_us(DurationTicks::from_raw(ticks.as_u64()))
                    .map_err(|_| QpcError::ConversionOverflow)
            }) {
                Ok(value) => value,
                Err(error) => {
                    core.runtime.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    macro_rules! qpc_ticks_or_terminal {
        () => {{
            match qpc_clock.now() {
                Ok(value) => value,
                Err(error) => {
                    core.runtime.terminal_error = Some(format!("QPC runtime failure: {error:?}"));
                    break;
                }
            }
        }};
    }

    catch_unwind(AssertUnwindSafe(|| {
        if core.runtime.terminal_error.is_some() {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
            hook.visit_startup_checkpoint(super::super::config::StartupCheckpoint::InitialWait);
        }
        let resources = core
            .resources
            .as_mut()
            .expect("worker resources initialized");
        let qpc_clock = resources.clock;
        let mut pending_modifier_up_deadline_wake: Option<(
            sky_dispatch_win32::clock::QpcTicks,
            sky_dispatch_win32::clock::QpcTicks,
        )> = None;
        while core.runtime.terminal_error.is_none() {
            // Reduce the previous packet's fixed-size forensic record outside
            // the post-send precision path and before this iteration can
            // admit another physical packet.
            core.runtime.drain_production_forensics(&mut core.metrics);
            // A deadline-wake sample belongs to exactly one physical send.
            // Re-entering the non-precision loop clears any stale sample
            // after interrupts, replans, command transitions, or failures.
            core.runtime.last_dispatch_deadline_wake_qpc = None;
            core.runtime.last_dispatch_deadline_target_qpc = None;
            if let CommandControl::Exit = process_command_control(CommandControlInput {
                signals: CommandControlSignals {
                    quit_requested,
                    skip_requested,
                    panic_requested,
                    supervisor_expired,
                    system_power,
                },
                runtime: CommandControlRuntime {
                    terminal_trigger: &mut core.runtime.terminal_trigger,
                },
            }) {
                break;
            }
            if !target.is_valid() {
                core.runtime.terminal_error.get_or_insert_with(|| {
                    "immutable session target or owner identity was invalidated".to_string()
                });
                break;
            }

            let now_ticks = qpc_ticks_or_terminal!();
            let focus_ok = focus_matches(config.focus.require_focus, focus_active);
            let manual_pause = desired_pause.load(Ordering::Acquire);
            let resume_wait_signals = FinalControlSignals {
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                supervisor_expired,
                system_power: Some(system_power),
            };
            let cleanup_terminal_requested = || {
                quit_requested.load(Ordering::Acquire)
                    || skip_requested.load(Ordering::Acquire)
                    || panic_requested.load(Ordering::Acquire)
                    || supervisor_expired.is_expired()
                    || system_power.system_suspend_terminal()
            };
            #[cfg(any(test, feature = "test-support"))]
            if command_timing.needs_observation() {
                let observed_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.terminal_error =
                            Some(format!("QPC pause observation failed: {error:?}"));
                        break;
                    }
                };
                command_timing.observe_pause(observed_ticks);
            }

            if preroll_manual_pause_cancels(
                manual_pause,
                core.runtime.musical_physical_attempt_started,
            ) {
                core.runtime.terminal_error = Some("manual_pause_during_preroll".to_string());
                break;
            }

            if !focus_ok {
                let entered_focus_pause = match enter_focus_pause(
                    &mut resources.playback,
                    &mut core.runtime,
                    now_ticks,
                    &shared.publication.progress_clock,
                ) {
                    Ok(entered) => entered,
                    Err(error) => {
                        core.runtime.terminal_error = Some(error);
                        break;
                    }
                };
                if entered_focus_pause {
                    *core.errors.abort_counts.entry("focus_lost").or_insert(0) += 1;
                    #[cfg(any(test, feature = "test-support"))]
                    if let Some(hook) = core.runtime.focus_pause_hook.as_ref() {
                        hook();
                    }
                    publish_backend_metrics(
                        &resources.backend,
                        &mut core.metrics,
                        metrics,
                        &mut core.errors.last_published,
                    );
                    publish_lifecycle_metrics(
                        &core.metrics,
                        metrics,
                        qpc_clock,
                        qpc_us_or_terminal!(),
                    );
                }
            } else if resources.playback.has_pause_reason(PauseReason::Focus) {
                let restored_at = *core
                    .runtime
                    .focus_restore_started_ticks
                    .get_or_insert(now_ticks);
                let focus_grace_elapsed = match now_ticks.checked_duration_since(restored_at) {
                    Ok(elapsed) => elapsed,
                    Err(error) => {
                        core.runtime.terminal_error =
                            Some(format!("focus grace clock failure: {error}"));
                        break;
                    }
                };
                if focus_grace_elapsed >= timing.focus_restore_grace_ticks {
                    let Some(preflight_target) = load_target_stamp(target) else {
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        if !wait_for_blocked_resume_replan(
                            now_ticks,
                            resume_wait_signals,
                            manual_pause,
                            focus_active,
                            focus_matches(config.focus.require_focus, focus_active),
                            config.focus.require_focus,
                            target,
                            None,
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            break;
                        }
                        continue;
                    };
                    let manual_pause_active =
                        manual_pause || resources.playback.has_pause_reason(PauseReason::Manual);
                    core.runtime.verified_target = None;
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(target, preflight_target)
                    {
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        if !wait_for_blocked_resume_replan(
                            now_ticks,
                            resume_wait_signals,
                            manual_pause,
                            focus_active,
                            focus_matches(config.focus.require_focus, focus_active),
                            config.focus.require_focus,
                            target,
                            Some(preflight_target),
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            break;
                        }
                        continue;
                    }
                    if !manual_pause_active {
                        if (core.runtime.musical_physical_attempt_started
                            || resources.backend.release_obligation_mask() != 0)
                            && let Err(error) = suspend_live_input_for_dispatch_with_control(
                                &mut resources.backend,
                                &mut resources.scheduler,
                                super::cleanup::CleanupTarget::for_session(
                                    target,
                                    Some(preflight_target),
                                    config.focus.require_focus,
                                ),
                                #[cfg(any(test, feature = "test-support"))]
                                &core.runtime,
                                cleanup_terminal_requested,
                                || qpc_clock.now().map_err(|error| format!("{error:?}")),
                            )
                        {
                            core.runtime.verified_target = None;
                            core.runtime.terminal_error =
                                Some(format!("focus restoration failed: {error}"));
                            break;
                        }
                        core.runtime.manual_pause_suspension_pending = false;
                        core.runtime.production_forensics.observe_lifecycle(
                            super::dispatch::observation::ObserverLifecycle::ResetAll,
                        );
                        if let Err(error) = ensure_preflight_for_target(
                            &resources.backend,
                            preflight_target,
                            &mut core.runtime.verified_target,
                        ) {
                            core.runtime.verified_target = None;
                            core.runtime.terminal_error = Some(format!(
                                "instrument key preflight failed during focus restoration; release the 15 instrument keys before playback: {error}"
                            ));
                            break;
                        }
                    } else if core.runtime.manual_pause_suspension_pending {
                        if let Err(error) = suspend_live_input_for_dispatch_with_control(
                            &mut resources.backend,
                            &mut resources.scheduler,
                            super::cleanup::CleanupTarget::for_session(
                                target,
                                Some(preflight_target),
                                config.focus.require_focus,
                            ),
                            #[cfg(any(test, feature = "test-support"))]
                            &core.runtime,
                            cleanup_terminal_requested,
                            || qpc_clock.now().map_err(|error| format!("{error:?}")),
                        ) {
                            core.runtime.verified_target = None;
                            core.runtime.terminal_error =
                                Some(format!("focus restoration failed: {error}"));
                            break;
                        }
                        core.runtime.manual_pause_suspension_pending = false;
                        core.runtime.production_forensics.observe_lifecycle(
                            super::dispatch::observation::ObserverLifecycle::ResetAll,
                        );
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    if let Some(hook) = core.runtime.restore_race_hook.as_ref() {
                        hook(
                            focus_active,
                            target,
                            resources.scheduler.resume_barrier_qpc(),
                        );
                    }
                    let Some(post_restore_target) = load_target_stamp(target) else {
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        if !wait_for_blocked_resume_replan(
                            now_ticks,
                            resume_wait_signals,
                            manual_pause,
                            focus_active,
                            focus_matches(config.focus.require_focus, focus_active),
                            config.focus.require_focus,
                            target,
                            None,
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            break;
                        }
                        continue;
                    };
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        post_restore_target.hwnd,
                    ) || !target_stamp_still_current(target, preflight_target)
                        || post_restore_target != preflight_target
                    {
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        if !wait_for_blocked_resume_replan(
                            now_ticks,
                            resume_wait_signals,
                            manual_pause,
                            focus_active,
                            focus_matches(config.focus.require_focus, focus_active),
                            config.focus.require_focus,
                            target,
                            Some(post_restore_target),
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            break;
                        }
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = resources
                        .playback
                        .exit_pause(PauseReason::Focus, resumed_ticks)
                    {
                        core.runtime.verified_target = None;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    shared
                        .publication
                        .progress_clock
                        .publish(&resources.playback);
                    if desired_pause.load(Ordering::Acquire) {
                        core.runtime.verified_target = None;
                    }
                    core.runtime.focus_restore_started_ticks = None;
                    publish_backend_metrics(
                        &resources.backend,
                        &mut core.metrics,
                        metrics,
                        &mut core.errors.last_published,
                    );
                    publish_lifecycle_metrics(
                        &core.metrics,
                        metrics,
                        qpc_clock,
                        qpc_us_or_terminal!(),
                    );
                }
            }

            if manual_pause && !resources.playback.has_pause_reason(PauseReason::Manual) {
                core.runtime.verified_target = None;
                if !resources.playback.is_paused() {
                    if let Err(error) = suspend_live_input_for_dispatch_with_control(
                        &mut resources.backend,
                        &mut resources.scheduler,
                        super::cleanup::CleanupTarget::for_session(
                            target,
                            load_target_stamp(target),
                            config.focus.require_focus,
                        ),
                        #[cfg(any(test, feature = "test-support"))]
                        &core.runtime,
                        cleanup_terminal_requested,
                        || qpc_clock.now().map_err(|error| format!("{error:?}")),
                    ) {
                        core.runtime.terminal_error =
                            Some(format!("manual pause suspension failed: {error}"));
                        break;
                    }
                    core.runtime.manual_pause_suspension_pending = false;
                    core.runtime.production_forensics.observe_lifecycle(
                        super::dispatch::observation::ObserverLifecycle::ResetAll,
                    );
                    *core.errors.abort_counts.entry("manual_pause").or_insert(0) += 1;
                    publish_backend_metrics(
                        &resources.backend,
                        &mut core.metrics,
                        metrics,
                        &mut core.errors.last_published,
                    );
                    publish_lifecycle_metrics(
                        &core.metrics,
                        metrics,
                        qpc_clock,
                        qpc_us_or_terminal!(),
                    );
                } else if resources.playback.has_pause_reason(PauseReason::Focus) {
                    core.runtime.manual_pause_suspension_pending = true;
                }
                if let Err(error) = resources
                    .playback
                    .enter_pause(PauseReason::Manual, now_ticks)
                {
                    core.runtime.terminal_error = Some(format!("playback clock failure: {error}"));
                    break;
                }
                shared
                    .publication
                    .progress_clock
                    .publish(&resources.playback);
            } else if !manual_pause && resources.playback.has_pause_reason(PauseReason::Manual) {
                if !resources.playback.has_pause_reason(PauseReason::Focus) {
                    if core.runtime.manual_pause_suspension_pending {
                        if let Err(error) = suspend_live_input_for_dispatch_with_control(
                            &mut resources.backend,
                            &mut resources.scheduler,
                            super::cleanup::CleanupTarget::for_session(
                                target,
                                load_target_stamp(target),
                                config.focus.require_focus,
                            ),
                            #[cfg(any(test, feature = "test-support"))]
                            &core.runtime,
                            cleanup_terminal_requested,
                            || qpc_clock.now().map_err(|error| format!("{error:?}")),
                        ) {
                            core.runtime.terminal_error =
                                Some(format!("manual resume deferred suspension failed: {error}"));
                            break;
                        }
                        core.runtime.manual_pause_suspension_pending = false;
                        core.runtime.production_forensics.observe_lifecycle(
                            super::dispatch::observation::ObserverLifecycle::ResetAll,
                        );
                    }
                    let Some(preflight_target) = load_target_stamp(target) else {
                        core.runtime.verified_target = None;
                        if !wait_for_blocked_resume_replan(
                            now_ticks,
                            resume_wait_signals,
                            manual_pause,
                            focus_active,
                            focus_matches(config.focus.require_focus, focus_active),
                            config.focus.require_focus,
                            target,
                            None,
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            break;
                        }
                        continue;
                    };
                    if let Err(error) = ensure_preflight_for_target(
                        &resources.backend,
                        preflight_target,
                        &mut core.runtime.verified_target,
                    ) {
                        core.runtime.verified_target = None;
                        core.runtime.terminal_error = Some(format!(
                            "instrument key preflight failed on manual resume; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(target, preflight_target)
                    {
                        core.runtime.verified_target = None;
                        if !wait_for_blocked_resume_replan(
                            now_ticks,
                            resume_wait_signals,
                            manual_pause,
                            focus_active,
                            focus_matches(config.focus.require_focus, focus_active),
                            config.focus.require_focus,
                            target,
                            Some(preflight_target),
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            break;
                        }
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = resources
                        .playback
                        .exit_pause(PauseReason::Manual, resumed_ticks)
                    {
                        core.runtime.verified_target = None;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                    shared
                        .publication
                        .progress_clock
                        .publish(&resources.playback);
                } else {
                    core.runtime.verified_target = None;
                }
            }

            #[cfg(any(test, feature = "test-support"))]
            let pause_ack_pending = core
                .runtime
                .startup_ordering_hook
                .as_ref()
                .is_some_and(|hook| hook.pause_ack_pending());
            #[cfg(any(test, feature = "test-support"))]
            if resources.playback.has_pause_reason(PauseReason::Manual)
                && (command_timing.needs_acknowledgment() || pause_ack_pending)
            {
                let acknowledged_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.terminal_error =
                            Some(format!("QPC pause acknowledgment failed: {error:?}"));
                        break;
                    }
                };
                if command_timing.needs_acknowledgment() {
                    command_timing.acknowledge_pause(acknowledged_ticks);
                }
                if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
                    hook.record_pause_ack_qpc(acknowledged_ticks.as_u64());
                }
            }

            let paused = resources.playback.is_paused();
            metrics.is_paused.store(paused, Ordering::Relaxed);
            if paused {
                let target_snapshot = target.load_stable();
                let focus_snapshot = focus_matches(config.focus.require_focus, focus_active);
                let focus_grace_deadline = match paused_focus_grace_deadline(
                    resources.playback.has_pause_reason(PauseReason::Focus),
                    focus_snapshot,
                    target_snapshot.is_some(),
                    core.runtime.focus_restore_started_ticks,
                    timing.focus_restore_grace_ticks,
                ) {
                    Ok(deadline) => deadline,
                    Err(error) => {
                        core.runtime.terminal_error = Some(error);
                        break;
                    }
                };
                if focus_grace_deadline.is_some_and(|deadline| deadline <= now_ticks) {
                    continue;
                }
                let wake_generation = interrupt.signal_generation();
                if !paused_wait_state_is_stable(
                    desired_pause,
                    manual_pause,
                    focus_active,
                    focus_snapshot,
                    config.focus.require_focus,
                    quit_requested,
                    skip_requested,
                    panic_requested,
                    supervisor_expired,
                    system_power,
                    target,
                    target_snapshot,
                    interrupt,
                    wake_generation,
                ) {
                    continue;
                }
                if let Some(deadline_qpc) = focus_grace_deadline {
                    match wait_for_next_boundary(WaitBoundaryInput {
                        deadline: WaitDeadline {
                            physical_target_qpc: Some(deadline_qpc),
                            spin_threshold_ticks: DurationTicks::ZERO,
                            qpc_clock,
                        },
                        signals: WaitSignals {
                            waiter: &resources.waiter,
                            interrupt,
                        },
                        mutable: WaitMutable {
                            local_metrics: &mut core.metrics,
                            terminal_error: &mut core.runtime.terminal_error,
                        },
                    }) {
                        WaitBoundary::Due { .. } => {}
                        WaitBoundary::Interrupted(_, _, _) => {
                            let _ = interrupt.try_take();
                        }
                        WaitBoundary::Exit => break,
                    }
                } else if let Err(win32_error) = interrupt.wait_indefinitely() {
                    core.runtime.terminal_error = Some(format!(
                        "paused worker event wait failed (Win32 error {win32_error})"
                    ));
                    break;
                }
                // Return through the state machine after a wake/deadline so
                // controls, focus, target, power and lease are rechecked.
                continue;
            }

            let (action, scheduler_wait_target_qpc, action_to_stage) =
                match evaluate_scheduler_action_for_staging(
                    &resources.scheduler,
                    now_ticks,
                    resources.playback.epoch,
                ) {
                    Ok(projected) => projected,
                    Err(error) => {
                        core.runtime.terminal_error = Some(error);
                        break;
                    }
                };
            if matches!(&action, SchedulerAction::Finished) {
                break;
            }
            if matches!(&action, SchedulerAction::Wait { deadline_qpc } if *deadline_qpc <= now_ticks)
            {
                continue;
            }
            let mut candidate = match action_to_stage {
                Some(send_action @ SchedulerAction::Send { .. }) => {
                    match stage_scheduler_send_candidate(
                        send_action,
                        SchedulerStageAdmission {
                            target,
                            require_focus: config.focus.require_focus,
                            focus_active,
                            capture_up_slot_timings: core.observer.pending.is_some(),
                            control_signals: FinalControlSignals {
                                quit_requested,
                                skip_requested,
                                panic_requested,
                                desired_pause,
                                supervisor_expired,
                                system_power: Some(system_power),
                            },
                        },
                        resources,
                        &mut core.runtime,
                    ) {
                        Ok(Some(candidate)) => Some(candidate),
                        Ok(None) => {
                            core.runtime.terminal_error.get_or_insert_with(|| {
                                if target.is_valid() {
                                    "staging rejected the immutable session target".to_string()
                                } else {
                                    "immutable session target or owner identity was invalidated"
                                        .to_string()
                                }
                            });
                            break;
                        }
                        Err(error) => {
                            core.runtime.terminal_error = Some(error);
                            break;
                        }
                    }
                }
                None | Some(SchedulerAction::Finished | SchedulerAction::Wait { .. }) => None,
            };
            let mut wait_target_qpc =
                candidate
                    .as_ref()
                    .map_or(scheduler_wait_target_qpc, |candidate| {
                        scheduler_wait_target_qpc.max(candidate.timing_window.packet_not_before_qpc)
                    });
            // Carry an Up-deadline wake for one selector pass only, and attach
            // it only to a packet with an Up at that exact release deadline.
            attribute_modifier_up_deadline_wake(
                pending_modifier_up_deadline_wake.take(),
                candidate.as_ref(),
                &mut core.runtime,
            );
            if wait_target_qpc > now_ticks {
                if let Some(staged_candidate) = candidate.as_ref() {
                    let control_signals = FinalControlSignals {
                        quit_requested,
                        skip_requested,
                        panic_requested,
                        desired_pause,
                        supervisor_expired,
                        system_power: Some(system_power),
                    };
                    match pre_wait_candidate_admission(
                        staged_candidate,
                        target,
                        config.focus.require_focus,
                        focus_active,
                        control_signals,
                        &mut core.runtime,
                    ) {
                        StagedDispatchOutcome::Dispatched => {}
                        StagedDispatchOutcome::Replan => {
                            let target_stale = candidate
                                .as_ref()
                                .and_then(|candidate| candidate.target_stamp)
                                .is_some_and(|expected| {
                                    !target_stamp_still_current(target, expected)
                                });
                            if target_stale {
                                core.runtime.verified_target = None;
                                core.runtime.terminal_error.get_or_insert_with(|| {
                                    if target.is_valid() {
                                        "staged target stamp no longer matches its immutable session binding"
                                            .to_string()
                                    } else {
                                        "immutable session target or owner identity was invalidated"
                                            .to_string()
                                    }
                                });
                                break;
                            }
                            let fallback = match stage_up_only_for_mixed_replan(
                                staged_candidate,
                                SchedulerStageAdmission {
                                    target,
                                    require_focus: config.focus.require_focus,
                                    focus_active,
                                    capture_up_slot_timings: core.observer.pending.is_some(),
                                    control_signals: FinalControlSignals {
                                        quit_requested,
                                        skip_requested,
                                        panic_requested,
                                        desired_pause,
                                        supervisor_expired,
                                        system_power: Some(system_power),
                                    },
                                },
                                resources,
                                &mut core.runtime,
                            ) {
                                Ok(fallback) => fallback,
                                Err(error) => {
                                    core.runtime.terminal_error = Some(error);
                                    break;
                                }
                            };
                            if let Some(fallback) = fallback {
                                wait_target_qpc = scheduler_wait_target_qpc
                                    .max(fallback.timing_window.packet_not_before_qpc);
                                candidate = Some(fallback);
                            } else {
                                let target_stale = candidate
                                    .as_ref()
                                    .and_then(|candidate| candidate.target_stamp)
                                    .is_some_and(|expected| {
                                        !target_stamp_still_current(target, expected)
                                    });
                                if target_stale {
                                    core.runtime.verified_target = None;
                                    core.runtime.terminal_error.get_or_insert_with(|| {
                                        if target.is_valid() {
                                            "staged target stamp no longer matches its immutable session binding"
                                                .to_string()
                                        } else {
                                            "immutable session target or owner identity was invalidated"
                                                .to_string()
                                        }
                                    });
                                    break;
                                }
                                continue;
                            }
                        }
                        StagedDispatchOutcome::Exit => break,
                    }
                }
                if wait_target_qpc > now_ticks {
                    #[cfg(any(test, feature = "test-support"))]
                    if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
                        hook.visit_startup_checkpoint(
                            super::super::config::StartupCheckpoint::DispatchWait,
                        );
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    if candidate
                        .as_ref()
                        .is_some_and(|candidate| candidate.packet_masks.down_mask != 0)
                        && let Some(hook) = core.runtime.startup_ordering_hook.as_ref()
                        && let Some(candidate) = candidate.as_ref()
                    {
                        hook.record_down_window(
                            candidate.timing_window.authored_target_qpc.as_u64(),
                            candidate.timing_window.packet_not_before_qpc.as_u64(),
                        );
                        hook.visit_startup_checkpoint(
                            super::super::config::StartupCheckpoint::PhysicalWait,
                        );
                    }
                    let authored_target_ticks = candidate.as_ref().map_or_else(
                        || {
                            TimelineTicks::from_raw(
                                wait_target_qpc
                                    .as_u64()
                                    .saturating_sub(resources.playback.epoch.as_u64()),
                            )
                        },
                        |candidate| candidate.authored_target_ticks,
                    );

                    match wait_for_next_boundary(WaitBoundaryInput {
                        deadline: WaitDeadline {
                            physical_target_qpc: Some(wait_target_qpc),
                            spin_threshold_ticks: if candidate.is_some() {
                                timing.effective_spin_threshold_ticks
                            } else {
                                DurationTicks::ZERO
                            },
                            qpc_clock,
                        },
                        signals: WaitSignals {
                            waiter: &resources.waiter,
                            interrupt,
                        },
                        mutable: WaitMutable {
                            local_metrics: &mut core.metrics,
                            terminal_error: &mut core.runtime.terminal_error,
                        },
                    }) {
                        WaitBoundary::Due {
                            wait_result,
                            target_qpc,
                            dispatch_qpc,
                            planned_wait_ticks,
                        } => {
                            super::dispatch::publish_pending_wait_observation(
                                &mut core.runtime,
                                core.observer.pending.as_ref(),
                                &mut core.metrics,
                            );
                            core.runtime.pending_wait_observation =
                                super::dispatch::record_wait_attempt(
                                    core.observer.pending.as_ref(),
                                    wait_result,
                                    target_qpc,
                                    planned_wait_ticks,
                                    authored_target_ticks,
                                    resources.playback.epoch,
                                    None,
                                    &mut core.metrics,
                                );
                            if let Some(wait_result) = wait_result {
                                core.runtime.last_dispatch_deadline_wake_qpc = wait_result.wake_qpc;
                                core.runtime.last_dispatch_deadline_target_qpc = Some(target_qpc);
                            } else {
                                core.runtime.last_dispatch_deadline_wake_qpc = Some(dispatch_qpc);
                                core.runtime.last_dispatch_deadline_target_qpc = Some(target_qpc);
                            }
                        }
                        WaitBoundary::Interrupted(wait_result, target_qpc, planned_wait_ticks) => {
                            core.runtime.last_dispatch_deadline_wake_qpc = None;
                            core.runtime.last_dispatch_deadline_target_qpc = None;
                            super::dispatch::publish_interrupted_physical_wait(
                                &mut core.runtime,
                                core.observer.pending.as_ref(),
                                &mut core.metrics,
                                wait_result,
                                target_qpc,
                                planned_wait_ticks,
                                authored_target_ticks,
                                resources.playback.epoch,
                            );
                            let _ = interrupt.try_take();
                            continue;
                        }
                        WaitBoundary::Exit => {
                            super::dispatch::publish_pending_wait_observation(
                                &mut core.runtime,
                                core.observer.pending.as_ref(),
                                &mut core.metrics,
                            );
                            break;
                        }
                    }
                }
            }
            let Some(candidate) = candidate else {
                continue;
            };
            let ctx = StagedDispatchContext {
                target,
                config,
                control: StagedControlSignals {
                    focus_active,
                    quit_requested,
                    skip_requested,
                    panic_requested,
                    desired_pause,
                    supervisor_expired,
                    system_power,
                },
                timing: &timing,
                qpc_clock,
                progress_clock: &shared.publication.progress_clock,
                metrics,
            };
            match dispatch_staged_candidate(
                candidate,
                ctx,
                resources,
                &mut core.runtime,
                &mut core.metrics,
                core.observer.pending.as_ref(),
            ) {
                DispatchAttemptOutcome::Dispatched => continue,
                DispatchAttemptOutcome::Replan => {
                    super::dispatch::publish_pending_wait_observation(
                        &mut core.runtime,
                        core.observer.pending.as_ref(),
                        &mut core.metrics,
                    );
                    let replan_action = dispatch_replan_action(DispatchReplanWaitContext {
                        target,
                        control_signals: FinalControlSignals {
                            quit_requested,
                            skip_requested,
                            panic_requested,
                            desired_pause,
                            supervisor_expired,
                            system_power: Some(system_power),
                        },
                        modifier_held: false,
                    });
                    match replan_action {
                        DispatchReplanAction::ContinueImmediately => {}
                        DispatchReplanAction::WaitForModifier => {
                            unreachable!("generic replans cannot request modifier-specific backoff")
                        }
                        DispatchReplanAction::Exit => {
                            if !target.is_valid() {
                                core.runtime.terminal_error.get_or_insert_with(|| {
                                    "immutable session target or owner identity was invalidated"
                                        .to_string()
                                });
                            }
                            break;
                        }
                    }
                    continue;
                }
                DispatchAttemptOutcome::ModifierHeld => {
                    super::dispatch::publish_pending_wait_observation(
                        &mut core.runtime,
                        core.observer.pending.as_ref(),
                        &mut core.metrics,
                    );
                    let replan_action = dispatch_replan_action(DispatchReplanWaitContext {
                        target,
                        control_signals: FinalControlSignals {
                            quit_requested,
                            skip_requested,
                            panic_requested,
                            desired_pause,
                            supervisor_expired,
                            system_power: Some(system_power),
                        },
                        modifier_held: true,
                    });
                    match replan_action {
                        DispatchReplanAction::ContinueImmediately => {}
                        DispatchReplanAction::WaitForModifier => {
                            let now_qpc = qpc_ticks_or_terminal!();
                            let next_active_up_deadline = resources
                                .scheduler
                                .earliest_active_up_due_qpc(resources.playback.epoch)
                                .map_err(|error| {
                                    format!("modifier-held active Up deadline failed: {error}")
                                });
                            let next_active_up_deadline = match next_active_up_deadline {
                                Ok(deadline) => deadline,
                                Err(error) => {
                                    core.runtime.terminal_error = Some(error);
                                    break;
                                }
                            };
                            let modifier_wait = wait_after_modifier_held(
                                now_qpc,
                                next_active_up_deadline,
                                qpc_clock,
                                |clock, deadline, spin_threshold| {
                                    resources.waiter.wait_until_ticks_with_metrics_typed(
                                        clock,
                                        deadline,
                                        spin_threshold,
                                        interrupt,
                                    )
                                },
                            );
                            let modifier_wait = match modifier_wait {
                                Ok(wait) => wait,
                                Err(error) => {
                                    core.runtime.terminal_error = Some(error);
                                    break;
                                }
                            };
                            let interrupted = matches!(
                                modifier_wait,
                                ModifierWaitResult::Waited {
                                    result: sky_dispatch_win32::wait::WaitResult {
                                        outcome: sky_dispatch_win32::wait::WaitOutcome::Interrupted,
                                        ..
                                    },
                                    ..
                                }
                            );
                            match account_modifier_wait_result(
                                modifier_wait,
                                &mut core.runtime,
                                core.observer.pending.as_ref(),
                                &mut core.metrics,
                                resources.playback.epoch,
                            ) {
                                Ok(Some(wake)) => {
                                    pending_modifier_up_deadline_wake = Some(wake);
                                }
                                Ok(None) => {}
                                Err(failure) => {
                                    record_wait_failure(
                                        failure,
                                        &mut core.metrics,
                                        &mut core.runtime.terminal_error,
                                    );
                                    break;
                                }
                            }
                            if interrupted {
                                let _ = interrupt.try_take();
                            }
                        }
                        DispatchReplanAction::Exit => {
                            if !target.is_valid() {
                                core.runtime.terminal_error.get_or_insert_with(|| {
                                    "immutable session target or owner identity was invalidated"
                                        .to_string()
                                });
                            }
                            break;
                        }
                    }
                    continue;
                }
                DispatchAttemptOutcome::Exit => {
                    super::dispatch::publish_pending_wait_observation(
                        &mut core.runtime,
                        core.observer.pending.as_ref(),
                        &mut core.metrics,
                    );
                    break;
                }
            }
        }
        super::dispatch::publish_pending_wait_observation(
            &mut core.runtime,
            core.observer.pending.as_ref(),
            &mut core.metrics,
        );
    }))
}

#[cfg(test)]
mod tests {
    #[path = "../../dispatch_loop_deadline_hg_projection_tests.rs"]
    mod deadline_hg_projection;
    #[path = "../../dispatch_loop_deadline_late_due_tests.rs"]
    mod deadline_late_due;
    #[path = "../../dispatch_loop_deadline_post_due_control_tests.rs"]
    mod deadline_post_due_control;
    #[path = "../../dispatch_loop_deadline_real_waiter_tests.rs"]
    mod deadline_real_waiter;
    #[path = "../../dispatch_loop_failure_tests.rs"]
    mod failure;
    #[path = "../../dispatch_loop_observer_tests.rs"]
    mod observer;
    #[path = "../../dispatch_loop_owner_proof_tests.rs"]
    mod owner_proof;
    #[path = "../../dispatch_loop_raw_zero_tests.rs"]
    mod raw_zero;
    #[path = "../../dispatch_loop_replan_timing_tests.rs"]
    mod replan_timing;
    #[path = "../../dispatch_loop_safety_tests.rs"]
    mod safety;
    #[path = "../../dispatch_loop_test_support.rs"]
    mod support;
    #[path = "../../dispatch_loop_transport_outcome_tests.rs"]
    mod transport_outcome;
}
