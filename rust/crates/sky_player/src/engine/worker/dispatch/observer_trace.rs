use super::super::DispatchStep;
use super::observation::{
    BlockedUnfocusedObservation, DownMissObservation, StaleMetadataObservation,
};
use crate::engine::worker::timing::signed_timeline_delta_ticks;
use crate::engine::{
    RtTraceRecord, TRACE_FLAG_ANOMALY, TRACE_KIND_DOWN, TRACE_KIND_MIXED, TRACE_KIND_UP,
    TRACE_SEND_STATUS_DOWN_EXPIRED, TRACE_SEND_STATUS_NOT_ATTEMPTED, TelemetryCollector,
    TraceContext, TraceDelivery, TraceTiming, trace_outcome_code,
};
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
                wake_ticks: observation.effective_now_ticks,
                wake_available: false,
                physical_target_qpc_ticks: None,
                physical_not_before_qpc_ticks: None,
                hold_floor_qpc_ticks: None,
                release_floor_qpc_ticks: None,
                latest_down_start_qpc_ticks: None,
                hold_floor_mask: 0,
                release_floor_mask: 0,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                observation_qpc_ticks: None,
                final_policy_ticks: None,
                pre_call_ticks: None,
                sendinput_completion_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks: 0,
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

pub(super) fn drain_down_miss(
    observation: &DownMissObservation,
    telemetry: &mut TelemetryCollector,
) -> Result<(), DispatchStep> {
    let dispatch_start_error_ticks = signed_timeline_delta_ticks(
        TimelineTicks::from_raw(observation.observed_qpc.as_u64()),
        TimelineTicks::from_raw(
            observation
                .physical_timing_window
                .authored_target_qpc
                .as_u64(),
        ),
    )
    .map_err(|error| {
        DispatchStep::Terminate(format!(
            "Down-miss trace QPC delta conversion failure: {error}"
        ))
    })?;
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
        super::observation::DownMissKind::PhysicalWindowExpired => (
            "down_physical_window_expired",
            TRACE_SEND_STATUS_NOT_ATTEMPTED,
        ),
        super::observation::DownMissKind::DownExpiredBeforeSend => (
            "down_final_sender_window_expired",
            TRACE_SEND_STATUS_DOWN_EXPIRED,
        ),
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
                wake_ticks: observation.wake_ticks,
                wake_available: false,
                physical_target_qpc_ticks: Some(
                    observation
                        .physical_timing_window
                        .authored_target_qpc
                        .as_u64(),
                ),
                physical_not_before_qpc_ticks: Some(
                    observation
                        .physical_timing_window
                        .packet_not_before_qpc
                        .as_u64(),
                ),
                hold_floor_qpc_ticks: Some(
                    observation
                        .physical_timing_window
                        .musical_up_not_before_qpc
                        .as_u64(),
                ),
                release_floor_qpc_ticks: Some(
                    observation
                        .physical_timing_window
                        .down_not_before_qpc
                        .as_u64(),
                ),
                latest_down_start_qpc_ticks: observation
                    .physical_timing_window
                    .latest_down_start_qpc
                    .map(|ticks| ticks.as_u64()),
                hold_floor_mask: observation.physical_timing_window.hold_floor_mask,
                release_floor_mask: observation.physical_timing_window.release_floor_mask,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                observation_qpc_ticks: Some(observation.observed_qpc.as_u64()),
                final_policy_ticks: None,
                pre_call_ticks: None,
                sendinput_completion_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks,
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
                wake_ticks: observation.effective_now_ticks,
                wake_available: false,
                physical_target_qpc_ticks: Some(observation.physical_target_qpc.as_u64()),
                physical_not_before_qpc_ticks: None,
                hold_floor_qpc_ticks: None,
                release_floor_qpc_ticks: None,
                latest_down_start_qpc_ticks: None,
                hold_floor_mask: 0,
                release_floor_mask: 0,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                observation_qpc_ticks: Some(observation.observed_qpc.as_u64()),
                final_policy_ticks: None,
                pre_call_ticks: None,
                sendinput_completion_ticks: None,
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks: 0,
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

#[cfg(test)]
mod tests {
    use super::super::observation::DownMissKind;
    use super::*;
    use crate::engine::{
        TRACE_KIND_DOWN, TRACE_KIND_MIXED, TRACE_SEND_STATUS_DOWN_EXPIRED,
        TRACE_SEND_STATUS_NOT_ATTEMPTED, TelemetryMode,
    };
    use sky_dispatch_win32::clock::QpcTicks;

    fn physical_window(
        authored_target: u64,
        hold_floor: u64,
        release_floor: u64,
        packet_not_before: u64,
        latest_down_start: Option<u64>,
        hold_floor_mask: u16,
        release_floor_mask: u16,
    ) -> crate::engine::worker::physical_timing_guard::PhysicalTimingWindow {
        crate::engine::worker::physical_timing_guard::PhysicalTimingWindow {
            authored_target_qpc: QpcTicks::from_raw(authored_target),
            musical_up_not_before_qpc: QpcTicks::from_raw(hold_floor),
            down_not_before_qpc: QpcTicks::from_raw(release_floor),
            packet_not_before_qpc: QpcTicks::from_raw(packet_not_before),
            latest_down_start_qpc: latest_down_start.map(QpcTicks::from_raw),
            hold_floor_mask,
            release_floor_mask,
        }
    }

    #[test]
    fn cutoff_trace_preserves_mixed_same_key_boundary_and_zero_attempts() {
        let observation = DownMissObservation {
            source_action_index: 41,
            compiled_packet_index: Some(37),
            authored_ticks: TimelineTicks::from_raw(10),
            effective_deadline_ticks: TimelineTicks::from_raw(12),
            wake_ticks: TimelineTicks::from_raw(20),
            physical_timing_window: physical_window(1_000, 1_005, 1_020, 1_020, Some(1_010), 1, 2),
            observed_qpc: QpcTicks::from_raw(1_021),
            up_mask: 0b0001,
            down_mask: 0b0001,
            kind: DownMissKind::DownExpiredBeforeSend,
        };
        let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 4);

        drain_down_miss(&observation, &mut collector).expect("record cutoff miss");

        let record = collector.output.records.front().expect("trace record");
        assert_eq!(record.trace_record_index, 0);
        assert_eq!(record.compiled_packet_index, 37);
        assert!(record.compiled_packet_index_available);
        assert_eq!(record.source_action_index, 41);
        assert_eq!(record.event_index, 41);
        assert_eq!(record.kind, TRACE_KIND_MIXED);
        assert_eq!(
            record.outcome,
            trace_outcome_code("down_final_sender_window_expired")
        );
        assert_eq!(record.send_status, TRACE_SEND_STATUS_DOWN_EXPIRED);
        assert_eq!(record.authored_target_qpc_ticks, 1_000);
        assert!(record.authored_target_qpc_available);
        assert_eq!(record.physical_not_before_qpc_ticks, 1_020);
        assert!(record.physical_not_before_qpc_available);
        assert_eq!(record.hold_floor_qpc_ticks, 1_005);
        assert!(record.hold_floor_qpc_available);
        assert_eq!(record.release_floor_qpc_ticks, 1_020);
        assert!(record.release_floor_qpc_available);
        assert_eq!(record.latest_down_start_qpc_ticks, 1_010);
        assert!(record.latest_down_start_qpc_available);
        assert_eq!(record.hold_floor_mask, 1);
        assert_eq!(record.release_floor_mask, 2);
        assert!(!record.pre_call_qpc_available);
        assert!(!record.sendinput_completion_qpc_available);
        assert_eq!(record.observation_qpc_ticks, 1_021);
        assert!(record.observation_qpc_available);
        assert_eq!(record.send_started_ticks, 0);
        assert_eq!(record.up_mask, 1);
        assert_eq!(record.down_mask, 1);
        assert_eq!(record.dispatch_start_error_ticks, 21);
        assert_eq!(record.requested_count, 1);
        assert_eq!(record.sent_count, 0);
        assert_eq!(record.skipped_count, 1);
        assert_eq!(record.send_attempts, 0);
        assert_eq!(collector.output.attempted, 1);
        assert_eq!(collector.output.accepted, 1);
        assert_eq!(collector.output.dropped, 0);
    }

    #[test]
    fn backlog_trace_is_distinct_and_records_unattempted_down() {
        let observation = DownMissObservation {
            source_action_index: 8,
            compiled_packet_index: Some(52),
            authored_ticks: TimelineTicks::from_raw(10),
            effective_deadline_ticks: TimelineTicks::from_raw(12),
            wake_ticks: TimelineTicks::from_raw(20),
            physical_timing_window: physical_window(1_000, 1_000, 1_000, 1_000, Some(1_010), 0, 0),
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
        assert_eq!(record.authored_target_qpc_ticks, 1_000);
        assert_eq!(record.latest_down_start_qpc_ticks, 1_010);
        assert!(!record.pre_call_qpc_available);
        assert!(!record.sendinput_completion_qpc_available);
        assert!(record.observation_qpc_available);
        assert_eq!(record.down_mask, 0b11);
        assert_eq!(record.requested_count, 2);
        assert_eq!(record.sent_count, 0);
        assert_eq!(record.skipped_count, 2);
        assert_eq!(record.send_attempts, 0);
    }

    #[test]
    fn physical_window_expiration_is_distinct_from_sender_race_and_backlog() {
        let observation = DownMissObservation {
            source_action_index: 12,
            compiled_packet_index: Some(61),
            authored_ticks: TimelineTicks::from_raw(10),
            effective_deadline_ticks: TimelineTicks::from_raw(12),
            wake_ticks: TimelineTicks::from_raw(20),
            physical_timing_window: physical_window(1_000, 1_000, 1_000, 1_000, Some(1_010), 0, 0),
            observed_qpc: QpcTicks::from_raw(1_021),
            up_mask: 0,
            down_mask: 0b101,
            kind: DownMissKind::PhysicalWindowExpired,
        };
        let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 4);

        drain_down_miss(&observation, &mut collector).expect("record physical window miss");

        let record = collector.output.records.front().expect("trace record");
        assert_eq!(
            record.outcome,
            trace_outcome_code("down_physical_window_expired")
        );
        assert_eq!(record.send_status, TRACE_SEND_STATUS_NOT_ATTEMPTED);
        assert_eq!(record.requested_count, 2);
        assert_eq!(record.sent_count, 0);
        assert_eq!(record.send_attempts, 0);
    }
}
