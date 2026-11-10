use super::super::DispatchStep;
use super::observation::StaleMetadataObservation;
#[cfg(any(test, feature = "test-support"))]
use super::observation::{
    BlockedUnfocusedObservation, DownMissObservation, DownMissTimingEvidence,
};
use crate::engine::{
    RtTraceRecord, TRACE_FLAG_ANOMALY, TRACE_KIND_UP, TRACE_SEND_STATUS_NOT_ATTEMPTED,
    TelemetryCollector, TraceContext, TraceDelivery, TraceTiming, trace_outcome_code,
};
#[cfg(any(test, feature = "test-support"))]
use crate::engine::{TRACE_KIND_DOWN, TRACE_KIND_MIXED};
#[cfg(test)]
use sky_dispatch_core::time::TimelineTicks;

pub(super) fn drain_stale_metadata_observation(
    observation: &StaleMetadataObservation,
    telemetry: &mut TelemetryCollector,
) -> Result<(), DispatchStep> {
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: observation.source_action_index,
                source_action_index: observation.source_action_index,
                compiled_packet_index: None,
                kind: TRACE_KIND_UP,
                outcome: trace_outcome_code("suppressed_stale_up"),
                polyphony: observation.suppressed_intent_count,
                flags: TRACE_FLAG_ANOMALY,
                send_status: TRACE_SEND_STATUS_NOT_ATTEMPTED,
                win32_error: 0,
                up_mask: 0,
                down_mask: 0,
            },
            TraceTiming {
                authored_ticks: observation.effective_scheduled_ticks,
                effective_deadline_ticks: observation.effective_scheduled_ticks,
                wake_ticks: None,
                wake_qpc_ticks: None,
                physical_eligibility_qpc_ticks: None,
                timer_arm_qpc_ticks: None,
                timer_arm_target_qpc_ticks: None,
                timer_return_qpc_ticks: None,
                timer_arm_generation: None,
                timer_return_generation: None,
                waiter_deadline_crossing_qpc_ticks: None,
                waiter_return_qpc_ticks: None,
                target_crossing_qpc_ticks: None,
                physical_target_qpc_ticks: None,
                physical_not_before_qpc_ticks: None,
                hold_floor_qpc_ticks: None,
                release_floor_qpc_ticks: None,
                previous_sendinput_completion_qpc_ticks: None,
                previous_sender_completion_known: false,
                hold_floor_mask: 0,
                release_floor_mask: 0,
                final_policy_qpc_ticks: None,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                post_send_qpc_ticks: None,
                observation_qpc_ticks: None,
                pre_call_ticks: None,
                sendinput_completion_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
            },
            TraceDelivery {
                requested: 0,
                sent: 0,
                skipped: 0,
                send_attempts: 0,
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn drain_down_miss(
    observation: &DownMissObservation,
    telemetry: &mut TelemetryCollector,
) -> Result<(), DispatchStep> {
    let (
        physical_target_qpc,
        physical_not_before_qpc,
        hold_floor_qpc,
        release_floor_qpc,
        hold_floor_mask,
        release_floor_mask,
    ) = match observation.timing_evidence {
        DownMissTimingEvidence::Physical(window) => (
            window.authored_target_qpc,
            Some(window.packet_not_before_qpc),
            Some(window.musical_up_not_before_qpc),
            Some(window.down_not_before_qpc),
            window.hold_floor_mask,
            window.release_floor_mask,
        ),
    };
    let down_count = observation.down_mask.count_ones() as usize;
    let kind = match (observation.up_mask != 0, observation.down_mask != 0) {
        (true, false) => TRACE_KIND_UP,
        (false, true) => TRACE_KIND_DOWN,
        (true, true) => TRACE_KIND_MIXED,
        (false, false) => TRACE_KIND_DOWN,
    };
    let (outcome, send_status) = match observation.kind {
        super::observation::DownMissKind::UnobservedBacklog => {
            ("down_unobserved_backlog", TRACE_SEND_STATUS_NOT_ATTEMPTED)
        }
    };
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: observation.source_action_index,
                source_action_index: observation.source_action_index,
                compiled_packet_index: observation.compiled_packet_index,
                kind,
                outcome: trace_outcome_code(outcome),
                polyphony: down_count,
                flags: TRACE_FLAG_ANOMALY,
                send_status,
                win32_error: 0,
                up_mask: observation.up_mask,
                down_mask: observation.down_mask,
            },
            TraceTiming {
                authored_ticks: observation.authored_ticks,
                effective_deadline_ticks: observation.effective_deadline_ticks,
                wake_ticks: None,
                wake_qpc_ticks: None,
                physical_eligibility_qpc_ticks: None,
                timer_arm_qpc_ticks: None,
                timer_arm_target_qpc_ticks: None,
                timer_return_qpc_ticks: None,
                timer_arm_generation: None,
                timer_return_generation: None,
                waiter_deadline_crossing_qpc_ticks: None,
                waiter_return_qpc_ticks: None,
                target_crossing_qpc_ticks: None,
                physical_target_qpc_ticks: Some(physical_target_qpc.as_u64()),
                physical_not_before_qpc_ticks: physical_not_before_qpc.map(|ticks| ticks.as_u64()),
                hold_floor_qpc_ticks: hold_floor_qpc.map(|ticks| ticks.as_u64()),
                release_floor_qpc_ticks: release_floor_qpc.map(|ticks| ticks.as_u64()),
                previous_sendinput_completion_qpc_ticks: None,
                previous_sender_completion_known: false,
                hold_floor_mask,
                release_floor_mask,
                final_policy_qpc_ticks: None,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                post_send_qpc_ticks: None,
                observation_qpc_ticks: Some(observation.observed_qpc.as_u64()),
                pre_call_ticks: None,
                sendinput_completion_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
            },
            TraceDelivery {
                requested: down_count,
                sent: 0,
                skipped: down_count,
                send_attempts: 0,
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn drain_blocked_unfocused_observation(
    observation: &BlockedUnfocusedObservation,
    telemetry: &mut TelemetryCollector,
) -> Result<(), DispatchStep> {
    let kind = match (observation.up_mask != 0, observation.down_mask != 0) {
        (true, false) => TRACE_KIND_UP,
        (false, true) => TRACE_KIND_DOWN,
        (true, true) => TRACE_KIND_MIXED,
        (false, false) => TRACE_KIND_DOWN,
    };
    if let Err(error) = telemetry.try_push(|| {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index: observation.event_index,
                source_action_index: observation.event_index,
                compiled_packet_index: observation.compiled_packet_index,
                kind,
                outcome: trace_outcome_code("blocked_unfocused"),
                polyphony: observation.polyphony,
                flags: TRACE_FLAG_ANOMALY,
                send_status: TRACE_SEND_STATUS_NOT_ATTEMPTED,
                win32_error: 0,
                up_mask: observation.up_mask,
                down_mask: observation.down_mask,
            },
            TraceTiming {
                authored_ticks: observation.authored_ticks,
                effective_deadline_ticks: observation.effective_deadline_ticks,
                wake_ticks: None,
                wake_qpc_ticks: None,
                physical_eligibility_qpc_ticks: None,
                timer_arm_qpc_ticks: None,
                timer_arm_target_qpc_ticks: None,
                timer_return_qpc_ticks: None,
                timer_arm_generation: None,
                timer_return_generation: None,
                waiter_deadline_crossing_qpc_ticks: None,
                waiter_return_qpc_ticks: None,
                target_crossing_qpc_ticks: None,
                physical_target_qpc_ticks: Some(observation.physical_target_qpc.as_u64()),
                physical_not_before_qpc_ticks: None,
                hold_floor_qpc_ticks: None,
                release_floor_qpc_ticks: None,
                previous_sendinput_completion_qpc_ticks: None,
                previous_sender_completion_known: false,
                hold_floor_mask: 0,
                release_floor_mask: 0,
                final_policy_qpc_ticks: None,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                post_send_qpc_ticks: None,
                observation_qpc_ticks: Some(observation.observed_qpc.as_u64()),
                pre_call_ticks: None,
                sendinput_completion_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
            },
            TraceDelivery {
                requested: 0,
                sent: 0,
                skipped: 0,
                send_attempts: 0,
            },
        )
    }) {
        return Err(DispatchStep::Terminate(format!(
            "native telemetry record overflow: {error}"
        )));
    }
    Ok(())
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) struct UpObservationDrainContext<'a> {
    pub(crate) health: &'a mut crate::engine::worker::WorkerHealthState,
    pub(crate) local_metrics: &'a mut crate::engine::WorkerMetricsLocal,
    pub(crate) metrics: &'a crate::engine::SharedMetrics,
    pub(crate) telemetry: &'a mut TelemetryCollector,
    pub(crate) qpc_clock: sky_dispatch_win32::clock::QpcClock,
    pub(crate) now_us: u64,
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn drain_up_send_outcome(
    observation: &super::observation::UpObservation,
    ctx: UpObservationDrainContext<'_>,
) -> Result<(), DispatchStep> {
    let health = ctx.health;
    let local_metrics = ctx.local_metrics;
    let metrics = ctx.metrics;
    let telemetry = ctx.telemetry;
    let qpc_clock = ctx.qpc_clock;
    let now_us = ctx.now_us;
    let health_budget = crate::engine::worker::health::build_dispatch_budget(
        crate::engine::worker::DispatchPath::UpOnly {
            up_count: observation.requested_mask.count_ones() as usize,
        },
        health.options,
    );
    let send_warn_us = health_budget.send_warn_us;
    let core_post_send_warn_us = health_budget.core_post_send_warn_us;
    let (scan_count, _sent_count, _skipped_count) =
        super::observation::up_transport_counts(observation);
    let pre_call_to_completion_us = qpc_clock
        .duration_to_us(observation.pre_call_to_completion_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off duration conversion failure: {error:?}"))
        })?;
    let completion_residual_us = qpc_clock
        .duration_to_us(
            observation
                .sendinput_completion_qpc
                .checked_duration_since(observation.physical_target_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-off completion precedes physical target: {error:?}"
                    ))
                })?,
        )
        .map_err(|error| {
            DispatchStep::Terminate(format!(
                "note-off completion-residual conversion failure: {error:?}"
            ))
        })?;
    let completed_effective_us = qpc_clock
        .timeline_to_us(observation.completed_effective_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off completion conversion failure: {error:?}"))
        })?;
    let scheduled_us = qpc_clock
        .timeline_to_us(observation.scheduled_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off scheduled conversion failure: {error:?}"))
        })?;
    let deferred_by_us = qpc_clock
        .duration_to_us(observation.deferred_ticks)
        .map_err(|error| {
            DispatchStep::Terminate(format!("note-off deferral conversion failure: {error:?}"))
        })?;
    let _up_completion_error_us =
        crate::engine::worker::signed_ticks_to_us(qpc_clock, observation.up_completion_error_ticks)
            .map_err(|error| {
                DispatchStep::Terminate(format!("note-off error conversion failure: {error:?}"))
            })?;
    let core_post_send_us = observation
        .dispatch_ready_qpc
        .map(|ready| {
            ready
                .checked_duration_since(observation.sendinput_completion_qpc)
                .map_err(|error| {
                    DispatchStep::Terminate(format!(
                        "note-off observer QPC ordering failure: {error:?}"
                    ))
                })
                .and_then(|ticks| {
                    qpc_clock.duration_to_us(ticks).map_err(|error| {
                        DispatchStep::Terminate(format!(
                            "note-off observer post-send conversion failure: {error:?}"
                        ))
                    })
                })
        })
        .transpose()?
        .unwrap_or(0);
    let wake_to_final_policy_us = observation
        .wake_qpc
        .map(|wake| {
            super::observer::qpc_span_us(
                qpc_clock,
                wake,
                observation.final_policy_qpc,
                "note-off wake-to-final-policy",
            )
        })
        .transpose()?;
    let final_policy_to_pre_call_us = super::observer::qpc_span_us(
        qpc_clock,
        observation.final_policy_qpc,
        observation.pre_call_qpc,
        "note-off final-policy-to-pre-call",
    )?;
    let wake_to_send_start_us = observation
        .wake_qpc
        .map(|wake| {
            super::observer::qpc_span_us(
                qpc_clock,
                wake,
                observation.pre_call_qpc,
                "note-off wake-to-pre-call",
            )
        })
        .transpose()?;
    local_metrics.sendinput_warn_threshold_us = send_warn_us;
    local_metrics.core_post_send_warn_threshold_us = core_post_send_warn_us;
    local_metrics.post_send_metrics_available |= observation.dispatch_ready_qpc.is_some();
    local_metrics.send_up_warn_threshold_us = send_warn_us;
    local_metrics.wait_warn_threshold_us = health.options.wait_warn_us;
    super::observation::record_release_telemetry(
        telemetry,
        observation,
        qpc_clock,
        core_post_send_us,
        completion_residual_us,
        observation.dispatch_ready_qpc.is_some(),
    )?;
    if let Some(wake_to_send_us) = wake_to_send_start_us {
        local_metrics.wake_to_send_max_us = local_metrics.wake_to_send_max_us.max(wake_to_send_us);
    }
    if let Some(pause_ticks) = observation.recovery_pause_ticks {
        let recovery_pause_us = qpc_clock.duration_to_us(pause_ticks).map_err(|error| {
            DispatchStep::Terminate(format!("recovery telemetry conversion failure: {error:?}"))
        })?;
        local_metrics.total_us = local_metrics.total_us.saturating_add(recovery_pause_us);
    }
    local_metrics.core_post_send_max_us =
        local_metrics.core_post_send_max_us.max(core_post_send_us);
    local_metrics.sendinput_duration_max_us = local_metrics
        .sendinput_duration_max_us
        .max(pre_call_to_completion_us);
    local_metrics.final_policy_to_pre_call_max_us = local_metrics
        .final_policy_to_pre_call_max_us
        .max(final_policy_to_pre_call_us);
    if let Some(wake_to_final_policy_us) = wake_to_final_policy_us {
        local_metrics.wake_to_final_policy_max_us = local_metrics
            .wake_to_final_policy_max_us
            .max(wake_to_final_policy_us);
    }
    crate::engine::worker::record_lateness(
        crate::engine::worker::signed_delta(completed_effective_us, scheduled_us),
        true,
        deferred_by_us > 0,
        local_metrics,
    );
    crate::engine::worker::observe_dispatch_health(
        crate::engine::worker::DispatchHealthObservation {
            send_duration_us: pre_call_to_completion_us,
            post_send_duration_us: core_post_send_us,
            post_send_metrics_available: observation.dispatch_ready_qpc.is_some(),
            path: crate::engine::worker::DispatchPath::UpOnly {
                up_count: scan_count,
            },
            send_warn_us,
            core_post_send_warn_us,
            elapsed_us: completed_effective_us,
        },
        health.options.window_policy(),
        &mut health.sendinput_window,
        &mut health.core_post_send_window,
        local_metrics,
    );
    crate::engine::try_publish_observer_metrics(
        local_metrics,
        metrics,
        qpc_clock,
        now_us,
        !super::timing::is_clean_dispatch_observation(super::observation::up_dispatch_evidence(
            observation,
        )) || observation.trace.recovery_required,
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::observation::DownMissKind;
    use super::*;
    use crate::engine::{TRACE_KIND_DOWN, TRACE_SEND_STATUS_NOT_ATTEMPTED, TelemetryMode};
    use sky_dispatch_win32::clock::QpcTicks;

    fn physical_window(
        authored_target: u64,
        hold_floor: u64,
        release_floor: u64,
        packet_not_before: u64,
        hold_floor_mask: u16,
        release_floor_mask: u16,
    ) -> crate::engine::worker::physical_timing_guard::PhysicalTimingWindow {
        crate::engine::worker::physical_timing_guard::PhysicalTimingWindow {
            authored_target_qpc: QpcTicks::from_raw(authored_target),
            musical_up_not_before_qpc: QpcTicks::from_raw(hold_floor),
            down_not_before_qpc: QpcTicks::from_raw(release_floor),
            packet_not_before_qpc: QpcTicks::from_raw(packet_not_before),
            hold_floor_mask,
            release_floor_mask,
        }
    }

    #[test]
    fn backlog_trace_is_distinct_and_records_unattempted_down() {
        let observation = DownMissObservation {
            source_action_index: 8,
            compiled_packet_index: Some(52),
            authored_ticks: TimelineTicks::from_raw(10),
            effective_deadline_ticks: TimelineTicks::from_raw(12),
            timing_evidence: DownMissTimingEvidence::Physical(physical_window(
                1_000, 1_000, 1_000, 1_000, 0, 0,
            )),
            observed_qpc: QpcTicks::from_raw(1_021),
            up_mask: 0,
            down_mask: 0b11,
            kind: DownMissKind::UnobservedBacklog,
        };
        let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 4);

        drain_down_miss(&observation, &mut collector).expect("record backlog miss");

        let record = collector.output.records.front().expect("trace record");
        assert_eq!(record.source_action_index, 8);
        assert_eq!(record.compiled_packet_index, 52);
        assert!(record.compiled_packet_index_available);
        assert_eq!(record.kind, TRACE_KIND_DOWN);
        assert_eq!(
            record.outcome,
            trace_outcome_code("down_unobserved_backlog")
        );
        assert_eq!(record.send_status, TRACE_SEND_STATUS_NOT_ATTEMPTED);
        assert_eq!(record.authored_offset_ticks, 10);
        assert!(!record.pre_call_qpc_available);
        assert!(!record.sendinput_completion_qpc_available);
        assert!(record.observation_qpc_available);
        assert_eq!(record.down_mask, 0b11);
        assert_eq!(record.requested_count, 2);
        assert_eq!(record.sent_count, 0);
        assert_eq!(record.skipped_count, 2);
        assert_eq!(record.send_attempts, 0);
    }
}
