use super::super::shared::{SessionTarget, SupervisorLeaseState, SystemPowerState};
use super::super::{
    DurationTicks, QpcError, TimelineTicks, publish_lifecycle_metrics, try_publish_metrics,
};
use super::admission::{FinalControlAdmission, final_control_precheck};
use super::{
    CommandControl, CommandControlInput, CommandControlRuntime, CommandControlSignals,
    FinalControlSignals, FinalDownAdmissionResult, TargetStamp, WaitBoundary, WaitBoundaryInput,
    WaitDeadline, WaitMutable, WaitSignals, Worker, ensure_preflight_for_target, enter_focus_pause,
    focus_matches, focus_matches_hwnd, load_target_stamp, process_command_control,
    publish_backend_counters, publish_backend_metrics, target_stamp_still_current,
    verify_final_down_admission, wait_for_next_boundary,
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
    local: &super::WorkerMetricsLocal,
    shared: &super::SharedMetrics,
    qpc_clock: sky_dispatch_win32::clock::QpcClock,
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
) {
    let Ok(now_us) = qpc_clock.duration_to_us(DurationTicks::from_raw(now_qpc.as_u64())) else {
        return;
    };
    let _ = try_publish_metrics(local, shared, qpc_clock, now_us, false);
}

#[inline]
pub(crate) const fn startup_focus_loss_is_terminal(
    focus_ok: bool,
    musical_physical_commit_started: bool,
) -> bool {
    !focus_ok && !musical_physical_commit_started
}

#[inline]
pub(crate) const fn preroll_manual_pause_cancels(
    manual_pause: bool,
    musical_physical_commit_started: bool,
) -> bool {
    manual_pause && !musical_physical_commit_started
}

fn suspend_live_input_for_dispatch(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    scheduler: &mut sky_dispatch_core::executor::DispatchExecutor,
    target_hwnd: isize,
    _runtime: &super::WorkerRuntime,
    post_cleanup_qpc: impl FnOnce() -> Result<sky_dispatch_win32::clock::QpcTicks, String>,
) -> Result<(), String> {
    super::cleanup::suspend_scheduler_input(backend, scheduler, target_hwnd, || {
        let cleanup_qpc = post_cleanup_qpc()?;
        #[cfg(any(test, feature = "test-support"))]
        if let Some(hook) = _runtime.startup_ordering_hook.as_ref() {
            hook.record_resume_cleanup_qpc(cleanup_qpc.as_u64());
        }
        Ok(cleanup_qpc)
    })
    .map(|_| ())
}

