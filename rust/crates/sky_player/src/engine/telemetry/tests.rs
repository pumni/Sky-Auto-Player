use super::{
    NativeTelemetrySummary, RtTraceRecord, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN, TRACE_KIND_MIXED,
    TRACE_KIND_UP, TRACE_SEND_STATUS_CLOCK_FAILURE_BEFORE_SEND, TRACE_SEND_STATUS_COMPLETE,
    TelemetryCollector, TelemetryMode, TimestampProvenance, TraceContext, TraceDelivery,
    TraceTiming, trace_outcome_code,
};
use sky_dispatch_core::time::TimelineTicks;

fn record_with_index(event_index: u32, kind: u8, requested: usize, sent: usize) -> RtTraceRecord {
    record_with_down_chord_index(event_index, kind, requested, sent, None)
}

fn record_with_down_chord_index(
    event_index: u32,
    kind: u8,
    requested: usize,
    sent: usize,
    authored_down_chord_index: Option<u64>,
) -> RtTraceRecord {
    RtTraceRecord::dispatched(
        TraceContext {
            event_index,
            source_action_index: event_index,
            authored_down_chord_index,
            kind,
            outcome: trace_outcome_code("sent"),
            polyphony: requested,
            flags: TRACE_FLAG_SENT_FULL,
            send_status: TRACE_SEND_STATUS_COMPLETE,
            win32_error: 0,
            up_mask: if kind == TRACE_KIND_UP {
                (1_u16 << requested) - 1
            } else if kind == TRACE_KIND_MIXED {
                1
            } else {
                0
            },
            down_mask: if kind == TRACE_KIND_UP {
                0
            } else if kind == TRACE_KIND_MIXED {
                0b10
            } else {
                (1_u16 << requested) - 1
            },
        },
        TraceTiming {
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: Some(TimelineTicks::ZERO),
            wake_qpc_ticks: Some(0),
            physical_eligibility_qpc_ticks: Some(1),
            timer_arm_qpc_ticks: Some(4),
            timer_arm_target_qpc_ticks: Some(5),
            timer_return_qpc_ticks: Some(6),
            timer_arm_generation: Some(7),
            timer_return_generation: Some(7),
            waiter_deadline_crossing_qpc_ticks: Some(3),
            waiter_return_qpc_ticks: Some(4),
            pre_call_qpc_ticks: Some(1),
            sendinput_completion_qpc_ticks: Some(2),
            physical_target_qpc_ticks: None,
            physical_not_before_qpc_ticks: None,
            hold_floor_qpc_ticks: None,
            down_not_before_qpc_ticks: None,
            hold_floor_mask: 0,
            observation_qpc_ticks: None,
            pre_call_ticks: Some(TimelineTicks::from_raw(1)),
            sendinput_completion_ticks: Some(TimelineTicks::from_raw(2)),
            completion_residual_us: 0,
            core_post_send_duration_us: 0,
            post_send_metrics_available: false,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            ..TraceTiming::default()
        },
        TraceDelivery {
            requested,
            sent,
            skipped: 0,
            send_attempts: 1,
        },
    )
    .expect("valid telemetry record")
}

fn record(kind: u8, requested: usize, sent: usize) -> RtTraceRecord {
    record_with_index(0, kind, requested, sent)
}

#[test]
fn up_only_summary_is_not_counted_as_down() {
    let record = record(TRACE_KIND_UP, 2, 2);
    let mut summary = NativeTelemetrySummary::default();

    summary.observe(&record);

    assert_eq!(summary.down_count, 0);
    assert_eq!(summary.up_count, 1);
    assert_eq!(summary.requested_key_count, 2);
    assert_eq!(summary.sent_key_count, 2);
}

#[test]
fn timing_semantics_names_the_true_pre_call_boundary() {
    let semantics = super::TimingSemantics::default();
    assert_eq!(
        semantics.sender_start_boundary,
        "pre_call_qpc_before_sendinput"
    );
    assert_eq!(
        semantics.sender_completion_boundary,
        "sendinput_completion_qpc"
    );
    assert_eq!(
        semantics.timer_arm_target_boundary,
        "qpc_sample_before_timer_arm_plus_relative_timeout_passed_to_timer_api"
    );
    assert!(
        semantics
            .timestamp_provenance
            .contains("derived_from_timer_arm")
    );
    assert!(!semantics.game_observed_available);
}

