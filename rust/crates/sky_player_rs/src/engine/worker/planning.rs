//! Immutable per-epoch physical-deadline planning.
//!
//! Planning never consults diagnostic state.  Authored and pending deadlines
//! are the coordinator's effective QPC-timeline deadlines; wake guards are
//! applied only by the wait strategy.

use super::health::{
    DispatchHealthOptions, DispatchPath, FrozenDispatchBudget, build_dispatch_budget,
};
#[cfg(any(test, feature = "test-support"))]
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{
    CoordinatorError, PendingDispatchPlan, RuntimeDispatchCoordinator,
};
use sky_dispatch_core::time::TimelineTicks;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AuthoredDispatchPlan {
    pub(crate) path: DispatchPath,
    pub(crate) deadline_ticks: TimelineTicks,
}

impl Default for AuthoredDispatchPlan {
    fn default() -> Self {
        Self {
            path: DispatchPath::DownOnly { down_count: 0 },
            deadline_ticks: TimelineTicks::ZERO,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) struct NextDispatchPlan {
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) pending: Option<PendingDispatchPlan>,
    pub(crate) pending_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg(any(test, feature = "test-support"))]
pub struct NextDispatchPlan {
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) pending: Option<PendingDispatchPlan>,
    pub(crate) pending_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
}

impl NextDispatchPlan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn authored_path(&self) -> Option<DispatchPath> {
        self.authored.as_ref().map(|plan| plan.path)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn has_pending(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn pending(&self) -> Option<&PendingDispatchPlan> {
        self.pending.as_ref()
    }
}

pub(crate) fn plan_structure_is_valid(plan: &NextDispatchPlan) -> bool {
    plan.authored.is_some() == plan.authored_budget.is_some()
        && plan.pending.is_some() == plan.pending_budget.is_some()
}

#[derive(Debug)]
pub(crate) enum PlanningError {
    Coordinator(CoordinatorError),
}

impl fmt::Display for PlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coordinator(error) => write!(f, "coordinator planning failure: {error}"),
        }
    }
}

impl From<CoordinatorError> for PlanningError {
    fn from(error: CoordinatorError) -> Self {
        Self::Coordinator(error)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn plan_next_dispatch(
    coordinator: &RuntimeDispatchCoordinator,
    _qpc_clock: sky_dispatch_win32::clock::QpcClock,
    _timing: &TimingOptions,
) -> Result<NextDispatchPlan, PlanningError> {
    plan_next_dispatch_projected(PlanningInput {
        coordinator,
        health_options: DispatchHealthOptions::default(),
    })
}

pub(crate) struct PlanningInput<'a> {
    pub(crate) coordinator: &'a RuntimeDispatchCoordinator,
    pub(crate) health_options: DispatchHealthOptions,
}

pub(crate) fn plan_next_dispatch_projected(
    input: PlanningInput<'_>,
) -> Result<NextDispatchPlan, PlanningError> {
    let PlanningInput {
        coordinator,
        health_options,
    } = input;

    let authored = match super::dispatch::timing::current_authored_physical_path(coordinator)? {
        Some(path) => Some(AuthoredDispatchPlan {
            path,
            deadline_ticks: coordinator.next_authored_ticks()?.ok_or_else(|| {
                PlanningError::Coordinator(CoordinatorError::TimeConversion(
                    "authored path exists but no authored deadline exists".to_string(),
                ))
            })?,
        }),
        None => None,
    };
    let pending = coordinator.plan_pending_dispatch_ticks()?;
    let authored_budget = authored
        .as_ref()
        .map(|plan| build_dispatch_budget(plan.path, health_options));
    let pending_budget = pending.as_ref().map(|plan| {
        build_dispatch_budget(
            DispatchPath::UpOnly {
                up_count: plan.polyphony,
            },
            health_options,
        )
    });
    let authored_deadline = authored.as_ref().map(|plan| plan.deadline_ticks);
    let pending_deadline = pending.as_ref().map(|plan| plan.deadline_ticks);
    let deadline_ticks = match (authored_deadline, pending_deadline) {
        (Some(authored), Some(pending)) => Some(authored.min(pending)),
        (Some(deadline), None) | (None, Some(deadline)) => Some(deadline),
        (None, None) => None,
    };
    let plan = NextDispatchPlan {
        authored,
        authored_budget,
        pending,
        pending_budget,
        deadline_ticks,
    };
    debug_assert!(plan_structure_is_valid(&plan));
    Ok(plan)
}
