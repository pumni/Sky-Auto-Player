use super::super::shared::{
    SYSTEM_POWER_RESUME_PENDING, SYSTEM_POWER_SUSPEND_PENDING, SystemPowerState,
};
use super::super::{DurationTicks, QpcError, TimelineTicks, WaitOutcome, try_publish_metrics};
use super::dispatch::{DownBoundaryAdmission, PhysicalBoundaryStamp};
use super::wait::WaitObservation;
use super::{
    CommandControl, CommandControlClock, CommandControlInput, CommandControlMetrics,
    CommandControlRuntime, CommandControlSignals, PlanningInput, WaitBoundary, WaitBoundaryInput,
    WaitDeadline, WaitMutable, WaitSignals, Worker, ensure_preflight_for_target, enter_focus_pause,
    focus_matches, focus_matches_hwnd, load_target_stamp, plan_next_dispatch_projected,
    process_command_control, publish_backend_counters, publish_backend_metrics,
    record_wait_failure, supervisor_lease_expired, suspend_live_input, target_stamp_still_current,
    wait_for_next_boundary,
};
use super::{PreparedDispatchEntry, PreparedDispatchStream, dispatch_prepared_normal_frame};
use sky_dispatch_core::clock::PauseReason;
use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

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
fn normal_prepared_target_qpc(
    epoch_qpc: sky_dispatch_win32::clock::QpcTicks,
    offset_ticks: TimelineTicks,
) -> Result<sky_dispatch_win32::clock::QpcTicks, String> {
    epoch_qpc
        .checked_add_duration(DurationTicks::from_raw(offset_ticks.as_u64()))
        .map_err(|error| format!("prepared frame target arithmetic failure: {error}"))
}

#[inline]
pub(crate) fn normal_prepared_timing_window(
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    masks: sky_dispatch_win32::input::PhysicalPacket,
    timing_margin_ticks: DurationTicks,
) -> Result<super::physical_timing_guard::PhysicalTimingWindow, String> {
    let latest_down_start_qpc = if masks.down_mask == 0 {
        None
    } else {
        Some(
            physical_target_qpc
                .checked_add_duration(timing_margin_ticks)
                .map_err(|error| {
                    format!("prepared Down latest-start arithmetic failure: {error}")
                })?,
        )
    };
    Ok(super::physical_timing_guard::PhysicalTimingWindow {
        authored_target_qpc: physical_target_qpc,
        musical_up_not_before_qpc: physical_target_qpc,
        down_not_before_qpc: physical_target_qpc,
        packet_not_before_qpc: physical_target_qpc,
        latest_down_start_qpc,
        hold_floor_mask: 0,
        release_floor_mask: 0,
    })
}

#[inline]
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

pub(crate) fn physical_wait_target_for_plan(
    plan: &super::planning::NextDispatchPlan,
    runtime: &super::WorkerRuntime,
    strict_timing: bool,
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
        strict_timing,
    )
    .map_err(|error| format!("physical timing window query failed: {error:?}"))?;
    let boundary = physical_boundary_stamp(plan, physical_target_qpc);
    let wait_target = if let Some(pending) = runtime.pending_up_recovery.as_ref() {
        if !boundary.is_some_and(|boundary| pending.matches_authored_boundary(boundary)) {
            return Err("pending Up recovery no longer matches its authored boundary".to_string());
        }
        window.musical_up_not_before_qpc
    } else if masks.down_mask != 0
        && window
            .latest_down_start_qpc
            .is_some_and(|latest| window.packet_not_before_qpc > latest)
    {
        window.authored_target_qpc
    } else {
        window.packet_not_before_qpc
    };
    Ok(Some(wait_target))
}

fn physical_timing_window_for_packet(
    guard: &super::physical_timing_guard::PhysicalTimingGuard,
    authored_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    up_mask: u16,
    down_mask: u16,
    strict_timing: bool,
) -> Result<super::physical_timing_guard::PhysicalTimingWindow, String> {
    if strict_timing {
        guard.query(authored_target_qpc, up_mask, down_mask)
    } else {
        guard.authored_only_window(authored_target_qpc, up_mask, down_mask)
    }
    .map_err(|error| format!("physical timing window query failed: {error:?}"))
}