#[test]
fn serialized_timestamp_provenance_distinguishes_measured_derived_and_unavailable() {
    let measured = record_with_index(1, TRACE_KIND_UP, 1, 1);
    let value = serde_json::to_value(measured).expect("serialize trace");
    assert_eq!(value["wake_offset_provenance"], "derived_from_wake_qpc");
    assert_eq!(value["wake_offset_available"], true);
    assert_eq!(value["wake_qpc_provenance"], "measured");
    assert_eq!(value["physical_eligibility_qpc_ticks"], 1);
    assert_eq!(
        value["physical_eligibility_qpc_provenance"],
        "configured_target"
    );
    assert_eq!(value["timer_arm_qpc_ticks"], 4);
    assert_eq!(value["timer_arm_qpc_provenance"], "measured");
    assert_eq!(value["timer_arm_target_qpc_ticks"], 5);
    assert_eq!(
        value["timer_arm_target_qpc_provenance"],
        "derived_from_timer_arm"
    );
    assert_eq!(value["timer_return_qpc_ticks"], 6);
    assert_eq!(value["timer_arm_generation"], 7);
    assert_eq!(value["timer_return_generation"], 7);
    assert_eq!(value["timer_return_qpc_provenance"], "measured");
    assert_eq!(value["waiter_deadline_crossing_qpc_ticks"], 3);
    assert_eq!(value["waiter_deadline_crossing_qpc_provenance"], "measured");
    assert_eq!(value["waiter_return_qpc_ticks"], 4);
    assert_eq!(value["waiter_return_qpc_provenance"], "measured");
    assert_eq!(value["pre_call_qpc_provenance"], "measured");
    assert_eq!(value["sendinput_completion_qpc_provenance"], "measured");
    assert_eq!(
        value["send_started_ticks_provenance"],
        "derived_from_pre_call"
    );
    assert_eq!(
        value["send_completed_ticks_provenance"],
        "derived_from_send_input_completion"
    );
    assert_eq!(value["final_policy_qpc_provenance"], "unavailable");
    assert_eq!(value["final_policy_qpc_available"], false);
    assert_eq!(value["final_policy_qpc_ticks"], 0);
    assert_eq!(super::NATIVE_TELEMETRY_SCHEMA_VERSION, 27);
}

#[test]
fn derived_offsets_are_unavailable_without_their_absolute_qpc_source() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 0,
            source_action_index: 0,
            authored_down_chord_index: None,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("sent"),
            polyphony: 1,
            flags: TRACE_FLAG_SENT_FULL,
            send_status: TRACE_SEND_STATUS_COMPLETE,
            win32_error: 0,
            up_mask: 0,
            down_mask: 1,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: Some(TimelineTicks::from_raw(7)),
            pre_call_ticks: Some(TimelineTicks::from_raw(8)),
            sendinput_completion_ticks: Some(TimelineTicks::from_raw(9)),
            ..TraceTiming::default()
        },
        TraceDelivery {
            requested: 1,
            sent: 1,
            skipped: 0,
            send_attempts: 1,
        },
    )
    .expect("trace with derived values but no source samples");
    let value = serde_json::to_value(record).expect("serialize trace");

    for (value_key, available_key, provenance_key, available) in [
        (
            "wake_offset_ticks",
            Some("wake_offset_available"),
            "wake_offset_provenance",
            record.wake_offset_available,
        ),
        (
            "send_started_ticks",
            None,
            "send_started_ticks_provenance",
            record.send_started_ticks_provenance == TimestampProvenance::Measured,
        ),
        (
            "send_completed_ticks",
            None,
            "send_completed_ticks_provenance",
            record.send_completed_ticks_provenance == TimestampProvenance::Measured,
        ),
    ] {
        assert!(!available, "{value_key} requires its absolute QPC source");
        assert_eq!(value[provenance_key], "unavailable");
        if let Some(available_key) = available_key {
            assert_eq!(value[available_key], false);
        }
        assert_eq!(value[value_key], 0);
    }
}

