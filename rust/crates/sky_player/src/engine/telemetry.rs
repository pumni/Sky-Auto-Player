pub(crate) mod metrics;

pub use metrics::WorkerMetricsLocal;
pub(crate) use metrics::{
    SharedMetrics, cpu_metrics_sample_due, publish_lifecycle_metrics, publish_terminal_metrics,
    try_publish_metrics, try_publish_observer_metrics,
};

use sky_dispatch_core::time::{TimeArithmeticError, TimelineTicks};
use sky_dispatch_win32::input::SendTransactionStatus;
use std::collections::VecDeque;

/// Provenance assigned to every exported QPC observation.
#[repr(u8)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampProvenance {
    Measured = 0,
    DerivedFromPreCall = 1,
    #[default]
    Unavailable = 2,
    DerivedFromWakeQpc = 3,
    DerivedFromSendInputCompletion = 4,
}

/// Fixed-size record retained on the real-time worker path.
///
/// Human-readable outcome names, authored reasons, scan-code lists and
/// microsecond projections are deliberately materialized only after the
/// worker has stopped.  This is the only representation kept in the bounded
/// native ring.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct RtTraceRecord {
    pub compiled_packet_index: u64,
    /// Authored and effective positions relative to the immutable playback
    /// epoch. These are schedule offsets, not sampled wall-clock timestamps.
    pub authored_offset_ticks: u64,
    pub effective_deadline_offset_ticks: u64,
    pub wake_offset_ticks: u64,
    /// Absolute QPC values below are present only when an actual sample exists.
    pub wake_qpc_ticks: u64,
    pub target_crossing_qpc_ticks: u64,
    pub final_policy_qpc_ticks: u64,
    pub pre_call_qpc_ticks: u64,
    pub sendinput_completion_qpc_ticks: u64,
    pub post_send_qpc_ticks: u64,
    pub observation_qpc_ticks: u64,
    /// Deprecated timeline-offset compatibility key converted from the
    /// sampled pre-call QPC and immutable epoch.
    pub send_started_ticks: u64,
    /// Deprecated timeline-offset compatibility key converted from the
    /// sampled SendInput completion QPC and immutable epoch.
    pub send_completed_ticks: u64,
    pub authored_lateness_ticks: i64,
    pub physical_floor_displacement_ticks: u64,
    pub hold_floor_displacement_ticks: u64,
    pub release_floor_displacement_ticks: u64,
    pub other_floor_displacement_ticks: u64,
    pub residual_lateness_ticks: i64,
    pub hol_delay_lower_bound_ticks: u64,
    pub unattributed_lateness_ticks: u64,
    pub sendinput_envelope_ticks: u64,
    /// Deprecated compatibility completion residual in microseconds.
    pub dispatch_cost_us: u64,
    pub core_post_send_duration_us: u64,
    /// Legacy start-error alias; `authored_lateness_ticks` is authoritative.
    pub dispatch_start_error_ticks: i64,
    pub completion_error_ticks: i64,
    pub authored_completion_error_ticks: i64,
    /// Zero-based sequence in the exported session trace. This is not a
    /// compiled schedule packet identity.
    pub trace_record_index: u32,
    /// Identity in the compiled source schedule. This remains equal to
    /// `event_index` today; it is explicit in schema 14 to make that contract
    /// reviewable by trace consumers.
    pub source_action_index: u32,
    pub event_index: u32,
    /// Historical compatibility field. Adaptive lead is no longer part of
    /// runtime evidence and this publication adapter always emits zero.
    pub applied_lead_ticks: u32,
    pub win32_error: u32,
    pub up_mask: u16,
    pub down_mask: u16,
    pub hold_floor_mask: u16,
    pub release_floor_mask: u16,
    /// Availability and provenance distinguish missing QPC samples from valid zero.
    pub compiled_packet_index_available: bool,
    pub wake_offset_available: bool,
    pub wake_qpc_available: bool,
    pub target_crossing_qpc_available: bool,
    pub final_policy_qpc_available: bool,
    pub pre_call_qpc_available: bool,
    pub sendinput_completion_qpc_available: bool,
    pub post_send_qpc_available: bool,
    pub observation_qpc_available: bool,
    pub authored_lateness_available: bool,
    pub authored_target_overdue: bool,
    pub floor_displacement_available: bool,
    pub residual_lateness_available: bool,
    pub hol_delay_available: bool,
    pub unattributed_lateness_available: bool,
    pub sendinput_envelope_available: bool,
    pub transport_anomaly: bool,
    pub wake_offset_provenance: TimestampProvenance,
    pub wake_qpc_provenance: TimestampProvenance,
    pub target_crossing_qpc_provenance: TimestampProvenance,
    pub final_policy_qpc_provenance: TimestampProvenance,
    pub pre_call_qpc_provenance: TimestampProvenance,
    pub sendinput_completion_qpc_provenance: TimestampProvenance,
    pub post_send_qpc_provenance: TimestampProvenance,
    pub observation_qpc_provenance: TimestampProvenance,
    pub send_started_ticks_provenance: TimestampProvenance,
    pub send_completed_ticks_provenance: TimestampProvenance,
    pub post_send_metrics_available: bool,
    pub kind: u8,
    pub outcome: u8,
    pub polyphony: u8,
    pub flags: u8,
    pub send_status: u8,
    pub requested_count: u8,
    pub sent_count: u8,
    pub skipped_count: u8,
    pub send_attempts: u8,
}

