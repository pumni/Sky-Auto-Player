pub(crate) mod metrics;

pub use metrics::WorkerMetricsLocal;
pub(crate) use metrics::{SharedMetrics, cpu_metrics_sample_due, try_publish_metrics};

use sky_dispatch_core::time::{DurationTicks, TimeArithmeticError, TimelineTicks};
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
    pub event_index: u32,
    pub kind: u8,
    pub outcome: u8,
    pub polyphony: u8,
    pub flags: u8,
    pub authored_ticks: u64,
    pub effective_deadline_ticks: u64,
    pub wake_ticks: u64,
    pub send_started_ticks: u64,
    pub send_completed_ticks: u64,
    pub bookkeeping_duration_us: u64,
    pub completion_error_ticks: i64,
    pub authored_completion_error_ticks: i64,
    pub applied_lead_ticks: u32,
    pub win32_error: u32,
    pub requested_count: u8,
    pub sent_count: u8,
    pub skipped_count: u8,
    pub send_attempts: u8,
}

pub const NATIVE_TELEMETRY_SCHEMA_VERSION: u32 = 7;

pub(crate) const TRACE_KIND_DOWN: u8 = 0;
pub(crate) const TRACE_KIND_UP: u8 = 1;
pub(crate) const TRACE_FLAG_SENT_FULL: u8 = 1 << 0;
pub(crate) const TRACE_FLAG_RECOVERY: u8 = 1 << 1;
pub(crate) const TRACE_FLAG_DEFERRED: u8 = 1 << 2;
pub(crate) const TRACE_FLAG_ANOMALY: u8 = 1 << 3;

#[derive(Debug, Clone, Copy)]
pub(crate) struct TraceTiming {
    pub(crate) authored_ticks: TimelineTicks,
    pub(crate) effective_deadline_ticks: TimelineTicks,
    pub(crate) wake_ticks: TimelineTicks,
    pub(crate) send_started_ticks: Option<TimelineTicks>,
    pub(crate) send_completed_ticks: Option<TimelineTicks>,
    pub(crate) bookkeeping_duration_us: u64,
    pub(crate) completion_error_ticks: i64,
    pub(crate) authored_completion_error_ticks: i64,
    pub(crate) applied_lead_ticks: DurationTicks,
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
    pub(crate) kind: u8,
    pub(crate) outcome: u8,
    pub(crate) polyphony: usize,
    pub(crate) flags: u8,
    pub(crate) win32_error: u32,
}

impl RtTraceRecord {
    pub(crate) fn dispatched(
        context: TraceContext,
        timing: TraceTiming,
        delivery: TraceDelivery,
    ) -> Result<Self, TimeArithmeticError> {
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
        let applied_lead_ticks = u32::try_from(timing.applied_lead_ticks.as_u64())
            .map_err(|_| TimeArithmeticError::Overflow)?;

        Ok(Self {
            event_index: context.event_index,
            kind: context.kind,
            outcome: context.outcome,
            polyphony,
            flags: context.flags,
            authored_ticks: timing.authored_ticks.as_u64(),
            effective_deadline_ticks: timing.effective_deadline_ticks.as_u64(),
            wake_ticks: timing.wake_ticks.as_u64(),
            send_started_ticks: timing.send_started_ticks.map_or(0, TimelineTicks::as_u64),
            send_completed_ticks: timing.send_completed_ticks.map_or(0, TimelineTicks::as_u64),
            bookkeeping_duration_us: timing.bookkeeping_duration_us,
            completion_error_ticks: timing.completion_error_ticks,
            authored_completion_error_ticks: timing.authored_completion_error_ticks,
            applied_lead_ticks,
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
    pub scheduled_boundary: &'static str,
    pub wake_boundary: &'static str,
    pub sender_start_boundary: &'static str,
    pub sender_completion_boundary: &'static str,
    pub game_observed_available: bool,
}

impl Default for TimingSemantics {
    fn default() -> Self {
        Self {
            evidence_kind: "sender_completion",
            scheduled_boundary: "authored_timeline",
            wake_boundary: "worker_wake_before_sendinput",
            sender_start_boundary: "sendinput_call_entry",
            sender_completion_boundary: "sendinput_call_return",
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
    pub truncated: bool,
    pub timing_semantics: TimingSemantics,
}

impl NativeTelemetryOutput {
    pub(crate) fn new(mode: TelemetryMode, capacity: usize) -> Self {
        Self {
            schema_version: NATIVE_TELEMETRY_SCHEMA_VERSION,
            qpc_frequency_hz: 0,
            records: if matches!(mode, TelemetryMode::Ring) {
                // Reserve the complete bounded buffer before the worker
                // epoch. Telemetry is an opt-in diagnostic mode; once it is
                // enabled, a later VecDeque growth/copy on the dispatch thread is
                // a worse failure mode than its predictable memory cost.
                VecDeque::with_capacity(capacity)
            } else {
                VecDeque::new()
            },
            summary: NativeTelemetrySummary::default(),
            attempted: 0,
            accepted: 0,
            dropped: 0,
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

        let record = build()?;
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