#[test]
fn absent_pre_call_qpc_never_uses_a_relative_or_schedule_value_as_a_sample() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 0,
            source_action_index: 0,
            authored_down_chord_index: None,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("sent"),
            polyphony: 1,
            flags: 0,
            send_status: TRACE_SEND_STATUS_COMPLETE,
            win32_error: 0,
            up_mask: 0,
            down_mask: 1,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::from_raw(90),
            effective_deadline_ticks: TimelineTicks::from_raw(95),
            pre_call_ticks: Some(TimelineTicks::from_raw(100)),
            ..TraceTiming::default()
        },
        TraceDelivery {
            requested: 1,
            sent: 1,
            skipped: 0,
            send_attempts: 1,
        },
    )
    .expect("trace without sampled QPC boundaries");
    let value = serde_json::to_value(record).expect("serialize trace");
    assert_eq!(
        record.pre_call_qpc_provenance,
        TimestampProvenance::Unavailable
    );
    assert_eq!(
        record.send_started_ticks_provenance,
        TimestampProvenance::Unavailable
    );
    assert!(!record.authored_lateness_available);
    assert_eq!(value["pre_call_qpc_ticks"], 0);
    assert_eq!(value["pre_call_qpc_provenance"], "unavailable");
    assert_eq!(value["send_started_ticks"], 0);
    assert_eq!(value["send_started_ticks_provenance"], "unavailable");

    let mut summary = NativeTelemetrySummary::default();
    summary.observe(&record);
    assert_eq!(summary.authored_lateness_samples, 0);
    assert_eq!(summary.overdue_packet_count, 0);
}

#[test]
fn pre_send_transaction_failure_is_counted_without_fabricating_an_attempt() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 0,
            source_action_index: 0,
            authored_down_chord_index: None,
            kind: TRACE_KIND_DOWN,
            outcome: trace_outcome_code("aborted"),
            polyphony: 1,
            flags: 0,
            send_status: TRACE_SEND_STATUS_CLOCK_FAILURE_BEFORE_SEND,
            win32_error: 0,
            up_mask: 0,
            down_mask: 1,
        },
        TraceTiming::default(),
        TraceDelivery {
            requested: 1,
            sent: 0,
            skipped: 0,
            send_attempts: 0,
        },
    )
    .expect("pre-send failure trace");
    assert_eq!(record.send_attempts, 0);
    assert!(record.transport_anomaly);

    let mut summary = NativeTelemetrySummary::default();
    summary.observe(&record);
    assert_eq!(summary.dispatch_count, 1);
    assert_eq!(summary.transport_anomaly_count, 1);
    assert_eq!(summary.authored_lateness_samples, 0);
}

#[test]
fn signed_start_error_max_preserves_all_negative_samples() {
    let mut summary = NativeTelemetrySummary::default();

    for (event_index, start_error_ticks) in [(0, -40), (1, -30), (2, -20)] {
        let mut record = record_with_index(event_index, TRACE_KIND_UP, 1, 1);
        record.authored_lateness_available = true;
        record.authored_lateness_ticks = start_error_ticks;
        record.dispatch_start_error_ticks = start_error_ticks;
        summary.observe(&record);
    }

    assert_eq!(summary.max_dispatch_start_error_ticks, -20);
    assert_eq!(summary.max_dispatch_start_error_abs_ticks, 40);
    assert_eq!(summary.max_authored_lateness_ticks, -20);
    assert_eq!(summary.max_authored_lateness_abs_ticks, 40);
}

#[test]
fn mixed_record_keeps_mixed_kind_and_physical_count() {
    let record = record(TRACE_KIND_MIXED, 2, 2);
    let mut summary = NativeTelemetrySummary::default();

    summary.observe(&record);

    assert_eq!(record.kind, TRACE_KIND_MIXED);
    assert_eq!(record.up_mask, 1);
    assert_eq!(record.down_mask, 0b10);
    assert_eq!(record.up_mask & record.down_mask, 0);
    assert_eq!(record.requested_count, 2);
    assert_eq!(record.sent_count, 2);
    assert_eq!(summary.dispatch_count, 1);
    assert_eq!(summary.requested_key_count, 2);
    assert_eq!(summary.sent_key_count, 2);
}

#[test]
fn packet_masks_classify_single_chord_and_up_only_records() {
    let single = record(TRACE_KIND_DOWN, 1, 1);
    let chord = record(TRACE_KIND_DOWN, 3, 3);
    let up_only = record(TRACE_KIND_UP, 2, 2);

    assert_eq!(
        (single.kind, single.down_mask.count_ones()),
        (TRACE_KIND_DOWN, 1)
    );
    assert_eq!(
        (chord.kind, chord.down_mask.count_ones()),
        (TRACE_KIND_DOWN, 3)
    );
    assert_eq!(
        (up_only.kind, up_only.up_mask.count_ones()),
        (TRACE_KIND_UP, 2)
    );
    assert_eq!(single.source_action_index, single.event_index);
    assert_eq!(chord.source_action_index, chord.event_index);
    assert_eq!(up_only.source_action_index, up_only.event_index);
}

