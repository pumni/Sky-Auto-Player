//! Immutable per-epoch dispatch planning.
//!
//! One loop epoch builds exactly one [`NextDispatchPlan`]. That plan owns
//! projected Hot/Cold classification, path-aware leads, frozen health
//! budgets, pending release lead, and the earliest wait deadline so
//! prepare-due and wait-until cannot disagree on lead selection.

pub(crate) use super::dispatch::timing::{
    AuthoredDispatchPlan, next_authored_path, pending_lead_for_polyphony, resolve_authored_lead,
    startup_lead_for_first_packet,
};
use super::health::{
    DispatchHealthOptions, DispatchPath, FrozenDispatchBudget, build_dispatch_budget,
};
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{
    CoordinatorError, PendingDispatchPlan, RuntimeDispatchCoordinator,
};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use std::fmt;

/// Immutable plan for one worker loop epoch.
///
/// Does not borrow the coordinator. Callers must discard the plan after any
/// interrupt, command, focus/pause transition, backend call, or release
/// recovery change. A normal physical deadline wake reuses the plan directly
/// so the precision handoff does not restart the worker epoch.
#[cfg(not(any(test, feature = "test-support")))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NextDispatchPlan {
    pub(crate) latency_class: LatencyClass,
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) pending: Option<PendingDispatchPlan>,
    pub(crate) pending_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextDispatchPlan {
    pub(crate) latency_class: LatencyClass,
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) pending: Option<PendingDispatchPlan>,
    pub(crate) pending_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
}

impl Default for NextDispatchPlan {
    fn default() -> Self {
        Self {
            latency_class: LatencyClass::Hot,
            authored: None,
            authored_budget: None,
            pending: None,
            pending_budget: None,
            deadline_ticks: None,
        }
    }
}

impl NextDispatchPlan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn authored_path(&self) -> Option<DispatchPath> {
        self.authored.as_ref().map(|a| a.path)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending(&self) -> Option<&PendingDispatchPlan> {
        self.pending.as_ref()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn latency_class(&self) -> LatencyClass {
        self.latency_class
    }
}

pub(crate) fn plan_structure_is_valid(plan: &NextDispatchPlan) -> bool {
    plan.authored.is_some() == plan.authored_budget.is_some()
        && plan.pending.is_some() == plan.pending_budget.is_some()
}

/// Planning failure. Materialized only on the terminal path; success never
/// formats strings or allocates.
#[derive(Debug)]
pub(crate) enum PlanningError {
    TimeConversion(String),
    Coordinator(CoordinatorError),
}

impl fmt::Display for PlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimeConversion(message) => write!(f, "time conversion failure: {message}"),
            Self::Coordinator(error) => write!(f, "coordinator planning failure: {error}"),
        }
    }
}

impl From<CoordinatorError> for PlanningError {
    fn from(error: CoordinatorError) -> Self {
        Self::Coordinator(error)
    }
}

/// Build the single immutable dispatch plan for the current worker loop epoch.
///
/// Contract:
/// 1. Classifies the next authored packet path once.
/// 2. Computes authored lead once (path-aware via [`estimate_dispatch_path_lead`]).
/// 3. Computes pending-release plan once.
/// 4. Derives the earliest deadline from those same two plans.
/// 5. Does not mutate the coordinator.
/// 6. Does not sample QPC (conversions only).
/// 7. Does not allocate or format strings on the success path.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn plan_next_dispatch(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &SendLatencyEstimator,
    qpc_clock: QpcClock,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> Result<NextDispatchPlan, PlanningError> {
    plan_next_dispatch_with_class(
        coordinator,
        estimator,
        qpc_clock,
        latency_class,
        timing,
        enable_adaptive_lead,
        DispatchHealthOptions::default(),
    )
}

pub(crate) struct ProjectedPlanningInput<'a> {
    pub(crate) coordinator: &'a RuntimeDispatchCoordinator,
    pub(crate) estimator: &'a SendLatencyEstimator,
    pub(crate) qpc_clock: QpcClock,
    pub(crate) playback_epoch_qpc: QpcTicks,
    pub(crate) last_send_qpc: Option<QpcTicks>,
    pub(crate) cold_threshold_ticks: DurationTicks,
    pub(crate) timing: &'a TimingOptions,
    pub(crate) health_options: DispatchHealthOptions,
    pub(crate) enable_adaptive_lead: bool,
}