#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_scheduler_resumable_lifecycle_transition_for_test(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    scheduler: &mut sky_dispatch_core::executor::DispatchExecutor,
    runtime: &mut super::WorkerRuntime,
    target_hwnd: isize,
    post_cleanup_qpc: impl FnOnce() -> Result<sky_dispatch_win32::clock::QpcTicks, String>,
) -> Result<(), String> {
    suspend_live_input_for_dispatch(backend, scheduler, target_hwnd, runtime, post_cleanup_qpc)
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
    pub(crate) preflight_target: Option<TargetStamp>,
    pub(crate) source_action_index: u32,
    pub(crate) trace_kind: u8,
    pub(crate) up_slot_timings: SmallVec<[sky_dispatch_core::executor::UpSlotTiming; 15]>,
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

pub(crate) struct StagedControlSignals<'a> {
    pub(crate) focus_active: &'a AtomicBool,
    pub(crate) quit_requested: &'a AtomicBool,
    pub(crate) skip_requested: &'a AtomicBool,
    pub(crate) panic_requested: &'a AtomicBool,
    pub(crate) desired_pause: &'a AtomicBool,
    pub(crate) supervisor_expired: &'a SupervisorLeaseState,
    pub(crate) system_power: &'a SystemPowerState,
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
    target: &SessionTarget,
    resources: &mut super::WorkerResources,
    runtime: &mut super::WorkerRuntime,
) -> Result<Option<StagedCandidate>, String> {
    let down_mask = down_chord.map_or(0, |c| c.down_mask);
    let has_down = down_mask != 0;
    let packet_masks = sky_dispatch_win32::input::PhysicalPacket::new(up_mask, down_mask);

    let preflight_target = if has_down {
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
        if !target_stamp_still_current(target, target_stamp) {
            runtime.verified_target = None;
            return Ok(None);
        }
        Some(target_stamp)
    } else {
        None
    };

    let up_slot_timings = if up_mask != 0 {
        resources.scheduler.up_slot_timings(up_mask)
    } else {
        SmallVec::new()
    };

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
    } else if let Some(first_up) = up_slot_timings.first() {
        first_up.source_action_index
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
        preflight_target,
        source_action_index,
        trace_kind,
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
) -> StagedDispatchOutcome {
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

    if has_down {
        let expected = candidate
            .preflight_target
            .expect("target stamp checked in staging");
        let control_signals = FinalControlSignals {
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
            supervisor_expired,
            system_power: Some(system_power),
        };
        match verify_final_down_admission(
            target,
            expected,
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
        ) {
            FinalDownAdmissionResult::Allowed => {}
            FinalDownAdmissionResult::Replan => {
                match stage_up_only_for_stale_mixed_candidate(
                    &candidate,
                    target,
                    config.focus.require_focus,
                    focus_active,
                    FinalControlSignals {
                        quit_requested,
                        skip_requested,
                        panic_requested,
                        desired_pause,
                        supervisor_expired,
                        system_power: Some(system_power),
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
                    Ok(None) => return StagedDispatchOutcome::Replan,
                    Err(error) => {
                        runtime.terminal_error = Some(error);
                        return StagedDispatchOutcome::Exit;
                    }
                }
            }
            FinalDownAdmissionResult::Terminal(err) => {
                runtime.terminal_error = Some(gate_rejection_record(&candidate, &err));
                return StagedDispatchOutcome::Exit;
            }
        }
    } else {
        let control_signals = FinalControlSignals {
            quit_requested,
            skip_requested,
            panic_requested,
            desired_pause,
            supervisor_expired,
            system_power: Some(system_power),
        };
        match final_control_precheck(control_signals) {
            FinalControlAdmission::Allowed => {}
            FinalControlAdmission::PauseRequested => return StagedDispatchOutcome::Replan,
            FinalControlAdmission::SystemSuspendRequested => {
                runtime.terminal_trigger = Some(super::control::TerminalTrigger::SystemSuspend);
                return StagedDispatchOutcome::Exit;
            }
            FinalControlAdmission::PanicRequested
            | FinalControlAdmission::QuitRequested
            | FinalControlAdmission::SkipRequested => return StagedDispatchOutcome::Exit,
        }
    }

    if candidate.up_mask != 0 && candidate.up_slot_timings.is_empty() {
        runtime.terminal_error =
            Some("missing Up slot timing attribution in staged candidate".to_string());
        return StagedDispatchOutcome::Exit;
    }

    let previous_sendinput_completion_qpc = runtime.last_physical_send_completion_qpc;
    let previous_sender_completion_known = runtime.last_physical_send_completion_qpc.is_some();
    runtime.previous_sendinput_completion_qpc = previous_sendinput_completion_qpc;
    runtime.previous_sender_completion_known = previous_sender_completion_known;

    let send_result = resources
        .backend
        .send_prepared_physical_packet_at_final_boundary_deferred_commit(
            &candidate.prepared_packet,
        );

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

    if system_power.system_suspend_terminal() {
        runtime.terminal_trigger = Some(super::control::TerminalTrigger::SystemSuspend);
        return StagedDispatchOutcome::Exit;
    }

    if matches!(
        send_result.status,
        sky_dispatch_win32::input::SendTransactionStatus::IntegrityLost
    ) {
        runtime.verified_target = None;
        runtime.terminal_error = Some(format!(
            "SendInput split authored chord: {}",
            sender_failure_record(&candidate, &send_result)
        ));
        return StagedDispatchOutcome::Exit;
    }
    if !send_result.is_success() {
        runtime.terminal_error = Some(format!(
            "SendInput transport failure: {}",
            sender_failure_record(&candidate, &send_result),
        ));
        return StagedDispatchOutcome::Exit;
    }

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

    let completion_qpc = match send_result.evidence.completed_ticks {
        Some(ticks) => ticks,
        None => match qpc_clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                runtime.terminal_error =
                    Some(format!("SendInput completion QPC unavailable: {error:?}"));
                return StagedDispatchOutcome::Exit;
            }
        },
    };

    if send_result.evidence.attempts != 0 {
        local_metrics.observe_precision_packet_timing(
            crate::engine::telemetry::metrics::PrecisionPacketTimingObservation {
                authored_target_qpc: candidate.timing_window.authored_target_qpc.as_u64(),
                packet_not_before_qpc: candidate.timing_window.packet_not_before_qpc.as_u64(),
                pre_call_qpc: send_result
                    .evidence
                    .started_ticks
                    .map(|ticks| ticks.as_u64()),
                sendinput_completion_qpc: Some(completion_qpc.as_u64()),
                previous_sendinput_completion_qpc: previous_sendinput_completion_qpc
                    .map(|ticks| ticks.as_u64()),
                previous_sender_completion_known,
                transport_anomaly: !send_result.is_success()
                    || send_result.evidence.last_win32_error.is_some()
                    || send_result.evidence.skipped_mask != 0
                    || send_result.evidence.attempts != 1,
            },
        );
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
        return StagedDispatchOutcome::Exit;
    }
    if let Err(error) = resources
        .backend
        .commit_physical_packet(candidate.packet_masks.up_mask | candidate.packet_masks.down_mask)
    {
        runtime.terminal_error = Some(format!("transport journal commit failed: {error}"));
        return StagedDispatchOutcome::Exit;
    }

    if has_down {
        runtime.musical_physical_commit_started = true;
    }

    let pre_call_qpc = send_result.evidence.started_ticks;
    runtime.production_forensics.observe_packet_result(
        candidate.packet_masks,
        candidate.source_action_index,
        source_action_target_qpc,
        pre_call_qpc,
        completion_qpc,
        send_result.status,
        local_metrics,
    );

    if let Some(observer) = observer {
        let mut up_slots = if candidate.up_slot_timings.is_empty() {
            None
        } else {
            observer.acquire_up_slots()
        };
        if !candidate.up_slot_timings.is_empty() && up_slots.is_none() {
            local_metrics.observer_dropped_samples =
                local_metrics.observer_dropped_samples.saturating_add(1);
        }
        if let Some(storage) = up_slots.as_mut() {
            for (slot, timing) in storage
                .slots
                .as_mut()
                .expect("owned slot storage")
                .iter_mut()
                .zip(&candidate.up_slot_timings)
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
            up_slot_count: candidate.up_slot_timings.len() as u8,
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
            candidate.up_slot_timings.len(),
            local_metrics,
        );
    } else if !candidate.up_slot_timings.is_empty() {
        account_unobserved_up_slots(local_metrics, candidate.up_slot_timings.len());
    }
    super::dispatch::publish_pending_wait_observation(runtime, observer, local_metrics);

    if metrics.live_diagnostics_enabled.load(Ordering::Relaxed) {
        publish_backend_counters(&resources.backend, local_metrics);
        publish_live_metrics_after_dispatch(local_metrics, metrics, qpc_clock, completion_qpc);
    }

    StagedDispatchOutcome::Dispatched
}