#[test]
fn telemetry_retains_all_records_above_303() {
    let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 304);

    for event_index in 0..304_u32 {
        collector
            .try_push(|| Ok(record_with_index(event_index, TRACE_KIND_UP, 1, 1)))
            .expect("telemetry record must be representable");
    }

    let output = collector.output;
    assert_eq!(output.attempted, 304);
    assert_eq!(output.accepted, 304);
    assert_eq!(output.dropped, 0);
    assert!(!output.truncated);
    assert_eq!(output.records.len(), 304);
    assert_eq!(
        output
            .records
            .iter()
            .map(|record| record.event_index)
            .collect::<Vec<_>>(),
        (0..304_u32).collect::<Vec<_>>()
    );
    assert_eq!(
        output
            .records
            .iter()
            .map(|record| record.trace_record_index)
            .collect::<Vec<_>>(),
        (0..304_u32).collect::<Vec<_>>()
    );
    assert!(
        output
            .records
            .iter()
            .all(|record| record.source_action_index == record.event_index)
    );
}

#[test]
fn trace_sequence_is_distinct_from_down_chord_identity() {
    let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 4);
    let first = record_with_down_chord_index(12, TRACE_KIND_UP, 1, 1, Some(37));
    let second = record_with_down_chord_index(13, TRACE_KIND_UP, 1, 1, Some(41));
    let zero_chord = record_with_down_chord_index(14, TRACE_KIND_UP, 1, 1, Some(0));
    collector
        .try_push(|| Ok(first))
        .expect("first trace record");
    collector
        .try_push(|| Ok(second))
        .expect("second trace record");
    collector
        .try_push(|| Ok(zero_chord))
        .expect("zero-valued authored Down chord id");

    let records = collector.output.records;
    assert_eq!(records[0].trace_record_index, 0);
    assert_eq!(records[0].authored_down_chord_index, 37);
    assert!(records[0].authored_down_chord_index_available);
    assert_eq!(records[1].trace_record_index, 1);
    assert_eq!(records[1].authored_down_chord_index, 41);
    assert!(records[1].authored_down_chord_index_available);
    assert_eq!(records[2].trace_record_index, 2);
    assert_eq!(records[2].authored_down_chord_index, 0);
    assert!(records[2].authored_down_chord_index_available);
}

#[test]
fn telemetry_sets_truncated_only_when_capacity_is_exceeded() {
    let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 304);

    for event_index in 0..305_u32 {
        collector
            .try_push(|| Ok(record_with_index(event_index, TRACE_KIND_UP, 1, 1)))
            .expect("telemetry record must be representable");
    }

    assert_eq!(collector.output.attempted, 305);
    assert_eq!(collector.output.accepted, 304);
    assert_eq!(collector.output.dropped, 1);
    assert!(collector.output.truncated);
    assert_eq!(
        collector
            .output
            .records
            .back()
            .map(|record| record.event_index),
        Some(303)
    );
    assert!(collector.output.truncated);
}

#[test]
fn native_telemetry_schema_27_serialized_key_sets_are_pinned() {
    let expected: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/native-telemetry-schema-27-keys.json"
    ))
    .expect("schema-27 key fixture");
    let record = record_with_index(7, TRACE_KIND_UP, 1, 1);
    let mut output = super::NativeTelemetryOutput::new(TelemetryMode::Off, 0);
    output.records.push_back(record);
    output.summary.observe(&record);

    let value = serde_json::to_value(output).expect("serialize current telemetry output");
    assert_eq!(value["schema_version"], 27);
    for (value, expected_key) in [
        (&value, "NativeTelemetryOutput"),
        (&value["summary"], "NativeTelemetrySummary"),
        (&value["timing_semantics"], "TimingSemantics"),
        (&value["records"][0], "RtTraceRecord"),
    ] {
        let mut actual_keys = value
            .as_object()
            .expect("serialized object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        actual_keys.sort_unstable();
        let mut expected_keys = expected[expected_key]
            .as_array()
            .expect("fixture key array")
            .iter()
            .map(|key| key.as_str().expect("fixture string").to_string())
            .collect::<Vec<_>>();
        expected_keys.sort_unstable();
        assert_eq!(
            actual_keys, expected_keys,
            "serialized {expected_key} fields"
        );
    }
    assert_eq!(value["timing_semantics"]["game_observed_available"], false);
    assert_eq!(
        value["timing_semantics"]["sender_completion_boundary"],
        "sendinput_completion_qpc"
    );
}