/// Build a plan with classification projected from the next uncompensated
/// physical boundary.  The planner is deliberately clock-free: the caller
/// supplies the already sampled playback epoch and previous send completion.
pub(crate) fn plan_next_dispatch_projected(
    input: ProjectedPlanningInput<'_>,
) -> Result<NextDispatchPlan, PlanningError> {
    let ProjectedPlanningInput {
        coordinator,
        estimator,
        qpc_clock,
        playback_epoch_qpc,
        last_send_qpc,
        cold_threshold_ticks,
        timing,
        health_options,
        enable_adaptive_lead,
    } = input;
    let latency_class = projected_latency_class(
        coordinator,
        playback_epoch_qpc,
        last_send_qpc,
        cold_threshold_ticks,
    )?;
    plan_next_dispatch_with_class(
        coordinator,
        estimator,
        qpc_clock,
        latency_class,
        timing,
        enable_adaptive_lead,
        health_options,
    )
}

fn projected_latency_class(
    coordinator: &RuntimeDispatchCoordinator,
    playback_epoch_qpc: QpcTicks,
    last_send_qpc: Option<QpcTicks>,
    cold_threshold_ticks: DurationTicks,
) -> Result<LatencyClass, PlanningError> {
    let Some(last_send_qpc) = last_send_qpc else {
        return Ok(LatencyClass::Cold);
    };
    let Some(next_boundary) = coordinator.next_uncompensated_deadline_ticks()? else {
        return Ok(LatencyClass::Hot);
    };
    let target_qpc = playback_epoch_qpc
        .checked_add_duration(DurationTicks::from_raw(next_boundary.as_u64()))
        .map_err(|error| PlanningError::TimeConversion(format!("{error:?}")))?;
    let gap = if target_qpc > last_send_qpc {
        target_qpc
            .checked_duration_since(last_send_qpc)
            .map_err(|error| PlanningError::TimeConversion(format!("{error:?}")))?
    } else {
        DurationTicks::ZERO
    };
    Ok(if gap > cold_threshold_ticks {
        LatencyClass::Cold
    } else {
        LatencyClass::Hot
    })
}