fn physical_timing_window_for_dispatch(
    physical_target_qpc: sky_dispatch_win32::clock::QpcTicks,
    masks: sky_dispatch_win32::input::PhysicalPacket,
    runtime: &super::WorkerRuntime,
    strict_timing: bool,
) -> Result<super::physical_timing_guard::PhysicalTimingWindow, String> {
    let Some(guard) = runtime.physical_timing_guard.as_ref() else {
        return Err("physical timing guard is not initialized".to_string());
    };
    physical_timing_window_for_packet(
        guard,
        physical_target_qpc,
        masks.up_mask,
        masks.down_mask,
        strict_timing,
    )
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

pub(crate) fn preflight_prepared_plan(
    plan: &mut super::planning::NextDispatchPlan,
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    runtime: &mut super::WorkerRuntime,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
) -> Result<bool, super::DispatchStep> {
    let Some(physical) = plan.physical_mut() else {
        return Ok(true);
    };
    let has_down_events = physical.authored_view.packet_masks.down_mask != 0;
    if !has_down_events {
        return Ok(true);
    }
    runtime.preparation_probe.record_preflight();
    let target = super::load_target_stamp(target_hwnd, target_generation);
    if let Err(error) =
        super::ensure_preflight_for_target(backend, target, &mut runtime.verified_target)
    {
        runtime.verified_target = None;
        return Err(super::DispatchStep::Terminate(format!(
            "instrument key preflight failed before timed wait; release the 15 instrument keys before playback: {error}"
        )));
    }
    if !super::target_stamp_still_current(target_hwnd, target_generation, target) {
        runtime.verified_target = None;
        runtime.invalidate_down_authorization();
        return Ok(false);
    }
    physical.target_proof = super::TargetProof::Verified(target);
    Ok(true)
}

/// Project the current QPC into playback time for stale metadata diagnostics.
///
/// A pre-epoch startup projection is intentionally zero, but once playback has
/// begun this reports the real elapsed timeline. This never enables or mutates
/// the physical pre-epoch startup admission path.
fn stale_metadata_effective_now(
    playback: &sky_dispatch_core::clock::PlaybackClockState,
    now_qpc: sky_dispatch_win32::clock::QpcTicks,
) -> Result<TimelineTicks, sky_dispatch_core::time::TimeArithmeticError> {
    if now_qpc < playback.epoch {
        Ok(TimelineTicks::ZERO)
    } else {
        playback.get_elapsed_allow_pre_epoch(now_qpc, false)
    }
}

/// Dispatch the work represented by one immutable plan. This helper is used
/// for both an already-due plan and a successful direct target wake, so a
/// normal timer wake never re-enters general orchestration before transport.
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_due_from_plan(
    plan: &super::planning::NextDispatchPlan,
    effective_now_ticks: TimelineTicks,
    now_ticks: sky_dispatch_win32::clock::QpcTicks,
    focus_loss_fault: bool,
    config: &super::WorkerConfig,
    resources: &mut super::WorkerResources,
    health: &mut super::WorkerHealthState,
    timing: &super::WorkerTimingState,
    runtime: &mut super::WorkerRuntime,
    local_metrics: &mut super::WorkerMetricsLocal,
    focus_active: &AtomicBool,
    target_hwnd: &AtomicIsize,
    target_generation: &AtomicU64,
    quit_requested: &AtomicBool,
    skip_requested: &AtomicBool,
    panic_requested: &AtomicBool,
    supervisor_expired: &AtomicBool,
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
        return resources
            .coordinator
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
    let masks = physical.authored_view.packet_masks;
    let window = match physical_timing_window_for_dispatch(
        physical_target_qpc,
        masks,
        runtime,
        config.timing.strict_timing,
    ) {
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
    let down_window_infeasible = masks.down_mask != 0
        && window
            .latest_down_start_qpc
            .is_some_and(|latest| window.packet_not_before_qpc > latest);
    let physical_wait_target = if pending_up_recovery.is_some() {
        window.musical_up_not_before_qpc
    } else if down_window_infeasible {
        window.authored_target_qpc
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
        if admission == DownBoundaryAdmission::Authorized
            && masks.down_mask != 0
            && window
                .latest_down_start_qpc
                .is_some_and(|latest| window.packet_not_before_qpc > latest)
        {
            DownBoundaryAdmission::PhysicalWindowExpired
        } else {
            admission
        }
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
            DownBoundaryAdmission::PhysicalWindowExpired => {
                super::dispatch::DownMissReason::PhysicalWindowExpired
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
            effective_now_ticks,
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
        health,
        timing,
        runtime,
        local_metrics,
        focus_active,
        target_hwnd,
        target_generation,
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
    authored_step
}

#[allow(clippy::too_many_arguments)]
fn suspend_live_input_and_reconcile_prepared(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    coordinator: &mut sky_dispatch_core::coordinator::RuntimeDispatchCoordinator,
    runtime: &mut super::WorkerRuntime,
    effective_now_ticks: Result<sky_dispatch_core::time::TimelineTicks, String>,
    target_hwnd: isize,
    prepared_stream: Option<&mut PreparedDispatchStream>,
) -> Result<(), String> {
    let cancelled = suspend_live_input(
        backend,
        coordinator,
        runtime,
        effective_now_ticks,
        target_hwnd,
    )?;
    if let Some(stream) = prepared_stream {
        stream.reconcile_resumable_suspension(&cancelled)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_system_suspend_transition(
    backend: &mut sky_dispatch_win32::input::TrackedKeyState,
    coordinator: &mut sky_dispatch_core::coordinator::RuntimeDispatchCoordinator,
    runtime: &mut super::WorkerRuntime,
    playback: &mut sky_dispatch_core::clock::PlaybackClockState,
    progress_clock: &super::super::shared::SharedProgressClock,
    now_ticks: sky_dispatch_win32::clock::QpcTicks,
    suspend_boundary_qpc: Option<sky_dispatch_win32::clock::QpcTicks>,
    target_hwnd: isize,
    prepared_stream: Option<&mut PreparedDispatchStream>,
) -> Result<(), String> {
    runtime.reset_wait_state_after_system_suspend();
    runtime.invalidate_down_authorization();
    runtime.verified_target = None;
    let effective_now = playback
        .get_elapsed_allow_pre_epoch(now_ticks, true)
        .map_err(|error| error.to_string());
    suspend_live_input_and_reconcile_prepared(
        backend,
        coordinator,
        runtime,
        effective_now,
        target_hwnd,
        prepared_stream,
    )?;
    runtime.manual_pause_suspension_pending = false;
    if let Some(guard) = runtime.physical_timing_guard.as_mut() {
        guard.reset();
    }
    runtime
        .production_forensics
        .observe_lifecycle(super::dispatch::observation::ObserverLifecycle::ResetAll);
    playback
        .enter_pause(
            PauseReason::SystemSuspend,
            suspend_boundary_qpc
                .ok_or_else(|| "system suspend QPC boundary was not captured".to_string())?,
        )
        .map_err(|error| format!("playback clock failure: {error}"))?;
    progress_clock.publish(playback);
    Ok(())
}

pub(crate) struct SystemResumeTransition<'a> {
    pub(crate) system_power: &'a SystemPowerState,
    pub(crate) config: &'a super::WorkerConfig,
    pub(crate) backend: &'a sky_dispatch_win32::input::TrackedKeyState,
    pub(crate) runtime: &'a mut super::WorkerRuntime,
    pub(crate) playback: &'a mut sky_dispatch_core::clock::PlaybackClockState,
    pub(crate) progress_clock: &'a super::super::shared::SharedProgressClock,
    pub(crate) qpc_clock: sky_dispatch_win32::clock::QpcClock,
    pub(crate) focus_active: &'a AtomicBool,
    pub(crate) target_hwnd: &'a AtomicIsize,
    pub(crate) target_generation: &'a AtomicU64,
    pub(crate) lease_timeout_ticks: DurationTicks,
    pub(crate) supervisor_heartbeat_ticks: &'a AtomicU64,
}

pub(crate) fn try_complete_system_resume_transition(
    suspend_applied: &mut bool,
    resume_pending: &mut bool,
    transition: SystemResumeTransition<'_>,
) -> Result<bool, String> {
    let SystemResumeTransition {
        system_power,
        config,
        backend,
        runtime,
        playback,
        progress_clock,
        qpc_clock,
        focus_active,
        target_hwnd,
        target_generation,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    } = transition;
    if !*suspend_applied || !*resume_pending || system_power.os_suspended() {
        return Ok(false);
    }
    let resume_target = load_target_stamp(target_hwnd, target_generation);
    let target_and_focus_current =
        target_stamp_still_current(target_hwnd, target_generation, resume_target)
            && focus_matches_hwnd(config.focus.require_focus, focus_active, resume_target.hwnd);
    if !target_and_focus_current {
        runtime.verified_target = None;
        return Ok(false);
    }
    ensure_preflight_for_target(backend, resume_target, &mut runtime.verified_target)
        .map_err(|error| format!("instrument key preflight failed after system resume: {error}"))?;
    let post_preflight_target = load_target_stamp(target_hwnd, target_generation);
    let preflight_and_focus_current = post_preflight_target == resume_target
        && target_stamp_still_current(target_hwnd, target_generation, resume_target)
        && focus_matches_hwnd(config.focus.require_focus, focus_active, resume_target.hwnd);
    let resumed_ticks = qpc_clock
        .now()
        .map_err(|error| format!("system resume lease QPC failure: {error:?}"))?;
    let lease_expired = supervisor_lease_expired(
        resumed_ticks,
        lease_timeout_ticks,
        supervisor_heartbeat_ticks,
    )
    .map_err(|error| format!("system resume lease QPC failure: {error:?}"))?;
    if !preflight_and_focus_current || lease_expired || !system_power.complete_resume() {
        runtime.verified_target = None;
        return Ok(false);
    }
    playback
        .exit_pause(PauseReason::SystemSuspend, resumed_ticks)
        .map_err(|error| format!("system resume playback clock failure: {error}"))?;
    system_power.clear_suspend_boundary();
    progress_clock.publish(playback);
    runtime.invalidate_down_authorization();
    if let Some(guard) = runtime.physical_timing_guard.as_mut() {
        guard.reset();
    }
    *suspend_applied = false;
    *resume_pending = false;
    Ok(true)
}

pub(super) fn dispatch(
    worker: &mut Worker<'_>,
    focus_loss_fault: bool,
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
    let target_hwnd = &shared.target.target_hwnd;
    let target_generation = &shared.target.target_generation;
    let metrics = &shared.publication.metrics;
    let supervisor_heartbeat_ticks = &shared.publication.supervisor_heartbeat_ticks;
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
                    core.runtime.force_full_cleanup = true;
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
                    core.runtime.force_full_cleanup = true;
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
        let resources = core
            .resources
            .as_mut()
            .expect("worker resources initialized");
        let qpc_clock = resources.clock;
        let mut prepared_stream = resources.prepared_stream.take();
        let mut system_suspend_applied = false;
        let mut system_resume_pending = false;
        while !resources.coordinator.is_finished() {
            // A deadline-wake sample belongs to exactly one physical send.
            // Re-entering the non-precision loop clears any stale sample
            // after interrupts, replans, command transitions, or failures.
            core.runtime.last_dispatch_deadline_wake_qpc = None;
            core.runtime.last_dispatch_deadline_target_qpc = None;
            if let CommandControl::Exit = process_command_control(CommandControlInput {
                clock: CommandControlClock { qpc_clock },
                signals: CommandControlSignals {
                    quit_requested,
                    skip_requested,
                    panic_requested,
                    supervisor_expired,
                    target_hwnd,
                },
                runtime: CommandControlRuntime {
                    backend: &mut resources.backend,
                    coordinator: &mut resources.coordinator,
                    force_full_cleanup: &mut core.runtime.force_full_cleanup,
                    terminal_error: &mut core.runtime.terminal_error,
                    secondary_errors: &mut core.errors.secondary,
                    abort_counts: &mut core.errors.abort_counts,
                },
                metrics: CommandControlMetrics {
                    local_metrics: &mut core.metrics,
                    metrics,
                    last_published_error: &mut core.errors.last_published,
                },
            }) {
                break;
            }

            let now_ticks = qpc_ticks_or_terminal!();
            let pending_power = system_power.take_pending();
            if pending_power & SYSTEM_POWER_RESUME_PENDING != 0 {
                system_resume_pending = true;
            }
            if pending_power & SYSTEM_POWER_SUSPEND_PENDING != 0 && !system_suspend_applied {
                if let Err(error) = apply_system_suspend_transition(
                    &mut resources.backend,
                    &mut resources.coordinator,
                    &mut core.runtime,
                    &mut resources.playback,
                    &shared.publication.progress_clock,
                    now_ticks,
                    system_power.suspend_boundary_qpc(),
                    target_hwnd.load(Ordering::Acquire),
                    prepared_stream.as_mut(),
                ) {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error =
                        Some(format!("system suspend safety release failed: {error}"));
                    break;
                }
                system_suspend_applied = true;
            }
            let focus_ok = focus_matches(config.focus.require_focus, focus_active);
            if startup_focus_loss_is_terminal(
                focus_ok,
                core.runtime.musical_physical_commit_started,
            ) {
                core.runtime.force_full_cleanup = true;
                core.runtime.terminal_error = Some("focus_lost_during_preroll".to_string());
                break;
            }
            let manual_pause = desired_pause.load(Ordering::Acquire);
            #[cfg(any(test, feature = "test-support"))]
            if command_timing.needs_observation() {
                let observed_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
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
                core.runtime.force_full_cleanup = true;
                core.runtime.terminal_error = Some("manual_pause_during_preroll".to_string());
                break;
            }

            if !system_suspend_applied && !focus_ok {
                let entered_focus_pause = match enter_focus_pause(
                    &mut resources.playback,
                    &mut core.runtime,
                    now_ticks,
                    &shared.publication.progress_clock,
                ) {
                    Ok(entered) => entered,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
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
                    try_publish_metrics(
                        &core.metrics,
                        metrics,
                        qpc_clock,
                        qpc_us_or_terminal!(),
                        true,
                    );
                }
            } else if !system_suspend_applied
                && resources.playback.has_pause_reason(PauseReason::Focus)
            {
                let restored_at = *core
                    .runtime
                    .focus_restore_started_ticks
                    .get_or_insert(now_ticks);
                let focus_grace_elapsed = match now_ticks.checked_duration_since(restored_at) {
                    Ok(elapsed) => elapsed,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("focus grace clock failure: {error}"));
                        break;
                    }
                };
                if focus_grace_elapsed >= timing.focus_restore_grace_ticks {
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    let manual_pause_active =
                        manual_pause || resources.playback.has_pause_reason(PauseReason::Manual);
                    core.runtime.verified_target = None;
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) {
                        if let Some(guard) = core.runtime.physical_timing_guard.as_mut() {
                            guard.invalidate();
                        }
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        continue;
                    }
                    if !manual_pause_active {
                        let lifecycle_effective_now = resources
                            .playback
                            .get_elapsed_allow_pre_epoch(
                                now_ticks,
                                core.runtime.allow_pre_epoch_startup_dispatch,
                            )
                            .map_err(|error| error.to_string());
                        if let Err(error) = suspend_live_input_and_reconcile_prepared(
                            &mut resources.backend,
                            &mut resources.coordinator,
                            &mut core.runtime,
                            lifecycle_effective_now,
                            preflight_target.hwnd,
                            prepared_stream.as_mut(),
                        ) {
                            core.runtime.verified_target = None;
                            core.runtime.force_full_cleanup = true;
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
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(format!(
                                "instrument key preflight failed during focus restoration; release the 15 instrument keys before playback: {error}"
                            ));
                            break;
                        }
                    } else if core.runtime.manual_pause_suspension_pending {
                        let lifecycle_effective_now = resources
                            .playback
                            .get_elapsed_allow_pre_epoch(
                                now_ticks,
                                core.runtime.allow_pre_epoch_startup_dispatch,
                            )
                            .map_err(|error| error.to_string());
                        if let Err(error) = suspend_live_input_and_reconcile_prepared(
                            &mut resources.backend,
                            &mut resources.coordinator,
                            &mut core.runtime,
                            lifecycle_effective_now,
                            preflight_target.hwnd,
                            prepared_stream.as_mut(),
                        ) {
                            core.runtime.verified_target = None;
                            core.runtime.force_full_cleanup = true;
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
                        hook(focus_active, target_hwnd, target_generation);
                    }
                    let post_restore_target = load_target_stamp(target_hwnd, target_generation);
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        post_restore_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) || post_restore_target != preflight_target
                    {
                        core.runtime.verified_target = None;
                        core.runtime.focus_restore_started_ticks = None;
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = resources
                        .playback
                        .exit_pause(PauseReason::Focus, resumed_ticks)
                    {
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
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
                    try_publish_metrics(
                        &core.metrics,
                        metrics,
                        qpc_clock,
                        qpc_us_or_terminal!(),
                        true,
                    );
                }
            }

            if manual_pause && !resources.playback.has_pause_reason(PauseReason::Manual) {
                core.runtime.invalidate_down_authorization();
                core.runtime.verified_target = None;
                if !resources.playback.is_paused() {
                    let lifecycle_effective_now = resources
                        .playback
                        .get_elapsed_allow_pre_epoch(
                            now_ticks,
                            core.runtime.allow_pre_epoch_startup_dispatch,
                        )
                        .map_err(|error| error.to_string());
                    if let Err(error) = suspend_live_input_and_reconcile_prepared(
                        &mut resources.backend,
                        &mut resources.coordinator,
                        &mut core.runtime,
                        lifecycle_effective_now,
                        target_hwnd.load(Ordering::Acquire),
                        prepared_stream.as_mut(),
                    ) {
                        core.runtime.force_full_cleanup = true;
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
                    try_publish_metrics(
                        &core.metrics,
                        metrics,
                        qpc_clock,
                        qpc_us_or_terminal!(),
                        true,
                    );
                } else if resources.playback.has_pause_reason(PauseReason::Focus)
                    && !resources
                        .playback
                        .has_pause_reason(PauseReason::SystemSuspend)
                {
                    core.runtime.manual_pause_suspension_pending = true;
                }
                if let Err(error) = resources
                    .playback
                    .enter_pause(PauseReason::Manual, now_ticks)
                {
                    core.runtime.force_full_cleanup = true;
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
                        let lifecycle_effective_now = resources
                            .playback
                            .get_elapsed_allow_pre_epoch(
                                now_ticks,
                                core.runtime.allow_pre_epoch_startup_dispatch,
                            )
                            .map_err(|error| error.to_string());
                        if let Err(error) = suspend_live_input_and_reconcile_prepared(
                            &mut resources.backend,
                            &mut resources.coordinator,
                            &mut core.runtime,
                            lifecycle_effective_now,
                            target_hwnd.load(Ordering::Acquire),
                            prepared_stream.as_mut(),
                        ) {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some(format!("manual resume deferred suspension failed: {error}"));
                            break;
                        }
                        core.runtime.manual_pause_suspension_pending = false;
                        core.runtime.production_forensics.observe_lifecycle(
                            super::dispatch::observation::ObserverLifecycle::ResetAll,
                        );
                    }
                    let preflight_target = load_target_stamp(target_hwnd, target_generation);
                    if let Err(error) = ensure_preflight_for_target(
                        &resources.backend,
                        preflight_target,
                        &mut core.runtime.verified_target,
                    ) {
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(format!(
                            "instrument key preflight failed on manual resume; release the 15 instrument keys before playback: {error}"
                        ));
                        break;
                    }
                    if !focus_matches_hwnd(
                        config.focus.require_focus,
                        focus_active,
                        preflight_target.hwnd,
                    ) || !target_stamp_still_current(
                        target_hwnd,
                        target_generation,
                        preflight_target,
                    ) {
                        core.runtime.verified_target = None;
                        continue;
                    }
                    let resumed_ticks = qpc_ticks_or_terminal!();
                    if let Err(error) = resources
                        .playback
                        .exit_pause(PauseReason::Manual, resumed_ticks)
                    {
                        core.runtime.verified_target = None;
                        core.runtime.force_full_cleanup = true;
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

            if let Err(error) = try_complete_system_resume_transition(
                &mut system_suspend_applied,
                &mut system_resume_pending,
                SystemResumeTransition {
                    system_power,
                    config,
                    backend: &resources.backend,
                    runtime: &mut core.runtime,
                    playback: &mut resources.playback,
                    progress_clock: &shared.publication.progress_clock,
                    qpc_clock,
                    focus_active,
                    target_hwnd,
                    target_generation,
                    lease_timeout_ticks: timing.lease_timeout_ticks,
                    supervisor_heartbeat_ticks,
                },
            ) {
                core.runtime.force_full_cleanup = true;
                core.runtime.terminal_error = Some(error);
                break;
            }

            #[cfg(any(test, feature = "test-support"))]
            if resources.playback.has_pause_reason(PauseReason::Manual)
                && command_timing.needs_acknowledgment()
            {
                let acknowledged_ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("QPC pause acknowledgment failed: {error:?}"));
                        break;
                    }
                };
                command_timing.acknowledge_pause(acknowledged_ticks);
            }

            let paused = resources.playback.is_paused();
            metrics.is_paused.store(paused, Ordering::Relaxed);
            if paused {
                let pause_target = match now_ticks.checked_add_duration(timing.paused_poll_ticks) {
                    Ok(target) => target,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("pause deadline arithmetic failure: {error}"));
                        break;
                    }
                };
                if let WaitOutcome::Failed(failure) = resources
                    .waiter
                    .wait_until_ticks_with_metrics_typed(
                        qpc_clock,
                        pause_target,
                        DurationTicks::ZERO,
                        interrupt,
                    )
                    .outcome
                {
                    record_wait_failure(
                        failure,
                        &mut core.metrics,
                        &mut core.runtime.force_full_cleanup,
                        &mut core.runtime.terminal_error,
                    );
                    break;
                }
                continue;
            }

            if prepared_stream
                .as_ref()
                .is_some_and(|stream| !stream.is_exhausted())
            {
                let (offset_ticks, has_physical) = {
                    let stream = prepared_stream
                        .as_ref()
                        .expect("prepared stream is present");
                    match stream.current().expect("prepared stream cursor is valid") {
                        PreparedDispatchEntry::Physical(frame) => (frame.offset_ticks, true),
                        PreparedDispatchEntry::Metadata { offset_ticks, .. } => {
                            (*offset_ticks, false)
                        }
                    }
                };
                let target_qpc =
                    match normal_prepared_target_qpc(resources.playback.epoch, offset_ticks) {
                        Ok(target) => target,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(error);
                            break;
                        }
                    };
                let preflight_target = if has_physical {
                    let frame = match prepared_stream.as_ref().and_then(|stream| stream.current()) {
                        Some(PreparedDispatchEntry::Physical(frame)) => frame,
                        _ => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(
                                "prepared physical cursor changed before preflight".to_string(),
                            );
                            break;
                        }
                    };
                    if frame.view.packet_masks.down_mask == 0 {
                        None
                    } else {
                        core.runtime.preparation_probe.record_preflight();
                        let target = load_target_stamp(target_hwnd, target_generation);
                        if let Err(error) = ensure_preflight_for_target(
                            &resources.backend,
                            target,
                            &mut core.runtime.verified_target,
                        ) {
                            core.runtime.verified_target = None;
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(format!(
                                "instrument key preflight failed before prepared wait; release the 15 instrument keys before playback: {error}"
                            ));
                            break;
                        }
                        if !target_stamp_still_current(target_hwnd, target_generation, target) {
                            core.runtime.verified_target = None;
                            continue;
                        }
                        Some(target)
                    }
                } else {
                    None
                };
                let timing_window = if has_physical {
                    let frame = match prepared_stream.as_ref().and_then(|stream| stream.current()) {
                        Some(PreparedDispatchEntry::Physical(frame)) => frame,
                        _ => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(
                                "prepared physical cursor changed before timing window".to_string(),
                            );
                            break;
                        }
                    };
                    match normal_prepared_timing_window(
                        target_qpc,
                        frame.view.packet_masks,
                        timing.timing_margin_ticks,
                    ) {
                        Ok(window) => Some(window),
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(error);
                            break;
                        }
                    }
                } else {
                    None
                };
                core.runtime.future_physical_wait_target_qpc = has_physical.then_some(target_qpc);

                let dispatch_result = if target_qpc <= now_ticks {
                    Some((None, target_qpc, now_ticks))
                } else {
                    match wait_for_next_boundary(WaitBoundaryInput {
                        deadline: WaitDeadline {
                            physical_target_qpc: Some(target_qpc),
                            spin_threshold_ticks: if has_physical {
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
                            force_full_cleanup: &mut core.runtime.force_full_cleanup,
                            terminal_error: &mut core.runtime.terminal_error,
                        },
                    }) {
                        WaitBoundary::Due {
                            wait_result,
                            target_qpc,
                            dispatch_qpc,
                            planned_wait_ticks,
                        } => {
                            if let Some(wait_result) = wait_result {
                                core.runtime.last_dispatch_deadline_wake_qpc = wait_result.wake_qpc;
                                core.runtime.last_dispatch_deadline_target_qpc = Some(target_qpc);
                                if core.observer.pending.is_some() {
                                    core.runtime.pending_wait_observation = Some(WaitObservation {
                                        outcome: wait_result.outcome,
                                        wake_qpc: wait_result.wake_qpc,
                                        spin_ticks: wait_result.spin_ticks,
                                        physical_target_qpc: target_qpc,
                                        planned_wait_ticks,
                                        deadline_ticks: offset_ticks,
                                        epoch_qpc: resources.playback.epoch,
                                        allow_pre_epoch_startup_dispatch: true,
                                    });
                                }
                            }
                            Some((wait_result, target_qpc, dispatch_qpc))
                        }
                        WaitBoundary::Replan {
                            wait_result,
                            target_qpc,
                            planned_wait_ticks,
                        } => {
                            core.runtime.pending_wait_observation = Some(WaitObservation {
                                outcome: wait_result.outcome,
                                wake_qpc: wait_result.wake_qpc,
                                spin_ticks: wait_result.spin_ticks,
                                physical_target_qpc: target_qpc,
                                planned_wait_ticks,
                                deadline_ticks: offset_ticks,
                                epoch_qpc: resources.playback.epoch,
                                allow_pre_epoch_startup_dispatch: true,
                            });
                            let _ = interrupt.try_take();
                            None
                        }
                        WaitBoundary::Exit => {
                            core.runtime.future_physical_wait_target_qpc = None;
                            break;
                        }
                    }
                };
                let Some((_wait_result, target_qpc, dispatch_qpc)) = dispatch_result else {
                    continue;
                };
                core.runtime.future_physical_wait_target_qpc = None;
                let effective_now_ticks = match resources
                    .playback
                    .get_elapsed_allow_pre_epoch(dispatch_qpc, true)
                {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(format!(
                            "playback clock failure at prepared boundary: {error}"
                        ));
                        break;
                    }
                };
                let step = if !has_physical {
                    let stream = prepared_stream
                        .as_mut()
                        .expect("prepared stream is present");
                    let commit = match stream.current() {
                        Some(PreparedDispatchEntry::Metadata { commit, .. }) => commit,
                        _ => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error =
                                Some("prepared metadata cursor changed before commit".to_string());
                            break;
                        }
                    };
                    match resources
                        .coordinator
                        .commit_prepared_authored_frame_metadata_frozen(commit)
                    {
                        Ok(()) => match stream.advance() {
                            Ok(()) => super::DispatchStep::Dispatched,
                            Err(error) => super::DispatchStep::TerminateStatic(error),
                        },
                        Err(error) => super::DispatchStep::Terminate(format!(
                            "prepared metadata commit failure: {error}"
                        )),
                    }
                } else {
                    let stream = prepared_stream
                        .as_ref()
                        .expect("prepared stream is present");
                    let Some(PreparedDispatchEntry::Physical(frame)) = stream.current() else {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("prepared physical cursor changed before dispatch".to_string());
                        break;
                    };
                    dispatch_prepared_normal_frame(
                        frame,
                        config,
                        resources,
                        core.health.as_mut().unwrap(),
                        &timing,
                        &mut core.runtime,
                        &mut core.metrics,
                        focus_active,
                        target_hwnd,
                        target_generation,
                        quit_requested,
                        skip_requested,
                        panic_requested,
                        desired_pause,
                        supervisor_expired,
                        system_power,
                        &shared.publication.progress_clock,
                        core.observer.pending.as_ref(),
                        preflight_target,
                        target_qpc,
                        timing_window.expect("prepared physical timing window"),
                        effective_now_ticks,
                        dispatch_qpc,
                        focus_loss_fault,
                        Some(dispatch_qpc),
                        stream.explicitly_cancelled_generation_ids(),
                        #[cfg(any(test, feature = "test-support"))]
                        false,
                    )
                };
                if matches!(&step, super::DispatchStep::Dispatched) && has_physical {
                    if let Some(stream) = prepared_stream.as_mut()
                        && let Err(error) = stream.advance()
                    {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(error.to_string());
                        break;
                    }
                    if metrics.live_diagnostics_enabled.load(Ordering::Relaxed) {
                        publish_backend_counters(&resources.backend, &mut core.metrics);
                        publish_live_metrics_after_dispatch(
                            &core.metrics,
                            metrics,
                            qpc_clock,
                            dispatch_qpc,
                        );
                    }
                }
                match step {
                    super::DispatchStep::Dispatched | super::DispatchStep::Continue => continue,
                    super::DispatchStep::NoWork => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("prepared frame did not complete its dispatch step".to_string());
                        break;
                    }
                    super::DispatchStep::Terminate(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(error);
                        break;
                    }
                    super::DispatchStep::TerminateStatic(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error = Some(error.to_string());
                        break;
                    }
                }
            }

            // Stale metadata is globally non-physical. Commit at most one
            // compiled packet per outer iteration, regardless of startup
            // phase, so every control/focus/pause/lease gate is re-admitted.
            match resources.coordinator.prepare_current_stale_packet() {
                Ok(Some(prepared)) => {
                    let stale_now =
                        match stale_metadata_effective_now(&resources.playback, now_ticks) {
                            Ok(ticks) => ticks,
                            Err(error) => {
                                core.runtime.force_full_cleanup = true;
                                core.runtime.terminal_error = Some(format!(
                                    "stale metadata clock projection failure: {error}"
                                ));
                                break;
                            }
                        };
                    match super::dispatch_stale_packet(
                        prepared,
                        &mut resources.coordinator,
                        core.observer.pending.as_ref(),
                        &mut core.metrics.observer_dropped_samples,
                        &mut core.metrics.observer_queue_high_watermark,
                        stale_now,
                    ) {
                        super::DispatchStep::Dispatched => {
                            #[cfg(any(test, feature = "test-support"))]
                            if let Some(hook) = core.runtime.startup_ordering_hook.as_ref() {
                                hook.mark_stale_packet_committed();
                            }
                            continue;
                        }
                        super::DispatchStep::Terminate(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(error);
                            break;
                        }
                        super::DispatchStep::TerminateStatic(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some((*error).to_string());
                            break;
                        }
                        super::DispatchStep::Continue | super::DispatchStep::NoWork => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(
                                "stale packet did not complete its metadata commit".to_string(),
                            );
                            break;
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!(
                        "coordinator stale-packet preparation failure: {error}"
                    ));
                    break;
                }
            }

            let effective_now_ticks = if now_ticks < resources.playback.epoch {
                TimelineTicks::ZERO
            } else {
                match resources
                    .playback
                    .get_elapsed_allow_pre_epoch(now_ticks, false)
                {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some(format!("playback clock failure: {error}"));
                        break;
                    }
                }
            };
            // Wait evidence is also deferred through the nonblocking producer
            // path. The observer consumer owns all conversion and health work.
            if let (Some(wait_observation), Some(observer)) = (
                core.runtime.pending_wait_observation.take(),
                core.observer.pending.as_ref(),
            ) {
                observer.push_wait(
                    wait_observation,
                    &mut core.metrics.observer_dropped_samples,
                    &mut core.metrics.observer_queue_high_watermark,
                );
            }
            let mut dispatch_plan = super::planning::NextDispatchPlan::default();
            match plan_next_dispatch_projected(
                PlanningInput {
                    coordinator: &resources.coordinator,
                    epoch_qpc: resources.playback.epoch,
                    preparation_probe: &core.runtime.preparation_probe,
                    instrument_key_profile: resources.backend.instrument_key_profile(),
                },
                &mut dispatch_plan,
            ) {
                Ok(()) => {}
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!("planning failure: {error}"));
                    break;
                }
            };
            match preflight_prepared_plan(
                &mut dispatch_plan,
                &mut resources.backend,
                &mut core.runtime,
                target_hwnd,
                target_generation,
            ) {
                Ok(true) => {}
                Ok(false) => continue,
                Err(super::DispatchStep::Terminate(error)) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(error);
                    break;
                }
                Err(step) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(format!(
                        "unexpected preflight preparation outcome: {step:?}"
                    ));
                    break;
                }
            }
            let authored_step = dispatch_due_from_plan(
                &dispatch_plan,
                effective_now_ticks,
                now_ticks,
                focus_loss_fault,
                config,
                resources,
                core.health.as_mut().unwrap(),
                &timing,
                &mut core.runtime,
                &mut core.metrics,
                focus_active,
                target_hwnd,
                target_generation,
                quit_requested,
                skip_requested,
                panic_requested,
                supervisor_expired,
                desired_pause,
                system_power,
                &shared.publication.progress_clock,
                core.observer.pending.as_ref(),
                None,
                false,
                #[cfg(any(test, feature = "test-support"))]
                None,
                #[cfg(any(test, feature = "test-support"))]
                false,
            );
            if matches!(&authored_step, super::DispatchStep::Dispatched)
                && metrics.live_diagnostics_enabled.load(Ordering::Relaxed)
            {
                publish_backend_counters(&resources.backend, &mut core.metrics);
                publish_live_metrics_after_dispatch(&core.metrics, metrics, qpc_clock, now_ticks);
            }
            match authored_step {
                super::DispatchStep::Dispatched | super::DispatchStep::Continue => continue,
                super::DispatchStep::NoWork => {}
                super::DispatchStep::Terminate(err) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(err);
                    break;
                }
                super::DispatchStep::TerminateStatic(err) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(err.to_string());
                    break;
                }
            }

            let deadline_ticks = dispatch_plan.deadline_ticks();
            let physical_wait_target_qpc = match physical_wait_target_for_plan(
                &dispatch_plan,
                &core.runtime,
                timing.strict_timing,
            ) {
                Ok(target) => target,
                Err(error) => {
                    core.runtime.force_full_cleanup = true;
                    core.runtime.terminal_error = Some(error);
                    break;
                }
            };
            core.runtime.future_physical_wait_target_qpc = if matches!(
                &dispatch_plan,
                super::planning::NextDispatchPlan::Physical(_)
            ) {
                dispatch_plan.physical_target_qpc()
            } else {
                None
            };

            match wait_for_next_boundary(WaitBoundaryInput {
                deadline: WaitDeadline {
                    physical_target_qpc: physical_wait_target_qpc,
                    spin_threshold_ticks: if matches!(
                        dispatch_plan,
                        super::planning::NextDispatchPlan::Physical(_)
                    ) {
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
                    force_full_cleanup: &mut core.runtime.force_full_cleanup,
                    terminal_error: &mut core.runtime.terminal_error,
                },
            }) {
                WaitBoundary::Due {
                    wait_result,
                    target_qpc,
                    dispatch_qpc,
                    planned_wait_ticks,
                } => {
                    // This is the global wait boundary only. Physical target
                    // attribution is resolved from the selected work below.
                    let _wait_boundary_target_qpc = target_qpc;
                    let Some(wait_deadline_ticks) = deadline_ticks else {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("wait returned a result without a dispatch deadline".to_string());
                        break;
                    };
                    if let Some(wait_result) = wait_result {
                        core.runtime.last_dispatch_deadline_wake_qpc = wait_result.wake_qpc;
                        core.runtime.last_dispatch_deadline_target_qpc = Some(target_qpc);
                        if core.observer.pending.is_some() {
                            core.runtime.pending_wait_observation = Some(WaitObservation {
                                outcome: wait_result.outcome,
                                wake_qpc: wait_result.wake_qpc,
                                spin_ticks: wait_result.spin_ticks,
                                physical_target_qpc: target_qpc,
                                planned_wait_ticks,
                                deadline_ticks: wait_deadline_ticks,
                                epoch_qpc: resources.playback.epoch,
                                allow_pre_epoch_startup_dispatch: true,
                            });
                        }
                    }
                    core.runtime.future_physical_wait_target_qpc = None;
                    let dispatch_now_ticks = dispatch_qpc;
                    let dispatch_effective_now = match resources
                        .playback
                        .get_elapsed_allow_pre_epoch(dispatch_now_ticks, true)
                    {
                        Ok(ticks) => ticks,
                        Err(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(format!(
                                "playback clock failure after deadline wake: {error}"
                            ));
                            break;
                        }
                    };
                    let authored_step = dispatch_due_from_plan(
                        &dispatch_plan,
                        dispatch_effective_now,
                        dispatch_now_ticks,
                        focus_loss_fault,
                        config,
                        resources,
                        core.health.as_mut().unwrap(),
                        &timing,
                        &mut core.runtime,
                        &mut core.metrics,
                        focus_active,
                        target_hwnd,
                        target_generation,
                        quit_requested,
                        skip_requested,
                        panic_requested,
                        supervisor_expired,
                        desired_pause,
                        system_power,
                        &shared.publication.progress_clock,
                        core.observer.pending.as_ref(),
                        Some(dispatch_qpc),
                        true,
                        #[cfg(any(test, feature = "test-support"))]
                        None,
                        #[cfg(any(test, feature = "test-support"))]
                        false,
                    );
                    if matches!(&authored_step, super::DispatchStep::Dispatched)
                        && metrics.live_diagnostics_enabled.load(Ordering::Relaxed)
                    {
                        publish_backend_counters(&resources.backend, &mut core.metrics);
                        publish_live_metrics_after_dispatch(
                            &core.metrics,
                            metrics,
                            qpc_clock,
                            dispatch_now_ticks,
                        );
                    }
                    match authored_step {
                        super::DispatchStep::Terminate(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(error);
                            break;
                        }
                        super::DispatchStep::TerminateStatic(error) => {
                            core.runtime.force_full_cleanup = true;
                            core.runtime.terminal_error = Some(error.to_string());
                            break;
                        }
                        super::DispatchStep::Dispatched
                        | super::DispatchStep::Continue
                        | super::DispatchStep::NoWork => continue,
                    }
                }
                WaitBoundary::Replan {
                    wait_result,
                    target_qpc,
                    planned_wait_ticks,
                } => {
                    core.runtime.future_physical_wait_target_qpc = None;
                    let Some(wait_deadline_ticks) = deadline_ticks else {
                        core.runtime.force_full_cleanup = true;
                        core.runtime.terminal_error =
                            Some("replan wait result without a dispatch deadline".to_string());
                        break;
                    };
                    core.runtime.pending_wait_observation = Some(WaitObservation {
                        outcome: wait_result.outcome,
                        wake_qpc: wait_result.wake_qpc,
                        spin_ticks: wait_result.spin_ticks,
                        physical_target_qpc: target_qpc,
                        planned_wait_ticks,
                        deadline_ticks: wait_deadline_ticks,
                        epoch_qpc: resources.playback.epoch,
                        allow_pre_epoch_startup_dispatch: true,
                    });
                    // Event consumption is intentionally outside the
                    // precision boundary.  A signal racing this drain is
                    // harmless; the next blocking wait will replan.
                    let _ = interrupt.try_take();
                    continue;
                }
                WaitBoundary::Exit => break,
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        normal_prepared_timing_window, physical_target_qpc_for_work, physical_wait_target_for_plan,
        publish_live_metrics_after_dispatch,
    };
    use crate::engine::shared::{SYSTEM_POWER_RESUME_PENDING, SYSTEM_POWER_SUSPEND_PENDING};
    use crate::engine::telemetry::metrics::{SharedMetrics, WorkerMetricsLocal};
    use crate::engine::test_support::ProductionDispatchTestHarness;
    use crate::engine::worker::{
        DownBoundaryState, PhysicalBoundaryStamp, WorkerRuntime, dispatch::DispatchStep,
    };
    use sky_dispatch_core::clock::PauseReason;
    use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
    use sky_dispatch_win32::clock::QpcClock;
    use sky_dispatch_win32::input::{
        PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome,
        SendTransactionStatus,
    };
    use std::num::NonZeroU64;
    use std::sync::atomic::Ordering;

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

    struct ForegroundOverrideResetGuard;

    impl Drop for ForegroundOverrideResetGuard {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    fn set_zero_timing_margin(harness: &mut ProductionDispatchTestHarness) {
        let clock = harness.resources.clock;
        let frame = clock
            .duration_from_us(harness.config.timing.frame_us)
            .expect("frame duration");
        let base_hold = clock
            .duration_from_us(harness.config.timing.frame_base_hold_us)
            .expect("base hold duration");
        harness.timing.timing_margin_ticks = DurationTicks::ZERO;
        harness
            .runtime
            .set_physical_timing_guard_for_test(base_hold, frame, DurationTicks::ZERO);
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
    fn normal_prepared_timing_window_is_authored_only() {
        let target = QpcTicks::from_raw(1_000);
        let margin = DurationTicks::from_raw(50);
        let window =
            normal_prepared_timing_window(target, PhysicalPacket::new(0b001, 0b010), margin)
                .expect("normal prepared timing window");

        assert_eq!(window.authored_target_qpc, target);
        assert_eq!(window.musical_up_not_before_qpc, target);
        assert_eq!(window.down_not_before_qpc, target);
        assert_eq!(window.packet_not_before_qpc, target);
        assert_eq!(
            window.latest_down_start_qpc,
            Some(QpcTicks::from_raw(1_050))
        );
        assert_eq!(window.hold_floor_mask, 0);
        assert_eq!(window.release_floor_mask, 0);
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
            .expect("one tick beyond zero-margin latest start");
        assert_dispatched(late.dispatch_at_qpc_for_test(&late_plan, one_tick_late));
        assert_eq!(late_packets.lock().expect("packet capture").len(), 1);
        assert_eq!(late.local_metrics.final_sender_window_expirations, 0);
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
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
        assert_eq!(harness.local_metrics.final_sender_window_expirations, 0);
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
                harness.local_metrics.final_sender_window_expirations, 0,
                "offset {offset_us}us must not be a sender miss"
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
    fn c1_strict_cutoff_remains_physical_latest_start() {
        let mut allowed = ProductionDispatchTestHarness::new_down_only();
        allowed.set_strict_timing_for_test(true);
        allowed.timing.strict_down_completion_late_ticks = allowed
            .resources
            .clock
            .duration_from_us(3_000)
            .expect("strict completion test allowance");
        let packets = allowed.configure_packet_capture();
        let plan = allowed.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_no_work(allowed.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(allowed.dispatch_at_qpc_for_test(&plan, add_us(&allowed, target, 500)));
        assert_eq!(packets.lock().expect("packet capture").len(), 1);
        assert_eq!(allowed.local_metrics.final_sender_window_expirations, 0);

        let mut rejected = ProductionDispatchTestHarness::new_down_only();
        rejected.set_strict_timing_for_test(true);
        rejected.timing.strict_down_completion_late_ticks = rejected
            .resources
            .clock
            .duration_from_us(3_000)
            .expect("strict completion test allowance");
        let packets = rejected.configure_packet_capture();
        let plan = rejected.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        assert_no_work(rejected.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        let physical_cutoff = add_us(&rejected, target, 500);
        let one_tick_beyond = physical_cutoff
            .checked_add_duration(DurationTicks::from_raw(1))
            .expect("one tick beyond strict cutoff");
        assert!(matches!(
            rejected.dispatch_at_qpc_for_test(&plan, one_tick_beyond),
            DispatchStep::TerminateStatic("down_final_sender_window_expired")
        ));
        assert!(packets.lock().expect("packet capture").is_empty());
        assert_eq!(rejected.local_metrics.final_sender_window_expirations, 1);
    }

    #[test]
    fn strict_timing_retains_physical_latest_start_rejection() {
        {
            let mut rejected = ProductionDispatchTestHarness::new_down_only();
            rejected.set_strict_timing_for_test(true);
            rejected.timing.strict_down_completion_late_ticks = rejected
                .resources
                .clock
                .duration_from_us(10_000)
                .expect("strict completion test allowance");
            let packets = rejected.configure_packet_capture();
            let plan = rejected.plan_current_dispatch();
            let target = plan.physical_target_qpc().expect("Down target");
            assert_no_work(rejected.dispatch_at_qpc_for_test(
                &plan,
                subtract_duration(target, DurationTicks::from_raw(1)),
            ));

            let physical_cutoff = add_us(&rejected, target, 500);
            let one_tick_beyond = physical_cutoff
                .checked_add_duration(DurationTicks::from_raw(1))
                .expect("one tick beyond strict cutoff");
            assert!(matches!(
                rejected.dispatch_at_qpc_for_test(&plan, one_tick_beyond),
                DispatchStep::TerminateStatic("down_final_sender_window_expired")
            ));
            assert!(packets.lock().expect("packet capture").is_empty());
            assert_eq!(rejected.local_metrics.final_sender_window_expirations, 1);

            // In contrast, under normal timing the note is sent without a cutoff.
            let mut normal = ProductionDispatchTestHarness::new_down_only();
            let normal_packets = normal.configure_packet_capture();
            let normal_plan = normal.plan_current_dispatch();
            let normal_target = normal_plan.physical_target_qpc().expect("Down target");
            assert_no_work(normal.dispatch_at_qpc_for_test(
                &normal_plan,
                subtract_duration(normal_target, DurationTicks::from_raw(1)),
            ));
            let late_now = add_us(&normal, normal_target, 1_000);
            assert_dispatched(normal.dispatch_at_qpc_for_test(&normal_plan, late_now));
            assert_eq!(
                normal_packets.lock().expect("normal packet capture").len(),
                1
            );
        }
    }

    #[test]
    fn normal_completion_floor_does_not_make_down_infeasible() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let clock = harness.resources.clock;
        let frame = clock.duration_from_us(16_667).expect("frame ticks");
        let beyond_latest = harness
            .timing
            .timing_margin_ticks
            .checked_add(DurationTicks::from_raw(1))
            .expect("latest-start plus one tick");
        let release_floor = target
            .checked_add_duration(beyond_latest)
            .expect("release floor");
        let completed_up = release_floor
            .as_u64()
            .checked_sub(frame.as_u64())
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
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, target));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 3)]
        );
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
        assert_eq!(harness.local_metrics.final_sender_window_expirations, 0);
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
        assert_eq!(harness.local_metrics.final_sender_window_expirations, 0);
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
        assert_eq!(harness.local_metrics.final_sender_window_expirations, 0);
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
        assert_eq!(harness.local_metrics.final_sender_window_expirations, 0);
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
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
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
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
                assert_eq!(
                    harness.missed_physical_window_boundaries_for_test(),
                    0,
                    "continuity must not bypass physical timing rules"
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
            assert_dispatched(harness.dispatch_at_phase_a_benchmark_boundary_for_test(
                &first_down,
                first_completion_delay_us,
            ));

            let first_up = harness.plan_current_dispatch();
            let first_up_evidence = harness
                .prepared_boundary_evidence_for_test(&first_up)
                .expect("first Up evidence");
            assert_dispatched(
                harness.dispatch_at_phase_a_benchmark_boundary_for_test(&first_up, 0),
            );

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
            assert_dispatched(
                harness.dispatch_at_phase_a_benchmark_boundary_for_test(&second_down, 0),
            );
            assert_eq!(harness.missed_physical_window_boundaries_for_test(), 0);

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
    fn sparse_comparison_sends_authorized_late_note_ons_without_cutoff() {
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
            assert_eq!(baseline.local_metrics.final_sender_window_expirations, 0);

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
            assert_eq!(prov_max.local_metrics.final_sender_window_expirations, 0);
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
        assert_eq!(prov_max.local_metrics.final_sender_window_expirations, 0);
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
            .expect("mixed authored-only timing window");
        assert_eq!(window.musical_up_not_before_qpc, mixed_target);
        assert_eq!(window.down_not_before_qpc, mixed_target);
        assert_eq!(window.packet_not_before_qpc, mixed_target);
        assert_eq!(window.hold_floor_mask, 0);
        assert_eq!(window.release_floor_mask, 0);
        let authored_wait_target =
            physical_wait_target_for_plan(&mixed, &harness.runtime, harness.timing.strict_timing)
                .expect("physical wait target")
                .expect("mixed target");
        assert_eq!(authored_wait_target, mixed_target);
        assert_dispatched(harness.dispatch_at_qpc_for_test(&mixed, mixed_target));
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture,
            "the authorized Down boundary is consumed at its authored boundary"
        );
        assert!(harness.runtime.pending_up_recovery.is_none());
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
        assert_eq!(harness.local_metrics.release_floor_infeasible_boundaries, 0);
        assert_eq!(harness.local_metrics.hold_floor_delay_boundaries, 0);
        assert_eq!(harness.local_metrics.release_floor_delay_boundaries, 0);
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 2)],
            "normal completion floors must not split or drop the mixed packet"
        );
        assert_eq!(harness.backend_active_mask(), 2);
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
        assert_eq!(harness.local_metrics.hold_floor_delay_boundaries, 0);
        assert_eq!(harness.local_metrics.last_hold_floor_delay_mask, 0);
        assert_eq!(harness.local_metrics.release_floor_delay_boundaries, 0);
        assert_eq!(harness.local_metrics.production_hold_pair_samples, 1);
    }

    #[test]
    fn system_suspend_invalidates_future_down_and_resume_requires_fresh_authorization() {
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
            SYSTEM_POWER_SUSPEND_PENDING
        );
        assert!(harness.system_power_down_blocked_for_test());
        let suspend_qpc = harness.resources.clock.now().expect("suspend QPC");
        harness
            .apply_system_suspend_for_test(suspend_qpc)
            .expect("suspend safety release and pause");
        assert_eq!(harness.full_instrument_release_calls(), 1);
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture
        );
        assert!(harness.system_power_down_blocked_for_test());
        assert!(packets.lock().expect("packet capture").is_empty());
        assert_no_work(harness.dispatch_at_qpc_for_test(&down, target));
        assert!(packets.lock().expect("packet capture").is_empty());

        assert!(harness.notify_system_power_for_test(false));
        assert_eq!(
            harness.take_system_power_pending_for_test(),
            SYSTEM_POWER_RESUME_PENDING
        );
        assert!(harness.system_power_down_blocked_for_test());
        assert!(
            harness
                .try_system_resume_for_test(DurationTicks::ZERO)
                .expect("system resume gates")
        );
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture
        );
        assert_no_work(harness.classify_future_plan_without_authorization_shortcut_for_test(&down));
        assert!(matches!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::FutureAuthorized(_)
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&down, target));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 1)]
        );
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
    fn system_resume_remains_blocked_until_focus_and_lease_are_fresh() {
        let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
        let _foreground_reset = ForegroundOverrideResetGuard;
        let mut harness = ProductionDispatchTestHarness::new_down_only();
        let _down = harness.plan_current_dispatch();
        assert!(harness.notify_system_power_for_test(true));
        assert_eq!(
            harness.take_system_power_pending_for_test(),
            SYSTEM_POWER_SUSPEND_PENDING
        );
        let suspend_qpc = harness.resources.clock.now().expect("suspend QPC");
        harness
            .apply_system_suspend_for_test(suspend_qpc)
            .expect("suspend transition");
        assert!(harness.notify_system_power_for_test(false));
        assert_eq!(
            harness.take_system_power_pending_for_test(),
            SYSTEM_POWER_RESUME_PENDING
        );

        harness.config.focus.require_focus = true;
        harness.focus_active.store(false, Ordering::Release);
        assert!(
            !harness
                .try_system_resume_for_test(DurationTicks::ZERO)
                .expect("focus rejection")
        );
        assert!(harness.system_power_down_blocked_for_test());
        assert!(
            harness
                .resources
                .playback
                .has_pause_reason(PauseReason::SystemSuspend)
        );

        sky_dispatch_win32::focus::set_foreground_window_for_test(Some(1));
        harness.focus_active.store(true, Ordering::Release);
        harness.set_supervisor_heartbeat_for_test(QpcTicks::from_raw(1));
        let lease_timeout = DurationTicks::from_raw(5_000_000);
        assert!(
            !harness
                .try_system_resume_for_test(lease_timeout)
                .expect("expired lease rejection")
        );
        assert!(harness.system_power_down_blocked_for_test());

        let fresh_heartbeat = harness.resources.clock.now().expect("fresh QPC");
        harness.set_supervisor_heartbeat_for_test(fresh_heartbeat);
        assert!(
            harness
                .try_system_resume_for_test(lease_timeout)
                .expect("fully revalidated resume")
        );
        assert!(!harness.system_power_down_blocked_for_test());
        assert!(
            !harness
                .resources
                .playback
                .has_pause_reason(PauseReason::SystemSuspend)
        );
        assert_eq!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::AwaitingFuture
        );
    }

    #[test]
    fn normal_completion_floor_does_not_delay_same_key_down() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(1);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let clock = harness.resources.clock;
        let frame = clock.duration_from_us(16_667).expect("frame ticks");
        let inside_margin = clock.duration_from_us(250).expect("inside margin");
        let release_floor = target
            .checked_add_duration(inside_margin)
            .expect("release floor");
        let completed_up = release_floor
            .as_u64()
            .checked_sub(frame.as_u64())
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
        assert!(release_floor <= window.latest_down_start_qpc.unwrap());
        assert_eq!(
            physical_wait_target_for_plan(&plan, &harness.runtime, harness.timing.strict_timing)
                .unwrap()
                .unwrap(),
            target
        );
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, target));
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
        let frame = clock.duration_from_us(16_667).expect("frame ticks");
        let base_hold = clock.duration_from_us(10_000).expect("base hold ticks");
        let margin = clock.duration_from_us(20_000).expect("timing margin");
        harness.timing.timing_margin_ticks = margin;
        harness
            .runtime
            .set_physical_timing_guard_for_test(base_hold, frame, margin);
        harness.align_next_plan_to_benchmark_margin_for_test(5_000);
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let release_floor = target
            .checked_add_duration(clock.duration_from_us(10_000).expect("floor offset"))
            .expect("release floor");
        let completed_up = QpcTicks::from_raw(
            release_floor
                .as_u64()
                .checked_sub(frame.as_u64())
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
        assert!(release_floor <= window.latest_down_start_qpc.unwrap());
        assert_eq!(
            physical_wait_target_for_plan(&plan, &harness.runtime, harness.timing.strict_timing)
                .unwrap()
                .unwrap(),
            target
        );
        assert!(harness.interrupt.signal(), "signal control interrupt");

        assert!(harness.wait_and_dispatch_current_plan(&plan).is_err());
        assert_eq!(harness.local_metrics.wait_interrupted_count, 1);
        assert_eq!(harness.resources.coordinator.cursor, 0);
        assert!(packets.lock().expect("packet capture").is_empty());
        assert!(matches!(
            harness.runtime.down_boundary_state,
            DownBoundaryState::FutureAuthorized(_)
        ));
    }

    #[test]
    fn normal_completion_floor_does_not_expire_a_down_chord() {
        let mut harness = ProductionDispatchTestHarness::new_down_chord(2);
        let packets = harness.configure_packet_capture();
        let plan = harness.plan_current_dispatch();
        let target = plan.physical_target_qpc().expect("Down target");
        let clock = harness.resources.clock;
        let frame = clock.duration_from_us(16_667).expect("frame ticks");
        let beyond_latest = harness
            .timing
            .timing_margin_ticks
            .checked_add(DurationTicks::from_raw(1))
            .expect("latest-start plus one tick");
        let release_floor = target
            .checked_add_duration(beyond_latest)
            .expect("release floor");
        let completed_up = release_floor
            .as_u64()
            .checked_sub(frame.as_u64())
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
            physical_wait_target_for_plan(&plan, &harness.runtime, harness.timing.strict_timing)
                .unwrap()
                .unwrap(),
            target,
            "guaranteed infeasibility waits only until the authored boundary"
        );
        assert_no_work(harness.dispatch_at_qpc_for_test(
            &plan,
            subtract_duration(target, DurationTicks::from_raw(1)),
        ));
        assert_dispatched(harness.dispatch_at_qpc_for_test(&plan, target));
        assert_eq!(
            packets.lock().expect("packet capture").as_slice(),
            &[PhysicalPacket::new(0, 3)]
        );
        assert_eq!(harness.local_metrics.missed_physical_window_boundaries, 0);
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
}
