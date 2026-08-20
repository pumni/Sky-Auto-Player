//! Immutable per-epoch dispatch planning.
//!
//! A plan is a typed state: an empty plan, a metadata boundary, or one
//! physical operation with its target and prepared authored view.  Diagnostic
//! health policy is deliberately not part of physical-plan validity.

use super::DispatchPreparationProbe;
use super::admission::TargetStamp;
use super::dispatch::{
    AuthoredBatchView, BatchViewResult, DispatchStep, PhysicalCommit,
    timing::{prepare_authored_frame_view_from_prepared, prepare_pending_release_view},
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
    pub(crate) commit: sky_dispatch_core::coordinator::PreparedAuthoredCommit,
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
    #[allow(dead_code)]
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
    let authored_packet = coordinator.prepare_current_authored_packet()?;
    if let Some(prepared) = authored_packet.as_ref() {
        preparation_probe.record_logical_prepare(
            prepared.packet.up_intents.len(),
            prepared.packet.down_intents.len(),
        );
    }
    let pending_target = coordinator.earliest_pending_release_ticks();
    let authored_target = authored_packet
        .as_ref()
        .map(|prepared| prepared.frame.authored_ticks);

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

    let Some(authored_packet) = authored_packet else {
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

    let frame = authored_packet.frame;
    let coalesced_pending_mask = match pending_target {
        Some(target) if target == frame.authored_ticks => {
            coordinator.pending_release_mask_due_at(target)
        }
        _ => 0,
    };
    let authored_is_physical = frame.immediate_up_mask != 0 || frame.down_mask != 0;
    if !authored_is_physical && coalesced_pending_mask == 0 {
        let commit = authored_packet.commit;
        let physical_target_qpc = epoch_qpc
            .checked_add_duration(DurationTicks::from_raw(frame.authored_ticks.as_u64()))
            .map_err(|error| {
                PlanningError::Prepared(format!("metadata target arithmetic failure: {error}"))
            })?;
        return Ok(NextDispatchPlan::Metadata(MetadataBoundaryPlan {
            commit,
            deadline_ticks: frame.authored_ticks,
            physical_target_qpc,
        }));
    }

    let authored_view = planning_view(prepare_authored_frame_view_from_prepared(
        coordinator,
        authored_packet,
        coalesced_pending_mask,
        frame.authored_ticks,
        preparation_probe,
    ))?;
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

#[cfg(test)]
mod layout_tests {
    use super::{AuthoredBatchView, NextDispatchPlan, PhysicalDispatchPlan};
    use sky_dispatch_core::clock::PlaybackClockState;
    use sky_dispatch_core::coordinator::{ActiveGeneration, PreparedAuthoredCommit};
    use sky_dispatch_win32::input::PreparedPhysicalPacket;
    use std::collections::HashSet;
    use std::mem::size_of;

    #[allow(dead_code)]
    struct LegacyPlaybackClockLayout {
        start_perf: sky_dispatch_core::time::QpcTicks,
        pause_time: sky_dispatch_core::time::DurationTicks,
        pause_reasons: HashSet<String>,
        pause_interval_started: Option<sky_dispatch_core::time::QpcTicks>,
        pause_open_reason: Option<String>,
        epoch: sky_dispatch_core::time::QpcTicks,
    }

    #[test]
    fn report_hot_dispatch_layout() {
        println!(
            "layout target_os_windows={} target_arch_x86_64={} target_pointer_width_64={}",
            cfg!(target_os = "windows"),
            cfg!(target_arch = "x86_64"),
            cfg!(target_pointer_width = "64"),
        );
        println!(
            "size_of::<PreparedPhysicalPacket>()={}",
            size_of::<PreparedPhysicalPacket>()
        );
        println!(
            "size_of::<PreparedAuthoredCommit>()={}",
            size_of::<PreparedAuthoredCommit>()
        );
        println!(
            "size_of::<AuthoredBatchView>()={}",
            size_of::<AuthoredBatchView>()
        );
        println!(
            "size_of::<PhysicalDispatchPlan>()={}",
            size_of::<PhysicalDispatchPlan>()
        );
        println!(
            "size_of::<NextDispatchPlan>()={}",
            size_of::<NextDispatchPlan>()
        );
        println!(
            "size_of::<ActiveGeneration>()={}",
            size_of::<ActiveGeneration>()
        );
        println!(
            "size_of::<PlaybackClockState>()={}",
            size_of::<PlaybackClockState>()
        );
        println!(
            "size_of::<LegacyPlaybackClockLayout>()={}",
            size_of::<LegacyPlaybackClockLayout>()
        );
    }
}