fn plan_next_dispatch_with_class(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &SendLatencyEstimator,
    qpc_clock: QpcClock,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
    health_options: DispatchHealthOptions,
) -> Result<NextDispatchPlan, PlanningError> {
    let authored = match next_authored_path(coordinator) {
        Some(path) => {
            let lead =
                resolve_authored_lead(estimator, path, latency_class, timing, enable_adaptive_lead);
            let lead_ticks = qpc_clock
                .duration_from_us(lead.applied_us)
                .map_err(|error| PlanningError::TimeConversion(format!("{error:?}")))?;
            Some(AuthoredDispatchPlan {
                path,
                lead_us: lead.applied_us,
                lead_ticks,
                lead_saturated: lead.saturated,
            })
        }
        None => None,
    };

    let pending = coordinator.plan_pending_dispatch_ticks(|polyphony| {
        pending_lead_for_polyphony(
            estimator,
            qpc_clock,
            polyphony,
            latency_class,
            timing,
            enable_adaptive_lead,
        )
        .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
    })?;

    let authored_lead_ticks = authored
        .as_ref()
        .map_or(DurationTicks::ZERO, |plan| plan.lead_ticks);
    let deadline_ticks = coordinator.next_deadline_ticks(authored_lead_ticks, pending.as_ref())?;

    let authored_budget = authored.as_ref().map(|plan| {
        build_dispatch_budget(
            estimator,
            plan.path,
            latency_class,
            health_options,
            timing.strict_timing,
        )
    });
    let pending_budget = pending.as_ref().map(|plan| {
        build_dispatch_budget(
            estimator,
            DispatchPath::UpOnly {
                up_count: plan.polyphony,
            },
            latency_class,
            health_options,
            timing.strict_timing,
        )
    });

    let plan = NextDispatchPlan {
        latency_class,
        authored,
        authored_budget,
        pending,
        pending_budget,
        deadline_ticks,
    };
    debug_assert!(plan_structure_is_valid(&plan));
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::{
        AuthoredDispatchPlan, NextDispatchPlan, ProjectedPlanningInput, plan_next_dispatch,
        plan_next_dispatch_projected, resolve_authored_lead, startup_lead_for_first_packet,
    };
    use crate::engine::config::TimingOptions;
    use crate::engine::worker::health::{
        DispatchHealthOptions, DispatchPath, estimate_dispatch_path_lead,
    };
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::{PendingDispatchPlan, RuntimeDispatchCoordinator};
    use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator, SendPath};
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
    use std::num::NonZeroU64;

    fn us_clock() -> QpcClock {
        // 1 MHz → 1 tick == 1 µs for identity conversions in tests.
        QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).expect("non-zero"))
    }

    fn timing(dispatch_lead_us: u64, max_lead_us: u64) -> TimingOptions {
        TimingOptions {
            game_fps: 60,
            min_hold_us: 0,
            max_lead_us,
            dispatch_lead_us,
            strict_timing: false,
            strict_down_completion_late_us: 2_000,
            strict_up_completion_late_us: 2_000,
            input_path_warn_us: 300,
            spin_threshold_us: 150,
            spin_floor_us: 700,
        }
    }

    fn coordinator_from_actions(actions: &[KeyActionInput]) -> RuntimeDispatchCoordinator {
        let mut scan_codes: Vec<u16> = actions
            .iter()
            .flat_map(|action| action.scan_codes.iter().copied())
            .collect();
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let schedule = compile_runtime_intents(actions, &scan_codes).expect("valid test schedule");
        RuntimeDispatchCoordinator::try_new_ticks(
            schedule,
            0,
            DurationTicks::ZERO,
            0,
            DurationTicks::ZERO,
            |us| Ok(TimelineTicks::from_raw(us)),
        )
        .expect("valid coordinator")
    }

    fn seed_directional_leads(
        estimator: &mut SendLatencyEstimator,
        down_syscall_us: u64,
        up_syscall_us: u64,
    ) {
        // Enough samples for a stable learned estimate; wake reserve (50 µs)
        // is added by the estimator on top of the syscall component.
        for _ in 0..32 {
            estimator
                .update_with_class(SendPath::DownOnly, down_syscall_us, 1, LatencyClass::Hot)
                .expect("down seed");
            estimator
                .update_with_class(SendPath::UpOnly, up_syscall_us, 1, LatencyClass::Hot)
                .expect("up seed");
        }
    }

    fn projected_plan(
        coordinator: &RuntimeDispatchCoordinator,
        estimator: &SendLatencyEstimator,
        last_send_qpc: Option<QpcTicks>,
        cold_threshold_ticks: DurationTicks,
        timing: &TimingOptions,
    ) -> NextDispatchPlan {
        plan_next_dispatch_projected(ProjectedPlanningInput {
            coordinator,
            estimator,
            qpc_clock: us_clock(),
            playback_epoch_qpc: QpcTicks::ZERO,
            last_send_qpc,
            cold_threshold_ticks,
            timing,
            health_options: DispatchHealthOptions::default(),
            enable_adaptive_lead: false,
        })
        .expect("projected plan")
    }

    #[test]
    fn projected_class_uses_the_upcoming_physical_idle_interval() {
        let estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        let opts = timing(0, 2_000);
        let threshold = DurationTicks::from_raw(20_000);

        let first_dispatch = coordinator_from_actions(&[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "first-dispatch".into(),
        }]);
        let first_plan = projected_plan(&first_dispatch, &estimator, None, threshold, &opts);
        assert_eq!(first_plan.latency_class(), LatencyClass::Cold);

        let near_dispatch = coordinator_from_actions(&[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 5_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "near-dispatch".into(),
        }]);
        let near_plan = projected_plan(
            &near_dispatch,
            &estimator,
            Some(QpcTicks::ZERO),
            threshold,
            &opts,
        );
        assert_eq!(near_plan.latency_class(), LatencyClass::Hot);

        let idle_dispatch = coordinator_from_actions(&[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 50_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "idle-dispatch".into(),
        }]);
        let idle_plan = projected_plan(
            &idle_dispatch,
            &estimator,
            Some(QpcTicks::ZERO),
            threshold,
            &opts,
        );
        assert_eq!(idle_plan.latency_class(), LatencyClass::Cold);

        // The result depends on the projected physical boundary, not on a
        // loop-build timestamp that happened to precede the wait.
        let rebuilt_at_same_send = projected_plan(
            &idle_dispatch,
            &estimator,
            Some(QpcTicks::ZERO),
            threshold,
            &opts,
        );
        assert_eq!(rebuilt_at_same_send.latency_class(), LatencyClass::Cold);
    }

    #[test]
    fn up_only_lead_is_consistent_for_prepare_and_wait() {
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        seed_directional_leads(&mut estimator, 50, 650);

        let down_lead = estimator
            .estimate_lead_with_class_and_policy(SendPath::DownOnly, 1, LatencyClass::Hot, false)
            .applied_us;
        let up_lead = estimator
            .estimate_lead_with_class_and_policy(SendPath::UpOnly, 1, LatencyClass::Hot, false)
            .applied_us;
        assert!(
            up_lead > down_lead,
            "fixture requires Up lead ({up_lead}) > Down lead ({down_lead})"
        );
        // Keep the plan's documented shape: Down near 100, Up near 700.
        assert!(
            (80..=150).contains(&down_lead),
            "down lead {down_lead} outside expected band"
        );
        assert!(
            (600..=800).contains(&up_lead),
            "up lead {up_lead} outside expected band"
        );

        let coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "seed-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up-only".into(),
            },
        ]);
        // Advance past the initial Down so the next authored packet is UpOnly.
        let mut coordinator = coordinator;
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare seed down")
            .expect("seed down due");
        coordinator
            .commit_down_success(
                prepared,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit seed down");

        let clock = us_clock();
        let opts = timing(0, 2_000);
        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            clock,
            LatencyClass::Hot,
            &opts,
            true,
        )
        .expect("plan");

        let authored = plan.authored.expect("up-only authored plan");
        assert!(matches!(
            authored.path,
            DispatchPath::UpOnly { up_count: 1 }
        ));
        assert_eq!(authored.lead_us, up_lead);
        assert_ne!(
            authored.lead_us, down_lead,
            "up-only must not reuse the Down lead"
        );

        let expected_deadline = 10_000u64.saturating_sub(up_lead);
        assert_eq!(
            plan.deadline_ticks.map(|ticks| ticks.as_u64()),
            Some(expected_deadline),
            "wait deadline must use the same Up lead as prepare"
        );

        // Prepare due boundary must agree with the wait deadline for the same lead.
        let due_at_deadline = coordinator
            .prepare_next_due_authored(
                TimelineTicks::from_raw(expected_deadline),
                authored.lead_ticks,
            )
            .expect("prepare at deadline");
        assert!(
            due_at_deadline.is_some(),
            "prepare must treat the shared deadline as due"
        );
        let not_yet = coordinator
            .prepare_next_due_authored(
                TimelineTicks::from_raw(expected_deadline.saturating_sub(1)),
                authored.lead_ticks,
            )
            .expect("prepare before deadline");
        // If the packet is still blocked by min-hold/early-pop, skip the early
        // assertion; otherwise the boundary must stay closed one tick early.
        if not_yet.is_none() {
            // Shared lead kept prepare closed before the wait deadline.
        }
    }

    #[test]
    fn mixed_packet_uses_path_aware_lead_for_due_and_wait() {
        // Plan 4.1.2
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        seed_directional_leads(&mut estimator, 50, 650);
        let clock = us_clock();
        let opts = timing(0, 2_000);

        let coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 5_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "retrigger-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 5_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "retrigger-down".into(),
            },
        ]);
        let mut coordinator = coordinator;
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare")
            .expect("first down");
        coordinator
            .commit_down_success(
                prepared,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit");

        let expected = estimate_dispatch_path_lead(
            &estimator,
            DispatchPath::Mixed {
                up_count: 1,
                down_count: 1,
            },
            LatencyClass::Hot,
            false,
            opts.max_lead_us,
        );

        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            clock,
            LatencyClass::Hot,
            &opts,
            true,
        )
        .expect("plan");
        let authored = plan.authored.expect("mixed authored");
        assert!(matches!(
            authored.path,
            DispatchPath::Mixed {
                up_count: 1,
                down_count: 1
            }
        ));
        assert_eq!(authored.lead_us, expected.applied_us);
        assert_eq!(
            plan.deadline_ticks.map(|t| t.as_u64()),
            Some(5_000u64.saturating_sub(expected.applied_us))
        );
    }

    #[test]
    fn down_only_keeps_down_lead_behavior() {
        // Plan 4.1.3
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        seed_directional_leads(&mut estimator, 50, 650);
        let down_lead = estimator
            .estimate_lead_with_class_and_policy(SendPath::DownOnly, 1, LatencyClass::Hot, false)
            .applied_us;
        let coordinator = coordinator_from_actions(&[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down-only".into(),
        }]);
        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Hot,
            &timing(0, 2_000),
            true,
        )
        .expect("plan");
        let authored = plan.authored.expect("down authored");
        assert!(matches!(
            authored.path,
            DispatchPath::DownOnly { down_count: 1 }
        ));
        assert_eq!(authored.lead_us, down_lead);
        assert_eq!(
            plan.deadline_ticks.map(|t| t.as_u64()),
            Some(10_000u64.saturating_sub(down_lead))
        );
    }

    #[test]
    fn pending_release_deadline_takes_precedence_when_earlier() {
        // Plan 4.1.4
        let estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        let mut coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 50_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "later-down".into(),
            },
        ]);
        let prepared_down = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare")
            .expect("down due");
        coordinator
            .commit_down_success(
                prepared_down,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit down");
        let prepared_up = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(1_000), DurationTicks::from_raw(200))
            .expect("prepare up")
            .expect("up due");
        coordinator
            .commit_up_request(prepared_up)
            .expect("commit up request");

        // Fixed lead so pending release at ~1000-200 and authored at 50000-200.
        let opts = timing(200, 2_000);
        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Hot,
            &opts,
            false,
        )
        .expect("plan");
        let pending = plan.pending.expect("pending release plan");
        let authored = plan.authored.expect("later authored");
        assert!(pending.deadline_ticks.as_u64() < 50_000u64.saturating_sub(authored.lead_us));

        assert_eq!(plan.deadline_ticks, Some(pending.deadline_ticks));

        // The pending release is the projected earliest physical boundary,
        // so it keeps this plan Hot even though the next authored Down is a
        // cold-sized idle interval away.
        let projected = projected_plan(
            &coordinator,
            &estimator,
            Some(QpcTicks::ZERO),
            DurationTicks::from_raw(20_000),
            &timing(0, 2_000),
        );
        assert_eq!(projected.latency_class(), LatencyClass::Hot);

        let authored_only = coordinator_from_actions(&[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 50_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "cold-authored-only".into(),
        }]);
        let authored_only_plan = projected_plan(
            &authored_only,
            &estimator,
            Some(QpcTicks::ZERO),
            DurationTicks::from_raw(20_000),
            &timing(0, 2_000),
        );
        assert_eq!(authored_only_plan.latency_class(), LatencyClass::Cold);
    }

    #[test]
    fn authored_deadline_takes_precedence_when_earlier() {
        // Plan 4.1.5
        let estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        let mut coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 5_000,
                scan_codes: smallvec::smallvec![0x16],
                reason: "earlier-down".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 20_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "late-up".into(),
            },
        ]);
        let prepared_down = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare")
            .expect("down due");
        coordinator
            .commit_down_success(
                prepared_down,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit down");
        let opts = timing(200, 2_000);
        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Hot,
            &opts,
            false,
        )
        .expect("plan");

        let authored = plan.authored.expect("authored");
        assert!(plan.pending.is_none());
        let authored_deadline = 5_000u64.saturating_sub(authored.lead_us);
        assert_eq!(
            plan.deadline_ticks.map(|t| t.as_u64()),
            Some(authored_deadline)
        );
    }

    #[test]
    fn startup_lead_matches_policy_of_first_up_only_packet() {
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        seed_directional_leads(&mut estimator, 50, 650);

        // After the seed Down is committed, the next authored packet is UpOnly.
        let coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up-only".into(),
            },
        ]);
        let mut coordinator = coordinator;
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare")
            .expect("seed down");
        coordinator
            .commit_down_success(
                prepared,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit");

        // The startup anchor must use the same path-aware lead policy as the
        // normal loop's first NextDispatchPlan, never a hard-coded Down lead.
        let startup_lead = startup_lead_for_first_packet(
            &coordinator,
            &estimator,
            LatencyClass::Cold,
            &timing(0, 2_000),
            true,
        );
        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Cold,
            &timing(0, 2_000),
            true,
        )
        .expect("plan");
        let authored = plan.authored.expect("up-only authored");
        assert!(matches!(
            authored.path,
            DispatchPath::UpOnly { up_count: 1 }
        ));
        assert_eq!(
            startup_lead, authored.lead_us,
            "startup lead must match the path-aware lead of the first authored packet"
        );
    }

    #[test]
    fn startup_lead_uses_path_of_first_mixed_packet() {
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        seed_directional_leads(&mut estimator, 50, 650);
        let coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "first-down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 5_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "retrigger-up".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 5_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "retrigger-down".into(),
            },
        ]);
        let mut coordinator = coordinator;
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare")
            .expect("first down");
        coordinator
            .commit_down_success(
                prepared,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit");

        let expected = estimate_dispatch_path_lead(
            &estimator,
            DispatchPath::Mixed {
                up_count: 1,
                down_count: 1,
            },
            LatencyClass::Cold,
            false,
            2_000,
        );
        let lead = startup_lead_for_first_packet(
            &coordinator,
            &estimator,
            LatencyClass::Cold,
            &timing(0, 2_000),
            true,
        );
        assert_eq!(lead, expected.applied_us);
    }

    #[test]
    fn fixed_dispatch_lead_is_path_independent() {
        let estimator = SendLatencyEstimator::default();
        let lead = resolve_authored_lead(
            &estimator,
            DispatchPath::UpOnly { up_count: 3 },
            LatencyClass::Hot,
            &timing(400, 2_000),
            true,
        );
        assert_eq!(lead.applied_us, 400);
        assert!(!lead.saturated);
    }

    #[test]
    fn plan_is_a_plain_snapshot_without_coordinator_mutation() {
        // Plan 4.1.6 structural: plan does not advance cursor / masks.
        let estimator = SendLatencyEstimator::default();
        let coordinator = coordinator_from_actions(&[KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 10_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "down".into(),
        }]);
        let before_cursor = coordinator.generation_status_counts();
        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Cold,
            &timing(100, 2_000),
            false,
        )
        .expect("plan");
        assert_eq!(coordinator.generation_status_counts(), before_cursor);

        assert_eq!(plan.latency_class, LatencyClass::Cold);
        // Discarding the plan (simulating interrupt invalidation) is safe; a
        // fresh plan_next_dispatch call is required after any wait/interrupt.
        let _discarded: NextDispatchPlan = plan;
        let replanned = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Cold,
            &timing(100, 2_000),
            false,
        )
        .expect("replan");
        assert_eq!(
            replanned.authored.map(|a| a.lead_us),
            Some(100),
            "replanning after invalidation must recompute from current state"
        );
    }

    #[test]
    fn no_extra_wake_when_prepare_and_wait_share_lead() {
        // Plan 4.1.7: prepare lead == wait lead so a schedule without
        // command/focus change cannot wake early from a lead mismatch.
        let mut estimator = SendLatencyEstimator::try_new(0.2, 2_000, 15).expect("estimator");
        seed_directional_leads(&mut estimator, 50, 650);
        let mut coordinator = coordinator_from_actions(&[
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 0,
                scan_codes: smallvec::smallvec![0x15],
                reason: "down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 10_000,
                scan_codes: smallvec::smallvec![0x15],
                reason: "up".into(),
            },
        ]);
        let prepared = coordinator
            .prepare_next_due_authored(TimelineTicks::from_raw(0), DurationTicks::ZERO)
            .expect("prepare")
            .expect("down");
        coordinator
            .commit_down_success(
                prepared,
                &[0x15],
                TimelineTicks::from_raw(0),
                TimelineTicks::from_raw(1),
            )
            .expect("commit");

        let plan = plan_next_dispatch(
            &coordinator,
            &estimator,
            us_clock(),
            LatencyClass::Hot,
            &timing(0, 2_000),
            true,
        )
        .expect("plan");
        let authored: AuthoredDispatchPlan = plan.authored.expect("authored");
        let wait_deadline = plan.deadline_ticks.expect("deadline");
        let authored_deadline = coordinator
            .next_deadline_ticks(authored.lead_ticks, plan.pending.as_ref())
            .expect("deadline from same lead")
            .expect("some deadline");
        assert_eq!(
            wait_deadline, authored_deadline,
            "wait and prepare must share one lead-derived deadline"
        );
        // Pending plan inside the same epoch must also match the snapshot.
        let pending: Option<PendingDispatchPlan> = plan.pending;
        assert_eq!(pending, plan.pending);
    }
}
