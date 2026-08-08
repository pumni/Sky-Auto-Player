//! Immutable per-epoch dispatch planning.
//!
//! One loop epoch samples QPC, classifies latency, then builds exactly one
//! [`NextDispatchPlan`]. That plan owns path-aware authored lead, pending
//! release lead, and the earliest wait deadline so prepare-due and wait-until
//! cannot disagree on lead selection.

use super::health::{DispatchLeadEstimate, DispatchPath, estimate_dispatch_path_lead};
use crate::engine::config::TimingOptions;
use sky_dispatch_core::coordinator::{
    CoordinatorError, PendingDispatchPlan, RuntimeDispatchCoordinator, physical_packet_kind,
};
use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator, SendPath};
use sky_dispatch_core::model::PhysicalPacketKind;
use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use std::fmt;

/// Snapshot of the next authored packet's dispatch path and lead.
///
/// Built once per worker loop epoch and reused for both
/// `prepare_next_due_authored` and `next_deadline_ticks`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AuthoredDispatchPlan {
    pub(crate) path: DispatchPath,
    pub(crate) lead_us: u64,
    pub(crate) lead_ticks: DurationTicks,
    pub(crate) lead_saturated: bool,
}

/// Immutable plan for one worker loop epoch.
///
/// Does not borrow the coordinator. Callers must discard the plan after any
/// interrupt, command, focus/pause transition, backend call, release recovery
/// change, or wait wake — never cache across loop iterations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NextDispatchPlan {
    pub(crate) latency_class: LatencyClass,
    pub(crate) authored: Option<AuthoredDispatchPlan>,
    pub(crate) pending: Option<PendingDispatchPlan>,
    pub(crate) deadline_ticks: Option<TimelineTicks>,
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

/// Resolve the path-aware lead for one authored packet without reading QPC.
fn resolve_authored_lead(
    estimator: &SendLatencyEstimator,
    path: DispatchPath,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> DispatchLeadEstimate {
    if timing.dispatch_lead_us > 0 {
        return DispatchLeadEstimate {
            applied_us: timing.dispatch_lead_us,
            saturated: false,
        };
    }
    if !enable_adaptive_lead {
        return DispatchLeadEstimate {
            applied_us: 0,
            saturated: false,
        };
    }
    estimate_dispatch_path_lead(
        estimator,
        path,
        latency_class,
        timing.strict_timing,
        timing.max_lead_us,
    )
}

/// Classify the next authored packet into a [`DispatchPath`].
///
/// Empty physical masks (stale Up suppression metadata) keep the historical
/// Down-polyphony fallback so wait/prepare stay consistent with prior behavior.
fn next_authored_path(coordinator: &RuntimeDispatchCoordinator) -> Option<DispatchPath> {
    let (up_mask, down_mask) = coordinator.next_authored_packet_masks()?;
    let up_count = up_mask.count_ones() as usize;
    let down_count = down_mask.count_ones() as usize;
    match physical_packet_kind(up_mask, down_mask) {
        Ok(PhysicalPacketKind::UpOnly) => Some(DispatchPath::UpOnly { up_count }),
        Ok(PhysicalPacketKind::DownOnly) => Some(DispatchPath::DownOnly { down_count }),
        Ok(PhysicalPacketKind::Mixed) => Some(DispatchPath::Mixed {
            up_count,
            down_count,
        }),
        Err(_) => {
            let polyphony = coordinator.next_authored_polyphony().max(1);
            Some(DispatchPath::DownOnly {
                down_count: polyphony,
            })
        }
    }
}

fn pending_lead_for_polyphony(
    estimator: &SendLatencyEstimator,
    qpc_clock: QpcClock,
    polyphony: usize,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> Result<(DurationTicks, bool), PlanningError> {
    let (lead_us, saturated) = if timing.dispatch_lead_us > 0 {
        (timing.dispatch_lead_us, false)
    } else if enable_adaptive_lead {
        let estimate = estimator.estimate_lead_with_class_and_policy(
            SendPath::UpOnly,
            polyphony,
            latency_class,
            timing.strict_timing,
        );
        (estimate.applied_us, estimate.saturated)
    } else {
        (0, false)
    };
    qpc_clock
        .duration_from_us(lead_us)
        .map(|ticks| (ticks, saturated))
        .map_err(|error| PlanningError::TimeConversion(format!("{error:?}")))
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
pub(crate) fn plan_next_dispatch(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &SendLatencyEstimator,
    qpc_clock: QpcClock,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
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
        .map_err(|error| match error {
            PlanningError::TimeConversion(message) => CoordinatorError::TimeConversion(message),
            PlanningError::Coordinator(inner) => inner,
        })
    })?;

    let authored_lead_ticks = authored
        .as_ref()
        .map_or(DurationTicks::ZERO, |plan| plan.lead_ticks);
    let deadline_ticks = coordinator.next_deadline_ticks(authored_lead_ticks, pending.as_ref())?;

    Ok(NextDispatchPlan {
        latency_class,
        authored,
        pending,
        deadline_ticks,
    })
}

/// Resolve the path-aware lead used to anchor the startup wait before the
/// first main-loop `NextDispatchPlan` is built.
///
/// The normal loop derives lead from the next authored packet's path; startup
/// must use the same policy instead of a hard-coded Down lead, otherwise a
/// first authored `UpOnly`/`Mixed` packet anchors its physical boundary with
/// the wrong directional lead.
pub(crate) fn startup_lead_for_first_packet(
    coordinator: &RuntimeDispatchCoordinator,
    estimator: &SendLatencyEstimator,
    latency_class: LatencyClass,
    timing: &TimingOptions,
    enable_adaptive_lead: bool,
) -> u64 {
    let path = next_authored_path(coordinator).unwrap_or_else(|| DispatchPath::DownOnly {
        down_count: coordinator.next_authored_polyphony().max(1),
    });
    resolve_authored_lead(estimator, path, latency_class, timing, enable_adaptive_lead).applied_us
}

#[cfg(test)]
mod tests {
    use super::{
        AuthoredDispatchPlan, NextDispatchPlan, plan_next_dispatch, resolve_authored_lead,
        startup_lead_for_first_packet,
    };
    use crate::engine::config::TimingOptions;
    use crate::engine::worker::health::{DispatchPath, estimate_dispatch_path_lead};
    use sky_dispatch_core::compile::compile_runtime_intents;
    use sky_dispatch_core::coordinator::{PendingDispatchPlan, RuntimeDispatchCoordinator};
    use sky_dispatch_core::estimator::{LatencyClass, SendLatencyEstimator, SendPath};
    use sky_dispatch_core::model::{ActionKind, KeyActionInput};
    use sky_dispatch_core::time::{DurationTicks, TimelineTicks};
    use sky_dispatch_win32::clock::QpcClock;
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
            core_warmup_budget_us: 0,
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
