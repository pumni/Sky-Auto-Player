//! Immutable per-epoch authored physical-deadline planning.
//!
//! Planning never consults diagnostic state. Authored deadlines are the
//! coordinator's effective QPC-timeline deadlines; wake guards are applied
//! only by the wait strategy.

use super::admission::TargetStamp;
use super::dispatch::{AuthoredBatchView, DispatchStep, timing::prepare_authored_batch_view};
use super::health::{
    DispatchHealthOptions, DispatchPath, FrozenDispatchBudget, build_dispatch_budget,
};
#[cfg(any(test, feature = "test-support"))]
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcTicks;
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

#[derive(Debug, Default)]
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) struct NextDispatchPlan {
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
    pub(crate) physical_target_qpc: Option<QpcTicks>,
    pub(crate) authored_view: Option<AuthoredBatchView>,
    pub(crate) preflight_target: Option<TargetStamp>,
}

#[derive(Debug, Default)]
#[cfg(any(test, feature = "test-support"))]
pub struct NextDispatchPlan {
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
    pub(crate) physical_target_qpc: Option<QpcTicks>,
    pub(crate) authored_view: Option<AuthoredBatchView>,
    pub(crate) preflight_target: Option<TargetStamp>,
}

impl NextDispatchPlan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn authored_path(&self) -> Option<DispatchPath> {
        self.authored.as_ref().map(|plan| plan.path)
    }
}

pub(crate) fn plan_structure_is_valid(plan: &NextDispatchPlan) -> bool {
    plan.authored.is_some() == plan.authored_budget.is_some()
        && plan.authored.is_some() == plan.authored_view.is_some()
        && plan.authored.is_some() == plan.physical_target_qpc.is_some()
}

#[derive(Debug)]
pub(crate) enum PlanningError {
    Coordinator(CoordinatorError),
    Prepared(String),
}

impl fmt::Display for PlanningError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Coordinator(error) => write!(f, "coordinator planning failure: {error}"),
            Self::Prepared(error) => write!(f, "prepared dispatch planning failure: {error}"),
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
    epoch_qpc: QpcTicks,
    _qpc_clock: sky_dispatch_win32::clock::QpcClock,
    _timing: &TimingOptions,
) -> Result<NextDispatchPlan, PlanningError> {
    plan_next_dispatch_projected(PlanningInput {
        coordinator,
        epoch_qpc,
        health_options: DispatchHealthOptions::default(),
    })
}

pub(crate) struct PlanningInput<'a> {
    pub(crate) coordinator: &'a RuntimeDispatchCoordinator,
    pub(crate) epoch_qpc: QpcTicks,
    pub(crate) health_options: DispatchHealthOptions,
}

pub(crate) fn plan_next_dispatch_projected(
    input: PlanningInput<'_>,
) -> Result<NextDispatchPlan, PlanningError> {
    let PlanningInput {
        coordinator,
        epoch_qpc,
        health_options,
    } = input;
    let authored_view = match coordinator.prepare_current_authored_packet()? {
        Some(prepared) => match prepare_authored_batch_view(coordinator, prepared) {
            Ok(Some(view)) => Some(view),
            Ok(None) => None,
            Err(DispatchStep::Terminate(error)) => return Err(PlanningError::Prepared(error)),
            Err(step) => {
                return Err(PlanningError::Prepared(format!(
                    "unexpected prepared view outcome: {step:?}"
                )));
            }
        },
        None => None,
    };
    let authored = authored_view.as_ref().map(|view| AuthoredDispatchPlan {
        path: view.dispatch_path,
        deadline_ticks: view.prepared_batch.effective_scheduled_ticks,
    });
    let authored_budget = authored
        .as_ref()
        .map(|plan| build_dispatch_budget(plan.path, health_options));
    let deadline_ticks = authored.as_ref().map(|plan| plan.deadline_ticks);
    let physical_target_qpc = deadline_ticks
        .map(|deadline| {
            epoch_qpc
                .checked_add_duration(DurationTicks::from_raw(deadline.as_u64()))
                .map_err(|error| {
                    PlanningError::Prepared(format!("physical target arithmetic failure: {error}"))
                })
        })
        .transpose()?;
    let plan = NextDispatchPlan {
        authored,
        authored_budget,
        deadline_ticks,
        physical_target_qpc,
        authored_view,
        preflight_target: None,
    };
    debug_assert!(plan_structure_is_valid(&plan));
    Ok(plan)
}
