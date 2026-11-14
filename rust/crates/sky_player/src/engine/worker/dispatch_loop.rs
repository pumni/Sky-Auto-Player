use super::super::shared::{SessionTarget, SupervisorLeaseState, SystemPowerState};
use super::super::{
    DurationTicks, QpcError, TimelineTicks, publish_lifecycle_metrics, try_publish_metrics,
};
use super::admission::{FinalControlAdmission, final_control_precheck};
#[cfg(any(test, feature = "test-support"))]
use super::dispatch::{DownBoundaryAdmission, PhysicalBoundaryStamp};
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

#[cfg(any(test, feature = "test-support"))]
fn physical_target_qpc_for_work(
    target: Option<sky_dispatch_win32::clock::QpcTicks>,
    now: sky_dispatch_win32::clock::QpcTicks,
    allow_pre_deadline: bool,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    let Some(target) = target else {
        return Ok(None);
    };
    Ok((allow_pre_deadline || target <= now).then_some(target))
}

#[inline]
#[cfg(any(test, feature = "test-support"))]
fn physical_boundary_stamp(
    plan: &super::planning::NextDispatchPlan,
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Option<PhysicalBoundaryStamp> {
    let physical = plan.physical()?;
    (physical.authored_view.packet_masks.down_mask != 0).then_some(PhysicalBoundaryStamp {
        first_batch_index: physical.authored_view.prepared_batch.index,
        packet_index: physical.authored_view.prepared_batch.packet_index,
        packet_batch_count: physical.authored_view.prepared_batch.packet_batch_count,
        source_action_index: physical.authored_view.batch_source_action_index,
        up_mask: physical.authored_view.packet_masks.up_mask,
        down_mask: physical.authored_view.packet_masks.down_mask,
        physical_target_qpc,
    })
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn physical_wait_target_for_plan(
    plan: &super::planning::NextDispatchPlan,
    runtime: &super::WorkerRuntime,
) -> Result<Option<sky_dispatch_win32::clock::QpcTicks>, String> {
    let Some(physical_target_qpc) = plan.physical_target_qpc() else {
        return Ok(None);
    };
    let Some(physical) = plan.physical() else {
        return Ok(Some(physical_target_qpc));
    };
    let masks = physical.authored_view.packet_masks;
    let guard = runtime
        .physical_timing_guard
        .as_ref()
        .ok_or_else(|| "physical timing guard is not initialized".to_string())?;
    let window = physical_timing_window_for_packet(
        guard,
        physical_target_qpc,
        masks.up_mask,
        masks.down_mask,
    )
    .map_err(|error| format!("physical timing window query failed: {error:?}"))?;
    let boundary = physical_boundary_stamp(plan, physical_target_qpc);
    let wait_target = if let Some(pending) = runtime.pending_up_recovery.as_ref() {
        if !boundary.is_some_and(|boundary| pending.matches_authored_boundary(boundary)) {
            return Err("pending Up recovery no longer matches its authored boundary".to_string());
        }
        window.musical_up_not_before_qpc
    } else {
        window.packet_not_before_qpc
    };
    Ok(Some(wait_target))
}

#[cfg(any(test, feature = "test-support"))]
fn physical_timing_window_for_packet(
    guard: &super::physical_timing_guard::PhysicalTimingGuard,
    authored_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    up_mask: u16,
    down_mask: u16,
) -> Result<super::physical_timing_guard::PhysicalTimingWindow, String> {
    guard
        .query(authored_target_qpc, up_mask, down_mask)
        .map_err(|error| format!("physical timing window query failed: {error:?}"))
}

#[cfg(any(test, feature = "test-support"))]
fn physical_timing_window_for_dispatch(
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    masks: sky_dispatch_win32::input::PhysicalPacket,
    runtime: &super::WorkerRuntime,
) -> Result<super::physical_timing_guard::PhysicalTimingWindow, String> {
    let Some(guard) = runtime.physical_timing_guard.as_ref() else {
        return Err("physical timing guard is not initialized".to_string());
    };
    physical_timing_window_for_packet(guard, physical_target_qpc, masks.up_mask, masks.down_mask)
}

#[inline]
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

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn preflight_prepared_plan(
    plan: &mut super::planning::NextDispatchPlan,
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    runtime: &mut super::WorkerRuntime,
    target: &SessionTarget,
) -> Result<bool, super::DispatchStep> {
    let Some(physical) = plan.physical_mut() else {
        return Ok(true);
    };
    let has_down_events = physical.authored_view.packet_masks.down_mask != 0;
    if !has_down_events {
        return Ok(true);
    }
    runtime.preparation_probe.record_preflight();
    let Some(target_stamp) = super::load_target_stamp(target) else {
        runtime.verified_target = None;
        runtime.invalidate_down_authorization();
        return Ok(false);
    };
    if let Err(error) =
        super::ensure_preflight_for_target(backend, target_stamp, &mut runtime.verified_target)
    {
        runtime.verified_target = None;
        return Err(super::DispatchStep::Terminate(format!(
            "instrument key preflight failed before timed wait; release the 15 instrument keys before playback: {error}"
        )));
    }
    if !super::target_stamp_still_current(target, target_stamp) {
        runtime.verified_target = None;
        runtime.invalidate_down_authorization();
        return Ok(false);
    }
    physical.target_proof = super::TargetProof::Verified(target_stamp);
    Ok(true)
}

/// Dispatch the work represented by one immutable plan. This helper is used
/// for both an already-due plan and a successful direct target wake, so a
/// normal timer wake never re-enters general orchestration before transport.
#[cfg(any(test, feature = "test-support"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_due_from_plan(
    plan: &super::planning::NextDispatchPlan,
    effective_now_ticks: TimelineTicks,
    now_ticks: sky_dispatch_win32::clock::QpcTicks,
    focus_loss_fault: bool,
    config: &super::WorkerConfig,
    resources: &mut super::WorkerResources,
    coordinator: &mut sky_dispatch_core::coordinator::RuntimeDispatchCoordinator,
    health: &mut super::WorkerHealthState,
    timing: &super::WorkerTimingState,
    runtime: &mut super::WorkerRuntime,
    local_metrics: &mut super::WorkerMetricsLocal,
    focus_active: &AtomicBool,
    target: &SessionTarget,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &SupervisorLeaseState,
    desired_pause: &AtomicBool,
    system_power: &SystemPowerState,
    progress_clock: &crate::engine::shared::SharedProgressClock,
    observer: Option<&super::dispatch::PendingObservationQueue>,
    boundary_crossing_qpc: Option<sky_dispatch_win32::clock::QpcTicks>,
    allow_pre_deadline: bool,
    #[cfg(any(test, feature = "test-support"))] test_physical_target_qpc: Option<
        sky_dispatch_win32::clock::QpcTicks,
    >,
    #[cfg(any(test, feature = "test-support"))] test_inject_sender_start: bool,
) -> super::DispatchStep {
    if let super::planning::NextDispatchPlan::Metadata(metadata) = plan {
        if metadata.physical_target_qpc > now_ticks {
            return super::DispatchStep::NoWork;
        }
        return coordinator
            .commit_prepared_authored_frame_metadata_frozen(&metadata.commit)
            .map(|()| super::DispatchStep::Dispatched)
            .unwrap_or_else(|error| {
                super::DispatchStep::Terminate(format!(
                    "coordinator authored metadata commit failure: {error}"
                ))
            });
    }
    if !matches!(plan, super::planning::NextDispatchPlan::Physical(_)) {
        return super::DispatchStep::NoWork;
    }
    /* stale authored metadata is drained by the outer global metadata phase */
    // A suspend notification blocks new Down authorization immediately. Do
    // this before inspecting a future target so a worker wake racing the OS
    // callback cannot publish FutureAuthorized while the gate is closed.
    if plan.physical().is_some_and(|physical| {
        physical.authored_view.packet_masks.down_mask != 0 && system_power.down_blocked()
    }) {
        return super::DispatchStep::NoWork;
    }
    #[cfg(any(test, feature = "test-support"))]
    let test_direct_boundary = test_physical_target_qpc.is_some();
    let candidate_target_qpc = {
        #[cfg(any(test, feature = "test-support"))]
        {
            test_physical_target_qpc.or_else(|| plan.physical_target_qpc())
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            plan.physical_target_qpc()
        }
    };
    let Some(candidate_target_qpc) = candidate_target_qpc else {
        return super::DispatchStep::NoWork;
    };
    let physical_target_qpc = match physical_target_qpc_for_work(
        Some(candidate_target_qpc),
        now_ticks,
        allow_pre_deadline,
    ) {
        Ok(Some(target)) => target,
        Ok(None) => {
            if candidate_target_qpc > now_ticks
                && let Some(boundary) = physical_boundary_stamp(plan, candidate_target_qpc)
                && runtime.pending_up_recovery.is_none()
                && runtime.down_boundary_state.awaiting_future()
            {
                runtime.observe_future_down_boundary(boundary);
            }
            return super::DispatchStep::NoWork;
        }
        Err(error) => return super::DispatchStep::Terminate(error),
    };
    let boundary_crossing_qpc =
        boundary_crossing_qpc.or_else(|| (physical_target_qpc <= now_ticks).then_some(now_ticks));

    let Some(physical) = plan.physical() else {
        return super::DispatchStep::NoWork;
    };
    let authored_commit = match &physical.authored_view.commit {
        super::dispatch::PhysicalCommit::Authored(commit)
        | super::dispatch::PhysicalCommit::Coalesced {
            authored: commit, ..
        } => Some(commit),
        super::dispatch::PhysicalCommit::PendingRelease { .. } => None,
    };
    if let Some(commit) = authored_commit
        && let Err(error) = coordinator.validate_frozen_authored_commit(commit)
    {
        return super::DispatchStep::Terminate(format!(
            "authored cancellation identity validation failed before SendInput: {error}"
        ));
    }
    let masks = physical.authored_view.packet_masks;
    let window = match physical_timing_window_for_dispatch(physical_target_qpc, masks, runtime) {
        Ok(window) => window,
        Err(error) => {
            return super::DispatchStep::Terminate(error);
        }
    };
    let boundary = physical_boundary_stamp(plan, physical_target_qpc);
    let pending_up_recovery = runtime
        .pending_up_recovery
        .as_ref()
        .map(|pending| (pending.boundary, pending.admission));
    if let Some((pending_boundary, _)) = pending_up_recovery {
        if !boundary.is_some_and(|boundary| pending_boundary.same_authored_boundary(boundary)) {
            return super::DispatchStep::TerminateStatic(
                "pending Up recovery no longer matches its authored boundary",
            );
        }
        if !runtime.down_boundary_state.awaiting_future() {
            return super::DispatchStep::TerminateStatic(
                "pending Up recovery retained Down authorization",
            );
        }
    }
    let physical_wait_target = if pending_up_recovery.is_some() {
        window.musical_up_not_before_qpc
    } else {
        window.packet_not_before_qpc
    };
    if physical_wait_target > now_ticks {
        return super::DispatchStep::NoWork;
    }

    #[cfg(any(test, feature = "test-support"))]
    if test_physical_target_qpc.is_some()
        && let Some(boundary_stamp) = boundary
        && pending_up_recovery.is_none()
        && runtime.down_boundary_state.awaiting_future()
    {
        // The harness passes an exact frozen target as a synthetic future
        // classification. This is test-only evidence; production authority
        // comes from the nonblocking observation that precedes the waiter.
        runtime.observe_future_down_boundary(boundary_stamp);
    }
    let down_admission = if let Some((_, admission)) = pending_up_recovery {
        admission
    } else if let Some(boundary) = boundary {
        let admission = if runtime.authorize_down_boundary(boundary) {
            DownBoundaryAdmission::Authorized
        } else {
            DownBoundaryAdmission::UnobservedBacklog
        };
        runtime.invalidate_down_authorization();
        admission
    } else {
        DownBoundaryAdmission::Authorized
    };
    if pending_up_recovery.is_none()
        && down_admission.is_missed()
        && masks.up_mask != 0
        && !config.timing.strict_timing
        && window.musical_up_not_before_qpc > now_ticks
    {
        let Some(boundary) = boundary else {
            return super::DispatchStep::TerminateStatic(
                "missed mixed Down boundary is missing its authored identity",
            );
        };
        let reason = match down_admission {
            DownBoundaryAdmission::UnobservedBacklog => {
                super::dispatch::DownMissReason::UnobservedBacklog
            }
            DownBoundaryAdmission::Authorized => {
                return super::DispatchStep::TerminateStatic(
                    "authorized Down cannot defer Up-prefix recovery",
                );
            }
        };
        let authored_commit = match &physical.authored_view.commit {
            super::dispatch::PhysicalCommit::Authored(commit) => commit.clone(),
            super::dispatch::PhysicalCommit::Coalesced { authored, .. } => authored.clone(),
            super::dispatch::PhysicalCommit::PendingRelease { .. } => {
                return super::DispatchStep::TerminateStatic(
                    "missed mixed Down boundary has no frozen authored commit",
                );
            }
        };
        super::dispatch::classify_missed_down_boundary(
            &physical.authored_view,
            local_metrics,
            observer,
            window,
            now_ticks,
            reason,
        );
        runtime.pending_up_recovery = Some(super::dispatch::PendingUpRecovery {
            boundary,
            admission: down_admission,
            authored_commit,
        });
        return super::DispatchStep::NoWork;
    }
    #[cfg(any(test, feature = "test-support"))]
    if masks.down_mask != 0
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.visit_startup_checkpoint(super::super::config::StartupCheckpoint::FirstDownAdmission);
    }
    let authored_step = super::dispatch_authored_packet(
        super::AuthoredPacketContext {
            dispatch_plan: plan,
            effective_now_ticks,
            now_ticks,
            physical_timing_window: window,
            down_admission,
            focus_loss_fault,
            supervisor_expired,
            boundary_crossing_qpc,
            #[cfg(any(test, feature = "test-support"))]
            test_direct_boundary,
            #[cfg(any(test, feature = "test-support"))]
            test_inject_sender_start,
        },
        config,
        resources,
        coordinator,
        health,
        timing,
        runtime,
        local_metrics,
        focus_active,
        target,
        quit_requested,
        skip_requested,
        panic_requested,
        desired_pause,
        system_power,
        progress_clock,
        observer,
    );
    if pending_up_recovery.is_some() && matches!(&authored_step, super::DispatchStep::Dispatched) {
        runtime.pending_up_recovery = None;
    }
    #[cfg(any(test, feature = "test-support"))]
    if masks.down_mask != 0
        && matches!(&authored_step, super::DispatchStep::Dispatched)
        && let Some(hook) = runtime.startup_ordering_hook.as_ref()
    {
        hook.visit_startup_checkpoint(super::super::config::StartupCheckpoint::FirstDownCommitted);
    }
    authored_step
}