#[test]
fn schema_27_missing_completion_keeps_attempt_and_error_without_timing_envelope() {
    let record = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 41,
            source_action_index: 41,
            authored_down_chord_index: Some(9),
            kind: TRACE_KIND_UP,
            outcome: trace_outcome_code("failed_note_off"),
            polyphony: 1,
            flags: super::TRACE_FLAG_ANOMALY,
            send_status: super::TRACE_SEND_STATUS_COMPLETE,
            win32_error: 0,
            up_mask: 0x0001,
            down_mask: 0,
        },
        TraceTiming {
            authored_ticks: TimelineTicks::from_raw(80),
            effective_deadline_ticks: TimelineTicks::from_raw(90),
            physical_target_qpc_ticks: Some(500),
            pre_call_qpc_ticks: Some(510),
            sendinput_completion_qpc_ticks: None,
            pre_call_ticks: Some(TimelineTicks::from_raw(100)),
            sendinput_completion_ticks: None,
            ..TraceTiming::default()
        },
        TraceDelivery {
            requested: 1,
            sent: 1,
            skipped: 0,
            send_attempts: 1,
        },
    )
    .expect("representable missing-completion trace");
    let mut output = super::NativeTelemetryOutput::new(TelemetryMode::Ring, 1);
    output.records.push_back(record);
    output.attempted = 1;
    output.accepted = 1;
    output.summary.observe(&record);

    let value = serde_json::to_value(output).expect("serialize attempted anomaly evidence");
    let trace = &value["records"][0];
    assert_eq!(value["schema_version"], 27);
    assert_eq!(trace["source_action_index"], 41);
    assert_eq!(trace["authored_down_chord_index"], 9);
    assert_eq!(trace["up_mask"], 1);
    assert_eq!(trace["down_mask"], 0);
    assert_eq!(trace["send_status"], super::TRACE_SEND_STATUS_COMPLETE);
    assert_eq!(trace["requested_count"], 1);
    assert_eq!(trace["sent_count"], 1);
    assert_eq!(trace["send_attempts"], 1);
    assert_eq!(trace["win32_error"], 0);
    assert_ne!(trace["flags"].as_u64().expect("flags") & 0b1000, 0);
    assert_eq!(trace["outcome"], trace_outcome_code("failed_note_off"));
    assert_eq!(trace["pre_call_qpc_ticks"], 510);
    assert_eq!(trace["pre_call_qpc_provenance"], "measured");
    assert_eq!(trace["sendinput_completion_qpc_ticks"], 0);
    assert_eq!(trace["sendinput_completion_qpc_available"], false);
    assert_eq!(trace["sendinput_completion_qpc_provenance"], "unavailable");
    assert_eq!(trace["send_completed_ticks"], 0);
    assert_eq!(trace["send_completed_ticks_provenance"], "unavailable");
    assert_eq!(trace["sendinput_envelope_available"], false);
    assert_eq!(value["summary"]["sender_envelope_samples"], 0);
    assert_eq!(value["summary"]["transport_anomaly_count"], 0);
}

#[test]
fn schema_25_reversed_sender_qpc_is_rejected_instead_of_clamped_or_reconstructed() {
    let result = RtTraceRecord::dispatched(
        TraceContext {
            event_index: 42,
            source_action_index: 42,
            authored_down_chord_index: None,
            kind: TRACE_KIND_UP,
            outcome: trace_outcome_code("failed_note_off"),
            polyphony: 1,
            flags: super::TRACE_FLAG_ANOMALY,
            send_status: super::TRACE_SEND_STATUS_COMPLETE,
            win32_error: 0,
            up_mask: 1,
            down_mask: 0,
        },
        TraceTiming {
            pre_call_qpc_ticks: Some(510),
            sendinput_completion_qpc_ticks: Some(509),
            pre_call_ticks: Some(TimelineTicks::from_raw(100)),
            sendinput_completion_ticks: Some(TimelineTicks::from_raw(99)),
            ..TraceTiming::default()
        },
        TraceDelivery {
            requested: 1,
            sent: 1,
            skipped: 0,
            send_attempts: 1,
        },
    );
    assert!(
        result.is_err(),
        "reversed QPC must not become a zero duration"
    );
}
