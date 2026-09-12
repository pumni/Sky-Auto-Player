pub(crate) mod metrics;

pub use metrics::WorkerMetricsLocal;
pub(crate) use metrics::{
    SharedMetrics, cpu_metrics_sample_due, publish_terminal_metrics, try_publish_metrics,
};

use sky_dispatch_core::time::{TimeArithmeticError, TimelineTicks};
use sky_dispatch_win32::input::SendTransactionStatus;
use std::collections::VecDeque;

/// Fixed-size record retained on the real-time worker path.
///
/// Human-readable outcome names, authored reasons, scan-code lists and
/// microsecond projections are deliberately materialized only after the
/// worker has stopped.  This is the only representation kept in the bounded
/// native ring.
#[repr(C)]
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct RtTraceRecord {
    /// Zero-based sequence in the exported session trace. This is not a
    /// compiled schedule packet identity.
    pub trace_record_index: u32,
    /// Compiled schedule packet identity when the observation came from an
    /// authored packet. Zero is a valid packet index, so consumers must check
    /// the availability field.
    pub compiled_packet_index: u64,
    pub compiled_packet_index_available: bool,
    /// Identity in the compiled source schedule. This remains equal to
    /// `event_index` today; it is explicit in schema 14 to make that contract
    /// reviewable by trace consumers.
    pub source_action_index: u32,
    pub event_index: u32,
    pub kind: u8,
    pub outcome: u8,
    pub polyphony: u8,
    pub flags: u8,
    pub send_status: u8,
    pub up_mask: u16,
    pub down_mask: u16,
    pub authored_ticks: u64,
    pub effective_deadline_ticks: u64,
    pub wake_ticks: u64,
    /// Raw QPC values must be interpreted only when their paired availability
    /// field is true; zero is a valid QPC value.
    pub physical_target_qpc_ticks: u64,
    pub physical_target_qpc_available: bool,
    pub pre_call_qpc_ticks: u64,
    pub pre_call_qpc_available: bool,
    pub sendinput_completion_qpc_ticks: u64,
    pub sendinput_completion_qpc_available: bool,
    pub observation_qpc_ticks: u64,
    pub observation_qpc_available: bool,
    /// Deprecated compatibility key. Its value is the trusted pre-call QPC
    /// boundary immediately before the prepared SendInput call.
    pub send_started_ticks: u64,
    /// Deprecated compatibility key for the SendInput return QPC boundary.
    pub send_completed_ticks: u64,
    /// Legacy completion-target residual, retained for diagnostics only.
    /// The primary timing evidence is `dispatch_start_error_ticks`.
    /// Deprecated compatibility key for completion residual diagnostics.
    pub dispatch_cost_us: u64,
    pub core_post_send_duration_us: u64,
    pub post_send_metrics_available: bool,
    pub dispatch_start_error_ticks: i64,
    pub completion_error_ticks: i64,
    pub authored_completion_error_ticks: i64,
    /// Historical compatibility field. Adaptive lead is no longer part of
    /// runtime evidence and this publication adapter always emits zero.
    pub applied_lead_ticks: u32,
    pub win32_error: u32,
    pub requested_count: u8,
    pub sent_count: u8,
    pub skipped_count: u8,
    pub send_attempts: u8,
}

pub const NATIVE_TELEMETRY_SCHEMA_VERSION: u32 = 14;

pub(crate) const TRACE_KIND_DOWN: u8 = 0;
pub(crate) const TRACE_KIND_UP: u8 = 1;
pub(crate) const TRACE_KIND_MIXED: u8 = 2;
pub(crate) const TRACE_FLAG_SENT_FULL: u8 = 1 << 0;
pub(crate) const TRACE_FLAG_RECOVERY: u8 = 1 << 1;
pub(crate) const TRACE_FLAG_DEFERRED: u8 = 1 << 2;
pub(crate) const TRACE_FLAG_ANOMALY: u8 = 1 << 3;

pub(crate) const TRACE_SEND_STATUS_COMPLETE: u8 = 0;
pub(crate) const TRACE_SEND_STATUS_PREPARATION_REJECTED: u8 = 1;
pub(crate) const TRACE_SEND_STATUS_ZERO_PROGRESS: u8 = 2;
pub(crate) const TRACE_SEND_STATUS_PARTIAL_PROGRESS: u8 = 3;
pub(crate) const TRACE_SEND_STATUS_INTEGRITY_LOST: u8 = 4;
pub(crate) const TRACE_SEND_STATUS_DEADLINE_MISSED: u8 = 5;
pub(crate) const TRACE_SEND_STATUS_CLOCK_FAILURE_BEFORE_SEND: u8 = 6;
pub(crate) const TRACE_SEND_STATUS_CLOCK_FAILURE_AFTER_SEND: u8 = 7;
pub(crate) const TRACE_SEND_STATUS_NOT_ATTEMPTED: u8 = 8;