#[allow(clippy::too_many_arguments)]
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
        runtime.preparation_probe.record_preflight();
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
    runtime.preparation_probe.record_input_build();

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
                        #[cfg(any(test, feature = "test-support"))]
                        if let Some(guard) = core.runtime.physical_timing_guard.as_mut() {
                            guard.invalidate();
                        }
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
    use super::{
        SchedulerAction, down_authored_target, evaluate_scheduler_action,
        paused_focus_grace_deadline, paused_wait_state_is_stable, physical_target_qpc_for_work,
        physical_wait_target_for_plan, pre_wait_candidate_admission,
        project_scheduler_action_for_staging, publish_live_metrics_after_dispatch,
        source_action_target, stage_scheduler_action, stage_scheduler_action_preserving_due_up,
        stage_up_only_for_stale_mixed_candidate, wait_after_unstable_target,
        wait_for_blocked_resume_replan,
    };
    use crate::engine::telemetry::metrics::{SharedMetrics, WorkerMetricsLocal};
    use crate::engine::test_support::ProductionDispatchTestHarness;
    use crate::engine::worker::{
        DownBoundaryState, NextDispatchPlan, PhysicalBoundaryStamp, WorkerRuntime,
        dispatch::{DispatchObservation, DispatchStep},
    };
    use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
    use sky_dispatch_win32::clock::QpcClock;
    use sky_dispatch_win32::event::OwnedEvent;
    use sky_dispatch_win32::input::{
        PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome,
        SendTransactionStatus,
    };
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicBool, Ordering};

    type MockPacketCapture = std::sync::Arc<std::sync::Mutex<Vec<(PhysicalPacket, SendEvidence)>>>;

    #[test]
    fn metadata_only_tail_waits_to_timeline_end_without_spinning_or_staging() {
        use sky_dispatch_core::compile::compile_runtime_intents;
        use sky_dispatch_core::executor::DispatchExecutor;

        let clock = QpcClock::initialize().expect("QPC clock");
        let tail_us = 80_000;
        let mut schedule = compile_runtime_intents(&[], &[0x15]).expect("empty schedule");
        schedule.schedule_end_offset_us = tail_us;
        let executor =
            DispatchExecutor::new(&schedule, DurationTicks::ZERO, DurationTicks::ZERO, |us| {
                clock
                    .timeline_from_us(us)
                    .map_err(|error| format!("{error:?}"))
            })
            .expect("metadata-tail executor");
        assert!(!executor.has_physical_work());

        let epoch = clock.now().expect("QPC epoch");
        let initial_action =
            evaluate_scheduler_action(&executor, epoch, epoch).expect("scheduler tail decision");
        let SchedulerAction::Wait { deadline_qpc } = initial_action else {
            panic!("metadata-only tail must wait for the schedule end");
        };
        assert!(deadline_qpc > epoch);
        let started = std::time::Instant::now();
        let (wait_target, action_to_stage) = project_scheduler_action_for_staging(
            &executor,
            SchedulerAction::Wait { deadline_qpc },
            epoch,
            epoch,
        )
        .expect("project deadline action");
        assert_eq!(wait_target, deadline_qpc);
        assert!(
            action_to_stage.is_none(),
            "Finished projection must not busy-loop"
        );

        let waiter = sky_dispatch_win32::wait::HybridWaiter::new();
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let mut local_metrics = WorkerMetricsLocal::default();
        let mut terminal_error = None;
        match super::wait_for_next_boundary(super::WaitBoundaryInput {
            deadline: super::WaitDeadline {
                physical_target_qpc: Some(wait_target),
                spin_threshold_ticks: DurationTicks::ZERO,
                qpc_clock: clock,
            },
            signals: super::WaitSignals {
                waiter: &waiter,
                interrupt: &interrupt,
            },
            mutable: super::WaitMutable {
                local_metrics: &mut local_metrics,
                terminal_error: &mut terminal_error,
            },
        }) {
            super::WaitBoundary::Due {
                target_qpc,
                dispatch_qpc,
                wait_result,
                ..
            } => {
                assert_eq!(target_qpc, deadline_qpc);
                assert!(dispatch_qpc >= deadline_qpc);
                assert_eq!(
                    wait_result
                        .expect("tail deadline used an interruptible wait")
                        .spin_ticks,
                    DurationTicks::ZERO,
                    "metadata-only tail must not precision-spin"
                );
            }
            super::WaitBoundary::Interrupted(..) => panic!("tail unexpectedly interrupted"),
            super::WaitBoundary::Exit => panic!("tail wait exited: {terminal_error:?}"),
        }
        assert!(started.elapsed() >= std::time::Duration::from_millis(40));
        assert_eq!(local_metrics.wait_interrupted_count, 0);
    }

    #[test]
    fn metadata_only_tail_can_be_interrupted_near_deadline_without_spinning() {
        use sky_dispatch_core::compile::compile_runtime_intents;
        use sky_dispatch_core::executor::DispatchExecutor;

        let clock = QpcClock::initialize().expect("QPC clock");
        let mut schedule = compile_runtime_intents(&[], &[0x15]).expect("empty schedule");
        schedule.schedule_end_offset_us = 200_000;
        let executor =
            DispatchExecutor::new(&schedule, DurationTicks::ZERO, DurationTicks::ZERO, |us| {
                clock
                    .timeline_from_us(us)
                    .map_err(|error| format!("{error:?}"))
            })
            .expect("metadata-tail executor");
        let epoch = clock.now().expect("QPC epoch");
        let SchedulerAction::Wait { deadline_qpc } =
            evaluate_scheduler_action(&executor, epoch, epoch).expect("tail action")
        else {
            panic!("metadata-only tail must retain the timeline-end wait");
        };
        let (wait_target, action_to_stage) = project_scheduler_action_for_staging(
            &executor,
            SchedulerAction::Wait { deadline_qpc },
            epoch,
            epoch,
        )
        .expect("project tail deadline");
        assert_eq!(wait_target, deadline_qpc);
        assert!(action_to_stage.is_none());

        let interrupt = std::sync::Arc::new(OwnedEvent::new_auto_reset().expect("interrupt event"));
        let signaler_event = std::sync::Arc::clone(&interrupt);
        let signaler = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(150));
            assert!(signaler_event.signal());
        });
        let started = std::time::Instant::now();
        let mut local_metrics = WorkerMetricsLocal::default();
        let mut terminal_error = None;
        match super::wait_for_next_boundary(super::WaitBoundaryInput {
            deadline: super::WaitDeadline {
                physical_target_qpc: Some(wait_target),
                spin_threshold_ticks: DurationTicks::ZERO,
                qpc_clock: clock,
            },
            signals: super::WaitSignals {
                waiter: &sky_dispatch_win32::wait::HybridWaiter::new(),
                interrupt: &interrupt,
            },
            mutable: super::WaitMutable {
                local_metrics: &mut local_metrics,
                terminal_error: &mut terminal_error,
            },
        }) {
            super::WaitBoundary::Interrupted(result, target, _) => {
                assert_eq!(target, deadline_qpc);
                assert_eq!(result.spin_ticks, DurationTicks::ZERO);
            }
            super::WaitBoundary::Due { .. } => panic!("signal should arrive before the deadline"),
            super::WaitBoundary::Exit => panic!("tail wait exited: {terminal_error:?}"),
        }
        signaler.join().expect("control signaler");
        assert!(started.elapsed() >= std::time::Duration::from_millis(120));
        assert!(started.elapsed() < std::time::Duration::from_millis(190));
        assert_eq!(local_metrics.wait_interrupted_count, 1);
    }

    #[test]
    fn paused_wait_recheck_closes_wake_race_and_rechecks_multiple_controls() {
        use crate::engine::shared::{SupervisorLeaseState, SystemPowerState};
        use crate::engine::target::SessionTarget;
        use sky_dispatch_win32::event::OwnedEvent;

        let target = SessionTarget::new(42, 7);
        let desired_pause = AtomicBool::new(true);
        let focus_active = AtomicBool::new(true);
        let quit_requested = AtomicBool::new(false);
        let skip_requested = AtomicBool::new(false);
        let panic_requested = AtomicBool::new(false);
        let supervisor_expired = SupervisorLeaseState::new(QpcTicks::ZERO);
        let system_power = SystemPowerState::default();
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let target_state = target.load_stable();
        let wake_generation = interrupt.signal_generation();

        assert!(interrupt.signal(), "raced signal before block");
        assert!(
            !paused_wait_state_is_stable(
                &desired_pause,
                true,
                &focus_active,
                true,
                true,
                &quit_requested,
                &skip_requested,
                &panic_requested,
                &supervisor_expired,
                &system_power,
                &target,
                target_state,
                &interrupt,
                wake_generation,
            ),
            "a signal racing the pre-block snapshot must return through the state machine"
        );
        assert!(interrupt.try_take());

        let stable_generation = interrupt.signal_generation();
        assert!(
            paused_wait_state_is_stable(
                &desired_pause,
                true,
                &focus_active,
                true,
                true,
                &quit_requested,
                &skip_requested,
                &panic_requested,
                &supervisor_expired,
                &system_power,
                &target,
                target_state,
                &interrupt,
                stable_generation,
            ),
            "a spurious wake with unchanged semantic state can safely re-enter the wait"
        );

        desired_pause.store(false, Ordering::Release);
        assert!(
            interrupt.signal(),
            "resume publisher signals the paused worker"
        );
        assert!(
            !paused_wait_state_is_stable(
                &desired_pause,
                true,
                &focus_active,
                true,
                true,
                &quit_requested,
                &skip_requested,
                &panic_requested,
                &supervisor_expired,
                &system_power,
                &target,
                target_state,
                &interrupt,
                interrupt.signal_generation(),
            ),
            "resume state is rechecked after the wake"
        );
        desired_pause.store(true, Ordering::Release);
        assert!(interrupt.try_take());

        skip_requested.store(true, Ordering::Release);
        quit_requested.store(true, Ordering::Release);
        assert!(interrupt.signal());
        assert!(
            !paused_wait_state_is_stable(
                &desired_pause,
                true,
                &focus_active,
                true,
                true,
                &quit_requested,
                &skip_requested,
                &panic_requested,
                &supervisor_expired,
                &system_power,
                &target,
                target_state,
                &interrupt,
                interrupt.signal_generation(),
            ),
            "coalesced event wakes cannot hide independent Stop and Skip state"
        );
    }

    #[test]
    fn blocked_resume_replan_waits_interruptibly_and_rechecks_target() {
        use crate::engine::shared::{SupervisorLeaseState, SystemPowerState};
        use crate::engine::target::SessionTarget;
        use std::sync::Arc;
        use std::time::{Duration, Instant};

        let clock = QpcClock::initialize().expect("QPC clock");
        let target = Arc::new(SessionTarget::new(0, 1));
        let interrupt = Arc::new(OwnedEvent::new_auto_reset().expect("interrupt event"));
        let target_signaler = Arc::clone(&target);
        let event_signaler = Arc::clone(&interrupt);
        let signaler = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            assert!(target_signaler.publish(123));
            assert!(event_signaler.signal());
        });

        let desired_pause = AtomicBool::new(false);
        let focus_active = AtomicBool::new(true);
        let quit_requested = AtomicBool::new(false);
        let skip_requested = AtomicBool::new(false);
        let panic_requested = AtomicBool::new(false);
        let supervisor_expired =
            SupervisorLeaseState::new(sky_dispatch_win32::clock::QpcTicks::ZERO);
        let system_power = SystemPowerState::default();
        let waiter = sky_dispatch_win32::wait::HybridWaiter::new();
        let mut local_metrics = WorkerMetricsLocal::default();
        let mut terminal_error = None;
        let deadline = Instant::now() + Duration::from_secs(1);

        while local_metrics.wait_interrupted_count == 0 && Instant::now() < deadline {
            let expected_target = target
                .load_stable()
                .map(|(hwnd, generation)| super::TargetStamp { hwnd, generation });
            assert!(wait_for_blocked_resume_replan(
                clock.now().expect("QPC now"),
                super::FinalControlSignals {
                    quit_requested: &quit_requested,
                    skip_requested: &skip_requested,
                    panic_requested: &panic_requested,
                    desired_pause: &desired_pause,
                    supervisor_expired: &supervisor_expired,
                    system_power: Some(&system_power),
                },
                false,
                &focus_active,
                true,
                false,
                &target,
                expected_target,
                clock,
                &waiter,
                &interrupt,
                &mut local_metrics,
                &mut terminal_error,
            ));
            assert_eq!(terminal_error, None);
        }

        signaler.join().expect("target signaler");
        assert_eq!(local_metrics.wait_interrupted_count, 1);
        assert_eq!(target.load_stable(), Some((123, 2)));
    }

    #[test]
    fn paused_focus_grace_wait_uses_one_absolute_deadline() {
        let restored_at = QpcTicks::from_raw(10_000);
        let deadline = paused_focus_grace_deadline(
            true,
            true,
            true,
            Some(restored_at),
            DurationTicks::from_raw(4_000),
        )
        .expect("checked focus grace deadline");
        assert_eq!(deadline, Some(QpcTicks::from_raw(14_000)));
        assert_eq!(
            paused_focus_grace_deadline(
                true,
                true,
                false,
                Some(restored_at),
                DurationTicks::from_raw(4_000),
            )
            .expect("target-less focus does not arm a grace timer"),
            None
        );
    }

    #[test]
    fn mixed_packet_down_lateness_uses_down_target_and_keeps_packet_target() {
        use sky_dispatch_core::executor::DownChordTicks;
        use smallvec::SmallVec;

        let epoch = QpcTicks::from_raw(0);
        let chord = DownChordTicks {
            chord_index: 1,
            scheduled_ticks: TimelineTicks::from_raw(20_000),
            scheduled_us: 20_000,
            down_mask: 0b10,
            source_action_index: 1,
            reason_id: 0,
            intents: SmallVec::new(),
        };
        let down_target = down_authored_target(epoch, Some((&chord).into()))
            .expect("checked Down target")
            .expect("Down target exists");
        let packet_authored_qpc = QpcTicks::from_raw(17_167);
        let packet_authored_ticks = TimelineTicks::from_raw(17_167);
        let (source_ticks, source_qpc) = source_action_target(
            packet_authored_ticks,
            packet_authored_qpc,
            Some(down_target),
        );
        let pre_call_qpc = QpcTicks::from_raw(20_005);

        assert_eq!(down_target.0.as_u64(), 20_000);
        assert_eq!(pre_call_qpc.as_u64() - source_qpc.as_u64(), 5);
        assert_eq!(source_ticks.as_u64(), 20_000);
        assert_eq!(packet_authored_qpc.as_u64(), 17_167);
        assert_eq!(packet_authored_ticks.as_u64(), 17_167);

        let mixed = PhysicalPacket::new(0b01, chord.down_mask);
        assert_eq!(mixed.up_mask.count_ones(), 1);
        assert_eq!(mixed.down_mask.count_ones(), 1);
        assert_eq!(mixed.up_mask & mixed.down_mask, 0);
        assert_eq!(mixed.event_count(), 2);
    }

    fn mixed_scheduler_after_accepted_a_down() -> (ProductionDispatchTestHarness, MockPacketCapture)
    {
        let mut harness = ProductionDispatchTestHarness::new_prepared_mixed_for_test(1, 1, 20_000)
            .expect("production Mixed fixture");
        let packets = harness.configure_mock_packet_sender_for_test(&[]);
        let clock = harness.resources.clock;
        let send_start = clock.now().expect("initial Down sender QPC");
        let one_second = clock.duration_from_us(1_000_000).expect("epoch offset");
        let epoch = QpcTicks::from_raw(
            send_start
                .as_u64()
                .checked_sub(one_second.as_u64())
                .expect("test epoch before current QPC"),
        );
        harness
            .reanchor_playback_clock_for_test(epoch)
            .expect("publish test playback epoch");
        harness.set_mock_sender_start_qpc_for_test(send_start);

        let action = evaluate_scheduler_action(&harness.resources.scheduler, send_start, epoch)
            .expect("initial production scheduler action");
        let candidate = super::stage_scheduler_action(
            action,
            &harness.target,
            &mut harness.resources,
            &mut harness.runtime,
        )
        .expect("stage initial production Down")
        .expect("initial production Down candidate");
        assert!(matches!(
            harness.dispatch_staged_candidate_for_test(candidate, false),
            super::StagedDispatchOutcome::Dispatched
        ));
        assert_eq!(harness.resources.scheduler.accounting().activated, 1);
        assert_eq!(harness.resources.scheduler.accounting().active, 1);

        std::thread::sleep(std::time::Duration::from_millis(30));
        (harness, packets)
    }

    fn next_mixed_action(harness: &ProductionDispatchTestHarness) -> SchedulerAction {
        let now = harness.resources.clock.now().expect("Mixed action QPC");
        evaluate_scheduler_action(
            &harness.resources.scheduler,
            now,
            harness.resources.playback.epoch,
        )
        .expect("Mixed production scheduler action")
    }

    fn assert_no_work(step: DispatchStep) {
        assert!(matches!(step, DispatchStep::NoWork), "step={step:?}");
    }

    fn assert_dispatched(step: DispatchStep) {
        assert!(matches!(step, DispatchStep::Dispatched), "step={step:?}");
    }

    fn subtract_duration(target: QpcTicks, duration: DurationTicks) -> QpcTicks {
        QpcTicks::from_raw(
            target
                .as_u64()
                .checked_sub(duration.as_u64())
                .expect("test QPC subtraction"),
        )
    }

    fn set_zero_timing_margin(harness: &mut ProductionDispatchTestHarness) {
        let clock = harness.resources.clock;
        let release_gap = clock
            .duration_from_us(harness.config.timing.min_release_gap_us)
            .expect("release-gap duration");
        let base_hold = clock
            .duration_from_us(harness.config.timing.frame_base_hold_us)
            .expect("base hold duration");
        harness.timing.timing_margin_ticks = DurationTicks::ZERO;
        harness
            .runtime
            .set_physical_timing_guard_for_test(base_hold, release_gap);
    }

    #[test]
    fn live_dispatch_publication_exposes_production_sender_samples() {
        let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let shared = SharedMetrics::default();
        shared
            .live_diagnostics_enabled
            .store(true, Ordering::Relaxed);
        let local = WorkerMetricsLocal {
            pre_call_lt_250us: 1,
            max_sendinput_pre_call_lateness_ticks: 25,
            ..WorkerMetricsLocal::default()
        };

        publish_live_metrics_after_dispatch(&local, &shared, qpc_clock, QpcTicks::from_raw(50_000));

        let snapshot = shared.snapshot.load();
        assert_eq!(snapshot.pre_call_lt_250us, 1);
        assert_eq!(snapshot.max_sendinput_pre_call_lateness_us, 25);
    }

    #[test]
    fn exact_future_authorization_survives_waiter_entry_stall() {
        let stamp = PhysicalBoundaryStamp {
            first_batch_index: 2,
            packet_index: 3,
            packet_batch_count: 1,
            source_action_index: 7,
            up_mask: 0,
            down_mask: 1,
            physical_target_qpc: QpcTicks::from_raw(1_000),
        };
        let mut runtime = WorkerRuntime::default();
        runtime.observe_future_down_boundary(stamp);
        assert_eq!(
            runtime.down_boundary_state,
            DownBoundaryState::FutureAuthorized(stamp)
        );
        assert!(runtime.authorize_down_boundary(stamp));
    }

    #[test]
    fn authorization_does_not_leak_to_equal_qpc_boundary() {
        let first = PhysicalBoundaryStamp {
            first_batch_index: 2,
            packet_index: 3,
            packet_batch_count: 1,
            source_action_index: 7,
            up_mask: 0,
            down_mask: 1,
            physical_target_qpc: QpcTicks::from_raw(1_000),
        };
        let second = PhysicalBoundaryStamp {
            first_batch_index: 4,
            ..first
        };
        let mut runtime = WorkerRuntime::default();
        runtime.observe_future_down_boundary(first);
        assert!(!runtime.authorize_down_boundary(second));
    }

    #[test]
    fn pause_or_replan_invalidation_clears_exact_authorization() {
        let stamp = PhysicalBoundaryStamp {
            first_batch_index: 2,
            packet_index: 3,
            packet_batch_count: 1,
            source_action_index: 7,
            up_mask: 0,
            down_mask: 1,
            physical_target_qpc: QpcTicks::from_raw(1_000),
        };
        let mut runtime = WorkerRuntime::default();
        runtime.observe_future_down_boundary(stamp);
        runtime.invalidate_down_authorization();
        assert_eq!(
            runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture
        );
        assert!(!runtime.authorize_down_boundary(stamp));
    }

    #[test]
    fn authored_overdue_work_uses_its_physical_target() {
        assert_eq!(
            physical_target_qpc_for_work(
                Some(QpcTicks::from_raw(1_000)),
                QpcTicks::from_raw(1_200),
                false,
            )
            .expect("authored target")
            .expect("authored deadline"),
            QpcTicks::from_raw(1_000)
        );
    }

    #[test]
    fn startup_target_cannot_precede_the_playback_epoch() {
        let target = physical_target_qpc_for_work(
            Some(QpcTicks::from_raw(10_000)),
            QpcTicks::from_raw(9_540),
            false,
        )
        .expect("target arithmetic");

        assert_eq!(target, None);
    }

    #[test]
    fn future_selected_target_is_not_replaced_with_now() {
        let target = physical_target_qpc_for_work(
            Some(QpcTicks::from_raw(1_000)),
            QpcTicks::from_raw(900),
            false,
        )
        .expect("prepared target");

        assert_eq!(target, None);
    }

    #[test]
    fn deadline_boundary_matrix_distinguishes_future_exact_and_overdue() {
        let target = QpcTicks::from_raw(1_000);
        assert_eq!(
            physical_target_qpc_for_work(Some(target), QpcTicks::from_raw(999), false)
                .expect("future target arithmetic"),
            None,
            "target - 1 tick remains future work"
        );
        assert_eq!(
            physical_target_qpc_for_work(Some(target), target, false)
                .expect("exact target arithmetic"),
            Some(target),
            "target exact is due"
        );
        assert_eq!(
            physical_target_qpc_for_work(Some(target), QpcTicks::from_raw(1_001), false)
                .expect("overdue target arithmetic"),
            Some(target),
            "target + 1 tick is overdue work"
        );
    }

    #[test]
    fn prepared_normal_observer_has_no_sender_cutoff_evidence() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        harness.enable_dispatch_ready_timing_for_benchmark();
        harness.configure_packet_capture();
        harness.prepare_prepared_stream_for_test();

        assert_dispatched(
            harness
                .wait_and_dispatch_prepared_current_for_test()
                .expect("prepared normal dispatch"),
        );

        let observation = harness.pop_observation().expect("normal Down observation");
        let DispatchObservation::Down(observation) = observation else {
            panic!("expected normal Down observation");
        };
        let wait = harness
            .pop_observation()
            .expect("prepared wait observation");
        let DispatchObservation::Wait(wait) = wait else {
            panic!("expected the staged wait attempt after its Down result");
        };
        assert!(matches!(
            wait.outcome,
            sky_dispatch_win32::wait::WaitOutcome::Deadline
        ));
        assert_eq!(wait.wake_qpc, Some(observation.wake_qpc.unwrap()));
        assert_eq!(wait.physical_target_qpc, observation.physical_target_qpc);
        assert_eq!(
            wait.deadline_ticks,
            observation.trace.effective_deadline_ticks
        );
        assert_eq!(wait.epoch_qpc, observation.epoch_qpc);
        assert_eq!(
            observation.physical_timing_window.authored_target_qpc,
            observation.physical_target_qpc
        );
        assert_eq!(
            observation.physical_timing_window.musical_up_not_before_qpc,
            observation.physical_target_qpc
        );
        assert_eq!(
            observation.physical_timing_window.down_not_before_qpc,
            observation.physical_target_qpc
        );
        assert_eq!(
            observation.physical_timing_window.packet_not_before_qpc,
            observation.physical_target_qpc
        );
        assert_eq!(observation.physical_timing_window.hold_floor_mask, 0);
        assert_eq!(observation.physical_timing_window.release_floor_mask, 0);
    }

    #[test]
    fn prepared_mixed_wait_keeps_physical_and_authored_deadline_attribution() {
        let mut harness = ProductionDispatchTestHarness::new_mixed();
        harness.prepare_prepared_stream_for_test();
        assert_dispatched(
            harness.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000),
        );
        assert!(matches!(
            harness.pop_observation(),
            Some(DispatchObservation::Down(_))
        ));
        harness
            .align_prepared_current_to_benchmark_margin_for_test(20_000)
            .expect("prepared mixed boundary alignment");
        assert_dispatched(
            harness
                .wait_and_dispatch_prepared_current_for_test()
                .expect("prepared mixed wait and dispatch"),
        );

        let down = harness.pop_observation().expect("mixed Down observation");
        let DispatchObservation::Down(down) = down else {
            panic!("expected mixed Down observation");
        };
        let wait = harness.pop_observation().expect("mixed wait observation");
        let DispatchObservation::Wait(wait) = wait else {
            panic!("expected the staged mixed wait after its Down result");
        };
        assert!(matches!(
            wait.outcome,
            sky_dispatch_win32::wait::WaitOutcome::Deadline
        ));
        assert_eq!(wait.physical_target_qpc, down.physical_target_qpc);
        assert_eq!(wait.deadline_ticks, down.trace.effective_deadline_ticks);
        assert_ne!(down.requested_packet.up_mask, 0);
        assert_ne!(down.requested_packet.down_mask, 0);
        assert!(harness.observer.is_empty());
    }

    #[test]
    fn metadata_wait_is_observed_with_its_original_deadline() {
        let mut harness = ProductionDispatchTestHarness::new_stale_metadata_boundary();
        let now = harness.qpc_now_for_test().expect("metadata wait QPC");
        let margin = harness
            .qpc_duration_from_us_for_test(20_000)
            .expect("metadata wait margin");
        let epoch = now
            .checked_add_duration(margin)
            .expect("future metadata epoch");
        harness.set_playback_epoch_qpc_for_test(epoch);
        let plan = harness.plan_current_dispatch();
        let NextDispatchPlan::Metadata(metadata) = &plan else {
            panic!("leading stale Up should produce a metadata boundary");
        };
        let target_qpc = metadata.physical_target_qpc;
        let deadline_ticks = metadata.deadline_ticks;

        assert_dispatched(
            harness
                .wait_and_dispatch_current_plan(&plan)
                .expect("metadata wait and commit"),
        );
        let observation = harness
            .pop_observation()
            .expect("metadata wait observation");
        let DispatchObservation::Wait(observation) = observation else {
            panic!("metadata wait must be published as a wait observation");
        };
        assert!(matches!(
            observation.outcome,
            sky_dispatch_win32::wait::WaitOutcome::Deadline
        ));
        assert_eq!(observation.physical_target_qpc, target_qpc);
        assert_eq!(observation.deadline_ticks, deadline_ticks);
        assert_eq!(observation.epoch_qpc, epoch);
        assert!(harness.observer.is_empty());
    }

    #[test]
    fn first_down_requires_future_observation_and_unobserved_due_work_is_missed() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");

        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert!(matches!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::FutureAuthorized(_)
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, target));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1)]
        );

        let mut unobserved = ProductionDispatchTestHarness::new_down_only();
        let packets = unobserved.configure_packet_capture();
        let plan = unobserved.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_dispatched(unobserved.dispatch_at_qpc_for_test(&plan, target));
        assert!(packets.lock().expect("packet capture").is_empty());
        assert_eq!(
            unobserved
                .local_metrics
                .missed_unobserved_backlog_boundaries,
            1
        );
    }

    #[test]
    fn normal_mode_sends_positive_pre_call_lateness_without_grace() {
        let mut exact = ProductionDispatchTestHarness::new_down_only();
        set_zero_timing_margin(&mut exact);
        let exact_packets = exact.configure_packet_capture();
        let exact_plan = exact.plan_current_dispatch();
        let exact_target = exact_plan.physical_target_qpc().expect("Down target");
        assert_eq!(exact.timing.timing_margin_ticks, DurationTicks::ZERO);
        assert_no_work(exact.dispatch_at_qpc_for_test(
            &exact_plan,
            subtract_duration(exact_target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(exact.dispatch_at_qpc_for_test(&exact_plan, exact_target));
        assert_eq!(exact_packets.lock().expect("packet capture").len(), 1);

        let mut late = ProductionDispatchTestHarness::new_down_only();
        set_zero_timing_margin(&mut late);
        let late_packets = late.configure_packet_capture();
        let late_plan = late.plan_current_dispatch();
        let late_target = late_plan.physical_target_qpc().expect("Down target");
        assert_eq!(late.timing.timing_margin_ticks, DurationTicks::ZERO);
        assert_no_work(late.dispatch_at_qpc_for_test(
            &late_plan,
            subtract_duration(late_target, DurationTicks::from_raw(1)),
        ));
        let one_tick_late = late_target
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("one tick beyond the authored target");
        assert_dispatched(late.dispatch_at_qpc_for_test(&late_plan, one_tick_late));
        assert_eq!(late_packets.lock().expect("packet capture").len(), 1);
        assert_eq!(late.local_metrics.missed_down_boundaries, 0);
    }

    #[test]
    fn positive_pre_call_lateness_inside_timing_margin_is_not_a_miss() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert!(harness.timing.timing_margin_ticks > DurationTicks::ZERO);
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));

        let one_tick_late = target
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("positive pre-call lateness inside timing margin");
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, one_tick_late));

        assert_eq!(packets.lock().expect("packet capture").len(), 1);
        assert!(harness.local_metrics.max_sendinput_pre_call_lateness_ticks > 0);
        assert_eq!(harness.local_metrics.missed_down_boundaries, 0);
        assert_eq!(
            harness.local_metrics.missed_unobserved_backlog_boundaries,
            0
        );
    }

    fn add_us(harness: &ProductionDispatchTestHarness, target: QpcTicks, us: u64) -> QpcTicks {
        target
            .checked_add_duration(
                harness
                    .resources
                    .clock
                    .duration_from_us(us)
                    .expect("QPC us"),
            )
            .expect("test QPC addition")
    }

    #[test]
    fn c2_normal_mode_sends_late_authorized_down_at_2_10_and_50_ms() {
        for offset_us in [2_000, 10_000, 50_000] {
            let mut harness = ProductionDispatchTestHarness::new_down_only();
            let packets = harness.configure_packet_capture();
            let plan = harness.plan_current_dispatch();
            let target = plan.physical_target_qpc().expect("Down target");
            assert_no_work(harness.dispatch_at_qpc_for_test(
                &plan,
                subtract_duration(target, DurationTicks::from_raw(1)),
            ));
            assert_dispatched(
                harness.dispatch_at_qpc_for_test(&plan, add_us(&harness, target, offset_us)),
            );
            assert_eq!(
                packets.lock().expect("packet capture").len(),
                1,
                "offset {offset_us}us should make exactly one sender attempt"
            );
            assert_eq!(
                packets.lock().expect("packet capture").as_slice(),
                &[PhysicalPacket::new(0, 1)],
                "offset {offset_us}us must preserve one atomic Down packet"
            );
            assert_eq!(
                harness.local_metrics.missed_down_boundaries, 0,
                "offset {offset_us}us must not terminalize the Down"
            );
            assert_eq!(
                harness.timeline_rebase_count_for_test(),
                0,
                "offset {offset_us}us must not rebase or catch up the timeline"
            );
        }
    }

    #[test]
    fn strict_late_authorized_down_is_sent() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        harness.set_strict_timing_for_test(true);
        harness.timing.strict_down_completion_late_ticks = harness
            .resources
            .clock
            .duration_from_us(3_000)
            .expect("strict completion test allowance");
        let evidence =
            harness.configure_prepared_transport_outcome_for_test(SendTransactionStatus::Complete);
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        let late_now = add_us(&harness, target, 501);
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, late_now));
        let captured = evidence.lock().expect("strict small-lateness evidence");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].requested_mask, 1);
        assert_eq!(captured[0].attempts, 1);
    }

    #[test]
    fn strict_late_authorized_down_remains_atomic_at_large_lateness() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        harness.set_strict_timing_for_test(true);
        harness.timing.strict_down_completion_late_ticks = harness
            .resources
            .clock
            .duration_from_us(60_000)
            .expect("strict completion test allowance");
        let evidence =
            harness.configure_prepared_transport_outcome_for_test(SendTransactionStatus::Complete);
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        let late_now = add_us(&harness, target, 50_000);
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, late_now));
        let captured = evidence.lock().expect("strict large-lateness evidence");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].requested_mask, 1);
        assert_eq!(captured[0].attempts, 1);
    }

    #[test]
    fn normal_completion_floor_delays_down_without_making_it_infeasible() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let clock = harness.resources.clock;
        let release_gap = clock
            .duration_from_us(harness.config.timing.min_release_gap_us)
            .expect("release-gap ticks");
        let beyond_authored_target = harness
            .timing
            .timing_margin_ticks
            .checked_add(DurationTicks::from_raw(1))
            .expect("floor delay beyond authored target");
        let release_floor = target
            .checked_add_duration(beyond_authored_target)
            .expect("release floor");
        let completed_up = release_floor
            .as_u64()
            .checked_sub(release_gap.as_u64())
            .map(QpcTicks::from_raw)
            .expect("prior Up completion");
        harness
            .runtime
            .physical_timing_guard
            .as_mut()
            .expect("production guard initialized")
            .observe_successful_packet(completed_up, 1, 0)
            .expect("seed trusted prior Up completion");

        assert!(release_floor < add_us(&harness, target, 2_500));
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_no_work(harness.dispatch_at_qpc_for_test(&plan, target));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, release_floor));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 3)]
        );
    }

    #[test]
    fn c2_mixed_late_start_sends_the_prepared_whole_packet() {
        let mut harness = ProductionDispatchTestHarness::new_mixed_events_with_gap(2, 100_000);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("mixed target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, add_us(&harness, target, 1_000)));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(1, 2)]
        );
        assert_eq!(harness.chord_integrity_lost_count(), 0);
    }

    #[test]
    fn normal_late_transport_failure_is_not_success() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let clock = harness.resources.clock;
        harness.resources.backend.set_packet_emitter(move |packet| {
            let requested_mask = packet.up_mask | packet.down_mask;
            SendTransactionOutcome {
                status: SendTransactionStatus::ZeroProgress,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: requested_mask,
                    first_inserted: 0,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: Some(5),
                    last_win32_error: Some(5),
                    started_ticks: Some(clock.now().expect("test QPC")),
                    completed_ticks: None,
                    timing_error: None,
                },
            }
        });
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert!(matches!(
            harness.dispatch_at_qpc_for_test(&plan, add_us(&harness, target, 1_000)),
            DispatchStep::Terminate(_)
        ));
    }

    #[test]
    fn c1_complete_without_completion_qpc_is_not_recorded_as_rescue() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let clock = harness.resources.clock;
        harness.resources.backend.set_packet_emitter(move |packet| {
            let requested_mask = packet.up_mask | packet.down_mask;
            SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask: 0,
                    first_inserted: requested_mask.count_ones() as u8,
                    attempts: 1,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(clock.now().expect("test QPC")),
                    completed_ticks: None,
                    timing_error: None,
                },
            }
        });
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert!(matches!(
            harness.dispatch_at_qpc_for_test(&plan, add_us(&harness, target, 1_000)),
            DispatchStep::TerminateStatic("successful Down missing completion QPC")
        ));
        assert_eq!(
            harness.local_metrics.missed_unobserved_backlog_boundaries,
            0
        );
    }

    #[test]
    fn c1_dense_future_boundary_keeps_authored_target_and_no_backlog() {
        let mut harness = ProductionDispatchTestHarness::new_dense_future_boundary_for_test();
        let packets = harness.configure_packet_capture();
        let first = harness.plan_current_dispatch();
        let first_target = first.physical_target_qpc().expect("first target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &first,
            subtract_duration(first_target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(
            harness.dispatch_at_qpc_for_test(&first, add_us(&harness, first_target, 2_200)),
        );

        let second = harness.plan_current_dispatch();
        let second_target = second.physical_target_qpc().expect("second target");
        let authored_gap = harness
            .resources
            .clock
            .duration_to_us(
                second_target
                    .checked_duration_since(first_target)
                    .expect("future boundary ordering"),
            )
            .expect("future authored gap");
        assert_eq!(authored_gap, 5_000);
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &second,
            subtract_duration(second_target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&second, second_target));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1), PhysicalPacket::new(0, 2)]
        );
        assert_eq!(
            harness.local_metrics.missed_unobserved_backlog_boundaries,
            0
        );
    }

    #[test]
    fn c1_1_dense_boundary_matrix_preserves_authored_targets_and_safety() {
        const GAPS_US: [u64; 7] = [1_000, 1_500, 2_000, 2_500, 3_000, 4_000, 5_000];
        const OFFSETS_US: [u64; 11] = [
            600, 1_000, 1_400, 1_600, 2_200, 2_500, 3_000, 3_500, 4_000, 5_000, 5_500,
        ];

        for gap_us in GAPS_US {
            for offset_us in OFFSETS_US {
                let mut harness =
                    ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(
                        gap_us,
                    );
                let packets = harness.configure_packet_capture();
                let first = harness.plan_current_dispatch();
                let first_target = first.physical_target_qpc().expect("first target");
                assert_no_work(harness.dispatch_at_qpc_for_test(
                    &first,
                    subtract_duration(first_target, DurationTicks::from_raw(1)),
                ));
                let first_now = add_us(&harness, first_target, offset_us);
                assert_dispatched(harness.dispatch_at_qpc_for_test(&first, first_now));

                let first_allowed = true;
                assert_eq!(
                    packets.lock().expect("packet capture").len(),
                    if first_allowed { 1 } else { 0 },
                    "first packet classification for gap={gap_us} offset={offset_us}"
                );

                let second = harness.plan_current_dispatch();
                let second_target = second.physical_target_qpc().expect("second target");
                let authored_gap = harness
                    .resources
                    .clock
                    .duration_to_us(
                        second_target
                            .checked_duration_since(first_target)
                            .expect("second target follows first"),
                    )
                    .expect("authored gap conversion");
                assert_eq!(
                    authored_gap, gap_us,
                    "second target must retain authored gap for offset={offset_us}"
                );

                let second_is_future = second_target > first_now;
                if second_is_future {
                    assert_no_work(harness.dispatch_at_qpc_for_test(
                        &second,
                        subtract_duration(second_target, DurationTicks::from_raw(1)),
                    ));
                    assert_dispatched(harness.dispatch_at_qpc_for_test(&second, second_target));
                    assert_eq!(
                        harness.missed_unobserved_backlog_boundaries_for_test(),
                        0,
                        "future second boundary must not become false backlog"
                    );
                } else {
                    assert_dispatched(harness.dispatch_at_qpc_for_test(&second, first_now));
                    assert_eq!(
                        harness.missed_unobserved_backlog_boundaries_for_test(),
                        1,
                        "overdue second boundary must follow existing backlog contract"
                    );
                }

                let expected_packets =
                    (if first_allowed { 1 } else { 0 }) + (if second_is_future { 1 } else { 0 });
                assert_eq!(
                    packets.lock().expect("packet capture").len(),
                    expected_packets,
                    "no catch-up or packet split for gap={gap_us} offset={offset_us}"
                );
            }
        }
    }

    #[test]
    fn normal_completion_does_not_retime_legal_same_key_retrigger() {
        fn run_variant(
            harness: &mut ProductionDispatchTestHarness,
            first_completion_delay_us: u64,
        ) -> (
            TimelineTicks,
            TimelineTicks,
            TimelineTicks,
            QpcTicks,
            QpcTicks,
            QpcTicks,
        ) {
            let first_down = harness.plan_current_dispatch();
            let first_down_evidence = harness
                .prepared_boundary_evidence_for_test(&first_down)
                .expect("first Down evidence");
            assert_dispatched(
                harness.dispatch_at_benchmark_boundary_for_test(
                    &first_down,
                    first_completion_delay_us,
                ),
            );

            let first_up = harness.plan_current_dispatch();
            let first_up_evidence = harness
                .prepared_boundary_evidence_for_test(&first_up)
                .expect("first Up evidence");
            assert_dispatched(harness.dispatch_at_benchmark_boundary_for_test(&first_up, 0));

            let second_down = harness.plan_current_dispatch();
            let second_down_evidence = harness
                .prepared_boundary_evidence_for_test(&second_down)
                .expect("second Down evidence");
            let second_down_floor = harness
                .physical_floor_evidence_for_test(&second_down)
                .expect("second Down authored-only window");
            let second_down_wait_target = harness
                .physical_wait_target_for_test(&second_down)
                .expect("second Down wait target")
                .expect("second Down physical target");
            assert_eq!(
                second_down_floor.authored_target_qpc,
                second_down_floor.packet_not_before_qpc
            );
            assert_eq!(second_down_floor.hold_floor_mask, 0);
            assert_eq!(second_down_floor.release_floor_mask, 0);
            assert_eq!(
                second_down_wait_target,
                second_down_evidence.physical_target_qpc
            );
            assert_dispatched(harness.dispatch_at_benchmark_boundary_for_test(&second_down, 0));

            (
                first_down_evidence.authored_ticks,
                first_up_evidence.authored_ticks,
                second_down_evidence.authored_ticks,
                first_down_evidence.physical_target_qpc,
                second_down_evidence.physical_target_qpc,
                second_down_wait_target,
            )
        }

        let mut control =
            ProductionDispatchTestHarness::new_same_key_retrigger_with_gap_for_test(50_000);
        control.align_next_plan_to_future_for_test(100_000);
        let shared_epoch = control.playback_epoch_qpc_for_test();
        let mut delayed =
            ProductionDispatchTestHarness::new_same_key_retrigger_with_gap_for_test(50_000);
        delayed.set_playback_epoch_qpc_for_test(shared_epoch);

        let control_result = run_variant(&mut control, 100);
        let delayed_result = run_variant(&mut delayed, 40_000);
        assert_eq!(
            control_result.0, delayed_result.0,
            "authored Down schedule changed"
        );
        assert_eq!(
            control_result.1, delayed_result.1,
            "authored Up schedule changed"
        );
        assert_eq!(
            control_result.2, delayed_result.2,
            "authored retrigger schedule changed"
        );
        assert_eq!(
            control_result.3, delayed_result.3,
            "first physical target changed"
        );
        assert_eq!(
            control_result.4, delayed_result.4,
            "later physical target changed"
        );
        assert_eq!(
            control_result.5, delayed_result.5,
            "later wait target changed"
        );
    }

    #[test]
    fn sparse_comparison_sends_authorized_late_note_ons() {
        let offsets_in_extended_range = [2_800, 3_000, 3_500, 4_000, 4_500, 5_000];

        for &offset_us in &offsets_in_extended_range {
            // Normal sender admission has no lateness cutoff.
            let mut baseline =
                ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(100_000);
            let packets_baseline = baseline.configure_packet_capture();
            let plan_baseline = baseline.plan_current_dispatch();
            let target_baseline = plan_baseline
                .physical_target_qpc()
                .expect("baseline target");
            assert_no_work(baseline.dispatch_at_qpc_for_test(
                &plan_baseline,
                subtract_duration(target_baseline, DurationTicks::from_raw(1)),
            ));
            let now_baseline = add_us(&baseline, target_baseline, offset_us);
            assert_dispatched(baseline.dispatch_at_qpc_for_test(&plan_baseline, now_baseline));
            assert_eq!(
                packets_baseline.lock().expect("packet capture").len(),
                1,
                "baseline 2.5 ms must send the authorized note at offset {offset_us} us"
            );

            // A separate prepared boundary has the same normal behavior.
            let mut prov_max =
                ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(100_000);
            let packets_prov = prov_max.configure_packet_capture();
            let plan_prov = prov_max.plan_current_dispatch();
            let target_prov = plan_prov.physical_target_qpc().expect("provisional target");
            assert_no_work(prov_max.dispatch_at_qpc_for_test(
                &plan_prov,
                subtract_duration(target_prov, DurationTicks::from_raw(1)),
            ));
            let now_prov = add_us(&prov_max, target_prov, offset_us);
            assert_dispatched(prov_max.dispatch_at_qpc_for_test(&plan_prov, now_prov));
            assert_eq!(
                packets_prov.lock().expect("packet capture").len(),
                1,
                "provisional max 5.0 ms must send note at offset {offset_us} us"
            );
        }

        // A late authorized normal Down is sent regardless of lateness.
        let offset_beyond_max = 5_500;
        let mut prov_max =
            ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(100_000);
        let packets_prov = prov_max.configure_packet_capture();
        let plan_prov = prov_max.plan_current_dispatch();
        let target_prov = plan_prov.physical_target_qpc().expect("provisional target");
        assert_no_work(prov_max.dispatch_at_qpc_for_test(
            &plan_prov,
            subtract_duration(target_prov, DurationTicks::from_raw(1)),
        ));
        let now_prov = add_us(&prov_max, target_prov, offset_beyond_max);
        assert_dispatched(prov_max.dispatch_at_qpc_for_test(&plan_prov, now_prov));
        assert_eq!(
            packets_prov.lock().expect("packet capture").len(),
            1,
            "normal lateness must not gate an authorized Down"
        );
    }

    #[test]
    fn normal_completion_floor_does_not_delay_mixed_packet() {
        let mut harness = ProductionDispatchTestHarness::new_mixed();
        let packets = harness.configure_packet_capture();
        let first = harness.plan_current_dispatch();
        let first_target = first.physical_target_qpc().expect("first Down target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &first,
            subtract_duration(first_target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&first, first_target));

        let mixed = harness.plan_current_dispatch();
        let physical = mixed.physical().expect("mixed physical plan");
        assert_eq!(
            physical.authored_view.packet_masks,
            PhysicalPacket::new(1, 2)
        );
        let mixed_target = mixed.physical_target_qpc().expect("mixed target");
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &mixed,
            subtract_duration(mixed_target, DurationTicks::from_raw(1)),
        ));

        let just_before_mixed = subtract_duration(mixed_target, DurationTicks::from_raw(1));
        harness
            .runtime
            .physical_timing_guard
            .as_mut()
            .expect("production guard initialized")
            .observe_successful_packet(just_before_mixed, 2, 0)
            .expect("seed an independent Down-key release floor");

        let window = harness
            .physical_floor_evidence_for_test(&mixed)
            .expect("mixed physical timing window");
        let release_floor = just_before_mixed
            .checked_add_duration(
                harness
                    .resources
                    .clock
                    .duration_from_us(harness.config.timing.min_release_gap_us)
                    .unwrap(),
            )
            .expect("mixed release floor");
        assert_eq!(
            window.down_not_before_qpc, release_floor,
            "window={window:?} release_floor={release_floor:?} mixed_target={mixed_target:?}"
        );
        assert!(window.musical_up_not_before_qpc >= mixed_target);
        let expected_wait_target = core::cmp::max(window.musical_up_not_before_qpc, release_floor);
        assert_eq!(window.packet_not_before_qpc, expected_wait_target);
        assert_eq!(window.hold_floor_mask, 1);
        assert_eq!(window.release_floor_mask, 2);
        let authored_wait_target = physical_wait_target_for_plan(&mixed, &harness.runtime)
            .expect("physical wait target")
            .expect("mixed target");
        assert_eq!(authored_wait_target, expected_wait_target);
        assert_no_work(harness.dispatch_at_qpc_for_test(&mixed, mixed_target));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&mixed, expected_wait_target));
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture,
            "the authorized Down boundary is consumed at its authored boundary"
        );
        assert!(harness.runtime.pending_up_recovery.is_none());
        assert!(harness.local_metrics.hold_floor_delay_boundaries >= 1);
        assert!(harness.local_metrics.release_floor_delay_boundaries >= 1);
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 2)],
            "normal completion floors must not split or drop the mixed packet"
        );
        assert_eq!(harness.backend_active_mask(), 2);
        assert!(harness.local_metrics.hold_floor_delay_boundaries >= 1);
        assert_eq!(harness.local_metrics.last_hold_floor_delay_mask, 1);
        assert!(harness.local_metrics.release_floor_delay_boundaries >= 1);
        assert_eq!(harness.local_metrics.production_hold_pair_samples, 1);
    }

    #[test]
    fn rapid_suspend_resume_stays_terminal_and_rejects_future_down() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let packets = harness.configure_packet_capture();
        let down = harness.plan_current_dispatch();
        let target = down.physical_target_qpc().expect("Down target");

        assert_no_work(harness.classify_future_plan_without_authorization_shortcut_for_test(&down));
        assert!(matches!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::FutureAuthorized(_)
        ));
        assert!(harness.notify_system_power_for_test(true));
        assert_eq!(
            harness.take_system_power_pending_for_test(),
            crate::engine::shared::SYSTEM_POWER_SUSPEND_PENDING
        );
        assert!(harness.system_power_down_blocked_for_test());
        assert!(harness.notify_system_power_for_test(false));
        assert_eq!(
            harness.take_system_power_pending_for_test(),
            crate::engine::shared::SYSTEM_POWER_RESUME_PENDING
        );
        assert!(harness.system_power_down_blocked_for_test());
        assert!(!harness.complete_system_resume_for_test());
        assert_no_work(harness.dispatch_at_qpc_for_test(&down, target));
        assert!(packets.lock().expect("packet capture").is_empty());
        assert!(harness.process_command_control_for_test());
        assert_eq!(
            harness.runtime.terminal_trigger,
            Some(super::super::TerminalTrigger::SystemSuspend)
        );
        let mut observation = super::super::FinalizeTestObservation::default();
        assert_eq!(
            harness.finalize_worker_for_test(false, &mut observation),
            crate::engine::OUTCOME_SYSTEM_SUSPENDED
        );
        assert!(packets.lock().expect("packet capture").is_empty());
    }

    #[test]
    fn blocked_future_down_stays_awaiting_future_before_classification() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let down = harness.plan_current_dispatch();
        let target = down.physical_target_qpc().expect("Down target");
        assert!(harness.notify_system_power_at_for_test(true, Some(QpcTicks::from_raw(123_456)),));

        assert_no_work(harness.dispatch_due_from_plan_for_test(&down));
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture,
            "suspend blocks authorization before future classification"
        );
        assert_no_work(harness.dispatch_at_qpc_for_test(&down, target));
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture
        );
    }

    #[test]
    fn simulated_sleep_sized_qpc_jump_keeps_expired_supervisor_lease_terminal() {
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let down = harness.plan_current_dispatch();
        assert!(harness.notify_system_power_for_test(true));
        assert!(harness.notify_system_power_for_test(false));
        let clock = harness.resources.clock;
        let heartbeat = clock.now().expect("heartbeat QPC");
        harness.set_supervisor_heartbeat_for_test(heartbeat);
        let timeout = clock.duration_from_us(3_000_000).expect("lease timeout");
        let jump = clock
            .duration_from_us(10_000_000)
            .expect("simulated sleep interval");
        let after_jump = heartbeat
            .checked_add_duration(jump)
            .expect("simulated QPC jump");
        let proof = harness
            .supervisor_expired
            .prove_down_age(after_jump, timeout);
        assert_eq!(
            proof.admission,
            crate::engine::shared::SupervisorLeaseAdmission::Expired
        );
        assert!(harness.system_power_down_blocked_for_test());
        assert!(harness.process_command_control_for_test());
        assert_eq!(
            harness.runtime.terminal_trigger,
            Some(super::super::TerminalTrigger::SupervisorLeaseExpired)
        );
        let target = down.physical_target_qpc().expect("Down target");
        assert_no_work(harness.dispatch_at_qpc_for_test(&down, target));
        let mut observation = super::super::FinalizeTestObservation::default();
        assert_eq!(
            harness.finalize_worker_for_test(false, &mut observation),
            crate::engine::OUTCOME_ERROR
        );
        assert_eq!(
            observation.terminal_trigger_reason.as_deref(),
            Some("supervisor_lease_expired")
        );
    }

    #[test]
    fn simulated_sleep_sized_qpc_jump_is_terminal_for_all_monitor_watchdog_worker_orders() {
        // These are the runnable orderings after a simulated sleep-sized QPC
        // discontinuity while the worker, lease monitor, and watchdog were
        // paused. The callback case delivers Resume before any worker poll.
        // No wall-clock sleep or operating-system suspend is involved.
        const ORDERS: [[u8; 3]; 6] = [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ];

        for suspend_callback in [false, true] {
            for order in ORDERS {
                let mut harness = ProductionDispatchTestHarness::new_down_only();
                let packets = harness.configure_packet_capture();
                let mut down = harness.plan_current_dispatch();
                for _ in 0..4 {
                    if down.physical().is_some() {
                        break;
                    }
                    let metadata_target = down
                        .physical_target_qpc()
                        .expect("metadata boundary target");
                    assert_dispatched(harness.dispatch_at_qpc_for_test(&down, metadata_target));
                    down = harness.plan_current_dispatch();
                }
                assert!(
                    down.physical().is_some(),
                    "fixture must expose a physical plan"
                );
                assert_ne!(
                    down.physical()
                        .expect("physical Down plan")
                        .authored_view
                        .packet_masks
                        .down_mask,
                    0,
                    "lease ordering coverage requires a Down-bearing physical plan"
                );
                let clock = harness.resources.clock;
                let heartbeat = clock.now().expect("initial supervisor heartbeat");
                harness.set_supervisor_heartbeat_for_test(heartbeat);
                let timeout = clock.duration_from_us(3_000_000).expect("lease timeout");
                harness
                    .set_supervisor_lease_timeout_for_test(3_000_000)
                    .expect("configure production lease timeout");
                let jump = clock
                    .duration_from_us(10_000_000)
                    .expect("simulated sleep interval");
                let after_jump = heartbeat
                    .checked_add_duration(jump)
                    .expect("simulated QPC jump");

                if suspend_callback {
                    assert!(harness.notify_system_power_for_test(true));
                    assert!(harness.notify_system_power_for_test(false));
                    assert!(harness.system_power_down_blocked_for_test());
                }

                let mut worker_exited = false;
                for actor in order {
                    match actor {
                        // The monitor's late heartbeat publication shares the
                        // production lease's fail-closed age check.
                        0 => assert!(
                            !harness
                                .supervisor_expired
                                .publish_progress(after_jump, timeout),
                            "late supervisor progress must not heal an expired lease"
                        ),
                        // The watchdog observes the same synthetic QPC value
                        // through the production lease expiration operation.
                        1 => assert!(
                            harness
                                .supervisor_expired
                                .check_expired(after_jump, timeout)
                        ),
                        // The worker can run before or after either other
                        // actor. If it runs before expiry without a power
                        // callback, its final Down age proof samples the
                        // simulated post-sleep QPC and latches expiry itself.
                        2 => {
                            worker_exited |= harness.process_command_control_for_test();
                            if !worker_exited {
                                let target =
                                    down.physical_target_qpc().expect("physical Down target");
                                harness
                                    .set_final_lease_qpc_sample_override_for_test(Ok(after_jump));
                                let dispatch_step = harness.dispatch_at_qpc_for_test(&down, target);
                                assert!(
                                    !matches!(dispatch_step, DispatchStep::Dispatched),
                                    "worker final Down age proof must reject the simulated expired lease; callback={suspend_callback}, order={order:?}, step={dispatch_step:?}"
                                );
                                worker_exited = harness.process_command_control_for_test();
                            }
                        }
                        _ => unreachable!("known runnable actor"),
                    }
                }

                assert!(harness.supervisor_expired.is_expired());
                if !worker_exited {
                    assert!(harness.process_command_control_for_test());
                }

                assert!(
                    packets.lock().expect("packet capture").is_empty(),
                    "no musical Down may reach the mock transport after the terminal lease decision; callback={suspend_callback}, order={order:?}, trigger={:?}",
                    harness.runtime.terminal_trigger
                );

                let mut observation = super::super::FinalizeTestObservation::default();
                assert_eq!(
                    harness.finalize_worker_for_test(false, &mut observation),
                    crate::engine::OUTCOME_ERROR,
                    "a ten-second lease expiry is a failure for callback={suspend_callback}, order={order:?}"
                );
                assert_eq!(
                    observation.terminal_trigger_reason.as_deref(),
                    Some("supervisor_lease_expired"),
                    "lease failure takes precedence over a clean suspend outcome"
                );
            }
        }
    }

    #[test]
    fn normal_completion_floor_delays_same_key_down_without_rejection() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(1);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let clock = harness.resources.clock;
        let release_gap = clock
            .duration_from_us(harness.config.timing.min_release_gap_us)
            .expect("release-gap ticks");
        let inside_margin = clock.duration_from_us(250).expect("inside margin");
        let release_floor = target
            .checked_add_duration(inside_margin)
            .expect("release floor");
        let completed_up = release_floor
            .as_u64()
            .checked_sub(release_gap.as_u64())
            .map(QpcTicks::from_raw)
            .expect("prior Up completion");
        harness
            .runtime
            .physical_timing_guard
            .as_mut()
            .expect("production guard initialized")
            .observe_successful_packet(completed_up, 1, 0)
            .expect("seed trusted prior Up completion");

        let window = harness
            .runtime
            .physical_timing_guard
            .as_ref()
            .unwrap()
            .query(target, 0, 1)
            .unwrap();
        assert_eq!(window.down_not_before_qpc, release_floor);
        assert_eq!(
            physical_wait_target_for_plan(&plan, &harness.runtime)
                .unwrap()
                .unwrap(),
            release_floor
        );
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_no_work(harness.dispatch_at_qpc_for_test(&plan, target));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, release_floor));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1)]
        );
    }

    #[test]
    fn control_interrupt_replans_without_completion_floor_delay() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(1);
        let packets = harness.configure_packet_capture();
        let clock = harness.resources.clock;
        let release_gap = clock
            .duration_from_us(harness.config.timing.min_release_gap_us)
            .expect("release-gap ticks");
        let base_hold = clock.duration_from_us(10_000).expect("base hold ticks");
        let margin = clock.duration_from_us(20_000).expect("timing margin");
        harness.timing.timing_margin_ticks = margin;
        harness
            .runtime
            .set_physical_timing_guard_for_test(base_hold, release_gap);
        harness.align_next_plan_to_benchmark_margin_for_test(5_000);
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let release_floor = target
            .checked_add_duration(clock.duration_from_us(10_000).expect("floor offset"))
            .expect("release floor");
        let completed_up = QpcTicks::from_raw(
            release_floor
                .as_u64()
                .checked_sub(release_gap.as_u64())
                .expect("prior Up completion"),
        );
        harness
            .runtime
            .physical_timing_guard
            .as_mut()
            .expect("production guard initialized")
            .observe_successful_packet(completed_up, 1, 0)
            .expect("seed trusted prior Up completion");

        let window = harness
            .runtime
            .physical_timing_guard
            .as_ref()
            .unwrap()
            .query(target, 0, 1)
            .unwrap();
        assert_eq!(window.down_not_before_qpc, release_floor);
        assert!(release_floor > target);
        assert_eq!(
            physical_wait_target_for_plan(&plan, &harness.runtime)
                .unwrap()
                .unwrap(),
            release_floor
        );
        assert!(harness.interrupt.signal(), "signal control interrupt");

        assert!(harness.wait_and_dispatch_current_plan(&plan).is_err());
        assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
        assert_eq!(harness.legacy_coordinator_fixture.cursor, 0);
        assert!(packets.lock().expect("packet capture").is_empty());
        assert!(matches!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::FutureAuthorized(_)
        ));
    }

    #[test]
    fn normal_completion_floor_delays_a_down_chord_without_expiring_it() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let clock = harness.resources.clock;
        let release_gap = clock
            .duration_from_us(harness.config.timing.min_release_gap_us)
            .expect("release-gap ticks");
        let beyond_authored_target = harness
            .timing
            .timing_margin_ticks
            .checked_add(DurationTicks::from_raw(1))
            .expect("floor delay beyond authored target");
        let release_floor = target
            .checked_add_duration(beyond_authored_target)
            .expect("release floor");
        let completed_up = release_floor
            .as_u64()
            .checked_sub(release_gap.as_u64())
            .map(QpcTicks::from_raw)
            .expect("prior Up completion");
        harness
            .runtime
            .physical_timing_guard
            .as_mut()
            .expect("production guard initialized")
            .observe_successful_packet(completed_up, 1, 0)
            .expect("seed trusted prior Up completion");

        assert_eq!(
            physical_wait_target_for_plan(&plan, &harness.runtime)
                .unwrap()
                .unwrap(),
            release_floor,
            "physical floor delays the authorized Down without rejecting it"
        );
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_no_work(harness.dispatch_at_qpc_for_test(&plan, target));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, release_floor));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 3)]
        );
        assert_eq!(harness.backend_active_mask(), 3);
    }

    #[test]
    fn several_unobserved_down_boundaries_drain_without_catch_up() {
        let mut harness = ProductionDispatchTestHarness::new_three_overdue_then_future();
        let packets = harness.configure_packet_capture();
        let mut first_plan = Some(harness.plan_current_dispatch());
        let first_target = first_plan
            .as_ref()
            .and_then(|plan| plan.physical_target_qpc())
            .expect("first Down target");
        let stalled_now = first_target
            .checked_add_duration(
                harness
                    .resources
                    .clock
                    .duration_from_us(3_500)
                    .expect("stalled-worker duration"),
            )
            .expect("stalled-worker QPC");

        for boundary in 0..4 {
            let plan = if boundary == 0 {
                first_plan.take().expect("first frozen plan")
            } else {
                harness.plan_current_dispatch()
            };
            let physical = plan.physical().expect("overdue Down plan");
            assert_ne!(physical.authored_view.packet_masks.down_mask, 0);
            assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, stalled_now));
        }
        assert_eq!(
            harness.local_metrics.missed_unobserved_backlog_boundaries,
            4
        );
        assert!(packets.lock().expect("packet capture").is_empty());

        let future = harness.plan_current_dispatch();
        let future_target = future.physical_target_qpc().expect("future Down target");
        assert!(future_target > stalled_now);
        assert_no_work(harness.dispatch_at_qpc_for_test(&future, stalled_now));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&future, future_target));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1 << 4)],
            "only the newly future-authorized Down may send"
        );
    }

    include!("dispatch_loop_observer_tests.rs");
    include!("dispatch_loop_failure_tests.rs");
}
