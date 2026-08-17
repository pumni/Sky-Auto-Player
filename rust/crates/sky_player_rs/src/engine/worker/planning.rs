//! Immutable per-epoch dispatch planning.
//!
//! A plan is a typed state: an empty plan, a metadata boundary, or one
//! physical operation with its target and prepared authored view.  Diagnostic
//! health policy is deliberately not part of physical-plan validity.

use super::DispatchPreparationProbe;
use super::admission::TargetStamp;
use super::dispatch::{
    AuthoredBatchView, BatchViewResult, DispatchStep, PhysicalCommit,
    timing::{
        prepare_authored_frame_view, prepare_authored_frame_view_with_pending,
        prepare_pending_release_view,
    },
};
use super::health::DispatchPath;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TargetProof {
    NotRequired,
    Required,
    Verified(TargetStamp),
}

impl TargetProof {
    pub(crate) fn verified_target(self) -> Option<TargetStamp> {
        match self {
            Self::Verified(stamp) => Some(stamp),
            Self::NotRequired | Self::Required => None,
        }
    }

    pub(crate) fn requires_down_proof(self) -> bool {
        matches!(self, Self::Required | Self::Verified(_))
    }
}

#[derive(Debug)]
pub struct PhysicalDispatchPlan {
    pub(crate) authored: AuthoredDispatchPlan,
    pub(crate) physical_target_qpc: QpcTicks,
    pub(crate) authored_view: AuthoredBatchView,
    pub(crate) target_proof: TargetProof,
}

#[derive(Debug)]
pub struct MetadataBoundaryPlan {
    pub(crate) frame: sky_dispatch_core::coordinator::PreparedAuthoredFrame,
    pub(crate) deadline_ticks: TimelineTicks,
    pub(crate) physical_target_qpc: QpcTicks,
}

#[derive(Debug, Default)]
#[allow(clippy::large_enum_variant)]
#[cfg(not(any(test, feature = "test-support")))]
pub(crate) enum NextDispatchPlan {
    #[default]
    NoWork,
    Metadata(MetadataBoundaryPlan),
    Physical(PhysicalDispatchPlan),
}

#[derive(Debug, Default)]
#[allow(clippy::large_enum_variant)]
#[cfg(any(test, feature = "test-support"))]
pub enum NextDispatchPlan {
    #[default]
    NoWork,
    Metadata(MetadataBoundaryPlan),
    Physical(PhysicalDispatchPlan),
}

impl NextDispatchPlan {
    #[cfg(any(test, feature = "test-support"))]
    pub fn authored_path(&self) -> Option<DispatchPath> {
        self.physical().map(|plan| plan.authored.path)
    }

    pub(crate) fn physical(&self) -> Option<&PhysicalDispatchPlan> {
        match self {
            Self::Physical(plan) => Some(plan),
            Self::NoWork | Self::Metadata(_) => None,
        }
    }

    pub(crate) fn physical_mut(&mut self) -> Option<&mut PhysicalDispatchPlan> {
        match self {
            Self::Physical(plan) => Some(plan),
            Self::NoWork | Self::Metadata(_) => None,
        }
    }

    pub(crate) fn deadline_ticks(&self) -> Option<TimelineTicks> {
        match self {
            Self::NoWork => None,
            Self::Metadata(plan) => Some(plan.deadline_ticks),
            Self::Physical(plan) => Some(plan.authored.deadline_ticks),
        }
    }

    pub(crate) fn physical_target_qpc(&self) -> Option<QpcTicks> {
        match self {
            Self::Physical(plan) => Some(plan.physical_target_qpc),
            Self::Metadata(plan) => Some(plan.physical_target_qpc),
            Self::NoWork => None,
        }
    }
}

pub(crate) fn plan_structure_is_valid(plan: &NextDispatchPlan) -> bool {
    match plan {
        NextDispatchPlan::NoWork | NextDispatchPlan::Metadata(_) => true,
        NextDispatchPlan::Physical(plan) => {
            let has_down = plan.authored_view.packet_masks.down_mask != 0;
            let proof_valid = if has_down {
                plan.target_proof.requires_down_proof()
            } else {
                plan.target_proof == TargetProof::NotRequired
            };
            proof_valid
                && plan.authored_view.prepared_packet.packet() == plan.authored_view.packet_masks
                && match &plan.authored_view.commit {
                    PhysicalCommit::Authored(commit) => {
                        commit.frame.immediate_up_mask == plan.authored_view.packet_masks.up_mask
                            && commit.frame.down_mask == plan.authored_view.packet_masks.down_mask
                    }
                    PhysicalCommit::PendingRelease { release_mask, .. } => {
                        plan.authored_view.packet_masks
                            == sky_dispatch_win32::input::PhysicalPacket::new(*release_mask, 0)
                    }
                    PhysicalCommit::Coalesced {
                        authored,
                        release_mask,
                        ..
                    } => {
                        authored.frame.immediate_up_mask | *release_mask
                            == plan.authored_view.packet_masks.up_mask
                            && authored.frame.down_mask == plan.authored_view.packet_masks.down_mask
                    }
                }
        }
    }
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
    preparation_probe: &DispatchPreparationProbe,
) -> Result<NextDispatchPlan, PlanningError> {
    plan_next_dispatch_projected(PlanningInput {
        coordinator,
        epoch_qpc,
        preparation_probe,
    })
}