pub(super) fn dispatch(
    worker: &mut Worker<'_>,
    _focus_loss_fault: bool,
) -> Result<(), Box<dyn Any + Send>> {
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
        while core.runtime.terminal_error.is_none() {
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

            let now_ticks = qpc_ticks_or_terminal!();
            let focus_ok = focus_matches(config.focus.require_focus, focus_active);
            if startup_focus_loss_is_terminal(
                focus_ok,
                core.runtime.musical_physical_commit_started,
            ) {
                core.runtime.terminal_error = Some("focus_lost_during_preroll".to_string());
                break;
            }
            let manual_pause = desired_pause.load(Ordering::Acquire);
            let resume_wait_signals = FinalControlSignals {
                quit_requested,
                skip_requested,
                panic_requested,
                desired_pause,
                supervisor_expired,
                system_power: Some(system_power),
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
                core.runtime.musical_physical_commit_started,
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
                        if let Err(error) = suspend_live_input_for_dispatch(
                            &mut resources.backend,
                            &mut resources.scheduler,
                            preflight_target.hwnd,
                            &core.runtime,
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
                        if let Err(error) = suspend_live_input_for_dispatch(
                            &mut resources.backend,
                            &mut resources.scheduler,
                            preflight_target.hwnd,
                            &core.runtime,
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
                        hook(focus_active, target);
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
                    if let Err(error) = suspend_live_input_for_dispatch(
                        &mut resources.backend,
                        &mut resources.scheduler,
                        target.hwnd_for_safety_release(),
                        &core.runtime,
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
                        if let Err(error) = suspend_live_input_for_dispatch(
                            &mut resources.backend,
                            &mut resources.scheduler,
                            target.hwnd_for_safety_release(),
                            &core.runtime,
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

            let action = match evaluate_scheduler_action(
                &resources.scheduler,
                now_ticks,
                resources.playback.epoch,
            ) {
                Ok(action) => action,
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
            let (scheduler_wait_target_qpc, action_to_stage) =
                match project_scheduler_action_for_staging(
                    &resources.scheduler,
                    action,
                    now_ticks,
                    resources.playback.epoch,
                ) {
                    Ok(projected) => projected,
                    Err(error) => {
                        core.runtime.terminal_error = Some(error);
                        break;
                    }
                };
            let packet_not_before_qpc = match &action_to_stage {
                Some(SchedulerAction::Send { timing_window, .. }) => {
                    timing_window.packet_not_before_qpc
                }
                None | Some(SchedulerAction::Finished | SchedulerAction::Wait { .. }) => now_ticks,
            };
            let mut candidate = match action_to_stage {
                Some(send_action @ SchedulerAction::Send { .. }) => {
                    match stage_scheduler_action_preserving_due_up(
                        send_action,
                        target,
                        config.focus.require_focus,
                        focus_active,
                        FinalControlSignals {
                            quit_requested,
                            skip_requested,
                            panic_requested,
                            desired_pause,
                            supervisor_expired,
                            system_power: Some(system_power),
                        },
                        resources,
                        &mut core.runtime,
                    ) {
                        Ok(Some(candidate)) => Some(candidate),
                        Ok(None) => {
                            match wait_after_unstable_target(
                                now_ticks,
                                packet_not_before_qpc,
                                qpc_clock,
                                &resources.waiter,
                                interrupt,
                                &mut core.metrics,
                                &mut core.runtime.terminal_error,
                            ) {
                                WaitBoundary::Due { .. } => continue,
                                WaitBoundary::Interrupted(_, _, _) => {
                                    let _ = interrupt.try_take();
                                    continue;
                                }
                                WaitBoundary::Exit => break,
                            }
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
                            let fallback = match stage_up_only_for_stale_mixed_candidate(
                                staged_candidate,
                                target,
                                config.focus.require_focus,
                                focus_active,
                                FinalControlSignals {
                                    quit_requested,
                                    skip_requested,
                                    panic_requested,
                                    desired_pause,
                                    supervisor_expired,
                                    system_power: Some(system_power),
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
                                    .and_then(|candidate| candidate.preflight_target)
                                    .is_some_and(|expected| {
                                        !target_stamp_still_current(target, expected)
                                    });
                                let controls_allow_wait = matches!(
                                    final_control_precheck(FinalControlSignals {
                                        quit_requested,
                                        skip_requested,
                                        panic_requested,
                                        desired_pause,
                                        supervisor_expired,
                                        system_power: Some(system_power),
                                    }),
                                    FinalControlAdmission::Allowed
                                );
                                if target_stale && controls_allow_wait {
                                    match wait_after_unstable_target(
                                        now_ticks,
                                        candidate.as_ref().map_or(
                                            packet_not_before_qpc,
                                            |candidate| {
                                                candidate.timing_window.packet_not_before_qpc
                                            },
                                        ),
                                        qpc_clock,
                                        &resources.waiter,
                                        interrupt,
                                        &mut core.metrics,
                                        &mut core.runtime.terminal_error,
                                    ) {
                                        WaitBoundary::Due { .. } => continue,
                                        WaitBoundary::Interrupted(_, _, _) => {
                                            let _ = interrupt.try_take();
                                            continue;
                                        }
                                        WaitBoundary::Exit => break,
                                    }
                                }
                                continue;
                            }
                        }
                        StagedDispatchOutcome::Exit => break,
                    }
                }
                if wait_target_qpc > now_ticks {
                    #[cfg(any(test, feature = "test-support"))]
                    if candidate
                        .as_ref()
                        .is_some_and(|candidate| candidate.packet_masks.down_mask != 0)
                        && let Some(hook) = core.runtime.startup_ordering_hook.as_ref()
                        && let Some(candidate) = candidate.as_ref()
                    {
                        hook.record_down_window_before_physical_wait(
                            candidate.timing_window.authored_target_qpc.as_u64(),
                            candidate.timing_window.packet_not_before_qpc.as_u64(),
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
            let candidate_preflight_target = candidate.preflight_target;
            let candidate_retry_deadline = candidate.timing_window.packet_not_before_qpc;
            match dispatch_staged_candidate(
                candidate,
                ctx,
                resources,
                &mut core.runtime,
                &mut core.metrics,
                core.observer.pending.as_ref(),
            ) {
                StagedDispatchOutcome::Dispatched => continue,
                StagedDispatchOutcome::Replan => {
                    core.runtime.last_dispatch_deadline_wake_qpc = None;
                    core.runtime.last_dispatch_deadline_target_qpc = None;
                    super::dispatch::publish_pending_wait_observation(
                        &mut core.runtime,
                        core.observer.pending.as_ref(),
                        &mut core.metrics,
                    );
                    let target_stale = candidate_preflight_target
                        .is_some_and(|expected| !target_stamp_still_current(target, expected));
                    let controls_allow_wait = matches!(
                        final_control_precheck(FinalControlSignals {
                            quit_requested,
                            skip_requested,
                            panic_requested,
                            desired_pause,
                            supervisor_expired,
                            system_power: Some(system_power),
                        }),
                        FinalControlAdmission::Allowed
                    );
                    if target_stale && controls_allow_wait {
                        let replan_now = match qpc_clock.now() {
                            Ok(now) => now,
                            Err(error) => {
                                core.runtime.terminal_error =
                                    Some(format!("target replan QPC unavailable: {error:?}"));
                                break;
                            }
                        };
                        match wait_after_unstable_target(
                            replan_now,
                            candidate_retry_deadline,
                            qpc_clock,
                            &resources.waiter,
                            interrupt,
                            &mut core.metrics,
                            &mut core.runtime.terminal_error,
                        ) {
                            WaitBoundary::Due { .. } => {}
                            WaitBoundary::Interrupted(_, _, _) => {
                                let _ = interrupt.try_take();
                            }
                            WaitBoundary::Exit => break,
                        }
                    }
                    continue;
                }
                StagedDispatchOutcome::Exit => {
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

    use crate::engine::test_support::ProductionDispatchTestHarness;
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};

    use sky_dispatch_win32::input::{
        PacketRetryReason, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
    };

    use std::sync::atomic::Ordering;

    include!("dispatch_loop_observer_tests.rs");
    include!("dispatch_loop_failure_tests.rs");
}
