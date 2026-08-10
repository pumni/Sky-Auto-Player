//! Immutable per-epoch dispatch planning.
//!
//! One loop epoch builds exactly one [`NextDispatchPlan`]. The plan owns the
//! path-aware dispatch-cost leads, frozen health budgets, pending-release lead,
//! and the earliest physical wait deadline.

pub(crate) use super::dispatch::timing::{
    AuthoredDispatchPlan, next_authored_path, pending_lead_for_polyphony, resolve_authored_lead,
    startup_lead_for_first_packet,
};
use super::health::{
    DispatchHealthOptions, DispatchPath, FrozenDispatchBudget, build_dispatch_budget,
};
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{
    CoordinatorError, CoordinatorInvariantError, PendingDispatchPlan, RuntimeDispatchCoordinator,
};
use sky_dispatch_core::estimator::DispatchCostEstimator;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use std::fmt;

/// Immutable plan for one worker loop epoch.
///
/// Does not borrow the coordinator. Callers discard the plan after any
/// interrupt, command, focus/pause transition, backend call, or release
/// recovery change. A normal physical deadline wake reuses the plan directly
/// so the precision handoff does not restart the worker epoch.
#[cfg(not(any(test, feature = "test-support")))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct NextDispatchPlan {
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) authored_budget: Option<FrozenDispatchBudget>,
    pub(crate) pending: Option<PendingDispatchPlan>,
    pub(crate) pending_budget: Option<FrozenDispatchBudget>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn plan_next_dispatch(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &DispatchCostEstimator,
    qpc_clock: QpcClock,
    timing: &TimingOptions,
    enable_dispatch_cost_lead: bool,
) -> Result<NextDispatchPlan, PlanningError> {
    plan_next_dispatch_inner(
        coordinator,
        estimator,
        qpc_clock,
        timing,
        enable_dispatch_cost_lead,
        DispatchHealthOptions::default(),
    )
}

pub(crate) struct PlanningInput<'a> {
    pub(crate) coordinator: &'a RuntimeDispatchCoordinator,
    pub(crate) estimator: &'a DispatchCostEstimator,
    pub(crate) qpc_clock: QpcClock,
    pub(crate) timing: &'a TimingOptions,
    pub(crate) health_options: DispatchHealthOptions,
    pub(crate) enable_dispatch_cost_lead: bool,
}

pub(crate) fn plan_next_dispatch_projected(
    input: PlanningInput<'_>,
) -> Result<NextDispatchPlan, PlanningError> {
    let PlanningInput {
        coordinator,
        estimator,
        qpc_clock,
        timing,
        health_options,
        enable_dispatch_cost_lead,
    } = input;
    plan_next_dispatch_inner(
        coordinator,
        estimator,
        qpc_clock,
        timing,
        enable_dispatch_cost_lead,
        health_options,
    )
}

fn plan_next_dispatch_inner(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &DispatchCostEstimator,
    qpc_clock: QpcClock,
    timing: &TimingOptions,
    enable_dispatch_cost_lead: bool,
    health_options: DispatchHealthOptions,
) -> Result<NextDispatchPlan, PlanningError> {
    let authored = match next_authored_path(coordinator) {
        Some(path) => {
            let lead = resolve_authored_lead(estimator, path, timing, enable_dispatch_cost_lead);
            let lead_ticks = qpc_clock
                .duration_from_us(lead.applied_us)
                .map_err(|error| PlanningError::TimeConversion(format!("{error:?}")))?;
            let deadline_ticks = coordinator
                .next_authored_ticks(lead_ticks)?
                .ok_or_else(|| {
                    PlanningError::Coordinator(CoordinatorError::Invariant(
                        CoordinatorInvariantError::Accounting(
                            "authored path exists but no authored deadline exists".to_string(),
                        ),
                    ))
                })?;
            Some(AuthoredDispatchPlan {
                path,
                lead_us: lead.applied_us,
                lead_ticks,
                lead_saturated: lead.saturated,
                deadline_ticks,
            })
        }
        None => None,
    };

    let pending = coordinator.plan_pending_dispatch_ticks(|polyphony| {
        pending_lead_for_polyphony(
            estimator,
            qpc_clock,
            polyphony,
            timing,
            enable_dispatch_cost_lead,
        )
        .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
    })?;

    let authored_lead_ticks = authored
        .as_ref()
        .map_or(DurationTicks::ZERO, |plan| plan.lead_ticks);
    let deadline_ticks = coordinator.next_deadline_ticks(authored_lead_ticks, pending.as_ref())?;

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

#[cfg(test)]
mod tests {
    use super::{NextDispatchPlan, plan_structure_is_valid, resolve_authored_lead};
    use crate::engine::config::TimingOptions;
    use crate::engine::worker::health::DispatchPath;
    use sky_dispatch_core::estimator::{DispatchCostEstimator, SendPath};
    use sky_dispatch_win32::clock::QpcClock;
    use std::num::NonZeroU64;

    fn timing(dispatch_lead_us: u64, max_lead_us: u64) -> TimingOptions {
        TimingOptions {
            game_fps: 60,
            min_hold_us: 10_000,
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

    #[test]
    fn default_plan_has_no_inconsistent_budget() {
        assert!(plan_structure_is_valid(&NextDispatchPlan::default()));
    }

    #[test]
    fn authored_lead_prefers_explicit_value_and_estimator_path() {
        let estimator = DispatchCostEstimator::try_new(2_000, 30).expect("estimator");
        let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
        let explicit = resolve_authored_lead(
            &estimator,
            DispatchPath::DownOnly { down_count: 2 },
            &timing(125, 2_000),
            true,
        );
        assert_eq!(explicit.applied_us, 125);

        let mut trained = DispatchCostEstimator::try_new(2_000, 30).expect("estimator");
        for _ in 0..5 {
            trained.update(SendPath::DownOnly, 2, 700).expect("sample");
        }
        let adaptive = resolve_authored_lead(
            &trained,
            DispatchPath::DownOnly { down_count: 2 },
            &timing(0, 2_000),
            true,
        );
        assert_eq!(adaptive.applied_us, 700);
        assert_eq!(
            clock
                .duration_from_us(adaptive.applied_us)
                .unwrap()
                .as_u64(),
            700
        );
    }
}