pub(crate) struct PlanningInput<'a> {
    pub(crate) coordinator: &'a RuntimeDispatchCoordinator,
    pub(crate) epoch_qpc: QpcTicks,
    pub(crate) preparation_probe: &'a DispatchPreparationProbe,
}

pub(crate) fn plan_next_dispatch_projected(
    input: PlanningInput<'_>,
) -> Result<NextDispatchPlan, PlanningError> {
    let PlanningInput {
        coordinator,
        epoch_qpc,
        preparation_probe,
    } = input;
    let authored_frame = coordinator.prepare_current_authored_frame()?;
    let pending_target = coordinator.earliest_pending_release_ticks();
    let authored_target = authored_frame.map(|frame| frame.authored_ticks);

    let select_pending = match (pending_target, authored_target) {
        (Some(pending), Some(authored)) => pending < authored,
        (Some(_), None) => true,
        (None, _) => false,
    };
    if select_pending {
        let target = pending_target.expect("pending target selected");
        let release_mask = coordinator.pending_release_mask_due_at(target);
        let view = planning_view(prepare_pending_release_view(
            coordinator,
            release_mask,
            target,
            preparation_probe,
        ))?;
        return physical_plan_from_view(view, epoch_qpc);
    }

    let Some(frame) = authored_frame else {
        if let Some(target) = pending_target {
            let release_mask = coordinator.pending_release_mask_due_at(target);
            let view = planning_view(prepare_pending_release_view(
                coordinator,
                release_mask,
                target,
                preparation_probe,
            ))?;
            return physical_plan_from_view(view, epoch_qpc);
        }
        return Ok(NextDispatchPlan::NoWork);
    };

    let coalesced_pending_mask = match pending_target {
        Some(target) if target == frame.authored_ticks => {
            coordinator.pending_release_mask_due_at(target)
        }
        _ => 0,
    };
    let authored_is_physical = frame.immediate_up_mask != 0 || frame.down_mask != 0;
    if !authored_is_physical && coalesced_pending_mask == 0 {
        let physical_target_qpc = epoch_qpc
            .checked_add_duration(DurationTicks::from_raw(frame.authored_ticks.as_u64()))
            .map_err(|error| {
                PlanningError::Prepared(format!("metadata target arithmetic failure: {error}"))
            })?;
        return Ok(NextDispatchPlan::Metadata(MetadataBoundaryPlan {
            frame,
            deadline_ticks: frame.authored_ticks,
            physical_target_qpc,
        }));
    }

    let authored_view = if coalesced_pending_mask == 0 {
        planning_view(prepare_authored_frame_view(
            coordinator,
            frame,
            preparation_probe,
        ))?
    } else {
        planning_view(prepare_authored_frame_view_with_pending(
            coordinator,
            frame,
            coalesced_pending_mask,
            frame.authored_ticks,
            preparation_probe,
        ))?
    };
    physical_plan_from_view(authored_view, epoch_qpc)
}

fn planning_view(result: BatchViewResult) -> Result<AuthoredBatchView, PlanningError> {
    match result {
        Ok(Some(view)) => Ok(view),
        Ok(None) => Err(PlanningError::Prepared("physical view was empty".into())),
        Err(DispatchStep::Terminate(error)) => Err(PlanningError::Prepared(error)),
        Err(DispatchStep::TerminateStatic(error)) => {
            Err(PlanningError::Prepared(error.to_string()))
        }
        Err(step) => Err(PlanningError::Prepared(format!(
            "unexpected prepared view outcome: {step:?}"
        ))),
    }
}

fn physical_plan_from_view(
    authored_view: AuthoredBatchView,
    epoch_qpc: QpcTicks,
) -> Result<NextDispatchPlan, PlanningError> {
    let deadline_ticks = authored_view.prepared_batch.effective_scheduled_ticks;
    let physical_target_qpc = epoch_qpc
        .checked_add_duration(DurationTicks::from_raw(deadline_ticks.as_u64()))
        .map_err(|error| {
            PlanningError::Prepared(format!("physical target arithmetic failure: {error}"))
        })?;
    let authored = AuthoredDispatchPlan {
        path: authored_view.dispatch_path,
        deadline_ticks,
    };
    let target_proof = if authored.path.down_count() != 0 {
        TargetProof::Required
    } else {
        TargetProof::NotRequired
    };
    let plan = NextDispatchPlan::Physical(PhysicalDispatchPlan {
        authored,
        physical_target_qpc,
        authored_view,
        target_proof,
    });
    debug_assert!(plan_structure_is_valid(&plan));
    Ok(plan)
}