pub const NATIVE_TELEMETRY_SCHEMA_VERSION: u32 = 19;

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
        SendTransactionStatus::ClockFailureBeforeSend => {
            TRACE_SEND_STATUS_CLOCK_FAILURE_BEFORE_SEND
        }
        SendTransactionStatus::ClockFailureAfterSend => TRACE_SEND_STATUS_CLOCK_FAILURE_AFTER_SEND,
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct TraceTiming {
    pub(crate) authored_ticks: TimelineTicks,
    pub(crate) effective_deadline_ticks: TimelineTicks,
    pub(crate) wake_ticks: Option<TimelineTicks>,
    pub(crate) wake_qpc_ticks: Option<u64>,
    pub(crate) target_crossing_qpc_ticks: Option<u64>,
    pub(crate) physical_target_qpc_ticks: Option<u64>,
    pub(crate) physical_not_before_qpc_ticks: Option<u64>,
    pub(crate) hold_floor_qpc_ticks: Option<u64>,
    pub(crate) release_floor_qpc_ticks: Option<u64>,
    pub(crate) previous_sendinput_completion_qpc_ticks: Option<u64>,
    pub(crate) previous_sender_completion_known: bool,
    pub(crate) hold_floor_mask: u16,
    pub(crate) release_floor_mask: u16,
    pub(crate) final_policy_qpc_ticks: Option<u64>,
    pub(crate) pre_call_qpc_ticks: Option<u64>,
    pub(crate) sendinput_completion_qpc_ticks: Option<u64>,
    pub(crate) post_send_qpc_ticks: Option<u64>,
    pub(crate) observation_qpc_ticks: Option<u64>,
    /// Compatibility output `send_started_ticks` is sourced from this
    /// prepared-sender pre-call boundary.
    pub(crate) pre_call_ticks: Option<TimelineTicks>,
    /// Compatibility output `send_completed_ticks` is the SendInput return
    /// boundary.
    pub(crate) sendinput_completion_ticks: Option<TimelineTicks>,
    pub(crate) completion_residual_us: u64,
    pub(crate) core_post_send_duration_us: u64,
    pub(crate) post_send_metrics_available: bool,
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
        let physical_floor_displacement_ticks = checked_displacement(
            timing.physical_not_before_qpc_ticks,
            timing.physical_target_qpc_ticks,
        )?;
        let hold_floor_displacement_ticks = checked_displacement(
            timing.hold_floor_qpc_ticks,
            timing.physical_target_qpc_ticks,
        )?;
        let release_floor_displacement_ticks = checked_displacement(
            timing.release_floor_qpc_ticks,
            timing.physical_target_qpc_ticks,
        )?;
        let other_floor_displacement_ticks = match (
            timing.physical_not_before_qpc_ticks,
            timing.physical_target_qpc_ticks,
        ) {
            (Some(not_before), Some(target)) => {
                let known_floor = target
                    .max(timing.hold_floor_qpc_ticks.unwrap_or(target))
                    .max(timing.release_floor_qpc_ticks.unwrap_or(target));
                Some(
                    not_before
                        .checked_sub(known_floor)
                        .ok_or(TimeArithmeticError::Overflow)?,
                )
            }
            _ => None,
        };
        let residual_lateness_ticks = checked_signed_delta(
            timing.pre_call_qpc_ticks,
            timing.physical_not_before_qpc_ticks,
        )?;
        let hol_delay_lower_bound_ticks = match (
            timing.pre_call_qpc_ticks,
            timing.physical_not_before_qpc_ticks,
            timing.previous_sendinput_completion_qpc_ticks,
            timing.previous_sender_completion_known,
        ) {
            (Some(pre_call), Some(not_before), previous, true) => Some(
                previous
                    .unwrap_or(not_before)
                    .min(pre_call)
                    .saturating_sub(not_before),
            ),
            _ => None,
        };
        let unattributed_lateness_ticks =
            match (residual_lateness_ticks, hol_delay_lower_bound_ticks) {
                (Some(residual), Some(hol)) => Some(
                    residual
                        .saturating_sub(
                            i64::try_from(hol).map_err(|_| TimeArithmeticError::Overflow)?,
                        )
                        .max(0) as u64,
                ),
                _ => None,
            };
        let sendinput_envelope_ticks = match (
            timing.pre_call_qpc_ticks,
            timing.sendinput_completion_qpc_ticks,
        ) {
            (Some(start), Some(completed)) => Some(
                completed
                    .checked_sub(start)
                    .ok_or(TimeArithmeticError::Overflow)?,
            ),
            _ => None,
        };
        let authored_lateness_ticks = checked_signed_delta(
            timing.pre_call_qpc_ticks.or(timing.observation_qpc_ticks),
            timing.physical_target_qpc_ticks,
        )?;
        let wake_offset_ticks = timing.wake_qpc_ticks.and(timing.wake_ticks);
        let send_started_ticks = timing.pre_call_qpc_ticks.and(timing.pre_call_ticks);
        let send_completed_ticks = timing
            .sendinput_completion_qpc_ticks
            .and(timing.sendinput_completion_ticks);
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
            authored_offset_ticks: timing.authored_ticks.as_u64(),
            effective_deadline_offset_ticks: timing.effective_deadline_ticks.as_u64(),
            wake_offset_ticks: wake_offset_ticks.map_or(0, TimelineTicks::as_u64),
            wake_offset_available: wake_offset_ticks.is_some(),
            wake_qpc_ticks: timing.wake_qpc_ticks.unwrap_or_default(),
            wake_qpc_available: timing.wake_qpc_ticks.is_some(),
            target_crossing_qpc_ticks: timing.target_crossing_qpc_ticks.unwrap_or_default(),
            target_crossing_qpc_available: timing.target_crossing_qpc_ticks.is_some(),
            final_policy_qpc_ticks: timing.final_policy_qpc_ticks.unwrap_or_default(),
            final_policy_qpc_available: timing.final_policy_qpc_ticks.is_some(),
            hold_floor_mask: timing.hold_floor_mask,
            release_floor_mask: timing.release_floor_mask,
            pre_call_qpc_ticks: timing.pre_call_qpc_ticks.unwrap_or_default(),
            pre_call_qpc_available: timing.pre_call_qpc_ticks.is_some(),
            sendinput_completion_qpc_ticks: timing
                .sendinput_completion_qpc_ticks
                .unwrap_or_default(),
            sendinput_completion_qpc_available: timing.sendinput_completion_qpc_ticks.is_some(),
            post_send_qpc_ticks: timing.post_send_qpc_ticks.unwrap_or_default(),
            post_send_qpc_available: timing.post_send_qpc_ticks.is_some(),
            observation_qpc_ticks: timing.observation_qpc_ticks.unwrap_or_default(),
            observation_qpc_available: timing.observation_qpc_ticks.is_some(),
            authored_lateness_ticks: authored_lateness_ticks.unwrap_or_default(),
            authored_target_overdue: authored_lateness_ticks.is_some_and(|late| late > 0),
            physical_floor_displacement_ticks: physical_floor_displacement_ticks
                .unwrap_or_default(),
            hold_floor_displacement_ticks: hold_floor_displacement_ticks.unwrap_or_default(),
            release_floor_displacement_ticks: release_floor_displacement_ticks.unwrap_or_default(),
            other_floor_displacement_ticks: other_floor_displacement_ticks.unwrap_or_default(),
            residual_lateness_ticks: residual_lateness_ticks.unwrap_or_default(),
            hol_delay_lower_bound_ticks: hol_delay_lower_bound_ticks.unwrap_or_default(),
            unattributed_lateness_ticks: unattributed_lateness_ticks.unwrap_or_default(),
            sendinput_envelope_ticks: sendinput_envelope_ticks.unwrap_or_default(),
            floor_displacement_available: physical_floor_displacement_ticks.is_some(),
            residual_lateness_available: residual_lateness_ticks.is_some(),
            authored_lateness_available: authored_lateness_ticks.is_some(),
            hol_delay_available: hol_delay_lower_bound_ticks.is_some(),
            unattributed_lateness_available: unattributed_lateness_ticks.is_some(),
            sendinput_envelope_available: sendinput_envelope_ticks.is_some(),
            transport_anomaly: context.send_status != TRACE_SEND_STATUS_NOT_ATTEMPTED
                && (context.send_status != TRACE_SEND_STATUS_COMPLETE
                    || context.win32_error != 0
                    || delivery.skipped != 0
                    || delivery.send_attempts != 1),
            wake_offset_provenance: if wake_offset_ticks.is_some() {
                TimestampProvenance::DerivedFromWakeQpc
            } else {
                TimestampProvenance::Unavailable
            },
            wake_qpc_provenance: provenance(timing.wake_qpc_ticks.is_some()),
            target_crossing_qpc_provenance: provenance(timing.target_crossing_qpc_ticks.is_some()),
            final_policy_qpc_provenance: provenance(timing.final_policy_qpc_ticks.is_some()),
            pre_call_qpc_provenance: provenance(timing.pre_call_qpc_ticks.is_some()),
            sendinput_completion_qpc_provenance: provenance(
                timing.sendinput_completion_qpc_ticks.is_some(),
            ),
            post_send_qpc_provenance: provenance(timing.post_send_qpc_ticks.is_some()),
            observation_qpc_provenance: provenance(timing.observation_qpc_ticks.is_some()),
            send_started_ticks_provenance: if send_started_ticks.is_some() {
                TimestampProvenance::DerivedFromPreCall
            } else {
                TimestampProvenance::Unavailable
            },
            send_completed_ticks_provenance: if send_completed_ticks.is_some() {
                TimestampProvenance::DerivedFromSendInputCompletion
            } else {
                TimestampProvenance::Unavailable
            },
            send_started_ticks: send_started_ticks.map_or(0, TimelineTicks::as_u64),
            send_completed_ticks: send_completed_ticks.map_or(0, TimelineTicks::as_u64),
            dispatch_cost_us: timing.completion_residual_us,
            core_post_send_duration_us: timing.core_post_send_duration_us,
            post_send_metrics_available: timing.post_send_metrics_available,
            dispatch_start_error_ticks: authored_lateness_ticks.unwrap_or_default(),
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

fn provenance(available: bool) -> TimestampProvenance {
    if available {
        TimestampProvenance::Measured
    } else {
        TimestampProvenance::Unavailable
    }
}

fn checked_displacement(
    value: Option<u64>,
    baseline: Option<u64>,
) -> Result<Option<u64>, TimeArithmeticError> {
    match (value, baseline) {
        (Some(value), Some(baseline)) => value
            .checked_sub(baseline)
            .map(Some)
            .ok_or(TimeArithmeticError::Overflow),
        _ => Ok(None),
    }
}

fn checked_signed_delta(
    value: Option<u64>,
    baseline: Option<u64>,
) -> Result<Option<i64>, TimeArithmeticError> {
    match (value, baseline) {
        (Some(value), Some(baseline)) => i64::try_from(i128::from(value) - i128::from(baseline))
            .map(Some)
            .map_err(|_| TimeArithmeticError::Overflow),
        _ => Ok(None),
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
        "down_unobserved_backlog" => 10,
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
    pub authored_lateness_samples: u64,
    pub max_authored_lateness_ticks: i64,
    pub max_authored_lateness_abs_ticks: u64,
    pub overdue_packet_count: u64,
    pub floor_displacement_samples: u64,
    pub max_physical_floor_displacement_ticks: u64,
    pub residual_lateness_samples: u64,
    pub max_residual_lateness_ticks: i64,
    pub max_residual_lateness_abs_ticks: u64,
    pub hol_delay_samples: u64,
    pub max_hol_delay_lower_bound_ticks: u64,
    pub unattributed_lateness_samples: u64,
    pub max_unattributed_lateness_ticks: u64,
    pub sender_envelope_samples: u64,
    pub max_sendinput_envelope_ticks: u64,
    pub transport_anomaly_count: u64,
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
    pub timestamp_provenance: &'static str,
    pub final_policy_boundary: &'static str,
    pub observer_boundary: &'static str,
    pub game_observed_available: bool,
}

impl Default for TimingSemantics {
    fn default() -> Self {
        Self {
            evidence_kind: "sender_pre_call_and_observer_lateness",
            primary_metric: "authored_lateness_ticks",
            completion_metric: "completion_error_ticks_diagnostic_only",
            scheduled_boundary: "authored_timeline",
            wake_boundary: "worker_wake_before_sendinput",
            sender_start_boundary: "pre_call_qpc_before_sendinput",
            sender_completion_boundary: "sendinput_completion_qpc",
            timestamp_provenance: concat!(
                "measured|derived_from_wake_qpc|derived_from_pre_call|",
                "derived_from_send_input_completion|unavailable",
            ),
            final_policy_boundary: "sampled_when_present_else_unavailable",
            observer_boundary: "post_send_capture_and_observer_drain_are_distinct",
            game_observed_available: false,
        }
    }
}

impl NativeTelemetrySummary {
    pub(crate) fn observe(&mut self, record: &RtTraceRecord) {
        let backend_dispatch = record.send_attempts > 0
            || record.sent_count > 0
            || record.skipped_count > 0
            || record.transport_anomaly;
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
        if record.authored_lateness_available {
            let first_lateness_sample = self.authored_lateness_samples == 0;
            self.authored_lateness_samples = self.authored_lateness_samples.saturating_add(1);
            self.max_dispatch_start_error_ticks = if first_lateness_sample {
                record.dispatch_start_error_ticks
            } else {
                self.max_dispatch_start_error_ticks
                    .max(record.dispatch_start_error_ticks)
            };
            self.max_dispatch_start_error_abs_ticks = self
                .max_dispatch_start_error_abs_ticks
                .max(record.dispatch_start_error_ticks.unsigned_abs());
            self.max_authored_lateness_ticks = if self.authored_lateness_samples == 1 {
                record.authored_lateness_ticks
            } else {
                self.max_authored_lateness_ticks
                    .max(record.authored_lateness_ticks)
            };
            self.max_authored_lateness_abs_ticks = self
                .max_authored_lateness_abs_ticks
                .max(record.authored_lateness_ticks.unsigned_abs());
            self.overdue_packet_count = self
                .overdue_packet_count
                .saturating_add(u64::from(record.authored_target_overdue));
        }
        if record.floor_displacement_available {
            self.floor_displacement_samples = self.floor_displacement_samples.saturating_add(1);
            self.max_physical_floor_displacement_ticks = self
                .max_physical_floor_displacement_ticks
                .max(record.physical_floor_displacement_ticks);
        }
        if record.residual_lateness_available {
            self.residual_lateness_samples = self.residual_lateness_samples.saturating_add(1);
            self.max_residual_lateness_ticks = if self.residual_lateness_samples == 1 {
                record.residual_lateness_ticks
            } else {
                self.max_residual_lateness_ticks
                    .max(record.residual_lateness_ticks)
            };
            self.max_residual_lateness_abs_ticks = self
                .max_residual_lateness_abs_ticks
                .max(record.residual_lateness_ticks.unsigned_abs());
        }
        if record.hol_delay_available {
            self.hol_delay_samples = self.hol_delay_samples.saturating_add(1);
            self.max_hol_delay_lower_bound_ticks = self
                .max_hol_delay_lower_bound_ticks
                .max(record.hol_delay_lower_bound_ticks);
        }
        if record.unattributed_lateness_available {
            self.unattributed_lateness_samples =
                self.unattributed_lateness_samples.saturating_add(1);
            self.max_unattributed_lateness_ticks = self
                .max_unattributed_lateness_ticks
                .max(record.unattributed_lateness_ticks);
        }
        if record.sendinput_envelope_available {
            self.sender_envelope_samples = self.sender_envelope_samples.saturating_add(1);
            self.max_sendinput_envelope_ticks = self
                .max_sendinput_envelope_ticks
                .max(record.sendinput_envelope_ticks);
        }
        if record.transport_anomaly {
            self.transport_anomaly_count = self.transport_anomaly_count.saturating_add(1);
        }
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
    pub(crate) fn try_new(mode: TelemetryMode, capacity: usize) -> Result<Self, String> {
        let mut records = VecDeque::new();
        if matches!(mode, TelemetryMode::Ring) {
            records
                .try_reserve_exact(capacity)
                .map_err(|error| format!("telemetry ring allocation failed: {error}"))?;
        }
        Ok(Self {
            schema_version: NATIVE_TELEMETRY_SCHEMA_VERSION,
            qpc_frequency_hz: 0,
            records,
            summary: NativeTelemetrySummary::default(),
            attempted: 0,
            accepted: 0,
            dropped: 0,
            observer_queue_dropped: 0,
            truncated: false,
            timing_semantics: TimingSemantics::default(),
        })
    }

    #[cfg(any(test, feature = "test-support"))]
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
    pub(crate) const MAX_RING_CAPACITY: usize = 65_536;

    pub(crate) fn try_new(mode: TelemetryMode, capacity: usize) -> Result<Self, String> {
        if matches!(mode, TelemetryMode::Ring) && capacity > Self::MAX_RING_CAPACITY {
            return Err(format!(
                "telemetry capacity {capacity} exceeds the {} record admission bound",
                Self::MAX_RING_CAPACITY
            ));
        }
        Ok(Self {
            mode,
            capacity,
            output: NativeTelemetryOutput::try_new(mode, capacity)?,
        })
    }

    #[cfg(any(test, feature = "test-support"))]
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
#[path = "telemetry/tests.rs"]
mod tests;