pub(crate) const fn trace_send_status_code(status: SendTransactionStatus) -> u8 {
    match status {
        SendTransactionStatus::Complete => TRACE_SEND_STATUS_COMPLETE,
        SendTransactionStatus::PreparationRejected => TRACE_SEND_STATUS_PREPARATION_REJECTED,
        SendTransactionStatus::ZeroProgress => TRACE_SEND_STATUS_ZERO_PROGRESS,
        SendTransactionStatus::PartialProgress => TRACE_SEND_STATUS_PARTIAL_PROGRESS,
        SendTransactionStatus::IntegrityLost => TRACE_SEND_STATUS_INTEGRITY_LOST,
        SendTransactionStatus::DeadlineMissedBeforeSend => TRACE_SEND_STATUS_DEADLINE_MISSED,
        SendTransactionStatus::ClockFailureBeforeSend => {
            TRACE_SEND_STATUS_CLOCK_FAILURE_BEFORE_SEND
        }
        SendTransactionStatus::ClockFailureAfterSend => TRACE_SEND_STATUS_CLOCK_FAILURE_AFTER_SEND,
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TraceTiming {
    pub(crate) authored_ticks: TimelineTicks,
    pub(crate) effective_deadline_ticks: TimelineTicks,
    pub(crate) wake_ticks: TimelineTicks,
    pub(crate) physical_target_qpc_ticks: Option<u64>,
    pub(crate) pre_call_qpc_ticks: Option<u64>,
    pub(crate) sendinput_completion_qpc_ticks: Option<u64>,
    pub(crate) observation_qpc_ticks: Option<u64>,
    /// Final target/focus/control proof, sampled before the pre-call sender
    /// boundary.
    pub(crate) final_policy_ticks: Option<TimelineTicks>,
    /// Compatibility output `send_started_ticks` is sourced from this
    /// prepared-sender pre-call boundary.
    pub(crate) pre_call_ticks: Option<TimelineTicks>,
    /// Compatibility output `send_completed_ticks` is the SendInput return
    /// boundary.
    pub(crate) sendinput_completion_ticks: Option<TimelineTicks>,
    pub(crate) completion_residual_us: u64,
    pub(crate) core_post_send_duration_us: u64,
    pub(crate) post_send_metrics_available: bool,
    pub(crate) dispatch_start_error_ticks: i64,
    pub(crate) completion_error_ticks: i64,
    pub(crate) authored_completion_error_ticks: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TraceDelivery {
    pub(crate) requested: usize,
    pub(crate) sent: usize,
    pub(crate) skipped: usize,
    pub(crate) send_attempts: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct TraceContext {
    pub(crate) event_index: u32,
    pub(crate) source_action_index: u32,
    pub(crate) compiled_packet_index: Option<u64>,
    pub(crate) kind: u8,
    pub(crate) outcome: u8,
    pub(crate) polyphony: usize,
    pub(crate) flags: u8,
    pub(crate) send_status: u8,
    pub(crate) win32_error: u32,
    pub(crate) up_mask: u16,
    pub(crate) down_mask: u16,
}

impl RtTraceRecord {
    pub(crate) fn dispatched(
        context: TraceContext,
        timing: TraceTiming,
        delivery: TraceDelivery,
    ) -> Result<Self, TimeArithmeticError> {
        let _final_policy_ticks = timing.final_policy_ticks;
        if delivery.sent > delivery.requested
            || delivery.skipped > delivery.requested
            || delivery.sent.saturating_add(delivery.skipped) > delivery.requested
            || delivery.requested > context.polyphony
        {
            return Err(TimeArithmeticError::Overflow);
        }
        let polyphony =
            u8::try_from(context.polyphony).map_err(|_| TimeArithmeticError::Overflow)?;
        let requested_count =
            u8::try_from(delivery.requested).map_err(|_| TimeArithmeticError::Overflow)?;
        let sent_count = u8::try_from(delivery.sent).map_err(|_| TimeArithmeticError::Overflow)?;
        let skipped_count =
            u8::try_from(delivery.skipped).map_err(|_| TimeArithmeticError::Overflow)?;
        let send_attempts =
            u8::try_from(delivery.send_attempts).map_err(|_| TimeArithmeticError::Overflow)?;
        Ok(Self {
            trace_record_index: 0,
            compiled_packet_index: context.compiled_packet_index.unwrap_or_default(),
            compiled_packet_index_available: context.compiled_packet_index.is_some(),
            source_action_index: context.source_action_index,
            event_index: context.event_index,
            kind: context.kind,
            outcome: context.outcome,
            polyphony,
            flags: context.flags,
            send_status: context.send_status,
            up_mask: context.up_mask,
            down_mask: context.down_mask,
            authored_ticks: timing.authored_ticks.as_u64(),
            effective_deadline_ticks: timing.effective_deadline_ticks.as_u64(),
            wake_ticks: timing.wake_ticks.as_u64(),
            physical_target_qpc_ticks: timing.physical_target_qpc_ticks.unwrap_or_default(),
            physical_target_qpc_available: timing.physical_target_qpc_ticks.is_some(),
            pre_call_qpc_ticks: timing.pre_call_qpc_ticks.unwrap_or_default(),
            pre_call_qpc_available: timing.pre_call_qpc_ticks.is_some(),
            sendinput_completion_qpc_ticks: timing
                .sendinput_completion_qpc_ticks
                .unwrap_or_default(),
            sendinput_completion_qpc_available: timing.sendinput_completion_qpc_ticks.is_some(),
            observation_qpc_ticks: timing.observation_qpc_ticks.unwrap_or_default(),
            observation_qpc_available: timing.observation_qpc_ticks.is_some(),
            send_started_ticks: timing.pre_call_ticks.map_or(0, TimelineTicks::as_u64),
            send_completed_ticks: timing
                .sendinput_completion_ticks
                .map_or(0, TimelineTicks::as_u64),
            dispatch_cost_us: timing.completion_residual_us,
            core_post_send_duration_us: timing.core_post_send_duration_us,
            post_send_metrics_available: timing.post_send_metrics_available,
            dispatch_start_error_ticks: timing.dispatch_start_error_ticks,
            completion_error_ticks: timing.completion_error_ticks,
            authored_completion_error_ticks: timing.authored_completion_error_ticks,
            applied_lead_ticks: 0,
            win32_error: context.win32_error,
            requested_count,
            sent_count,
            skipped_count,
            send_attempts,
        })
    }
}

pub(crate) fn trace_outcome_code(outcome: &str) -> u8 {
    match outcome {
        "sent" => 0,
        "deferred_release" => 1,
        "failed_note_off" => 2,
        "blocked_unfocused" => 3,
        "suppressed_stale_up" => 4,
        "recovered_zero_progress_but_late" => 5,
        "strict_completion_slo_exceeded" => 6,
        "chord_integrity_lost" => 7,
        "aborted" => 8,
        "down_cutoff_miss" => 9,
        "down_backlog_miss" => 10,
        _ => 255,
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct NativeTelemetrySummary {
    pub dispatch_count: u64,
    pub down_count: u64,
    pub up_count: u64,
    pub requested_key_count: u64,
    pub sent_key_count: u64,
    pub skipped_key_count: u64,
    pub max_dispatch_start_error_ticks: i64,
    pub max_dispatch_start_error_abs_ticks: u64,
    pub max_lateness_us: i64,
    pub max_send_duration_us: u64,
    pub lateness_histogram_50us: [u64; 16],
    pub send_duration_histogram_50us: [u64; 16],
    /// Values at or above the final finite histogram bucket. The last array
    /// slot is retained for compatibility; this counter makes the tail
    /// explicit instead of pretending it is a narrow 750–800 µs bucket.
    pub lateness_overflow_count: u64,
    pub send_duration_overflow_count: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TimingSemantics {
    pub evidence_kind: &'static str,
    pub primary_metric: &'static str,
    pub completion_metric: &'static str,
    pub scheduled_boundary: &'static str,
    pub wake_boundary: &'static str,
    pub sender_start_boundary: &'static str,
    pub sender_completion_boundary: &'static str,
    pub game_observed_available: bool,
}

impl Default for TimingSemantics {
    fn default() -> Self {
        Self {
            evidence_kind: "sender_start_error",
            primary_metric: "dispatch_start_error_ticks",
            completion_metric: "completion_error_ticks_diagnostic_only",
            scheduled_boundary: "authored_timeline",
            wake_boundary: "worker_wake_before_sendinput",
            sender_start_boundary: "pre_call_qpc_before_sendinput",
            sender_completion_boundary: "sendinput_completion_qpc",
            game_observed_available: false,
        }
    }
}

impl NativeTelemetrySummary {
    pub(crate) fn observe(&mut self, record: &RtTraceRecord) {
        let backend_dispatch =
            record.send_attempts > 0 || record.sent_count > 0 || record.skipped_count > 0;
        if !backend_dispatch {
            return;
        }
        let first_dispatch = self.dispatch_count == 0;
        self.dispatch_count = self.dispatch_count.saturating_add(1);
        match record.kind {
            TRACE_KIND_DOWN => self.down_count = self.down_count.saturating_add(1),
            TRACE_KIND_UP => self.up_count = self.up_count.saturating_add(1),
            _ => {}
        }
        self.requested_key_count = self
            .requested_key_count
            .saturating_add(u64::from(record.requested_count));
        self.sent_key_count = self
            .sent_key_count
            .saturating_add(u64::from(record.sent_count));
        self.skipped_key_count = self
            .skipped_key_count
            .saturating_add(u64::from(record.skipped_count));
        if first_dispatch {
            self.max_dispatch_start_error_ticks = record.dispatch_start_error_ticks;
        } else {
            self.max_dispatch_start_error_ticks = self
                .max_dispatch_start_error_ticks
                .max(record.dispatch_start_error_ticks);
        }
        self.max_dispatch_start_error_abs_ticks = self
            .max_dispatch_start_error_abs_ticks
            .max(record.dispatch_start_error_ticks.unsigned_abs());
    }
}

#[derive(Debug, Default, serde::Serialize)]
pub struct NativeTelemetryOutput {
    pub schema_version: u32,
    pub qpc_frequency_hz: u64,
    pub records: VecDeque<RtTraceRecord>,
    pub summary: NativeTelemetrySummary,
    pub attempted: u64,
    pub accepted: u64,
    pub dropped: u64,
    /// Observations dropped by the bounded producer queue before telemetry
    /// could inspect them. Kept separate from ring-capacity drops.
    pub observer_queue_dropped: u64,
    pub truncated: bool,
    pub timing_semantics: TimingSemantics,
}

impl NativeTelemetryOutput {
    pub(crate) fn new(mode: TelemetryMode, capacity: usize) -> Self {
        Self {
            schema_version: NATIVE_TELEMETRY_SCHEMA_VERSION,
            qpc_frequency_hz: 0,
            records: if matches!(mode, TelemetryMode::Ring) {
                // Reserve the complete bounded buffer at construction time so no
                // heap allocation occurs on the first push_back during dispatch.
                let mut records = VecDeque::with_capacity(capacity);
                records.reserve_exact(capacity);
                records
            } else {
                VecDeque::new()
            },
            summary: NativeTelemetrySummary::default(),
            attempted: 0,
            accepted: 0,
            dropped: 0,
            observer_queue_dropped: 0,
            truncated: false,
            timing_semantics: TimingSemantics::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TelemetryMode {
    Off,
    Ring,
}

#[derive(Debug)]
pub(crate) struct TelemetryCollector {
    pub(crate) mode: TelemetryMode,
    pub(crate) capacity: usize,
    pub(crate) output: NativeTelemetryOutput,
}

impl TelemetryCollector {
    pub(crate) fn new(mode: TelemetryMode, capacity: usize) -> Self {
        Self {
            mode,
            capacity,
            output: NativeTelemetryOutput::new(mode, capacity),
        }
    }

    pub(crate) fn try_push<F>(&mut self, build: F) -> Result<(), TimeArithmeticError>
    where
        F: FnOnce() -> Result<RtTraceRecord, TimeArithmeticError>,
    {
        self.output.attempted = self.output.attempted.saturating_add(1);
        if self.mode == TelemetryMode::Off {
            return Ok(());
        }

        if self.output.records.len() == self.capacity {
            self.output.dropped = self.output.dropped.saturating_add(1);
            self.output.truncated = true;
            return Ok(());
        }

        let mut record = build()?;
        record.trace_record_index = u32::try_from(self.output.accepted).unwrap_or(u32::MAX);
        self.output.summary.observe(&record);

        match self.mode {
            TelemetryMode::Off => unreachable!(),
            TelemetryMode::Ring => {
                self.output.records.push_back(record);
                self.output.accepted = self.output.accepted.saturating_add(1);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NativeTelemetrySummary, RtTraceRecord, TRACE_FLAG_SENT_FULL, TRACE_KIND_DOWN,
        TRACE_KIND_MIXED, TRACE_KIND_UP, TRACE_SEND_STATUS_COMPLETE, TelemetryCollector,
        TelemetryMode, TraceContext, TraceDelivery, TraceTiming, trace_outcome_code,
    };
    use sky_dispatch_core::time::TimelineTicks;

    fn record_with_index(
        event_index: u32,
        kind: u8,
        requested: usize,
        sent: usize,
    ) -> RtTraceRecord {
        record_with_packet_index(event_index, kind, requested, sent, None)
    }

    fn record_with_packet_index(
        event_index: u32,
        kind: u8,
        requested: usize,
        sent: usize,
        compiled_packet_index: Option<u64>,
    ) -> RtTraceRecord {
        RtTraceRecord::dispatched(
            TraceContext {
                event_index,
                source_action_index: event_index,
                compiled_packet_index,
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
                } else {
                    (1_u16 << requested) - 1
                },
            },
            TraceTiming {
                authored_ticks: TimelineTicks::ZERO,
                effective_deadline_ticks: TimelineTicks::ZERO,
                wake_ticks: TimelineTicks::ZERO,
                physical_target_qpc_ticks: None,
                pre_call_qpc_ticks: None,
                sendinput_completion_qpc_ticks: None,
                observation_qpc_ticks: None,
                final_policy_ticks: Some(TimelineTicks::from_raw(1)),
                pre_call_ticks: Some(TimelineTicks::from_raw(1)),
                sendinput_completion_ticks: Some(TimelineTicks::from_raw(2)),
                completion_residual_us: 0,
                core_post_send_duration_us: 0,
                post_send_metrics_available: false,
                dispatch_start_error_ticks: 0,
                completion_error_ticks: 0,
                authored_completion_error_ticks: 0,
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
        assert!(!semantics.game_observed_available);
    }

    #[test]
    fn signed_start_error_max_preserves_all_negative_samples() {
        let mut summary = NativeTelemetrySummary::default();

        for (event_index, start_error_ticks) in [(0, -40), (1, -30), (2, -20)] {
            let mut record = record_with_index(event_index, TRACE_KIND_UP, 1, 1);
            record.dispatch_start_error_ticks = start_error_ticks;
            summary.observe(&record);
        }

        assert_eq!(summary.max_dispatch_start_error_ticks, -20);
        assert_eq!(summary.max_dispatch_start_error_abs_ticks, 40);
    }

    #[test]
    fn mixed_record_keeps_mixed_kind_and_physical_count() {
        let record = record(TRACE_KIND_MIXED, 2, 2);
        let mut summary = NativeTelemetrySummary::default();

        summary.observe(&record);

        assert_eq!(record.kind, TRACE_KIND_MIXED);
        assert_eq!(record.up_mask, 1);
        assert_eq!(record.down_mask, 0b11);
        assert_ne!(record.up_mask & record.down_mask, 0, "same-key retrigger");
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
    fn trace_sequence_is_distinct_from_compiled_packet_identity() {
        let mut collector = TelemetryCollector::new(TelemetryMode::Ring, 4);
        let first = record_with_packet_index(12, TRACE_KIND_UP, 1, 1, Some(37));
        let second = record_with_packet_index(13, TRACE_KIND_UP, 1, 1, Some(41));
        let zero_packet = record_with_packet_index(14, TRACE_KIND_UP, 1, 1, Some(0));
        collector
            .try_push(|| Ok(first))
            .expect("first trace record");
        collector
            .try_push(|| Ok(second))
            .expect("second trace record");
        collector
            .try_push(|| Ok(zero_packet))
            .expect("zero-valued compiled packet id");

        let records = collector.output.records;
        assert_eq!(records[0].trace_record_index, 0);
        assert_eq!(records[0].compiled_packet_index, 37);
        assert!(records[0].compiled_packet_index_available);
        assert_eq!(records[1].trace_record_index, 1);
        assert_eq!(records[1].compiled_packet_index, 41);
        assert!(records[1].compiled_packet_index_available);
        assert_eq!(records[2].trace_record_index, 2);
        assert_eq!(records[2].compiled_packet_index, 0);
        assert!(records[2].compiled_packet_index_available);
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
}
