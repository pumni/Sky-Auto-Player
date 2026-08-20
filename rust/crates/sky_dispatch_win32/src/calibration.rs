//! Chord-aware `SendInput` delivery-proxy calibration harness.
//!
//! # Evidence scope
//!
//! This module measures **injected Raw Input target-to-receipt total hold
//! proxy** evidence only. Concretely it captures four QPC boundaries for each
//! direction: the absolute target, the fused sender crossing immediately
//! before `SendInput`, syscall completion, and the first `WM_INPUT` receipt.
//!
//! The measured boundary is:
//! ```text
//! target_ticks         — absolute target requested by the calibration pair
//! call_started_ticks   — fused crossing QPC immediately before SendInput
//! call_completed_ticks — QPC immediately after SendInput returns
//! first_receipt_ticks  — QPC when the first WM_INPUT for this packet arrives
//! ```
//!
//! This is **not** game-observed latency.  Do not label it as such.
//!
//! # Design
//!
//! A dedicated invisible top-level window is created for the duration of the
//! calibration run.  Raw keyboard input is registered for that window using
//! `RIDEV_INPUTSINK` so receipts arrive regardless of foreground focus.
//!
//! Each calibration packet carries an optional 8-bit marker plus 24-bit
//! `sequence_id` in `dwExtraInfo`. Windows Raw Input does not document that
//! this value preserves `KEYBDINPUT.dwExtraInfo`, so the tag is corroborating
//! evidence only. Admission uses one active packet, an ordered message-queue
//! barrier, and exact scan-code/direction/extended-flag identity.
//!
//! The window message pump runs on a dedicated thread so it does not interfere
//! with the calling thread's timing measurements.
//!
//! # Non-Windows
//!
//! On non-Windows targets the public surface compiles but every function
//! returns [`CalibrationError::PlatformUnsupported`].

use crate::clock::{DurationTicks, QpcClock, QpcTicks, qpc_now_ticks_checked, qpc_ticks_to_us};
use crate::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, PlatformSendResult};
use serde::{Deserialize, Serialize};
use sky_dispatch_core::time::SEND_COLD_THRESHOLD_US;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::time::Duration;

// ─── Public error type ────────────────────────────────────────────────────────

#[derive(Debug, Clone, thiserror::Error)]
pub enum CalibrationError {
    #[error("calibration is not supported on this platform")]
    PlatformUnsupported,

    #[error("failed to create calibration window: win32 error {0}")]
    WindowCreateFailed(u32),

    #[error("failed to register Raw Input: win32 error {0}")]
    RawInputRegisterFailed(u32),

    #[error("window thread panicked or could not start")]
    WindowThreadFailed,

    #[error("calibration correlation boundary was lost; session cannot arm another packet")]
    CorrelationBoundaryLost,

    #[error("sequence {sequence_id}: timeout waiting for {expected} receipts (got {received})")]
    ReceiptTimeout {
        sequence_id: u32,
        expected: u8,
        received: u8,
    },

    #[error("scan code {scan_code} is not an instrument key")]
    InvalidScanCode { scan_code: u16 },

    #[error("scan code {scan_code} appears more than once in a calibration packet")]
    DuplicateScanCode { scan_code: u16 },

    #[error("polyphony {0} exceeds maximum of 15")]
    PolyphonyTooLarge(usize),

    #[error("sample count must be at least 1")]
    ZeroSamples,

    #[error("QPC failed during calibration")]
    ClockFailure,

    #[error("calibration statistics overflowed")]
    StatisticsOverflow,

    #[error("calibration timestamp ordering is invalid: {field}")]
    TimestampOrder { field: &'static str },

    #[error("calibration timestamp arithmetic overflowed")]
    TimestampArithmeticOverflow,

    #[error("calibration precision wait failed: {detail}")]
    PrecisionWaitFailed { detail: String },

    #[error("calibration measurement budget expired before cleanup")]
    BudgetExceeded,

    #[error(
        "cold idle gap {configured_us}us is shorter than the shared threshold {threshold_us}us"
    )]
    ColdIdleGapTooShort {
        configured_us: u64,
        threshold_us: u64,
    },

    #[error(
        "hot gap target {configured_us}us must be shorter than the shared threshold {threshold_us}us"
    )]
    HotGapTargetTooLong {
        configured_us: u64,
        threshold_us: u64,
    },

    #[error("calibration state lock was poisoned")]
    StateLockFailed,

    #[error("calibration scheduling-aid provenance changed between buckets")]
    SchedulingAidProvenanceMismatch,

    #[error("calibration sequence id exhausted")]
    SequenceOverflow,

    #[error("calibration correlation self-test failed: {detail}")]
    CorrelationSelfTestFailed { detail: String },

    #[error("{phase} packet {sequence_id} was not fully clean: {received}/{expected} receipts")]
    PacketIntegrity {
        phase: &'static str,
        sequence_id: u32,
        expected: u8,
        received: u8,
        win32_error: Option<u32>,
    },

    #[error("failed to acquire foreground for calibration window within timeout")]
    ForegroundAcquireFailed,

    #[error("calibration window lost foreground focus during measurement")]
    ForegroundLost,

    #[error("calibration window was closed by user or system")]
    CalibrationWindowClosed,

    #[error("keyboard cleanup could not be verified; stuck keys: {stuck_keys:?}")]
    CleanupFailed { stuck_keys: Vec<u16> },

    #[error("{report}")]
    BucketFailed {
        report: Box<CalibrationFailureReport>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationFailureReport {
    pub kind: String,
    pub class: String,
    pub polyphony: u8,
    pub sample_index: u32,
    pub phase: String,
    pub exact_error: String,
    pub win32_error: Option<u32>,
    pub cleanup_success: bool,
    pub cleanup_stuck_keys: Vec<u16>,
    pub cleanup_verification_inconclusive: bool,
    pub raw_input_restore_failed: bool,
    pub pump_thread_failed: bool,
}

impl std::fmt::Display for CalibrationFailureReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "bucket {}/{}/{} sample {} phase {}: {}; win32_error={:?}; cleanup_success={}; stuck_keys={:?}; cleanup_verification_inconclusive={}; raw_input_restore_failed={}; pump_thread_failed={}",
            self.kind,
            self.class,
            self.polyphony,
            self.sample_index,
            self.phase,
            self.exact_error,
            self.win32_error,
            self.cleanup_success,
            self.cleanup_stuck_keys,
            self.cleanup_verification_inconclusive,
            self.raw_input_restore_failed,
            self.pump_thread_failed,
        )
    }
}

impl CalibrationError {
    fn win32_error(&self) -> Option<u32> {
        match self {
            Self::PacketIntegrity { win32_error, .. } => *win32_error,
            Self::BucketFailed { report } => report.win32_error,
            _ => None,
        }
    }
}

/// The four authoritative QPC boundaries for one key direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedTimingPoint {
    pub target_ticks: QpcTicks,
    pub pre_call_ticks: QpcTicks,
    pub completion_ticks: QpcTicks,
    pub receipt_ticks: QpcTicks,
}

/// Signed component and direct total shrink in QPC ticks.
///
/// Tick-domain arithmetic is intentionally kept separate from microsecond
/// presentation so a frequency conversion cannot affect pair qualification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedTimingShrinkTicks {
    pub scheduler_shrink_ticks: i128,
    pub sendinput_shrink_ticks: i128,
    pub delivery_shrink_ticks: i128,
    pub total_proxy_shrink_ticks: i128,
}

/// Signed component and direct total shrink in microseconds for reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairedTimingShrinkUs {
    pub scheduler_shrink_us: i64,
    pub sendinput_shrink_us: i64,
    pub delivery_shrink_us: i64,
    pub total_proxy_shrink_us: i64,
}

fn ordered_delta(
    later: QpcTicks,
    earlier: QpcTicks,
    field: &'static str,
) -> Result<i128, CalibrationError> {
    if later < earlier {
        return Err(CalibrationError::TimestampOrder { field });
    }
    Ok(i128::from(later.as_u64()) - i128::from(earlier.as_u64()))
}

fn signed_delta_ticks(later: QpcTicks, earlier: QpcTicks) -> i128 {
    i128::from(later.as_u64()) - i128::from(earlier.as_u64())
}

/// Compute paired scheduler, SendInput, delivery, and direct total shrink.
///
/// Each direction must have the monotonic target → pre-call → completion
/// ordering. Receipt delivery is signed: Raw Input may be observed by the
/// pump thread before `SendInput` returns, so `R - C` is allowed to be
/// negative. The direct total is checked against the decomposed sum so a
/// future schema cannot silently publish inconsistent component evidence.
pub fn paired_timing_shrink_ticks(
    down: PairedTimingPoint,
    up: PairedTimingPoint,
) -> Result<PairedTimingShrinkTicks, CalibrationError> {
    let down_scheduler = ordered_delta(
        down.pre_call_ticks,
        down.target_ticks,
        "down pre_call before target",
    )?;
    let up_scheduler = ordered_delta(
        up.pre_call_ticks,
        up.target_ticks,
        "up pre_call before target",
    )?;
    let down_send = ordered_delta(
        down.completion_ticks,
        down.pre_call_ticks,
        "down completion before pre_call",
    )?;
    let up_send = ordered_delta(
        up.completion_ticks,
        up.pre_call_ticks,
        "up completion before pre_call",
    )?;
    let down_delivery = signed_delta_ticks(down.receipt_ticks, down.completion_ticks);
    let up_delivery = signed_delta_ticks(up.receipt_ticks, up.completion_ticks);
    let target_hold = ordered_delta(
        up.target_ticks,
        down.target_ticks,
        "up target before down target",
    )?;
    let receipt_hold = ordered_delta(
        up.receipt_ticks,
        down.receipt_ticks,
        "up receipt before down receipt",
    )?;
    let scheduler_shrink = down_scheduler
        .checked_sub(up_scheduler)
        .ok_or(CalibrationError::TimestampArithmeticOverflow)?;
    let sendinput_shrink = down_send
        .checked_sub(up_send)
        .ok_or(CalibrationError::TimestampArithmeticOverflow)?;
    let delivery_shrink = down_delivery
        .checked_sub(up_delivery)
        .ok_or(CalibrationError::TimestampArithmeticOverflow)?;
    let total_proxy_shrink = target_hold
        .checked_sub(receipt_hold)
        .ok_or(CalibrationError::TimestampArithmeticOverflow)?;
    let decomposed = scheduler_shrink
        .checked_add(sendinput_shrink)
        .and_then(|value| value.checked_add(delivery_shrink))
        .ok_or(CalibrationError::TimestampArithmeticOverflow)?;
    if total_proxy_shrink != decomposed {
        return Err(CalibrationError::TimestampArithmeticOverflow);
    }
    Ok(PairedTimingShrinkTicks {
        scheduler_shrink_ticks: scheduler_shrink,
        sendinput_shrink_ticks: sendinput_shrink,
        delivery_shrink_ticks: delivery_shrink,
        total_proxy_shrink_ticks: total_proxy_shrink,
    })
}

fn signed_ticks_to_us(clock: QpcClock, ticks: i128) -> Result<i64, CalibrationError> {
    let magnitude = ticks.unsigned_abs();
    let magnitude = u64::try_from(magnitude).map_err(|_| CalibrationError::ClockFailure)?;
    let micros = clock
        .duration_to_us(crate::clock::DurationTicks::from_raw(magnitude))
        .map_err(|_| CalibrationError::ClockFailure)?;
    let micros = i64::try_from(micros).map_err(|_| CalibrationError::ClockFailure)?;
    Ok(if ticks.is_negative() { -micros } else { micros })
}

pub fn paired_timing_shrink_us(
    clock: QpcClock,
    shrink: PairedTimingShrinkTicks,
) -> Result<PairedTimingShrinkUs, CalibrationError> {
    Ok(PairedTimingShrinkUs {
        scheduler_shrink_us: signed_ticks_to_us(clock, shrink.scheduler_shrink_ticks)?,
        sendinput_shrink_us: signed_ticks_to_us(clock, shrink.sendinput_shrink_ticks)?,
        delivery_shrink_us: signed_ticks_to_us(clock, shrink.delivery_shrink_ticks)?,
        total_proxy_shrink_us: signed_ticks_to_us(clock, shrink.total_proxy_shrink_ticks)?,
    })
}

// ─── Sample record ────────────────────────────────────────────────────────────

/// A single calibration sample for one polyphony/direction bucket.
///
/// All times are QPC ticks at the time of collection and are converted to
/// microseconds only when building the output JSON so that internal logic
/// stays in tick-domain.
#[derive(Debug, Clone)]
pub struct CalibrationSample {
    pub sequence_id: u32,
    pub target_ticks: QpcTicks,
    pub call_started_ticks: QpcTicks,
    pub call_completed_ticks: QpcTicks,
    /// `None` means no receipt arrived within the timeout window.
    pub first_receipt_ticks: Option<QpcTicks>,
    /// `None` means fewer receipts than expected arrived within the timeout.
    pub last_receipt_ticks: Option<QpcTicks>,
    pub receipt_count: u8,
    pub expected_receipt_count: u8,
    pub win32_error: Option<u32>,
    /// Physical idle gap from the immediately previous SendInput completion
    /// to this packet's exact SendInput entry, when measured evidence exists.
    pub actual_idle_gap_ticks: Option<sky_dispatch_core::time::DurationTicks>,
    /// Class derived from `actual_idle_gap_ticks`, never from requested sleep.
    pub observed_class: Option<SampleClass>,
    /// Anomalies detected for this packet.
    pub anomalies: SampleAnomalies,
    /// Per-key receipts retained for paired Down/Up correlation. This is
    /// bounded by the instrument packet maximum and never serialized raw.
    receipts: SmallVec<[RawInputReceipt; 15]>,
}

impl CalibrationSample {
    pub fn call_duration_us(&self) -> Result<u64, CalibrationError> {
        let ticks = self
            .call_completed_ticks
            .as_u64()
            .checked_sub(self.call_started_ticks.as_u64())
            .ok_or(CalibrationError::ClockFailure)?;
        qpc_ticks_to_us(QpcTicks::from_raw(ticks)).map_err(|_| CalibrationError::ClockFailure)
    }

    /// Signed error: first_receipt relative to call_completed. `None` if
    /// no first receipt arrived.
    pub fn first_receipt_latency_us(&self) -> Result<Option<i64>, CalibrationError> {
        let Some(first) = self.first_receipt_ticks else {
            return Ok(None);
        };
        let completed = self.call_completed_ticks;
        Ok(Some(signed_delta_us(first, completed)?))
    }

    /// Signed error: last_receipt relative to call_completed. `None` if not
    /// all expected receipts arrived.
    pub fn last_receipt_latency_us(&self) -> Result<Option<i64>, CalibrationError> {
        let Some(last) = self.last_receipt_ticks else {
            return Ok(None);
        };
        let completed = self.call_completed_ticks;
        Ok(Some(signed_delta_us(last, completed)?))
    }

    /// Spread between first and last receipt (intra-chord spread). `None` for
    /// monophonic packets or when either timestamp is missing.
    pub fn intra_chord_spread_us(&self) -> Result<Option<u64>, CalibrationError> {
        let (Some(first), Some(last)) = (self.first_receipt_ticks, self.last_receipt_ticks) else {
            return Ok(None);
        };
        let ticks = last
            .as_u64()
            .checked_sub(first.as_u64())
            .ok_or(CalibrationError::ClockFailure)?;
        Ok(Some(
            qpc_ticks_to_us(QpcTicks::from_raw(ticks))
                .map_err(|_| CalibrationError::ClockFailure)?,
        ))
    }

    pub fn is_complete(&self) -> bool {
        self.receipt_count == self.expected_receipt_count
    }
}

/// Signed per-key evidence from one balanced Down/Up pair.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyShrinkEvidence {
    pub scan_code: u16,
    pub down_target_ticks: u64,
    pub down_pre_call_ticks: u64,
    pub down_completion_ticks: u64,
    pub down_receipt_ticks: u64,
    pub up_target_ticks: u64,
    pub up_pre_call_ticks: u64,
    pub up_completion_ticks: u64,
    pub up_receipt_ticks: u64,
    pub down_latency_us: i64,
    pub up_latency_us: i64,
    /// Legacy delivery-only alias retained for diagnostic readers.
    pub shrink_us: i64,
    pub scheduler_shrink_us: i64,
    pub sendinput_shrink_us: i64,
    pub delivery_shrink_us: i64,
    pub total_proxy_shrink_us: i64,
}

/// A balanced Down/Up evidence unit. Directional receipts are paired by scan
/// code before any aggregation so common-mode observer jitter is cancelled.
#[derive(Debug, Clone)]
pub struct PairSample {
    pub down: CalibrationSample,
    pub up: CalibrationSample,
    pub down_idle_gap_ticks: sky_dispatch_core::time::DurationTicks,
    pub up_idle_gap_ticks: sky_dispatch_core::time::DurationTicks,
    pub pair_worst_shrink_us: Option<i64>,
    pub pair_worst_total_proxy_shrink_us: Option<i64>,
    pub pair_worst_scheduler_shrink_us: Option<i64>,
    pub pair_worst_sendinput_shrink_us: Option<i64>,
    pub pair_worst_delivery_shrink_us: Option<i64>,
    pub key_evidence: SmallVec<[KeyShrinkEvidence; 15]>,
    pub pairing_anomaly: bool,
    pub receipt_before_completion_count: u64,
}

impl PairSample {
    pub fn is_clean(&self) -> bool {
        self.pair_worst_total_proxy_shrink_us.is_some()
            && !self.pairing_anomaly
            && self.down.is_complete()
            && self.up.is_complete()
            && !self.down.anomalies.any()
            && !self.up.anomalies.any()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairSampleEvidence {
    pub clean: bool,
    pub actual_down_gap_us: u64,
    pub actual_up_gap_us: u64,
    pub pair_worst_shrink_us: Option<i64>,
    pub pair_worst_total_proxy_shrink_us: Option<i64>,
    pub pair_worst_scheduler_shrink_us: Option<i64>,
    pub pair_worst_sendinput_shrink_us: Option<i64>,
    pub pair_worst_delivery_shrink_us: Option<i64>,
    pub key_evidence: Vec<KeyShrinkEvidence>,
    pub down_call_duration_us: u64,
    pub up_call_duration_us: u64,
    pub down_receipt_us: Option<SignedQuantileStats>,
    pub up_receipt_us: Option<SignedQuantileStats>,
    pub pairing_anomaly: bool,
    pub receipt_before_completion_count: u64,
    pub down_anomalies: SampleAnomalies,
    pub up_anomalies: SampleAnomalies,
}

/// Signed tick delta in microseconds: `later - earlier`.
fn signed_delta_us(later: QpcTicks, earlier: QpcTicks) -> Result<i64, CalibrationError> {
    if later.as_u64() >= earlier.as_u64() {
        let micros = qpc_ticks_to_us(QpcTicks::from_raw(later.as_u64() - earlier.as_u64()))
            .map_err(|_| CalibrationError::ClockFailure)?;
        i64::try_from(micros).map_err(|_| CalibrationError::ClockFailure)
    } else {
        let micros = qpc_ticks_to_us(QpcTicks::from_raw(earlier.as_u64() - later.as_u64()))
            .map_err(|_| CalibrationError::ClockFailure)?;
        i64::try_from(micros)
            .map(|value| -value)
            .map_err(|_| CalibrationError::ClockFailure)
    }
}

/// Classify a measured packet from the physical QPC idle interval immediately
/// preceding its exact SendInput entry.
pub fn classify_idle_gap(
    previous_completion: QpcTicks,
    current_start: QpcTicks,
    cold_threshold: sky_dispatch_core::time::DurationTicks,
) -> Result<(SampleClass, sky_dispatch_core::time::DurationTicks), CalibrationError> {
    let gap = current_start
        .checked_duration_since(previous_completion)
        .map_err(|_| CalibrationError::ClockFailure)?;
    let observed = if gap >= cold_threshold {
        SampleClass::Cold
    } else {
        SampleClass::Hot
    };
    Ok((observed, gap))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SampleAnomalies {
    pub duplicate_receipt: bool,
    pub reordered_receipt: bool,
    pub unexpected_scan_code: bool,
    pub timeout: bool,
    pub partial_send: bool,
    pub class_mismatch: bool,
    pub direction_mismatch: bool,
}

impl SampleAnomalies {
    pub fn any(&self) -> bool {
        self.duplicate_receipt
            || self.reordered_receipt
            || self.unexpected_scan_code
            || self.timeout
            || self.partial_send
            || self.class_mismatch
            || self.direction_mismatch
    }
}

/// Decide whether a receipt wait should continue or yield to the caller.
/// Receipt timeout is deliberately distinct from the global QPC budget; the
/// caller records it as a correlation-boundary failure and closes the session.
fn receipt_wait_duration(
    receipt_remaining: Duration,
    budget_remaining: Option<Duration>,
) -> Result<Option<Duration>, CalibrationError> {
    if let Some(budget_remaining) = budget_remaining {
        if budget_remaining.is_zero() {
            return Err(CalibrationError::BudgetExceeded);
        }
        let remaining = receipt_remaining.min(budget_remaining);
        if remaining.is_zero() {
            return if receipt_remaining.is_zero() {
                Ok(None)
            } else {
                Err(CalibrationError::BudgetExceeded)
            };
        }
        return Ok(Some(remaining));
    }
    if receipt_remaining.is_zero() {
        Ok(None)
    } else {
        Ok(Some(receipt_remaining))
    }
}

// ─── Bucket-level statistics ──────────────────────────────────────────────────

/// Aggregated statistics for one (kind, polyphony, class) bucket.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BucketStats {
    /// Number of measured (non-warm-up) attempts in this bucket.
    pub attempted: u64,
    /// Attempts with all expected receipts and no anomaly.
    pub clean: u64,
    /// Explicit count of samples admitted to timing quantiles.
    pub clean_sample_count: u64,
    /// Attempts rejected from timing quantiles.
    pub rejected: u64,
    pub partial_send: u64,
    pub sample_count: u64,
    pub timeout_count: u64,
    pub anomaly_count: u64,
    pub pairing_anomaly_count: u64,
    pub duplicate_receipt_count: u64,
    pub unexpected_scan_code_count: u64,
    pub direction_mismatch_count: u64,
    pub reordered_receipt_count: u64,
    pub class_mismatch_count: u64,
    /// Count of per-key Raw Input timestamps observed before SendInput return.
    pub receipt_before_completion_count: u64,
    pub down_call_duration_us: QuantileStats,
    pub up_call_duration_us: QuantileStats,
    /// Legacy delivery-only diagnostic retained for audit readers.
    #[serde(default)]
    pub pair_worst_shrink_us: Option<SignedQuantileStats>,
    /// vNext qualification evidence: worst per-key total target-to-receipt
    /// hold shrink for each clean pair.
    #[serde(default)]
    pub pair_worst_total_proxy_shrink_us: Option<SignedQuantileStats>,
    /// Component diagnostics only; these are never summed independently for
    /// qualification.
    #[serde(default)]
    pub scheduler_shrink_us: Option<SignedQuantileStats>,
    #[serde(default)]
    pub sendinput_shrink_us: Option<SignedQuantileStats>,
    #[serde(default)]
    pub delivery_shrink_us: Option<SignedQuantileStats>,
    #[serde(default)]
    pub down_receipt_us: Option<SignedQuantileStats>,
    #[serde(default)]
    pub up_receipt_us: Option<SignedQuantileStats>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuantileStats {
    pub min: u64,
    pub p50: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
    pub mean: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignedQuantileStats {
    pub min: i64,
    pub p50: i64,
    pub p90: i64,
    pub p95: i64,
    pub p99: i64,
    pub max: i64,
    pub mean: i64,
}

// ─── Configuration ────────────────────────────────────────────────────────────

/// Polyphony classes used for bucket splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleClass {
    Hot,
    Cold,
}

pub const MEASUREMENT_PROTOCOL_VERSION: u32 = 8;
pub const CALIBRATION_SCHEMA_VERSION: u32 = 13;
pub const HOST_FINGERPRINT_VERSION: u32 = 2;
pub const CALIBRATION_EVIDENCE_KIND: &str = "injected_raw_input_total_hold_proxy";
pub const CALIBRATION_CLEANUP_RESERVE_SECONDS: u64 = 5;
pub const CALIBRATION_MIN_MEASUREMENT_SECONDS: u64 = 1;
pub const CALIBRATION_MIN_TOTAL_BUDGET_SECONDS: u64 =
    CALIBRATION_CLEANUP_RESERVE_SECONDS + CALIBRATION_MIN_MEASUREMENT_SECONDS;
/// Fixed production precision handoff. The waiter reaches `T - 700 µs` with
/// no busy-spin; the fused sender owns the final crossing to physical `T`.
pub const CALIBRATION_PRECISION_HANDOFF_US: u64 = 700;
/// Bounded retry allowance used to collect the configured number of clean
/// pairs without allowing a pathological host to run indefinitely.
pub const CALIBRATION_MAX_ATTEMPT_MULTIPLIER: u32 = 2;
pub const MAX_ANOMALOUS_PAIR_EVIDENCE: usize = 64;

fn exact_sendinput_boundaries(
    result: &PlatformSendResult,
) -> Result<(QpcTicks, QpcTicks), CalibrationError> {
    if result.timing_error.is_some() {
        return Err(CalibrationError::ClockFailure);
    }
    let completed = result
        .completed_ticks
        .ok_or(CalibrationError::ClockFailure)?;
    Ok((result.started_ticks, completed))
}

/// Parameters for a single calibration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationConfig {
    /// Polyphonies to measure (1–15). Must not be empty.
    pub polyphonies: Vec<u8>,
    /// Target number of clean measured pairs per hot bucket.
    pub samples_per_hot_bucket: u32,
    /// Target number of clean measured pairs per cold bucket.
    pub samples_per_cold_bucket: u32,
    /// Warm-up injections that are not included in any measured bucket.
    pub warmup_samples: u32,
    /// Milliseconds to wait for Raw Input receipts before marking a packet as
    /// timed out. Recommended: 200 ms.
    pub receipt_timeout_ms: u32,
    /// Requested microseconds between hot samples. Actual QPC time is still
    /// required to admit a sample to the hot bucket.
    pub hot_gap_target_us: u64,
    /// Idle interval before a cold sample. This must be at least the
    /// production cold-class threshold.
    pub cold_idle_gap_us: u64,
    /// Shared physical threshold used to classify measured packets.
    pub cold_threshold_us: u64,
    /// Hard native child budget. The native process reserves
    /// [`CALIBRATION_CLEANUP_RESERVE_SECONDS`] seconds for final cleanup;
    /// callers must keep this in 6..=120 seconds.
    pub budget_seconds: u64,
}

impl Default for CalibrationConfig {
    fn default() -> Self {
        Self {
            polyphonies: vec![1, 2, 3, 5, 8, 15],
            samples_per_hot_bucket: 500,
            samples_per_cold_bucket: 500,
            warmup_samples: 20,
            receipt_timeout_ms: 200,
            hot_gap_target_us: 5_000,
            cold_idle_gap_us: 25_000,
            cold_threshold_us: SEND_COLD_THRESHOLD_US,
            budget_seconds: 120,
        }
    }
}

impl CalibrationConfig {
    /// Minimal quick-calibration preset (user setup).
    pub fn quick() -> Self {
        Self {
            polyphonies: vec![1, 5, 15],
            // Quick is the publishable interactive protocol. Diagnostic
            // callers pass an explicitly smaller count and are non-publishable
            // at the Python boundary.
            samples_per_hot_bucket: 100,
            samples_per_cold_bucket: 100,
            warmup_samples: 4,
            ..Self::default()
        }
    }

    /// Full calibration preset for timing and release-gate measurements.
    pub fn full() -> Self {
        Self {
            samples_per_hot_bucket: 5_000,
            samples_per_cold_bucket: 5_000,
            warmup_samples: 50,
            ..Self::default()
        }
    }
}

/// Scheduling aids acquired for the native calibration measurement.
///
/// `power_throttling_active` means that the guard successfully disabled
/// execution-speed throttling (the HighQoS opt-out), not that throttling is
/// being enabled. Labels are captured at acquisition time so the evidence
/// records the actual runtime path rather than only the requested policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulingAidProvenance {
    pub mmcss_acquired: &'static str,
    pub mmcss_active: bool,
    pub power_throttling_active: bool,
    pub waiter_mode: &'static str,
}

// ─── Output schema ────────────────────────────────────────────────────────────

/// The complete output of one protocol-vNext calibration run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationOutput {
    /// Output schema version — bump when fields are added or renamed.
    pub version: u32,
    /// Measurement protocol version — bump when packet sequencing changes.
    pub measurement_protocol_version: u32,
    pub source_git_sha: &'static str,
    pub native_build_id: &'static str,
    pub dirty_worktree: bool,
    pub native_source_fingerprint: &'static str,
    pub rustc_version: &'static str,
    pub evidence_kind: &'static str,
    pub host_fingerprint: HostFingerprint,
    pub scheduling_aids: SchedulingAidProvenance,
    pub configuration: CalibrationConfig,
    /// Protocol-vNext pair matrix. The six required production cells live here.
    pub pair_buckets: HashMap<u8, HashMap<String, BucketStats>>,
    /// Bounded diagnostic evidence for rejected pairs in each required bucket.
    pub anomalous_pairs: HashMap<u8, HashMap<String, Vec<PairSampleEvidence>>>,
    /// Warm-up attempts, kept separate from measured evidence.
    pub warmup_attempted: u64,
    /// Measured attempts represented by the bucket map.
    pub measured_attempted: u64,
    /// Physical setup Down samples, excluded from timing quantiles.
    pub setup_attempted: u64,
    pub setup_anomalous: u64,
    pub setup_timed_out: u64,
    /// Total sample attempts (warm-up plus measured plus setup Down).
    pub total_attempted: u64,
    pub warmup_anomalous: u64,
    pub measured_anomalous: u64,
    /// Total samples with at least one anomaly.
    pub total_anomalous: u64,
    pub warmup_timed_out: u64,
    pub measured_timed_out: u64,
    pub measured_class_mismatch: u64,
    /// Total samples that timed out completely.
    pub total_timed_out: u64,
    pub cleanup: CleanupOutcome,
}

/// Pair-centric bucket artifact used by the protocol-vNext runner. It is kept
/// separate from the legacy directional shape so no caller can accidentally
/// treat a directional bucket as publishable evidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationPairBucketOutput {
    pub version: u32,
    pub measurement_protocol_version: u32,
    pub source_git_sha: &'static str,
    pub native_build_id: &'static str,
    pub dirty_worktree: bool,
    pub native_source_fingerprint: &'static str,
    pub rustc_version: &'static str,
    pub evidence_kind: &'static str,
    pub host_fingerprint: HostFingerprint,
    pub scheduling_aids: SchedulingAidProvenance,
    pub configuration: CalibrationConfig,
    pub class: SampleClass,
    pub polyphony: u8,
    pub attempted_pairs: u64,
    pub warmup_pairs: u64,
    pub warmup_rejected: u64,
    pub pair_bucket: BucketStats,
    pub worst_pairs: Vec<PairSampleEvidence>,
    pub anomalous_pairs: Vec<PairSampleEvidence>,
    pub cleanup: CleanupOutcome,
}

/// Outcome of the final bounded full-instrument release. This is part of the
/// evidence so callers cannot mistake a calibration that left the keyboard in
/// an unknown state for a successful run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupOutcome {
    pub cleanup_attempted: bool,
    pub cleanup_success: bool,
    pub cleanup_stuck_keys: Vec<u16>,
    pub cleanup_verification_inconclusive: bool,
    /// The calibration process could not prove that its Raw Input registration
    /// was restored before the pump thread exited.
    pub raw_input_restore_failed: bool,
    pub pump_thread_failed: bool,
}

// RAWKEYBOARD.ExtraInformation is a 32-bit ULONG even on x64 Windows. Keep a
// calibration tag in the high byte and the packet sequence in the remaining
// 24 bits; putting the sequence in the high 32 bits of dwExtraInfo is silently
// truncated before WM_INPUT reaches the pump.
const CALIBRATION_EXTRA_TAG_MASK: u32 = 0xFF00_0000;
const CALIBRATION_EXTRA_TAG: u32 = (crate::input::SKY_PLAYER_SIGNATURE as u32 & 0xFF) << 24;
const CALIBRATION_EXTRA_SEQUENCE_MASK: u32 = 0x00FF_FFFF;

fn make_calibration_extra_info(sequence_id: u32) -> Option<usize> {
    (sequence_id > 0 && sequence_id <= CALIBRATION_EXTRA_SEQUENCE_MASK)
        .then_some((CALIBRATION_EXTRA_TAG | sequence_id) as usize)
}

fn calibration_extra_info_sequence(extra: usize) -> Option<u32> {
    let raw = u32::try_from(extra).ok()?;
    if raw & CALIBRATION_EXTRA_TAG_MASK != CALIBRATION_EXTRA_TAG {
        return None;
    }
    let sequence_id = raw & CALIBRATION_EXTRA_SEQUENCE_MASK;
    (sequence_id > 0).then_some(sequence_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostFingerprint {
    pub host_fingerprint_version: u32,
    pub qpc_frequency_hz: u64,
    pub win32_build: Option<String>,
    pub processor_architecture: String,
    pub cpu_vendor: String,
    pub cpu_family: u32,
    pub cpu_model: u32,
    pub cpu_stepping: u32,
    pub logical_processor_count: u32,
    pub processor_group_count: u16,
    pub cpu_set_efficiency_classes: Vec<u32>,
    pub highest_efficiency_class: Option<u8>,
    pub lowest_efficiency_class: Option<u8>,
    pub sampled_at_us: u64,
}

fn validate_calibration_config(config: &CalibrationConfig) -> Result<(), CalibrationError> {
    if config.polyphonies.is_empty()
        || config.samples_per_hot_bucket == 0
        || config.samples_per_cold_bucket == 0
    {
        return Err(CalibrationError::ZeroSamples);
    }
    if !(CALIBRATION_MIN_TOTAL_BUDGET_SECONDS..=120).contains(&config.budget_seconds) {
        return Err(CalibrationError::BudgetExceeded);
    }
    if config.cold_threshold_us == 0 {
        return Err(CalibrationError::ColdIdleGapTooShort {
            configured_us: config.cold_threshold_us,
            threshold_us: SEND_COLD_THRESHOLD_US,
        });
    }
    if config.hot_gap_target_us >= config.cold_threshold_us {
        return Err(CalibrationError::HotGapTargetTooLong {
            configured_us: config.hot_gap_target_us,
            threshold_us: config.cold_threshold_us,
        });
    }
    if config.cold_idle_gap_us < config.cold_threshold_us {
        return Err(CalibrationError::ColdIdleGapTooShort {
            configured_us: config.cold_idle_gap_us,
            threshold_us: config.cold_threshold_us,
        });
    }
    for &p in &config.polyphonies {
        if p == 0 || p as usize > crate::input::PHYSICAL_INSTRUMENT_SCAN_CODES.len() {
            return Err(CalibrationError::PolyphonyTooLarge(p as usize));
        }
    }
    Ok(())
}

fn validate_packet_scan_codes(scan_codes: &[u16]) -> Result<(), CalibrationError> {
    if scan_codes.is_empty() || scan_codes.len() > 15 {
        return Err(CalibrationError::PolyphonyTooLarge(scan_codes.len()));
    }
    let mut seen = SmallVec::<[u16; 15]>::new();
    for &scan_code in scan_codes {
        if !PHYSICAL_INSTRUMENT_SCAN_CODES.contains(&scan_code) {
            return Err(CalibrationError::InvalidScanCode { scan_code });
        }
        if seen.contains(&scan_code) {
            return Err(CalibrationError::DuplicateScanCode { scan_code });
        }
        seen.push(scan_code);
    }
    Ok(())
}

// ─── Internal raw-input receipt state (shared between pump and collector) ─────

/// A single Raw Input receipt delivered by the message pump.
#[derive(Debug, Clone, Copy)]
struct RawInputReceipt {
    arrived_ticks: QpcTicks,
    scan_code: u16,
    sequence_id: u32,
    key_up: bool,
    extended_flags: u8,
}

const MAX_DIAGNOSTIC_ACCEPTED_IDENTITIES: usize = 32;
const MAX_PENDING_RECEIPTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AcceptedReceiptIdentity {
    sequence_id: u32,
    scan_code: u16,
    key_up: bool,
    extended_flags: u8,
}

#[derive(Debug, Clone, Default)]
struct PumpDiagnostics {
    wm_input_seen: u64,
    qpc_failed: u64,
    state_lock_failed: u64,
    raw_size_query_failed: u64,
    raw_size_invalid: u64,
    raw_read_failed: u64,
    raw_payload_too_small: u64,
    raw_alignment_failed: u64,
    non_keyboard: u64,
    tag_decode_failed: u64,
    stale_sequence: u64,
    wrong_direction: u64,
    unexpected_identity: u64,
    duplicate_receipt: u64,
    pending_receipt_overflow: u64,
    accepted_receipts: u64,
    accepted_identities: SmallVec<[AcceptedReceiptIdentity; 32]>,
}

impl PumpDiagnostics {
    fn remember_accepted(&mut self, receipt: RawInputReceipt) {
        self.accepted_receipts = self.accepted_receipts.saturating_add(1);
        if self.accepted_identities.len() < MAX_DIAGNOSTIC_ACCEPTED_IDENTITIES {
            self.accepted_identities.push(AcceptedReceiptIdentity {
                sequence_id: receipt.sequence_id,
                scan_code: receipt.scan_code,
                key_up: receipt.key_up,
                extended_flags: receipt.extended_flags,
            });
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawInputParseError {
    BufferLengthInvalid,
    Misaligned,
    TruncatedHeader,
    InvalidHeaderSize,
    TruncatedKeyboardPayload,
    NonKeyboard { raw_type: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedRawKeyboard {
    scan_code: u16,
    flags: u16,
    extra_information: usize,
}

/// Shared state between the calibration thread (sends packets and awaits
/// receipts) and the window pump thread (delivers `WM_INPUT` events).
struct SharedCalibState {
    /// Receipts delivered for the currently active packet sequence.
    pending_receipts: SmallVec<[RawInputReceipt; 15]>,
    /// Sequence ID of the currently expected packet, `None` when idle.
    active_sequence: Option<u32>,
    /// Expected packet identity used by the pump for bounded diagnostics.
    active_expected_scan_codes: SmallVec<[u16; 15]>,
    active_expected_key_up: Option<bool>,
    pump_diagnostics: PumpDiagnostics,
    /// Message-queue barrier generations completed by the pump. A
    /// barrier drains currently queued WM_INPUT messages while no packet is
    /// active; a stale message or incomplete packet invalidates the boundary
    /// instead of permitting another tagless packet.
    barrier_completed_generation: u64,
    /// Set after an incomplete packet or stale receipt is observed. The
    /// session must not arm another tagless packet after this point.
    correlation_boundary_lost: bool,
    /// Set by the pump thread when the window is ready.
    window_ready: bool,
    /// Set to signal the pump thread to exit gracefully (checked on resume).
    should_exit: bool,
    window_closed: bool,
    foreground_lost: bool,
    /// HWND of the calibration window (as `isize`).
    hwnd: isize,
    clock_failed: bool,
    raw_input_restore_failed: bool,
    pump_thread_failed: bool,
}

fn clear_active_packet(state: &mut SharedCalibState) {
    state.active_sequence = None;
    state.active_expected_scan_codes.clear();
    state.active_expected_key_up = None;
}

fn invalidate_correlation_boundary(state: &mut SharedCalibState) {
    clear_active_packet(state);
    state.correlation_boundary_lost = true;
}

fn can_arm_next_packet(state: &SharedCalibState) -> bool {
    state.active_sequence.is_none() && !state.correlation_boundary_lost
}

#[cfg(any(test, feature = "test-support"))]
type ForegroundProbe = Box<dyn Fn(isize) -> bool + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static TEST_FOREGROUND_OVERRIDE: std::cell::RefCell<Option<ForegroundProbe>> = const { std::cell::RefCell::new(None) };
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_test_foreground_override<F: Fn(isize) -> bool + Send + Sync + 'static>(f: Option<F>) {
    TEST_FOREGROUND_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = f.map(|func| Box::new(func) as ForegroundProbe);
    });
}

pub fn check_foreground_owned(hwnd: isize) -> bool {
    #[cfg(any(test, feature = "test-support"))]
    {
        let overridden =
            TEST_FOREGROUND_OVERRIDE.with(|cell| cell.borrow().as_ref().map(|f| f(hwnd)));
        if let Some(res) = overridden {
            return res;
        }
    }
    #[cfg(windows)]
    {
        if hwnd == 0 {
            return false;
        }
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow()
                == (hwnd as windows_sys::Win32::Foundation::HWND)
        }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

// ─── Platform-specific implementation ────────────────────────────────────────

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::event::OwnedEvent;
    use crate::input::send_input_raw;
    use crate::mmcss::{MmcssGuard, PriorityMode};
    use crate::power::PowerThrottlingGuard;
    use crate::wait::{HybridWaiter, WaitOutcome};

    use std::sync::{Arc, Condvar, Mutex};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::Input::{
        GetRawInputData, GetRegisteredRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
        RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RegisterRawInputDevices,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, MSG,
        PM_REMOVE, PeekMessageW, PostMessageW, RegisterClassExW, SW_SHOW, SetForegroundWindow,
        ShowWindow, TranslateMessage, WM_CLOSE, WM_DESTROY, WM_INPUT, WM_QUIT, WM_USER,
        WNDCLASSEXW, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
    };

    // HID_USAGE_PAGE_GENERIC = 0x01 (USB HID spec, no feature flag needed)
    const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
    const RIM_TYPEKEYBOARD: u32 = 1;

    /// Parse one `GetRawInputData(RID_INPUT)` result without dereferencing a
    /// header or keyboard payload until both alignment and byte length have
    /// been proved. The Windows API requires DWORD-aligned output storage;
    /// the caller provides the aligned storage while this function remains
    /// pure and directly testable.
    fn parse_raw_keyboard_input(
        buffer: &[u8],
        bytes_read: usize,
    ) -> Result<ParsedRawKeyboard, RawInputParseError> {
        if bytes_read > buffer.len() {
            return Err(RawInputParseError::BufferLengthInvalid);
        }
        let data = &buffer[..bytes_read];
        let header_align = std::mem::align_of::<RAWINPUTHEADER>();
        if data.as_ptr().align_offset(header_align) != 0 {
            return Err(RawInputParseError::Misaligned);
        }
        let header_size = std::mem::size_of::<RAWINPUTHEADER>();
        if data.len() < header_size {
            return Err(RawInputParseError::TruncatedHeader);
        }

        // SAFETY: the slice is aligned for RAWINPUTHEADER and has been
        // checked to contain the complete header.
        let header = unsafe { &*(data.as_ptr().cast::<RAWINPUTHEADER>()) };
        if (header.dwSize as usize) < header_size || (header.dwSize as usize) > data.len() {
            return Err(RawInputParseError::InvalidHeaderSize);
        }
        if header.dwType != RIM_TYPEKEYBOARD {
            return Err(RawInputParseError::NonKeyboard {
                raw_type: header.dwType,
            });
        }

        let keyboard_offset = std::mem::offset_of!(RAWINPUT, data);
        let keyboard_size = std::mem::size_of::<windows_sys::Win32::UI::Input::RAWKEYBOARD>();
        let required_size = keyboard_offset
            .checked_add(keyboard_size)
            .ok_or(RawInputParseError::TruncatedKeyboardPayload)?;
        if (header.dwSize as usize) < required_size || data.len() < required_size {
            return Err(RawInputParseError::TruncatedKeyboardPayload);
        }
        let keyboard_ptr = unsafe { data.as_ptr().add(keyboard_offset) };
        if keyboard_ptr.align_offset(std::mem::align_of::<
            windows_sys::Win32::UI::Input::RAWKEYBOARD,
        >()) != 0
        {
            return Err(RawInputParseError::Misaligned);
        }
        // SAFETY: the keyboard union arm is selected by the validated header,
        // and the complete RAWKEYBOARD payload and alignment are present.
        let keyboard =
            unsafe { &*(keyboard_ptr.cast::<windows_sys::Win32::UI::Input::RAWKEYBOARD>()) };
        Ok(ParsedRawKeyboard {
            scan_code: keyboard.MakeCode,
            flags: keyboard.Flags,
            extra_information: keyboard.ExtraInformation as usize,
        })
    }

    fn record_pump_diagnostic(
        shared: &Arc<(Mutex<SharedCalibState>, Condvar)>,
        update: impl FnOnce(&mut PumpDiagnostics),
    ) {
        let (lock, cvar) = shared.as_ref();
        match lock.lock() {
            Ok(mut state) => update(&mut state.pump_diagnostics),
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.pump_thread_failed = true;
                state.pump_diagnostics.state_lock_failed =
                    state.pump_diagnostics.state_lock_failed.saturating_add(1);
                cvar.notify_all();
            }
        }
    }

    fn tagged_sequence_matches_active(active_sequence: u32, tagged_sequence: Option<u32>) -> bool {
        tagged_sequence.is_none_or(|tagged| tagged == active_sequence)
    }

    fn observe_stale_correlation_evidence(state: &mut SharedCalibState) {
        state.pump_diagnostics.stale_sequence =
            state.pump_diagnostics.stale_sequence.saturating_add(1);
        invalidate_correlation_boundary(state);
    }

    /// Reject a receipt that cannot be proven to belong to the active packet.
    ///
    /// With an optional Raw Input sequence tag, an identity or generation
    /// mismatch is observer-integrity evidence, not merely a bad sample. The
    /// session must stop before a later tagless receipt can alias the active
    /// packet. Each applicable diagnostic counter is retained, but the
    /// boundary is invalidated exactly once.
    fn observe_incompatible_receipt(
        state: &mut SharedCalibState,
        receipt: RawInputReceipt,
    ) -> bool {
        let unexpected_identity = !state
            .active_expected_scan_codes
            .contains(&receipt.scan_code)
            || receipt.extended_flags != 0;
        if unexpected_identity {
            state.pump_diagnostics.unexpected_identity =
                state.pump_diagnostics.unexpected_identity.saturating_add(1);
        }

        let wrong_direction = state.active_expected_key_up != Some(receipt.key_up);
        if wrong_direction {
            state.pump_diagnostics.wrong_direction =
                state.pump_diagnostics.wrong_direction.saturating_add(1);
        }

        let duplicate = state.pending_receipts.iter().any(|pending| {
            pending.sequence_id == receipt.sequence_id
                && pending.scan_code == receipt.scan_code
                && pending.key_up == receipt.key_up
                && pending.extended_flags == receipt.extended_flags
        });
        if duplicate {
            state.pump_diagnostics.duplicate_receipt =
                state.pump_diagnostics.duplicate_receipt.saturating_add(1);
        }

        let overflow = state.pending_receipts.len() >= MAX_PENDING_RECEIPTS;
        if overflow {
            state.pump_diagnostics.pending_receipt_overflow = state
                .pump_diagnostics
                .pending_receipt_overflow
                .saturating_add(1);
        }

        let incompatible = unexpected_identity || wrong_direction || duplicate || overflow;
        if incompatible {
            observe_stale_correlation_evidence(state);
        }
        incompatible
    }

    #[cfg(any(test, not(feature = "test-support")))]
    fn format_probe_failure_detail(
        direction: &str,
        scan_codes: &[u16],
        sample: &CalibrationSample,
        diagnostics: &PumpDiagnostics,
    ) -> String {
        let accepted_receipts: Vec<AcceptedReceiptIdentity> = sample
            .receipts
            .iter()
            .take(MAX_DIAGNOSTIC_ACCEPTED_IDENTITIES)
            .map(|receipt| AcceptedReceiptIdentity {
                sequence_id: receipt.sequence_id,
                scan_code: receipt.scan_code,
                key_up: receipt.key_up,
                extended_flags: receipt.extended_flags,
            })
            .collect();
        format!(
            "tagged {direction} probe was not clean: anomalies={:?}; expected_receipt_count={}; accepted_receipt_count={}; expected_scan_codes={scan_codes:?}; accepted_receipts={accepted_receipts:?}; pump_diagnostics={diagnostics:?}",
            sample.anomalies, sample.expected_receipt_count, sample.receipt_count,
        )
    }

    #[cfg(any(test, not(feature = "test-support")))]
    fn format_probe_error_detail(
        direction: &str,
        scan_codes: &[u16],
        error: &CalibrationError,
        diagnostics: &PumpDiagnostics,
    ) -> String {
        format!(
            "tagged {direction} probe failed before a complete sample: error={error}; expected_receipt_count={}; accepted_receipt_count={}; expected_scan_codes={scan_codes:?}; accepted_receipts={:?}; pump_diagnostics={diagnostics:?}",
            scan_codes.len(),
            diagnostics.accepted_receipts,
            diagnostics.accepted_identities,
        )
    }

    fn snapshot_raw_input_devices() -> Option<Vec<RAWINPUTDEVICE>> {
        let mut count = 0u32;
        // SAFETY: the null output buffer is the documented size-query form.
        let result = unsafe {
            GetRegisteredRawInputDevices(
                std::ptr::null_mut(),
                &mut count,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        if result == u32::MAX {
            return None;
        }
        let mut devices = vec![RAWINPUTDEVICE::default(); count as usize];
        if count == 0 {
            return Some(devices);
        }
        // SAFETY: `devices` has exactly the capacity reported by the size
        // query and the API writes at most that many records.
        let copied = unsafe {
            GetRegisteredRawInputDevices(
                devices.as_mut_ptr(),
                &mut count,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        (copied != u32::MAX).then_some(devices)
    }

    fn restore_raw_input_devices(devices: &[RAWINPUTDEVICE]) -> bool {
        if devices.is_empty() {
            return true;
        }
        // SAFETY: each record came from GetRegisteredRawInputDevices and the
        // slice stays alive for the duration of the call.
        let result = unsafe {
            RegisterRawInputDevices(
                devices.as_ptr(),
                devices.len() as u32,
                std::mem::size_of::<RAWINPUTDEVICE>() as u32,
            )
        };
        result != 0
    }

    const WM_CALIB_EXIT: u32 = WM_USER + 1;
    const WM_CALIB_ACTIVATE: u32 = WM_USER + 2;
    const WM_CALIB_BARRIER: u32 = WM_USER + 3;

    // ── Window procedure ──────────────────────────────────────────────────────

    // Thread-local pointer to the shared state, set before the message loop
    // starts and cleared after it exits.
    thread_local! {
        static PUMP_STATE: std::cell::Cell<*const PumpContext> =
            const { std::cell::Cell::new(std::ptr::null()) };
    }

    struct PumpContext {
        shared: Arc<(Mutex<SharedCalibState>, Condvar)>,
        // GetRawInputData requires DWORD-aligned output storage. `usize`
        // provides at least that alignment on every supported Windows target.
        input_buffer: std::cell::RefCell<Vec<usize>>,
    }

    fn complete_wm_input(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // GET_RAWINPUT_CODE_WPARAM(wParam) is the low byte. Foreground raw
        // input (RIM_INPUT == 0) must reach DefWindowProcW for system cleanup;
        // sink input is fully handled by this observer.
        if (wparam & 0xffusize) == 0 {
            // SAFETY: the window procedure received these message parameters.
            unsafe { DefWindowProcW(hwnd, WM_INPUT, wparam, lparam) }
        } else {
            0
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum PendingInputDrain {
        Clean,
        StaleInput,
        Quit,
        Failed,
    }

    /// Remove every pending WM_INPUT message while no packet is active. A
    /// posted barrier alone is not sufficient: GetMessage prioritizes posted
    /// messages over input messages, so the barrier handler must explicitly
    /// select and remove the input range before publishing completion.
    fn drain_pending_wm_input(
        shared: &Arc<(Mutex<SharedCalibState>, Condvar)>,
        hwnd: HWND,
    ) -> PendingInputDrain {
        let active = match shared.0.lock() {
            Ok(state) => state.active_sequence.is_some(),
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.pump_thread_failed = true;
                shared.1.notify_all();
                return PendingInputDrain::Failed;
            }
        };
        if active {
            if let Ok(mut state) = shared.0.lock() {
                state.pump_thread_failed = true;
                shared.1.notify_all();
            }
            return PendingInputDrain::Failed;
        }

        let mut stale_input_found = false;
        loop {
            let mut pending = MSG {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            };
            // SAFETY: `pending` is a valid output record and the filter
            // removes only WM_INPUT from this pump thread's queue.
            let found = unsafe {
                PeekMessageW(
                    &mut pending,
                    std::ptr::null_mut(),
                    WM_INPUT,
                    WM_INPUT,
                    PM_REMOVE,
                )
            };
            if found == 0 {
                break;
            }
            // PeekMessageW retrieves WM_QUIT regardless of the range filter.
            // Put it back and let the normal pump lifecycle consume it; never
            // treat the shutdown record as a raw-input receipt.
            if pending.message == WM_QUIT {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(
                        pending.wParam as i32,
                    );
                }
                match shared.0.lock() {
                    Ok(mut state) => {
                        state.window_closed = true;
                        state.should_exit = true;
                        shared.1.notify_all();
                    }
                    Err(poisoned) => {
                        let mut state = poisoned.into_inner();
                        state.pump_thread_failed = true;
                        shared.1.notify_all();
                        return PendingInputDrain::Failed;
                    }
                }
                return PendingInputDrain::Quit;
            }
            if pending.message != WM_INPUT {
                if let Ok(mut state) = shared.0.lock() {
                    state.pump_thread_failed = true;
                    shared.1.notify_all();
                }
                return PendingInputDrain::Failed;
            }
            stale_input_found = true;
            match shared.0.lock() {
                Ok(mut state) => {
                    state.pump_diagnostics.wm_input_seen =
                        state.pump_diagnostics.wm_input_seen.saturating_add(1);
                    observe_stale_correlation_evidence(&mut state);
                    shared.1.notify_all();
                }
                Err(poisoned) => {
                    let mut state = poisoned.into_inner();
                    state.pump_thread_failed = true;
                    state.pump_diagnostics.state_lock_failed =
                        state.pump_diagnostics.state_lock_failed.saturating_add(1);
                    shared.1.notify_all();
                    return PendingInputDrain::Failed;
                }
            }
            // Foreground WM_INPUT requires DefWindowProcW cleanup even when
            // the stale message is intentionally discarded.
            complete_wm_input(hwnd, pending.wParam, pending.lParam);
        }
        if stale_input_found {
            PendingInputDrain::StaleInput
        } else {
            PendingInputDrain::Clean
        }
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // SAFETY: we only set a raw pointer within the message loop on the same
        // thread where the window was created, and clear it before the pointer
        // could dangle.
        let ctx_ptr: *const PumpContext = PUMP_STATE.with(|c| c.get());
        if ctx_ptr.is_null() {
            // SAFETY: DefWindowProcW is always safe to call with the provided
            // message parameters.
            return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
        }
        // SAFETY: pointer is valid for the lifetime of the pump loop (see
        // above), and we do not move or free the context here.
        let ctx: &PumpContext = unsafe { &*ctx_ptr };

        match msg {
            WM_INPUT => {
                record_pump_diagnostic(&ctx.shared, |diagnostics| {
                    diagnostics.wm_input_seen = diagnostics.wm_input_seen.saturating_add(1);
                });
                let arrived = match qpc_now_ticks_checked() {
                    Ok(ticks) => ticks,
                    Err(_) => {
                        record_pump_diagnostic(&ctx.shared, |diagnostics| {
                            diagnostics.qpc_failed = diagnostics.qpc_failed.saturating_add(1);
                        });
                        let (lock, cvar) = ctx.shared.as_ref();
                        if let Ok(mut guard) = lock.lock() {
                            guard.clock_failed = true;
                            cvar.notify_all();
                        }
                        return complete_wm_input(hwnd, wparam, lparam);
                    }
                };
                let hri = lparam as HRAWINPUT;
                let mut size: u32 = 0;
                // SAFETY: querying size with null buffer is the documented
                // pattern for GetRawInputData.
                let queried = unsafe {
                    GetRawInputData(
                        hri,
                        RID_INPUT,
                        std::ptr::null_mut(),
                        &mut size,
                        std::mem::size_of::<RAWINPUTHEADER>() as u32,
                    )
                };
                if queried == u32::MAX {
                    record_pump_diagnostic(&ctx.shared, |diagnostics| {
                        diagnostics.raw_size_query_failed =
                            diagnostics.raw_size_query_failed.saturating_add(1);
                    });
                    return complete_wm_input(hwnd, wparam, lparam);
                }
                if size == 0
                    || size > 4096
                    || (size as usize) < std::mem::size_of::<RAWINPUTHEADER>()
                {
                    record_pump_diagnostic(&ctx.shared, |diagnostics| {
                        diagnostics.raw_size_invalid =
                            diagnostics.raw_size_invalid.saturating_add(1);
                    });
                    return complete_wm_input(hwnd, wparam, lparam);
                }
                let parsed = {
                    let mut buf = ctx.input_buffer.borrow_mut();
                    let word_size = std::mem::size_of::<usize>();
                    let word_count = (size as usize).div_ceil(word_size);
                    buf.resize(word_count, 0);
                    // SAFETY: `buf` is DWORD-aligned and has enough byte
                    // storage for the size reported by the previous query.
                    let read = unsafe {
                        GetRawInputData(
                            hri,
                            RID_INPUT,
                            buf.as_mut_ptr().cast(),
                            &mut size,
                            std::mem::size_of::<RAWINPUTHEADER>() as u32,
                        )
                    };
                    if read == u32::MAX || read as usize > buf.len() * word_size {
                        Err(RawInputParseError::BufferLengthInvalid)
                    } else {
                        // SAFETY: `buf` is aligned storage owned by this
                        // thread and stays alive for the pure parser call.
                        let bytes = unsafe {
                            std::slice::from_raw_parts(
                                buf.as_ptr().cast::<u8>(),
                                buf.len() * word_size,
                            )
                        };
                        parse_raw_keyboard_input(bytes, read as usize)
                    }
                };
                let parsed = match parsed {
                    Ok(parsed) => parsed,
                    Err(RawInputParseError::NonKeyboard { .. }) => {
                        record_pump_diagnostic(&ctx.shared, |diagnostics| {
                            diagnostics.non_keyboard = diagnostics.non_keyboard.saturating_add(1);
                        });
                        return complete_wm_input(hwnd, wparam, lparam);
                    }
                    Err(RawInputParseError::Misaligned) => {
                        record_pump_diagnostic(&ctx.shared, |diagnostics| {
                            diagnostics.raw_alignment_failed =
                                diagnostics.raw_alignment_failed.saturating_add(1);
                        });
                        return complete_wm_input(hwnd, wparam, lparam);
                    }
                    Err(RawInputParseError::InvalidHeaderSize) => {
                        record_pump_diagnostic(&ctx.shared, |diagnostics| {
                            diagnostics.raw_size_invalid =
                                diagnostics.raw_size_invalid.saturating_add(1);
                        });
                        return complete_wm_input(hwnd, wparam, lparam);
                    }
                    Err(RawInputParseError::TruncatedHeader)
                    | Err(RawInputParseError::TruncatedKeyboardPayload) => {
                        record_pump_diagnostic(&ctx.shared, |diagnostics| {
                            diagnostics.raw_payload_too_small =
                                diagnostics.raw_payload_too_small.saturating_add(1);
                        });
                        return complete_wm_input(hwnd, wparam, lparam);
                    }
                    Err(RawInputParseError::BufferLengthInvalid) => {
                        record_pump_diagnostic(&ctx.shared, |diagnostics| {
                            diagnostics.raw_read_failed =
                                diagnostics.raw_read_failed.saturating_add(1);
                        });
                        return complete_wm_input(hwnd, wparam, lparam);
                    }
                };
                // `RAWKEYBOARD.ExtraInformation` has no documented contract
                // that it preserves `KEYBDINPUT.dwExtraInfo` across
                // SendInput. Treat the tag as optional corroboration rather
                // than the admission key; the active packet and exact
                // physical identity below are authoritative.
                let tagged_sequence = calibration_extra_info_sequence(parsed.extra_information);
                if tagged_sequence.is_none() {
                    record_pump_diagnostic(&ctx.shared, |diagnostics| {
                        diagnostics.tag_decode_failed =
                            diagnostics.tag_decode_failed.saturating_add(1);
                    });
                }

                // RI_KEY_BREAK is the documented Raw Input make/break bit.
                // Keep the direction in the correlated receipt; scan-code and
                // sequence equality alone cannot prove a balanced pair.
                let (lock, cvar) = ctx.shared.as_ref();
                match lock.lock() {
                    Ok(mut guard) => {
                        let Some(active_sequence) = guard.active_sequence else {
                            observe_stale_correlation_evidence(&mut guard);
                            cvar.notify_all();
                            return complete_wm_input(hwnd, wparam, lparam);
                        };
                        if !tagged_sequence_matches_active(active_sequence, tagged_sequence) {
                            observe_stale_correlation_evidence(&mut guard);
                            cvar.notify_all();
                        } else {
                            let receipt = RawInputReceipt {
                                arrived_ticks: arrived,
                                scan_code: parsed.scan_code,
                                // The active packet generation is authoritative
                                // when the optional tag is absent.
                                sequence_id: active_sequence,
                                key_up: (parsed.flags & 0x0001) != 0,
                                // RI_KEY_E0/RI_KEY_E1 are part of the physical
                                // identity; never alias an extended key.
                                extended_flags: parsed.flags as u8 & (0x0002 | 0x0004),
                            };
                            if observe_incompatible_receipt(&mut guard, receipt) {
                                cvar.notify_all();
                            } else {
                                guard.pending_receipts.push(receipt);
                                guard.pump_diagnostics.remember_accepted(receipt);
                                cvar.notify_one();
                            }
                        }
                    }
                    Err(poisoned) => {
                        let mut guard = poisoned.into_inner();
                        guard.pump_thread_failed = true;
                        cvar.notify_all();
                    }
                }
                complete_wm_input(hwnd, wparam, lparam)
            }
            WM_CLOSE | WM_DESTROY => {
                let (lock, cvar) = ctx.shared.as_ref();
                if let Ok(mut guard) = lock.lock() {
                    guard.window_closed = true;
                    guard.should_exit = true;
                    cvar.notify_all();
                }
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
                0
            }
            WM_CALIB_EXIT => {
                // SAFETY: PostQuitMessage is safe to call from within a window
                // procedure.
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
                0
            }
            WM_CALIB_ACTIVATE => {
                unsafe {
                    ShowWindow(hwnd, SW_SHOW);
                    SetForegroundWindow(hwnd);
                    SetFocus(hwnd);
                };
                0
            }
            WM_CALIB_BARRIER => {
                match drain_pending_wm_input(&ctx.shared, hwnd) {
                    PendingInputDrain::Clean => {
                        let (lock, cvar) = ctx.shared.as_ref();
                        match lock.lock() {
                            Ok(mut guard) => {
                                guard.barrier_completed_generation =
                                    guard.barrier_completed_generation.max(wparam as u64);
                                cvar.notify_all();
                            }
                            Err(poisoned) => {
                                let mut guard = poisoned.into_inner();
                                guard.pump_thread_failed = true;
                                cvar.notify_all();
                            }
                        }
                    }
                    PendingInputDrain::StaleInput => {}
                    PendingInputDrain::Quit | PendingInputDrain::Failed => {}
                }
                0
            }
            _ => {
                // SAFETY: forwarding to the default handler is always safe.
                unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
            }
        }
    }

    // ── Window pump thread ────────────────────────────────────────────────────

    fn pump_thread(shared: Arc<(Mutex<SharedCalibState>, Condvar)>) {
        // RegisterRawInputDevices is process-global. Preserve every existing
        // registration and restore it before this calibration thread exits.
        // A missing snapshot is treated as a setup failure rather than
        // silently deleting another subsystem's registration.
        let Some(previous_raw_input_devices) = snapshot_raw_input_devices() else {
            let (lock, _cvar) = shared.as_ref();
            if let Ok(mut g) = lock.lock() {
                g.window_ready = true;
            }
            eprintln!("[calibration] GetRegisteredRawInputDevices failed");
            return;
        };
        // Register window class.
        let class_name: Vec<u16> = "SkyCalibWindow\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: std::ptr::null_mut(),
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        // SAFETY: wc is a fully initialised WNDCLASSEXW with valid pointers
        // that outlive this call.
        let atom = unsafe { RegisterClassExW(&wc) };
        if atom == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            if err != windows_sys::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS {
                let (lock, _cvar) = shared.as_ref();
                if let Ok(mut g) = lock.lock() {
                    g.window_ready = true; // signal failure with hwnd = 0
                    drop(g);
                }
                eprintln!("[calibration] RegisterClassExW failed: {err}");
                return;
            }
        }

        let window_name: Vec<u16> = "Sky Auto Player — Input Latency Calibration\0"
            .encode_utf16()
            .collect();
        // SAFETY: top-level window for calibration foreground ownership.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                100,
                100,
                520,
                180,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            let (lock, _cvar) = shared.as_ref();
            if let Ok(mut g) = lock.lock() {
                g.window_ready = true;
                drop(g);
            }
            eprintln!("[calibration] CreateWindowExW failed: {err}");
            return;
        }

        let static_class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let label_text: Vec<u16> =
            "Input latency calibration is running...\n\nKeep this window focused until calibration completes.\nDo not press any keys."
                .encode_utf16()
                .collect();
        unsafe {
            CreateWindowExW(
                0,
                static_class.as_ptr(),
                label_text.as_ptr(),
                WS_CHILD | WS_VISIBLE,
                20,
                20,
                460,
                100,
                hwnd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            ShowWindow(hwnd, SW_SHOW);
        }

        // Register Raw Input for keyboard on this window.
        let rid = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: 0x06, // HID_USAGE_GENERIC_KEYBOARD
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        // SAFETY: rid is a valid RAWINPUTDEVICE and remains alive for the call.
        let ok = unsafe {
            RegisterRawInputDevices(&rid, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32)
        };
        if ok == 0 {
            let err = unsafe { windows_sys::Win32::Foundation::GetLastError() };
            unsafe { DestroyWindow(hwnd) };
            let (lock, _cvar) = shared.as_ref();
            if let Ok(mut g) = lock.lock() {
                g.window_ready = true;
                drop(g);
            }
            eprintln!("[calibration] RegisterRawInputDevices failed: {err}");
            return;
        }

        // Publish hwnd and signal ready.
        {
            let (lock, cvar) = shared.as_ref();
            if let Ok(mut g) = lock.lock() {
                g.hwnd = hwnd as isize;
                g.window_ready = true;
                cvar.notify_all();
            }
        }

        // Install the thread-local context pointer.
        let ctx = PumpContext {
            shared: Arc::clone(&shared),
            input_buffer: std::cell::RefCell::new(Vec::with_capacity(4096)),
        };
        let ctx_ptr: *const PumpContext = &ctx;
        PUMP_STATE.with(|c| c.set(ctx_ptr));

        // Message loop.
        let mut msg = MSG {
            hwnd: std::ptr::null_mut(),
            message: 0,
            wParam: 0,
            lParam: 0,
            time: 0,
            pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
        };
        loop {
            // SAFETY: msg is a valid MSG out-parameter; filter is 0 so we
            // receive all messages for any window on this thread.
            let r = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if r == -1 {
                let (lock, cvar) = shared.as_ref();
                if let Ok(mut guard) = lock.lock() {
                    guard.pump_thread_failed = true;
                    cvar.notify_all();
                }
                break;
            }
            if r == 0 {
                break;
            }
            // SAFETY: msg is freshly filled by GetMessageW.
            unsafe { TranslateMessage(&msg) };
            // SAFETY: msg is freshly filled by GetMessageW.
            unsafe { DispatchMessageW(&msg) };
        }

        // Clear the thread-local pointer before the context goes out of scope.
        PUMP_STATE.with(|c| c.set(std::ptr::null()));
        // Clean up registration so other processes can register normally.
        let unregister = RAWINPUTDEVICE {
            usUsagePage: HID_USAGE_PAGE_GENERIC,
            usUsage: 0x06,
            dwFlags: windows_sys::Win32::UI::Input::RIDEV_REMOVE,
            hwndTarget: std::ptr::null_mut(),
        };
        // SAFETY: unregister is a valid RAWINPUTDEVICE.
        let unregister_ok = unsafe {
            RegisterRawInputDevices(&unregister, 1, std::mem::size_of::<RAWINPUTDEVICE>() as u32)
        } != 0;
        let restore_ok = restore_raw_input_devices(&previous_raw_input_devices);
        if !unregister_ok || !restore_ok {
            let (lock, cvar) = shared.as_ref();
            if let Ok(mut g) = lock.lock() {
                g.raw_input_restore_failed = true;
                cvar.notify_all();
            }
        }
        // SAFETY: hwnd is a live window owned by this thread.
        unsafe { DestroyWindow(hwnd) };
    }

    // ── CalibrationSession ────────────────────────────────────────────────────

    pub struct CalibrationSession {
        pub(crate) shared: Arc<(Mutex<SharedCalibState>, Condvar)>,
        hwnd: isize,
        pump_thread: Option<std::thread::JoinHandle<()>>,
        qpc_clock: QpcClock,
        next_sequence: u32,
        pub(crate) possibly_active_mask: u16,
        last_send_completed_ticks: Option<QpcTicks>,
        measurement_deadline: Option<QpcTicks>,
        precision_waiter: HybridWaiter,
        wait_interrupt: OwnedEvent,
        next_barrier_generation: u64,
    }

    fn stop_pump_on_startup_failure(
        shared: &Arc<(Mutex<SharedCalibState>, Condvar)>,
        hwnd: isize,
        handle: std::thread::JoinHandle<()>,
    ) {
        if let Ok(mut state) = shared.0.lock() {
            state.should_exit = true;
            shared.1.notify_all();
        }
        if hwnd != 0 {
            // SAFETY: the HWND was published by the pump thread and remains
            // valid until that thread processes the exit message.
            unsafe { PostMessageW(hwnd as HWND, WM_CALIB_EXIT, 0, 0) };
        }
        let _ = handle.join();
    }

    fn measurement_deadline_from_clock(
        qpc_clock: &QpcClock,
        budget_seconds: u64,
    ) -> Result<QpcTicks, CalibrationError> {
        let measurement_us = remaining_measurement_budget_us(budget_seconds, 0)?;
        let duration = qpc_clock
            .duration_from_us(measurement_us)
            .map_err(|_| CalibrationError::ClockFailure)?;
        let now = qpc_clock
            .now()
            .map_err(|_| CalibrationError::ClockFailure)?;
        now.checked_add_duration(duration)
            .map_err(|_| CalibrationError::ClockFailure)
    }

    fn remaining_measurement_budget_us(
        budget_seconds: u64,
        elapsed_us: u64,
    ) -> Result<u64, CalibrationError> {
        let budget_us = budget_seconds
            .checked_mul(1_000_000)
            .ok_or(CalibrationError::ClockFailure)?;
        let cleanup_reserve_us = CALIBRATION_CLEANUP_RESERVE_SECONDS.saturating_mul(1_000_000);
        budget_us
            .checked_sub(cleanup_reserve_us)
            .and_then(|measurement_us| measurement_us.checked_sub(elapsed_us))
            .ok_or(CalibrationError::BudgetExceeded)
    }

    fn global_measurement_deadline(budget_seconds: u64) -> Result<QpcTicks, CalibrationError> {
        let qpc_clock = QpcClock::initialize().map_err(|_| CalibrationError::ClockFailure)?;
        measurement_deadline_from_clock(&qpc_clock, budget_seconds)
    }

    impl CalibrationSession {
        pub fn open() -> Result<Self, CalibrationError> {
            Self::open_with_measurement_deadline(None)
        }

        fn open_with_measurement_deadline(
            measurement_deadline: Option<QpcTicks>,
        ) -> Result<Self, CalibrationError> {
            let qpc_clock = QpcClock::initialize().map_err(|_| CalibrationError::ClockFailure)?;
            let precision_waiter = HybridWaiter::production();
            if let Some(failure) = precision_waiter.initial_failure() {
                return Err(CalibrationError::PrecisionWaitFailed {
                    detail: format!("{failure:?}"),
                });
            }
            let wait_interrupt = OwnedEvent::new_auto_reset().ok_or_else(|| {
                CalibrationError::PrecisionWaitFailed {
                    detail: "could not create calibration wait interrupt".to_string(),
                }
            })?;
            let initial = SharedCalibState {
                pending_receipts: SmallVec::new(),
                active_sequence: None,
                active_expected_scan_codes: SmallVec::new(),
                active_expected_key_up: None,
                pump_diagnostics: PumpDiagnostics::default(),
                barrier_completed_generation: 0,
                correlation_boundary_lost: false,
                window_ready: false,
                should_exit: false,
                window_closed: false,
                foreground_lost: false,
                hwnd: 0,
                clock_failed: false,
                raw_input_restore_failed: false,
                pump_thread_failed: false,
            };
            let shared = Arc::new((Mutex::new(initial), Condvar::new()));
            let shared_clone = Arc::clone(&shared);

            let handle = std::thread::Builder::new()
                .name("sky-calib-pump".into())
                .spawn(move || pump_thread(shared_clone))
                .map_err(|_| CalibrationError::WindowThreadFailed)?;

            // Wait for the pump to signal window ready.
            let hwnd = {
                let (lock, cvar) = shared.as_ref();
                let guard = match lock.lock() {
                    Ok(guard) => guard,
                    Err(_) => {
                        stop_pump_on_startup_failure(&shared, 0, handle);
                        return Err(CalibrationError::WindowThreadFailed);
                    }
                };
                let guard = match cvar
                    .wait_timeout_while(guard, Duration::from_secs(5), |g| !g.window_ready)
                {
                    Ok((guard, _)) => guard,
                    Err(_) => {
                        stop_pump_on_startup_failure(&shared, 0, handle);
                        return Err(CalibrationError::WindowThreadFailed);
                    }
                };
                let hwnd = guard.hwnd;
                drop(guard);
                if hwnd == 0 {
                    stop_pump_on_startup_failure(&shared, hwnd, handle);
                    return Err(CalibrationError::WindowCreateFailed(0));
                }
                hwnd
            };

            let mut session = Self {
                shared,
                hwnd,
                pump_thread: Some(handle),
                qpc_clock,
                next_sequence: 1,
                possibly_active_mask: 0,
                last_send_completed_ticks: None,
                measurement_deadline,
                precision_waiter,
                wait_interrupt,
                next_barrier_generation: 0,
            };

            if let Err(err) = session.acquire_foreground(Duration::from_secs(5)) {
                let shared = Arc::clone(&session.shared);
                let hwnd = session.hwnd;
                if let Some(handle) = session.pump_thread.take() {
                    stop_pump_on_startup_failure(&shared, hwnd, handle);
                }
                return Err(err);
            }

            #[cfg(not(any(test, feature = "test-support")))]
            if let Err(err) = session.correlation_self_test() {
                // The probe is deliberately excluded from all statistics, but
                // a failed probe must still clean up before the session is
                // allowed to escape.
                let _ = session.close();
                return Err(err);
            }

            Ok(session)
        }

        /// Drain all WM_INPUT messages currently queued on the pump thread
        /// before arming the next packet. A stale or incomplete boundary is
        /// fail-closed because Raw Input does not preserve the optional tag.
        fn drain_pump_before_arm(&mut self) -> Result<(), CalibrationError> {
            {
                let (lock, _cvar) = self.shared.as_ref();
                let guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                if !can_arm_next_packet(&guard) {
                    return Err(CalibrationError::CorrelationBoundaryLost);
                }
            }
            self.next_barrier_generation = self
                .next_barrier_generation
                .checked_add(1)
                .ok_or(CalibrationError::SequenceOverflow)?;
            let generation = self.next_barrier_generation;
            if self.hwnd == 0
                || unsafe {
                    PostMessageW(self.hwnd as HWND, WM_CALIB_BARRIER, generation as WPARAM, 0)
                } == 0
            {
                return Err(CalibrationError::WindowThreadFailed);
            }
            let (lock, cvar) = self.shared.as_ref();
            let guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
            let (guard, timeout) = cvar
                .wait_timeout_while(guard, Duration::from_millis(100), |state| {
                    state.barrier_completed_generation < generation
                        && !state.pump_thread_failed
                        && !state.window_closed
                        && !state.correlation_boundary_lost
                })
                .map_err(|_| CalibrationError::StateLockFailed)?;
            if guard.correlation_boundary_lost {
                return Err(CalibrationError::CorrelationBoundaryLost);
            }
            if timeout.timed_out() || guard.pump_thread_failed || guard.window_closed {
                return Err(CalibrationError::WindowThreadFailed);
            }
            Ok(())
        }

        fn reset_pump_diagnostics(&self) -> Result<(), CalibrationError> {
            let (lock, _cvar) = self.shared.as_ref();
            let mut state = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
            state.pump_diagnostics = PumpDiagnostics::default();
            Ok(())
        }

        #[cfg(not(any(test, feature = "test-support")))]
        fn probe_failure_detail(
            &self,
            direction: &str,
            scan_codes: &[u16],
            sample: &CalibrationSample,
        ) -> String {
            let diagnostics = self
                .shared
                .0
                .lock()
                .map(|state| state.pump_diagnostics.clone())
                .unwrap_or_default();
            format_probe_failure_detail(direction, scan_codes, sample, &diagnostics)
        }

        #[cfg(not(any(test, feature = "test-support")))]
        fn probe_error_detail(
            &self,
            direction: &str,
            scan_codes: &[u16],
            error: &CalibrationError,
        ) -> String {
            let diagnostics = self
                .shared
                .0
                .lock()
                .map(|state| state.pump_diagnostics.clone())
                .unwrap_or_default();
            format_probe_error_detail(direction, scan_codes, error, &diagnostics)
        }

        fn wait_to_precision_boundary(
            &self,
            physical_target_qpc: QpcTicks,
        ) -> Result<(), CalibrationError> {
            self.ensure_budget()?;
            let handoff_ticks = self
                .qpc_clock
                .duration_from_us(CALIBRATION_PRECISION_HANDOFF_US)
                .map_err(|_| CalibrationError::ClockFailure)?;
            let handoff_target_qpc = QpcTicks::from_raw(
                physical_target_qpc
                    .as_u64()
                    .saturating_sub(handoff_ticks.as_u64()),
            );
            let result = self.precision_waiter.wait_until_ticks_with_metrics_typed(
                self.qpc_clock,
                handoff_target_qpc,
                DurationTicks::ZERO,
                &self.wait_interrupt,
            );
            match result.outcome {
                WaitOutcome::Deadline => Ok(()),
                WaitOutcome::Interrupted => Err(CalibrationError::PrecisionWaitFailed {
                    detail: "calibration wait was interrupted".to_string(),
                }),
                WaitOutcome::Failed(failure) => Err(CalibrationError::PrecisionWaitFailed {
                    detail: format!("{failure:?}"),
                }),
            }
        }

        #[cfg(not(any(test, feature = "test-support")))]
        fn correlation_self_test(&mut self) -> Result<(), CalibrationError> {
            let scan_codes = &PHYSICAL_INSTRUMENT_SCAN_CODES[..5];
            let timeout = Duration::from_millis(200);
            self.reset_pump_diagnostics()?;
            let down = match self.measure_packet(scan_codes, false, timeout) {
                Ok(sample) => sample,
                Err(error) => {
                    return Err(CalibrationError::CorrelationSelfTestFailed {
                        detail: self.probe_error_detail("Down", scan_codes, &error),
                    });
                }
            };
            if !down.is_complete() || down.anomalies.any() {
                return Err(CalibrationError::CorrelationSelfTestFailed {
                    detail: self.probe_failure_detail("Down", scan_codes, &down),
                });
            }
            self.reset_pump_diagnostics()?;
            let up = match self.measure_packet(scan_codes, true, timeout) {
                Ok(sample) => sample,
                Err(error) => {
                    return Err(CalibrationError::CorrelationSelfTestFailed {
                        detail: self.probe_error_detail("Up", scan_codes, &error),
                    });
                }
            };
            if !up.is_complete() || up.anomalies.any() {
                return Err(CalibrationError::CorrelationSelfTestFailed {
                    detail: self.probe_failure_detail("Up", scan_codes, &up),
                });
            }
            if self.last_send_completed_ticks.is_none() {
                return Err(CalibrationError::CorrelationSelfTestFailed {
                    detail: "probe did not produce a completion anchor".to_string(),
                });
            }
            let cleanup = self.cleanup_keyboard();
            if !cleanup.cleanup_success
                || cleanup.cleanup_verification_inconclusive
                || !cleanup.cleanup_stuck_keys.is_empty()
            {
                return Err(CalibrationError::CorrelationSelfTestFailed {
                    detail: format!("probe did not prove physical All-Up: {cleanup:?}"),
                });
            }
            Ok(())
        }

        pub fn acquire_foreground(&mut self, timeout: Duration) -> Result<(), CalibrationError> {
            let hwnd = self.hwnd as HWND;
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                SetForegroundWindow(hwnd);
                SetFocus(hwnd);
            }

            let start = std::time::Instant::now();
            let effective_timeout = if cfg!(test) {
                Duration::from_millis(50)
            } else {
                timeout
            };
            while start.elapsed() < effective_timeout {
                if check_foreground_owned(self.hwnd) {
                    return Ok(());
                }
                unsafe {
                    ShowWindow(hwnd, SW_SHOW);
                    SetForegroundWindow(hwnd);
                    SetFocus(hwnd);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if check_foreground_owned(self.hwnd) {
                Ok(())
            } else {
                Err(CalibrationError::ForegroundAcquireFailed)
            }
        }

        pub fn ensure_foreground_owned(&mut self) -> Result<(), CalibrationError> {
            self.ensure_budget()?;
            let (lock, _cvar) = self.shared.as_ref();
            let guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
            if guard.window_closed || guard.should_exit {
                return Err(CalibrationError::CalibrationWindowClosed);
            }
            if guard.foreground_lost {
                return Err(CalibrationError::ForegroundLost);
            }
            drop(guard);

            if !check_foreground_owned(self.hwnd) {
                let (lock, _cvar) = self.shared.as_ref();
                if let Ok(mut g) = lock.lock() {
                    g.foreground_lost = true;
                }
                return Err(CalibrationError::ForegroundLost);
            }
            Ok(())
        }

        pub fn set_measurement_deadline(&mut self, deadline: QpcTicks) {
            self.measurement_deadline = Some(deadline);
        }

        /// Return a QPC deadline after reserving time for final cleanup.
        pub fn measurement_deadline(
            &self,
            budget_seconds: u64,
        ) -> Result<QpcTicks, CalibrationError> {
            measurement_deadline_from_clock(&self.qpc_clock, budget_seconds)
        }

        pub fn budget_expired(&self, deadline: QpcTicks) -> Result<bool, CalibrationError> {
            Ok(self
                .qpc_clock
                .now()
                .map_err(|_| CalibrationError::ClockFailure)?
                >= deadline)
        }

        fn ensure_budget(&self) -> Result<(), CalibrationError> {
            if let Some(deadline) = self.measurement_deadline
                && self.budget_expired(deadline)?
            {
                return Err(CalibrationError::BudgetExceeded);
            }
            Ok(())
        }

        pub(crate) fn cleanup_keyboard(&mut self) -> CleanupOutcome {
            let attempted = PHYSICAL_INSTRUMENT_SCAN_CODES.to_vec();
            let expected = attempted.len() as u8;
            let mut cleanup_success = false;
            for _ in 0..3 {
                let result = send_input_raw(&attempted, true);
                if result.inserted == expected {
                    cleanup_success = true;
                    break;
                }
            }

            let mut stuck_keys = Vec::new();
            let mut verification_inconclusive = false;
            for &scan_code in &attempted {
                match crate::input::is_scan_code_physically_down(scan_code, 0) {
                    Some(true) => stuck_keys.push(scan_code),
                    Some(false) => {}
                    None => verification_inconclusive = true,
                }
            }
            self.possibly_active_mask = 0;
            CleanupOutcome {
                cleanup_attempted: true,
                cleanup_success: cleanup_success && stuck_keys.is_empty(),
                cleanup_stuck_keys: stuck_keys,
                cleanup_verification_inconclusive: verification_inconclusive,
                raw_input_restore_failed: false,
                pump_thread_failed: false,
            }
        }

        /// Inject one calibration packet and collect receipts.
        ///
        /// `scan_codes` must be a subset of `PHYSICAL_INSTRUMENT_SCAN_CODES`.
        /// `key_up` controls the direction. The caller is responsible for
        /// maintaining down/up balance across samples.
        #[allow(dead_code)]
        fn measure_packet(
            &mut self,
            scan_codes: &[u16],
            key_up: bool,
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            let target = self
                .qpc_clock
                .now()
                .map_err(|_| CalibrationError::ClockFailure)?;
            self.measure_packet_at_target(scan_codes, key_up, target, receipt_timeout)
        }

        /// Inject one tagged packet at an absolute QPC target. The complete
        /// tagged INPUT array is prepared before the `T - 700 µs` handoff;
        /// the sender owns the final target crossing and completion boundary.
        pub fn measure_packet_at_target(
            &mut self,
            scan_codes: &[u16],
            key_up: bool,
            physical_target_qpc: QpcTicks,
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            self.ensure_foreground_owned()?;
            self.reset_pump_diagnostics()?;

            let n = scan_codes.len();
            validate_packet_scan_codes(scan_codes)?;

            self.drain_pump_before_arm()?;

            let seq = self.next_sequence;
            if seq == 0 {
                return Err(CalibrationError::SequenceOverflow);
            }
            self.next_sequence = seq
                .checked_add(1)
                .ok_or(CalibrationError::SequenceOverflow)?;

            // Prepare extra_info with sequence tag.
            let extra =
                make_calibration_extra_info(seq).ok_or(CalibrationError::SequenceOverflow)?;

            // Arm the active sequence BEFORE injecting so we cannot miss any
            // receipt delivered before the mutex is acquired after SendInput.
            {
                let (lock, _cvar) = self.shared.as_ref();
                let mut g = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                g.active_sequence = Some(seq);
                g.active_expected_scan_codes.clear();
                g.active_expected_scan_codes.extend_from_slice(scan_codes);
                g.active_expected_key_up = Some(key_up);
                g.pending_receipts.clear();
            }

            // Materialize the tagged INPUT payload before entering the same
            // precision handoff used by production dispatch. Nothing after
            // the handoff may construct or allocate an INPUT payload.
            let prepared =
                crate::input::PreparedTaggedCalibrationPacket::try_new(scan_codes, key_up, extra)
                    .ok_or(CalibrationError::PolyphonyTooLarge(n))?;
            self.wait_to_precision_boundary(physical_target_qpc)?;
            // The shared sender owns the final QPC crossing, authoritative P,
            // one SendInput call, and completion C.
            let psr = prepared.send_at_target(self.qpc_clock, physical_target_qpc);
            let (call_started, call_completed) = match exact_sendinput_boundaries(&psr) {
                Ok(boundaries) => boundaries,
                Err(error) => {
                    let (lock, cvar) = self.shared.as_ref();
                    let mut guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                    invalidate_correlation_boundary(&mut guard);
                    cvar.notify_all();
                    return Err(error);
                }
            };
            self.last_send_completed_ticks = Some(call_completed);
            let expected = scan_codes.len() as u8;
            let partial_send = psr.inserted < expected;
            if !key_up {
                for &scan_code in scan_codes.iter().take(psr.inserted as usize) {
                    if let Some(index) = PHYSICAL_INSTRUMENT_SCAN_CODES
                        .iter()
                        .position(|&candidate| candidate == scan_code)
                    {
                        self.possibly_active_mask |= 1u16 << index;
                    }
                }
            }

            // A partial call is not evidence for a known prefix. Fail closed
            // for the whole requested packet and let session cleanup release
            // every instrument key before the process exits.
            if partial_send {
                let (lock, cvar) = self.shared.as_ref();
                let mut guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                invalidate_correlation_boundary(&mut guard);
                cvar.notify_all();
                return Err(CalibrationError::PacketIntegrity {
                    phase: "partial_send",
                    sequence_id: seq,
                    expected,
                    received: psr.inserted.min(expected),
                    win32_error: (psr.win32_error != 0).then_some(psr.win32_error),
                });
            }

            // Wait for expected receipts.
            let receipt_deadline = std::time::Instant::now() + receipt_timeout;
            let (first, last, count, anomalies, receipts) = {
                let (lock, cvar) = self.shared.as_ref();
                let mut guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                loop {
                    if guard.correlation_boundary_lost {
                        clear_active_packet(&mut guard);
                        cvar.notify_all();
                        return Err(CalibrationError::CorrelationBoundaryLost);
                    }
                    if guard.pump_thread_failed {
                        clear_active_packet(&mut guard);
                        cvar.notify_all();
                        return Err(CalibrationError::WindowThreadFailed);
                    }
                    if guard.window_closed {
                        clear_active_packet(&mut guard);
                        cvar.notify_all();
                        return Err(CalibrationError::CalibrationWindowClosed);
                    }
                    if guard.clock_failed {
                        clear_active_packet(&mut guard);
                        cvar.notify_all();
                        return Err(CalibrationError::ClockFailure);
                    }
                    if has_expected_receipts(&guard.pending_receipts, scan_codes, seq, key_up) {
                        break;
                    }
                    let receipt_remaining =
                        receipt_deadline.saturating_duration_since(std::time::Instant::now());
                    let budget_remaining = if let Some(budget_deadline) = self.measurement_deadline
                    {
                        let now = self
                            .qpc_clock
                            .now()
                            .map_err(|_| CalibrationError::ClockFailure)?;
                        let budget_remaining =
                            budget_deadline.checked_duration_since(now).map_err(|_| {
                                clear_active_packet(&mut guard);
                                cvar.notify_all();
                                CalibrationError::BudgetExceeded
                            })?;
                        let budget_remaining_us = self
                            .qpc_clock
                            .duration_to_us(budget_remaining)
                            .map_err(|_| CalibrationError::ClockFailure)?;
                        if budget_remaining_us == 0 {
                            clear_active_packet(&mut guard);
                            cvar.notify_all();
                            return Err(CalibrationError::BudgetExceeded);
                        }
                        Some(Duration::from_micros(budget_remaining_us))
                    } else {
                        None
                    };
                    let Some(remaining) =
                        receipt_wait_duration(receipt_remaining, budget_remaining)?
                    else {
                        break;
                    };
                    guard = cvar
                        .wait_timeout(guard, remaining)
                        .map_err(|_| CalibrationError::StateLockFailed)?
                        .0;
                }

                let receipts = std::mem::take(&mut guard.pending_receipts);
                clear_active_packet(&mut guard);
                cvar.notify_all();
                drop(guard);

                analyse_receipts(&receipts, scan_codes, seq, key_up)
            };

            if anomalies.timeout {
                let (lock, cvar) = self.shared.as_ref();
                let mut guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                invalidate_correlation_boundary(&mut guard);
                cvar.notify_all();
                return Err(CalibrationError::ReceiptTimeout {
                    sequence_id: seq,
                    expected,
                    received: count,
                });
            }

            if key_up && psr.inserted == expected {
                self.possibly_active_mask = 0;
            }

            Ok(CalibrationSample {
                sequence_id: seq,
                target_ticks: physical_target_qpc,
                call_started_ticks: call_started,
                call_completed_ticks: call_completed,
                first_receipt_ticks: first,
                last_receipt_ticks: last,
                receipt_count: count,
                expected_receipt_count: expected,
                win32_error: (psr.win32_error != 0).then_some(psr.win32_error),
                actual_idle_gap_ticks: None,
                observed_class: None,
                anomalies: SampleAnomalies {
                    partial_send,
                    ..anomalies
                },
                receipts,
            })
        }

        /// Measure one packet and classify it from the immediately previous
        /// exact SendInput completion. A mismatch is retained in the
        /// requested bucket only as rejected evidence.
        pub fn measure_classified_packet(
            &mut self,
            scan_codes: &[u16],
            key_up: bool,
            expected_class: SampleClass,
            cold_threshold: sky_dispatch_core::time::DurationTicks,
            physical_target_qpc: QpcTicks,
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            let previous_completion = self
                .last_send_completed_ticks
                .ok_or(CalibrationError::ClockFailure)?;
            let mut sample = self.measure_packet_at_target(
                scan_codes,
                key_up,
                physical_target_qpc,
                receipt_timeout,
            )?;
            let (observed_class, actual_idle_gap_ticks) = classify_idle_gap(
                previous_completion,
                sample.call_started_ticks,
                cold_threshold,
            )?;
            sample.observed_class = Some(observed_class);
            sample.actual_idle_gap_ticks = Some(actual_idle_gap_ticks);
            sample.anomalies.class_mismatch = observed_class != expected_class;
            Ok(sample)
        }

        pub fn close(mut self) -> CleanupOutcome {
            let mut cleanup = self.cleanup_keyboard();
            // Signal the pump thread to exit.
            if self.hwnd != 0 {
                // SAFETY: hwnd is a live message-only window on the pump thread.
                unsafe { PostMessageW(self.hwnd as HWND, WM_CALIB_EXIT, 0, 0) };
                self.hwnd = 0;
            }
            if let Some(h) = self.pump_thread.take()
                && h.join().is_err()
                && let Ok(mut state) = self.shared.0.lock()
            {
                state.pump_thread_failed = true;
            }
            let restore_failed = self
                .shared
                .0
                .lock()
                .map(|state| state.raw_input_restore_failed)
                .unwrap_or(true);
            cleanup.raw_input_restore_failed = restore_failed;
            cleanup.pump_thread_failed = self
                .shared
                .0
                .lock()
                .map(|state| state.pump_thread_failed)
                .unwrap_or(true);
            cleanup.cleanup_success &= !restore_failed;
            cleanup.cleanup_success &= !cleanup.pump_thread_failed;
            cleanup
        }
    }

    impl Drop for CalibrationSession {
        fn drop(&mut self) {
            let _ = self.cleanup_keyboard();
            if let Ok(mut state) = self.shared.0.lock() {
                state.should_exit = true;
                self.shared.1.notify_all();
            }
            if self.hwnd != 0 {
                unsafe {
                    PostMessageW(self.hwnd as HWND, WM_CALIB_EXIT, 0, 0);
                }
            }
            if let Some(h) = self.pump_thread.take()
                && h.join().is_err()
                && let Ok(mut state) = self.shared.0.lock()
            {
                state.pump_thread_failed = true;
            }
        }
    }

    fn has_expected_receipts(
        receipts: &[RawInputReceipt],
        expected_scan_codes: &[u16],
        expected_seq: u32,
        expected_key_up: bool,
    ) -> bool {
        expected_scan_codes.iter().all(|scan_code| {
            receipts.iter().any(|receipt| {
                receipt.sequence_id == expected_seq
                    && receipt.scan_code == *scan_code
                    && receipt.key_up == expected_key_up
                    && receipt.extended_flags == 0
            })
        })
    }

    fn analyse_receipts(
        receipts: &[RawInputReceipt],
        expected_scan_codes: &[u16],
        expected_seq: u32,
        expected_key_up: bool,
    ) -> (
        Option<QpcTicks>,
        Option<QpcTicks>,
        u8,
        SampleAnomalies,
        SmallVec<[RawInputReceipt; 15]>,
    ) {
        let mut anomalies = SampleAnomalies::default();
        let count = receipts.len().min(u8::MAX as usize) as u8;

        if count == 0 {
            anomalies.timeout = true;
            return (None, None, 0, anomalies, SmallVec::new());
        }
        let complete =
            has_expected_receipts(receipts, expected_scan_codes, expected_seq, expected_key_up);
        if !complete {
            anomalies.timeout = true;
        }

        // Detect unexpected scan codes.
        for r in receipts {
            if r.sequence_id != expected_seq {
                // Shouldn't happen due to sequence guard, but flag anyway.
                anomalies.unexpected_scan_code = true;
            }
            if !expected_scan_codes.contains(&r.scan_code) {
                anomalies.unexpected_scan_code = true;
            }
            if r.extended_flags != 0 {
                anomalies.unexpected_scan_code = true;
            }
            if r.key_up != expected_key_up {
                anomalies.direction_mismatch = true;
            }
        }

        // Detect duplicates: same scan code appearing more than once.
        for i in 0..receipts.len() {
            for j in (i + 1)..receipts.len() {
                if receipts[i].scan_code == receipts[j].scan_code
                    && receipts[i].sequence_id == receipts[j].sequence_id
                    && receipts[i].key_up == receipts[j].key_up
                    && receipts[i].extended_flags == receipts[j].extended_flags
                {
                    anomalies.duplicate_receipt = true;
                    break;
                }
            }
        }

        // Detect temporal reordering (not physically meaningful for scan
        // codes, but useful as a jitter diagnostic).
        for w in receipts.windows(2) {
            if w[1].arrived_ticks < w[0].arrived_ticks {
                anomalies.reordered_receipt = true;
                break;
            }
        }

        let first = receipts.iter().map(|r| r.arrived_ticks).min();
        let last = receipts.iter().map(|r| r.arrived_ticks).max();

        let last = if complete { last } else { None };

        (
            first,
            last,
            count,
            anomalies,
            receipts.iter().copied().collect(),
        )
    }

    fn pair_packets(
        down: CalibrationSample,
        up: CalibrationSample,
        expected_scan_codes: &[u16],
        down_idle_gap_ticks: sky_dispatch_core::time::DurationTicks,
        up_idle_gap_ticks: sky_dispatch_core::time::DurationTicks,
    ) -> Result<PairSample, CalibrationError> {
        let mut key_evidence = SmallVec::<[KeyShrinkEvidence; 15]>::new();
        let mut pairing_anomaly = false;
        let mut receipt_before_completion_count = 0u64;
        let mut worst_delivery: Option<i64> = None;
        let mut worst_total: Option<(i64, i64, i64, i64)> = None;
        let qpc_clock = QpcClock::initialize().map_err(|_| CalibrationError::ClockFailure)?;

        for &scan_code in expected_scan_codes {
            let down_matches: SmallVec<[RawInputReceipt; 2]> = down
                .receipts
                .iter()
                .copied()
                .filter(|receipt| receipt.scan_code == scan_code && !receipt.key_up)
                .collect();
            let up_matches: SmallVec<[RawInputReceipt; 2]> = up
                .receipts
                .iter()
                .copied()
                .filter(|receipt| receipt.scan_code == scan_code && receipt.key_up)
                .collect();
            if down_matches.len() != 1 || up_matches.len() != 1 {
                pairing_anomaly = true;
                continue;
            }

            let down_latency =
                signed_delta_us(down_matches[0].arrived_ticks, down.call_completed_ticks)?;
            let up_latency = signed_delta_us(up_matches[0].arrived_ticks, up.call_completed_ticks)?;
            if down_matches[0].arrived_ticks < down.call_completed_ticks {
                receipt_before_completion_count = receipt_before_completion_count.saturating_add(1);
            }
            if up_matches[0].arrived_ticks < up.call_completed_ticks {
                receipt_before_completion_count = receipt_before_completion_count.saturating_add(1);
            }
            let shrink = down_latency
                .checked_sub(up_latency)
                .ok_or(CalibrationError::StatisticsOverflow)?;
            let down_timing = PairedTimingPoint {
                target_ticks: down.target_ticks,
                pre_call_ticks: down.call_started_ticks,
                completion_ticks: down.call_completed_ticks,
                receipt_ticks: down_matches[0].arrived_ticks,
            };
            let up_timing = PairedTimingPoint {
                target_ticks: up.target_ticks,
                pre_call_ticks: up.call_started_ticks,
                completion_ticks: up.call_completed_ticks,
                receipt_ticks: up_matches[0].arrived_ticks,
            };
            let timing_ticks = match paired_timing_shrink_ticks(down_timing, up_timing) {
                Ok(value) => value,
                Err(_) => {
                    pairing_anomaly = true;
                    continue;
                }
            };
            let timing_us = paired_timing_shrink_us(qpc_clock, timing_ticks)?;
            worst_delivery = Some(worst_delivery.map_or(shrink, |current| current.max(shrink)));
            let candidate = (
                timing_us.total_proxy_shrink_us,
                timing_us.scheduler_shrink_us,
                timing_us.sendinput_shrink_us,
                timing_us.delivery_shrink_us,
            );
            if worst_total.is_none_or(|current| candidate.0 > current.0) {
                worst_total = Some(candidate);
            }
            key_evidence.push(KeyShrinkEvidence {
                scan_code,
                down_target_ticks: down.target_ticks.as_u64(),
                down_pre_call_ticks: down.call_started_ticks.as_u64(),
                down_completion_ticks: down.call_completed_ticks.as_u64(),
                down_receipt_ticks: down_matches[0].arrived_ticks.as_u64(),
                up_target_ticks: up.target_ticks.as_u64(),
                up_pre_call_ticks: up.call_started_ticks.as_u64(),
                up_completion_ticks: up.call_completed_ticks.as_u64(),
                up_receipt_ticks: up_matches[0].arrived_ticks.as_u64(),
                down_latency_us: down_latency,
                up_latency_us: up_latency,
                shrink_us: shrink,
                scheduler_shrink_us: timing_us.scheduler_shrink_us,
                sendinput_shrink_us: timing_us.sendinput_shrink_us,
                delivery_shrink_us: timing_us.delivery_shrink_us,
                total_proxy_shrink_us: timing_us.total_proxy_shrink_us,
            });
        }

        Ok(PairSample {
            down,
            up,
            down_idle_gap_ticks,
            up_idle_gap_ticks,
            pair_worst_shrink_us: worst_delivery,
            pair_worst_total_proxy_shrink_us: worst_total.map(|value| value.0),
            pair_worst_scheduler_shrink_us: worst_total.map(|value| value.1),
            pair_worst_sendinput_shrink_us: worst_total.map(|value| value.2),
            pair_worst_delivery_shrink_us: worst_total.map(|value| value.3),
            key_evidence,
            pairing_anomaly,
            receipt_before_completion_count,
        })
    }

    fn pair_sample_evidence(pair: &PairSample) -> Result<PairSampleEvidence, CalibrationError> {
        let down_receipt_us = pair
            .down
            .first_receipt_latency_us()?
            .map(|value| quantile_stats_i64(&[value]).expect("one-value quantile cannot overflow"));
        let up_receipt_us = pair
            .up
            .first_receipt_latency_us()?
            .map(|value| quantile_stats_i64(&[value]).expect("one-value quantile cannot overflow"));
        Ok(PairSampleEvidence {
            clean: pair.is_clean(),
            actual_down_gap_us: qpc_ticks_to_us(QpcTicks::from_raw(
                pair.down_idle_gap_ticks.as_u64(),
            ))
            .map_err(|_| CalibrationError::ClockFailure)?,
            actual_up_gap_us: qpc_ticks_to_us(QpcTicks::from_raw(pair.up_idle_gap_ticks.as_u64()))
                .map_err(|_| CalibrationError::ClockFailure)?,
            pair_worst_shrink_us: pair.pair_worst_shrink_us,
            pair_worst_total_proxy_shrink_us: pair.pair_worst_total_proxy_shrink_us,
            pair_worst_scheduler_shrink_us: pair.pair_worst_scheduler_shrink_us,
            pair_worst_sendinput_shrink_us: pair.pair_worst_sendinput_shrink_us,
            pair_worst_delivery_shrink_us: pair.pair_worst_delivery_shrink_us,
            key_evidence: pair.key_evidence.to_vec(),
            down_call_duration_us: pair.down.call_duration_us()?,
            up_call_duration_us: pair.up.call_duration_us()?,
            down_receipt_us,
            up_receipt_us,
            pairing_anomaly: pair.pairing_anomaly,
            receipt_before_completion_count: pair.receipt_before_completion_count,
            down_anomalies: pair.down.anomalies.clone(),
            up_anomalies: pair.up.anomalies.clone(),
        })
    }

    fn aggregate_pairs(pairs: &[PairSample]) -> Result<BucketStats, CalibrationError> {
        let clean_pairs: Vec<&PairSample> = pairs.iter().filter(|pair| pair.is_clean()).collect();
        let legacy_pair_values: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| pair.pair_worst_shrink_us)
            .collect();
        let total_pair_values: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| pair.pair_worst_total_proxy_shrink_us)
            .collect();
        let scheduler_values: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| pair.pair_worst_scheduler_shrink_us)
            .collect();
        let sendinput_values: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| pair.pair_worst_sendinput_shrink_us)
            .collect();
        let delivery_values: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| pair.pair_worst_delivery_shrink_us)
            .collect();
        let down_receipts: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| match pair.down.first_receipt_latency_us() {
                Ok(Some(value)) => Some(Ok(value)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<_, _>>()?;
        let up_receipts: Vec<i64> = clean_pairs
            .iter()
            .filter_map(|pair| match pair.up.first_receipt_latency_us() {
                Ok(Some(value)) => Some(Ok(value)),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<Result<_, _>>()?;
        let down_calls: Vec<u64> = clean_pairs
            .iter()
            .map(|pair| pair.down.call_duration_us())
            .collect::<Result<_, _>>()?;
        let up_calls: Vec<u64> = clean_pairs
            .iter()
            .map(|pair| pair.up.call_duration_us())
            .collect::<Result<_, _>>()?;
        let attempted = pairs.len() as u64;
        let clean = clean_pairs.len() as u64;
        let rejected = attempted.saturating_sub(clean);
        let anomaly_count = pairs
            .iter()
            .filter(|pair| {
                pair.pairing_anomaly || pair.down.anomalies.any() || pair.up.anomalies.any()
            })
            .count() as u64;
        let timeout_count = pairs
            .iter()
            .filter(|pair| pair.down.anomalies.timeout || pair.up.anomalies.timeout)
            .count() as u64;
        let class_mismatch_count = pairs
            .iter()
            .filter(|pair| pair.down.anomalies.class_mismatch || pair.up.anomalies.class_mismatch)
            .count() as u64;
        let partial_send = pairs
            .iter()
            .filter(|pair| pair.down.anomalies.partial_send || pair.up.anomalies.partial_send)
            .count() as u64;
        let pairing_anomaly_count = pairs.iter().filter(|pair| pair.pairing_anomaly).count() as u64;
        let duplicate_receipt_count = pairs
            .iter()
            .filter(|pair| {
                pair.down.anomalies.duplicate_receipt || pair.up.anomalies.duplicate_receipt
            })
            .count() as u64;
        let unexpected_scan_code_count = pairs
            .iter()
            .filter(|pair| {
                pair.down.anomalies.unexpected_scan_code || pair.up.anomalies.unexpected_scan_code
            })
            .count() as u64;
        let direction_mismatch_count = pairs
            .iter()
            .filter(|pair| {
                pair.down.anomalies.direction_mismatch || pair.up.anomalies.direction_mismatch
            })
            .count() as u64;
        let reordered_receipt_count = pairs
            .iter()
            .filter(|pair| {
                pair.down.anomalies.reordered_receipt || pair.up.anomalies.reordered_receipt
            })
            .count() as u64;
        let receipt_before_completion_count = pairs
            .iter()
            .map(|pair| pair.receipt_before_completion_count)
            .sum();
        let legacy_pair_stats = if legacy_pair_values.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&legacy_pair_values)?)
        };
        let total_pair_stats = if total_pair_values.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&total_pair_values)?)
        };
        let scheduler_stats = if scheduler_values.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&scheduler_values)?)
        };
        let sendinput_stats = if sendinput_values.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&sendinput_values)?)
        };
        let delivery_stats = if delivery_values.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&delivery_values)?)
        };
        let down_stats = if down_receipts.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&down_receipts)?)
        };
        let up_stats = if up_receipts.is_empty() {
            None
        } else {
            Some(quantile_stats_i64(&up_receipts)?)
        };
        Ok(BucketStats {
            attempted,
            clean,
            clean_sample_count: clean,
            rejected,
            partial_send,
            sample_count: attempted,
            timeout_count,
            anomaly_count,
            pairing_anomaly_count,
            duplicate_receipt_count,
            unexpected_scan_code_count,
            direction_mismatch_count,
            reordered_receipt_count,
            class_mismatch_count,
            receipt_before_completion_count,
            down_call_duration_us: quantile_stats_u64(&down_calls)?,
            up_call_duration_us: quantile_stats_u64(&up_calls)?,
            pair_worst_shrink_us: legacy_pair_stats,
            pair_worst_total_proxy_shrink_us: total_pair_stats,
            scheduler_shrink_us: scheduler_stats,
            sendinput_shrink_us: sendinput_stats,
            delivery_shrink_us: delivery_stats,
            down_receipt_us: down_stats,
            up_receipt_us: up_stats,
        })
    }

    fn compact_worst_pairs(
        pairs: &[PairSample],
        limit: usize,
    ) -> Result<Vec<PairSampleEvidence>, CalibrationError> {
        let mut evidence = pairs
            .iter()
            .map(pair_sample_evidence)
            .collect::<Result<Vec<_>, _>>()?;
        evidence.sort_by_key(|sample| {
            std::cmp::Reverse(sample.pair_worst_total_proxy_shrink_us.unwrap_or(i64::MIN))
        });
        evidence.truncate(limit);
        Ok(evidence)
    }

    fn anomalous_pair_evidence(
        pairs: &[PairSample],
    ) -> Result<Vec<PairSampleEvidence>, CalibrationError> {
        let mut evidence = pairs
            .iter()
            .filter(|pair| !pair.is_clean())
            .map(pair_sample_evidence)
            .collect::<Result<Vec<_>, _>>()?;
        evidence.truncate(MAX_ANOMALOUS_PAIR_EVIDENCE);
        Ok(evidence)
    }

    // ── Host fingerprint ──────────────────────────────────────────────────────

    pub fn build_host_fingerprint() -> Result<HostFingerprint, CalibrationError> {
        use crate::clock::qpc_frequency;
        let freq = qpc_frequency();
        let sampled_at_us = qpc_now_ticks_checked()
            .map_err(|_| CalibrationError::ClockFailure)
            .and_then(|ticks| qpc_ticks_to_us(ticks).map_err(|_| CalibrationError::ClockFailure))?;
        let win32_build = windows_build_string();
        let (processor_architecture, cpu_vendor, cpu_family, cpu_model, cpu_stepping) =
            cpu_identity();
        let (logical_processor_count, processor_group_count) = processor_topology();
        let cpu_set_efficiency_classes = cpu_set_efficiency_histogram();
        let (lowest_efficiency_class, highest_efficiency_class) =
            efficiency_class_bounds(&cpu_set_efficiency_classes);
        Ok(HostFingerprint {
            host_fingerprint_version: HOST_FINGERPRINT_VERSION,
            qpc_frequency_hz: freq,
            win32_build,
            processor_architecture,
            cpu_vendor,
            cpu_family,
            cpu_model,
            cpu_stepping,
            logical_processor_count,
            processor_group_count,
            cpu_set_efficiency_classes,
            highest_efficiency_class,
            lowest_efficiency_class,
            sampled_at_us,
        })
    }

    fn cpu_identity() -> (String, String, u32, u32, u32) {
        #[cfg(target_arch = "x86_64")]
        {
            let leaf0 = core::arch::x86_64::__cpuid(0);
            let vendor_bytes = [
                leaf0.ebx.to_le_bytes(),
                leaf0.edx.to_le_bytes(),
                leaf0.ecx.to_le_bytes(),
            ]
            .concat();
            let vendor = String::from_utf8_lossy(&vendor_bytes).to_string();
            if leaf0.eax < 1 {
                return ("x86_64".to_string(), vendor, 0, 0, 0);
            }
            let leaf1 = core::arch::x86_64::__cpuid(1);
            let base_family = (leaf1.eax >> 8) & 0x0f;
            let extended_family = (leaf1.eax >> 20) & 0xff;
            let family = if base_family == 0x0f {
                base_family + extended_family
            } else {
                base_family
            };
            let base_model = (leaf1.eax >> 4) & 0x0f;
            let extended_model = (leaf1.eax >> 16) & 0x0f;
            let model = if base_family == 0x06 || base_family == 0x0f {
                base_model + (extended_model << 4)
            } else {
                base_model
            };
            (
                "x86_64".to_string(),
                vendor,
                family,
                model,
                leaf1.eax & 0x0f,
            )
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            (
                std::env::consts::ARCH.to_string(),
                "unknown".to_string(),
                0,
                0,
                0,
            )
        }
    }

    fn processor_topology() -> (u32, u16) {
        use windows_sys::Win32::System::Threading::{
            ALL_PROCESSOR_GROUPS, GetActiveProcessorCount, GetActiveProcessorGroupCount,
        };
        (
            unsafe { GetActiveProcessorCount(ALL_PROCESSOR_GROUPS) },
            unsafe { GetActiveProcessorGroupCount() },
        )
    }

    fn cpu_set_efficiency_histogram() -> Vec<u32> {
        use windows_sys::Win32::System::SystemInformation::{
            CpuSetInformation, GetSystemCpuSetInformation, SYSTEM_CPU_SET_INFORMATION,
        };
        let mut required = 0u32;
        // SAFETY: the null buffer is the documented size-query form.
        let _ = unsafe {
            GetSystemCpuSetInformation(
                std::ptr::null_mut(),
                0,
                &mut required,
                std::ptr::null_mut(),
                0,
            )
        };
        if required == 0 {
            return Vec::new();
        }
        let mut buffer = vec![0u8; required as usize];
        let mut returned = required;
        // SAFETY: buffer is writable and exactly the size returned by the
        // preceding query; the API does not retain the pointer.
        let ok = unsafe {
            GetSystemCpuSetInformation(
                buffer.as_mut_ptr().cast(),
                returned,
                &mut returned,
                std::ptr::null_mut(),
                0,
            )
        } != 0;
        if !ok {
            return Vec::new();
        }
        let header_size = std::mem::size_of::<SYSTEM_CPU_SET_INFORMATION>();
        let mut histogram: Vec<u32> = Vec::new();
        let mut offset = 0usize;
        while offset.saturating_add(header_size) <= returned as usize {
            // SAFETY: offset is advanced by each validated record size and
            // the header is within the returned byte count.
            let info = unsafe {
                &*(buffer
                    .as_ptr()
                    .add(offset)
                    .cast::<SYSTEM_CPU_SET_INFORMATION>())
            };
            if info.Type == CpuSetInformation {
                // SAFETY: the record type selects the CpuSet union member.
                let efficiency = unsafe { info.Anonymous.CpuSet.EfficiencyClass } as usize;
                if histogram.len() <= efficiency {
                    histogram.resize(efficiency + 1, 0);
                }
                histogram[efficiency] = histogram[efficiency].saturating_add(1);
            }
            let size = info.Size as usize;
            if size < header_size {
                break;
            }
            offset = offset.saturating_add(size);
        }
        histogram
    }

    fn efficiency_class_bounds(histogram: &[u32]) -> (Option<u8>, Option<u8>) {
        let lowest = histogram
            .iter()
            .position(|count| *count != 0)
            .and_then(|value| u8::try_from(value).ok());
        let highest = histogram
            .iter()
            .rposition(|count| *count != 0)
            .and_then(|value| u8::try_from(value).ok());
        (lowest, highest)
    }

    fn windows_build_string() -> Option<String> {
        use windows_sys::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
        let mut info = OSVERSIONINFOW {
            dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOW>() as u32,
            dwMajorVersion: 0,
            dwMinorVersion: 0,
            dwBuildNumber: 0,
            dwPlatformId: 0,
            szCSDVersion: [0u16; 128],
        };
        // SAFETY: info is a valid out-parameter of the correct size.
        if unsafe { GetVersionExW(&mut info) } != 0 {
            Some(format!(
                "{}.{}.{}",
                info.dwMajorVersion, info.dwMinorVersion, info.dwBuildNumber
            ))
        } else {
            None
        }
    }

    // ── Top-level run function ────────────────────────────────────────────────

    /// Run a complete calibration and return the aggregated output.
    fn pair_bucket_failure(
        class: SampleClass,
        polyphony: u8,
        sample_index: u32,
        phase: &'static str,
        source: CalibrationError,
        cleanup: CleanupOutcome,
    ) -> CalibrationError {
        CalibrationError::BucketFailed {
            report: Box::new(CalibrationFailureReport {
                kind: "pair".to_string(),
                class: format!("{class:?}").to_lowercase(),
                polyphony,
                sample_index,
                phase: phase.to_string(),
                exact_error: source.to_string(),
                win32_error: source.win32_error(),
                cleanup_success: cleanup.cleanup_success,
                cleanup_stuck_keys: cleanup.cleanup_stuck_keys,
                cleanup_verification_inconclusive: cleanup.cleanup_verification_inconclusive,
                raw_input_restore_failed: cleanup.raw_input_restore_failed,
                pump_thread_failed: cleanup.pump_thread_failed,
            }),
        }
    }

    enum BalancedPairAction {
        Measure { key_up: bool, target_qpc: QpcTicks },
    }

    fn balanced_pair_measurements<S>(
        down_target: QpcTicks,
        gap_ticks: DurationTicks,
        mut step: S,
    ) -> Result<(CalibrationSample, CalibrationSample), CalibrationError>
    where
        S: FnMut(BalancedPairAction) -> Result<Option<CalibrationSample>, CalibrationError>,
    {
        let down = step(BalancedPairAction::Measure {
            key_up: false,
            target_qpc: down_target,
        })?
        .ok_or(CalibrationError::ClockFailure)?;
        let up_target = down
            .call_completed_ticks
            .checked_add_duration(gap_ticks)
            .map_err(|_| CalibrationError::ClockFailure)?;
        let up = step(BalancedPairAction::Measure {
            key_up: true,
            target_qpc: up_target,
        })?
        .ok_or(CalibrationError::ClockFailure)?;
        Ok((down, up))
    }

    fn run_balanced_pair(
        session: &mut CalibrationSession,
        scan_codes: &[u16],
        class: SampleClass,
        gap_us: u64,
        cold_threshold_ticks: sky_dispatch_core::time::DurationTicks,
        receipt_timeout: Duration,
    ) -> Result<PairSample, CalibrationError> {
        let previous_completion = session
            .last_send_completed_ticks
            .ok_or(CalibrationError::ClockFailure)?;
        let gap_ticks = session
            .qpc_clock
            .duration_from_us(gap_us)
            .map_err(|_| CalibrationError::ClockFailure)?;
        let down_target = previous_completion
            .checked_add_duration(gap_ticks)
            .map_err(|_| CalibrationError::ClockFailure)?;
        let (down, up) =
            balanced_pair_measurements(down_target, gap_ticks, |action| match action {
                BalancedPairAction::Measure { key_up, target_qpc } => session
                    .measure_classified_packet(
                        scan_codes,
                        key_up,
                        class,
                        cold_threshold_ticks,
                        target_qpc,
                        receipt_timeout,
                    )
                    .map(Some),
            })?;
        let down_gap = down
            .actual_idle_gap_ticks
            .ok_or(CalibrationError::ClockFailure)?;
        let up_gap = up
            .actual_idle_gap_ticks
            .ok_or(CalibrationError::ClockFailure)?;
        pair_packets(down, up, scan_codes, down_gap, up_gap)
    }

    pub fn run_calibration_pair_bucket(
        config: &CalibrationConfig,
        class: SampleClass,
    ) -> Result<CalibrationPairBucketOutput, CalibrationError> {
        run_calibration_pair_bucket_with_deadline(config, class, None)
    }

    fn run_calibration_pair_bucket_with_deadline(
        config: &CalibrationConfig,
        class: SampleClass,
        global_deadline: Option<QpcTicks>,
    ) -> Result<CalibrationPairBucketOutput, CalibrationError> {
        super::validate_calibration_config(config)?;
        if config.polyphonies.len() != 1 {
            return Err(CalibrationError::PolyphonyTooLarge(
                config.polyphonies.len(),
            ));
        }
        let polyphony = config.polyphonies[0];
        let mmcss = MmcssGuard::acquire(PriorityMode::Auto);
        let power = PowerThrottlingGuard::disable_current_thread();
        let mut session = match global_deadline {
            Some(deadline) => CalibrationSession::open_with_measurement_deadline(Some(deadline))?,
            None => CalibrationSession::open()?,
        };
        let scheduling_aids = SchedulingAidProvenance {
            mmcss_acquired: mmcss.acquired(),
            mmcss_active: mmcss.is_active(),
            power_throttling_active: power.is_active(),
            waiter_mode: session.precision_waiter.mode(),
        };
        let measurement_deadline = global_deadline
            .map(Ok)
            .unwrap_or_else(|| session.measurement_deadline(config.budget_seconds))?;
        session.set_measurement_deadline(measurement_deadline);
        let scan_codes = &PHYSICAL_INSTRUMENT_SCAN_CODES[..polyphony as usize];
        let receipt_timeout = Duration::from_millis(config.receipt_timeout_ms as u64);
        let cold_threshold_ticks = session
            .qpc_clock
            .duration_from_us(config.cold_threshold_us)
            .map_err(|_| CalibrationError::ClockFailure)?;
        let gap_us = match class {
            SampleClass::Hot => config.hot_gap_target_us,
            SampleClass::Cold => config.cold_idle_gap_us,
        };

        let mut warmup_rejected = 0u64;
        for warmup_index in 0..config.warmup_samples {
            if session.budget_expired(measurement_deadline)? {
                let cleanup = session.close();
                return Err(pair_bucket_failure(
                    class,
                    polyphony,
                    warmup_index + 1,
                    "warmup",
                    CalibrationError::BudgetExceeded,
                    cleanup,
                ));
            }
            let warmup = run_balanced_pair(
                &mut session,
                scan_codes,
                class,
                gap_us,
                cold_threshold_ticks,
                receipt_timeout,
            );
            match warmup {
                Ok(pair) => {
                    if !pair.is_clean() {
                        warmup_rejected = warmup_rejected.saturating_add(1);
                    }
                }
                Err(source) => {
                    let cleanup = session.close();
                    return Err(pair_bucket_failure(
                        class,
                        polyphony,
                        warmup_index + 1,
                        "warmup",
                        source,
                        cleanup,
                    ));
                }
            }
        }

        let expected = match class {
            SampleClass::Hot => config.samples_per_hot_bucket,
            SampleClass::Cold => config.samples_per_cold_bucket,
        };
        let max_attempts = expected.saturating_mul(CALIBRATION_MAX_ATTEMPT_MULTIPLIER);
        let mut pairs = Vec::with_capacity(expected as usize);
        let mut clean_pairs = 0u32;
        let mut attempt_index = 0u32;
        while clean_pairs < expected && attempt_index < max_attempts {
            attempt_index = attempt_index.saturating_add(1);
            eprintln!(
                "[calibration] polyphony {} / {:?} — attempt {} / {} (clean {} / {})",
                polyphony, class, attempt_index, max_attempts, clean_pairs, expected
            );
            if session.budget_expired(measurement_deadline)? {
                let cleanup = session.close();
                return Err(pair_bucket_failure(
                    class,
                    polyphony,
                    attempt_index,
                    "measurement",
                    CalibrationError::BudgetExceeded,
                    cleanup,
                ));
            }
            match run_balanced_pair(
                &mut session,
                scan_codes,
                class,
                gap_us,
                cold_threshold_ticks,
                receipt_timeout,
            ) {
                Ok(pair) => {
                    if pair.is_clean() {
                        clean_pairs = clean_pairs.saturating_add(1);
                    }
                    pairs.push(pair);
                }
                Err(source) => {
                    let cleanup = session.close();
                    return Err(pair_bucket_failure(
                        class,
                        polyphony,
                        attempt_index,
                        "measurement",
                        source,
                        cleanup,
                    ));
                }
            }
        }

        let cleanup = session.close();
        if !cleanup.cleanup_success || cleanup.cleanup_verification_inconclusive {
            return Err(pair_bucket_failure(
                class,
                polyphony,
                expected,
                "cleanup",
                CalibrationError::CleanupFailed {
                    stuck_keys: cleanup.cleanup_stuck_keys.clone(),
                },
                cleanup,
            ));
        }
        if clean_pairs < expected {
            eprintln!(
                "[calibration] polyphony {} / {:?} — only {} / {} clean pairs after {} attempts",
                polyphony, class, clean_pairs, expected, attempt_index
            );
        }
        let pair_bucket = aggregate_pairs(&pairs)?;
        Ok(CalibrationPairBucketOutput {
            version: CALIBRATION_SCHEMA_VERSION,
            measurement_protocol_version: MEASUREMENT_PROTOCOL_VERSION,
            source_git_sha: env!("SKY_NATIVE_BUILD_COMMIT"),
            native_build_id: env!("SKY_NATIVE_BUILD_COMMIT"),
            dirty_worktree: env!("SKY_NATIVE_DIRTY_WORKTREE") == "true",
            native_source_fingerprint: env!("SKY_NATIVE_SOURCE_FINGERPRINT"),
            rustc_version: env!("SKY_RUSTC_VERSION"),
            evidence_kind: CALIBRATION_EVIDENCE_KIND,
            host_fingerprint: build_host_fingerprint()?,
            scheduling_aids,
            configuration: config.clone(),
            class,
            polyphony,
            attempted_pairs: pairs.len() as u64,
            warmup_pairs: config.warmup_samples as u64,
            warmup_rejected,
            pair_bucket,
            worst_pairs: compact_worst_pairs(&pairs, 16)?,
            anomalous_pairs: anomalous_pair_evidence(&pairs)?,
            cleanup,
        })
    }

    pub fn run_calibration(
        config: &CalibrationConfig,
    ) -> Result<CalibrationOutput, CalibrationError> {
        super::validate_calibration_config(config)?;
        // Quick mode is one process and one calibration run. Establish the
        // measurement deadline once so each of the six buckets consumes the
        // same global budget instead of resetting a fresh 120-second window.
        let global_deadline = global_measurement_deadline(config.budget_seconds)?;
        let mut pair_buckets = HashMap::new();
        let mut warmup_attempted = 0u64;
        let mut warmup_anomalous = 0u64;
        let mut measured_attempted = 0u64;
        let mut measured_anomalous = 0u64;
        let mut measured_timed_out = 0u64;
        let mut measured_class_mismatch = 0u64;
        let mut anomalous_pairs = HashMap::new();
        let mut scheduling_aids: Option<SchedulingAidProvenance> = None;
        let mut cleanup = CleanupOutcome {
            cleanup_attempted: true,
            cleanup_success: true,
            cleanup_stuck_keys: Vec::new(),
            cleanup_verification_inconclusive: false,
            raw_input_restore_failed: false,
            pump_thread_failed: false,
        };

        for &polyphony in &config.polyphonies {
            for class in [SampleClass::Hot, SampleClass::Cold] {
                let mut bucket_config = config.clone();
                bucket_config.polyphonies = vec![polyphony];
                let bucket = run_calibration_pair_bucket_with_deadline(
                    &bucket_config,
                    class,
                    Some(global_deadline),
                )?;
                if let Some(expected) = scheduling_aids.as_ref() {
                    if expected != &bucket.scheduling_aids {
                        return Err(CalibrationError::SchedulingAidProvenanceMismatch);
                    }
                } else {
                    scheduling_aids = Some(bucket.scheduling_aids.clone());
                }
                let stats = bucket.pair_bucket.clone();
                warmup_attempted = warmup_attempted.saturating_add(bucket.warmup_pairs);
                warmup_anomalous = warmup_anomalous.saturating_add(bucket.warmup_rejected);
                measured_attempted = measured_attempted.saturating_add(bucket.attempted_pairs);
                measured_anomalous = measured_anomalous.saturating_add(stats.anomaly_count);
                measured_timed_out = measured_timed_out.saturating_add(stats.timeout_count);
                measured_class_mismatch =
                    measured_class_mismatch.saturating_add(stats.class_mismatch_count);
                cleanup.cleanup_success &= bucket.cleanup.cleanup_success;
                cleanup.raw_input_restore_failed |= bucket.cleanup.raw_input_restore_failed;
                cleanup.pump_thread_failed |= bucket.cleanup.pump_thread_failed;
                pair_buckets
                    .entry(polyphony)
                    .or_insert_with(HashMap::new)
                    .insert(format!("{class:?}").to_lowercase(), stats);
                anomalous_pairs
                    .entry(polyphony)
                    .or_insert_with(HashMap::new)
                    .insert(format!("{class:?}").to_lowercase(), bucket.anomalous_pairs);
            }
        }
        let total_attempted = warmup_attempted.saturating_add(measured_attempted);
        let total_anomalous = warmup_anomalous.saturating_add(measured_anomalous);
        let total_timed_out = measured_timed_out;
        Ok(CalibrationOutput {
            version: CALIBRATION_SCHEMA_VERSION,
            measurement_protocol_version: MEASUREMENT_PROTOCOL_VERSION,
            source_git_sha: env!("SKY_NATIVE_BUILD_COMMIT"),
            native_build_id: env!("SKY_NATIVE_BUILD_COMMIT"),
            dirty_worktree: env!("SKY_NATIVE_DIRTY_WORKTREE") == "true",
            native_source_fingerprint: env!("SKY_NATIVE_SOURCE_FINGERPRINT"),
            rustc_version: env!("SKY_RUSTC_VERSION"),
            evidence_kind: CALIBRATION_EVIDENCE_KIND,
            host_fingerprint: build_host_fingerprint()?,
            scheduling_aids: scheduling_aids
                .ok_or(CalibrationError::SchedulingAidProvenanceMismatch)?,
            configuration: config.clone(),
            pair_buckets,
            anomalous_pairs,
            warmup_attempted,
            measured_attempted,
            setup_attempted: 0,
            setup_anomalous: 0,
            setup_timed_out: 0,
            total_attempted,
            warmup_anomalous,
            measured_anomalous,
            total_anomalous,
            warmup_timed_out: 0,
            measured_timed_out,
            measured_class_mismatch,
            total_timed_out,
            cleanup,
        })
    }

    #[cfg(test)]
    mod pair_tests {
        use super::*;
        use windows_sys::Win32::UI::WindowsAndMessaging::PM_NOREMOVE;

        fn raw_keyboard_fixture(
            raw_type: u32,
            flags: u16,
            extra_information: usize,
        ) -> (Vec<usize>, usize) {
            let payload_size = std::mem::offset_of!(RAWINPUT, data)
                + std::mem::size_of::<windows_sys::Win32::UI::Input::RAWKEYBOARD>();
            let word_size = std::mem::size_of::<usize>();
            let mut storage = vec![0usize; payload_size.div_ceil(word_size)];
            // SAFETY: `storage` is zeroed, usize-aligned storage large enough
            // for the RAWINPUT header and keyboard union arm.
            let raw = unsafe { &mut *storage.as_mut_ptr().cast::<RAWINPUT>() };
            raw.header.dwType = raw_type;
            raw.header.dwSize = payload_size as u32;
            raw.data.keyboard.MakeCode = 30;
            raw.data.keyboard.Flags = flags;
            raw.data.keyboard.ExtraInformation = extra_information as u32;
            (storage, payload_size)
        }

        fn raw_bytes(storage: &[usize]) -> &[u8] {
            // SAFETY: usize storage is aligned and the byte slice remains
            // borrowed from the original storage.
            unsafe {
                std::slice::from_raw_parts(
                    storage.as_ptr().cast::<u8>(),
                    std::mem::size_of_val(storage),
                )
            }
        }

        #[test]
        fn raw_input_parser_accepts_minimum_keyboard_payload() {
            let (storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let parsed = parse_raw_keyboard_input(raw_bytes(&storage), bytes_read).unwrap();
            assert_eq!(parsed.scan_code, 30);
            assert_eq!(parsed.flags, 0);
            assert_eq!(parsed.extra_information, 0);
        }

        #[test]
        fn raw_input_parser_rejects_truncated_header_and_keyboard_payload() {
            let (storage, _bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let bytes = raw_bytes(&storage);
            assert_eq!(
                parse_raw_keyboard_input(bytes, std::mem::size_of::<RAWINPUTHEADER>() - 1),
                Err(RawInputParseError::TruncatedHeader)
            );
            let (mut truncated_storage, truncated_bytes) =
                raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let truncated_raw = unsafe { &mut *truncated_storage.as_mut_ptr().cast::<RAWINPUT>() };
            truncated_raw.header.dwSize = (truncated_bytes - 1) as u32;
            assert_eq!(
                parse_raw_keyboard_input(raw_bytes(&truncated_storage), truncated_bytes - 1),
                Err(RawInputParseError::TruncatedKeyboardPayload)
            );

            let (mut storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let raw = unsafe { &mut *storage.as_mut_ptr().cast::<RAWINPUT>() };
            raw.header.dwSize = std::mem::size_of::<RAWINPUTHEADER>() as u32 - 1;
            assert_eq!(
                parse_raw_keyboard_input(raw_bytes(&storage), bytes_read),
                Err(RawInputParseError::InvalidHeaderSize)
            );

            let (mut storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let raw = unsafe { &mut *storage.as_mut_ptr().cast::<RAWINPUT>() };
            raw.header.dwSize = std::mem::offset_of!(RAWINPUT, data) as u32;
            assert_eq!(
                parse_raw_keyboard_input(raw_bytes(&storage), bytes_read),
                Err(RawInputParseError::TruncatedKeyboardPayload)
            );
        }

        #[test]
        fn raw_input_parser_rejects_unaligned_storage() {
            let (storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let source = raw_bytes(&storage);
            let alignment = std::mem::align_of::<RAWINPUTHEADER>();
            let mut unaligned = vec![0u8; source.len() + alignment];
            let base = unaligned.as_ptr() as usize;
            let offset = (0..alignment)
                .find(|candidate| !(base + candidate).is_multiple_of(alignment))
                .expect("an unaligned byte offset");
            unaligned[offset..offset + source.len()].copy_from_slice(source);
            assert_eq!(
                parse_raw_keyboard_input(&unaligned[offset..], bytes_read),
                Err(RawInputParseError::Misaligned)
            );
        }

        #[test]
        fn raw_input_parser_classifies_non_keyboard_input() {
            let (storage, bytes_read) = raw_keyboard_fixture(0, 0, 0);
            assert_eq!(
                parse_raw_keyboard_input(raw_bytes(&storage), bytes_read),
                Err(RawInputParseError::NonKeyboard { raw_type: 0 })
            );
        }

        #[test]
        fn raw_input_parser_preserves_extended_flags() {
            let (storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0x0006, 0);
            let parsed = parse_raw_keyboard_input(raw_bytes(&storage), bytes_read).unwrap();
            assert_eq!(parsed.flags & (0x0002 | 0x0004), 0x0006);
        }

        #[test]
        fn raw_input_parser_reports_tag_present_absent_and_undecodable() {
            let tagged = make_calibration_extra_info(7).unwrap();
            let (storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, tagged);
            let parsed = parse_raw_keyboard_input(raw_bytes(&storage), bytes_read).unwrap();
            assert_eq!(
                calibration_extra_info_sequence(parsed.extra_information),
                Some(7)
            );

            let (storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0);
            let parsed = parse_raw_keyboard_input(raw_bytes(&storage), bytes_read).unwrap();
            assert_eq!(
                calibration_extra_info_sequence(parsed.extra_information),
                None
            );

            let (storage, bytes_read) = raw_keyboard_fixture(RIM_TYPEKEYBOARD, 0, 0xAB00_0001);
            let parsed = parse_raw_keyboard_input(raw_bytes(&storage), bytes_read).unwrap();
            assert_eq!(
                calibration_extra_info_sequence(parsed.extra_information),
                None
            );
        }

        #[test]
        fn optional_sequence_tag_cannot_admit_stale_packets() {
            assert!(tagged_sequence_matches_active(7, Some(7)));
            assert!(tagged_sequence_matches_active(7, None));
            assert!(!tagged_sequence_matches_active(7, Some(6)));
        }

        #[test]
        fn global_budget_remaining_is_not_reset_per_bucket() {
            assert_eq!(remaining_measurement_budget_us(6, 0).unwrap(), 1_000_000);
            assert_eq!(
                remaining_measurement_budget_us(120, 110_000_000).unwrap(),
                5_000_000
            );
            assert_eq!(
                remaining_measurement_budget_us(120, 115_000_000).unwrap(),
                0
            );
            assert!(matches!(
                remaining_measurement_budget_us(120, 115_000_001),
                Err(CalibrationError::BudgetExceeded)
            ));
        }

        fn sample(receipts: SmallVec<[RawInputReceipt; 15]>) -> CalibrationSample {
            let first = receipts.iter().map(|receipt| receipt.arrived_ticks).min();
            let last = receipts.iter().map(|receipt| receipt.arrived_ticks).max();
            CalibrationSample {
                sequence_id: 7,
                target_ticks: QpcTicks::from_raw(0),
                call_started_ticks: QpcTicks::from_raw(1),
                call_completed_ticks: QpcTicks::from_raw(10_000_000),
                first_receipt_ticks: first,
                last_receipt_ticks: last,
                receipt_count: receipts.len() as u8,
                expected_receipt_count: receipts.len() as u8,
                win32_error: None,
                actual_idle_gap_ticks: None,
                observed_class: Some(SampleClass::Hot),
                anomalies: SampleAnomalies::default(),
                receipts,
            }
        }

        #[test]
        fn correlation_failure_detail_contains_bounded_observer_diagnostics() {
            let mut failed = sample(SmallVec::new());
            failed.expected_receipt_count = 5;
            let diagnostics = PumpDiagnostics {
                wm_input_seen: 5,
                tag_decode_failed: 5,
                ..PumpDiagnostics::default()
            };
            let detail =
                format_probe_failure_detail("Down", &[30, 31, 32, 33, 34], &failed, &diagnostics);
            assert!(detail.contains("expected_receipt_count=5"));
            assert!(detail.contains("accepted_receipt_count=0"));
            assert!(detail.contains("expected_scan_codes=[30, 31, 32, 33, 34]"));
            assert!(detail.contains("tag_decode_failed: 5"));
        }

        #[test]
        fn correlation_error_detail_keeps_observer_diagnostics() {
            let diagnostics = PumpDiagnostics {
                wm_input_seen: 3,
                raw_read_failed: 1,
                ..PumpDiagnostics::default()
            };
            let detail = format_probe_error_detail(
                "Up",
                &[30, 31, 32, 33, 34],
                &CalibrationError::WindowThreadFailed,
                &diagnostics,
            );
            assert!(detail.contains("expected_receipt_count=5"));
            assert!(detail.contains("accepted_receipt_count=0"));
            assert!(detail.contains("raw_read_failed: 1"));
        }

        #[test]
        fn receipt_direction_mismatch_is_anomaly() {
            let receipts: SmallVec<[RawInputReceipt; 15]> = smallvec::smallvec![RawInputReceipt {
                arrived_ticks: QpcTicks::from_raw(100),
                scan_code: 30,
                sequence_id: 7,
                key_up: true,
                extended_flags: 0,
            }];
            let (_, _, _, anomalies, _) = analyse_receipts(&receipts, &[30], 7, false);
            assert!(anomalies.direction_mismatch);
            assert!(anomalies.any());
        }

        #[test]
        fn sequence_mismatch_is_not_silently_correlated() {
            let receipts: SmallVec<[RawInputReceipt; 15]> = smallvec::smallvec![RawInputReceipt {
                arrived_ticks: QpcTicks::from_raw(100),
                scan_code: 30,
                sequence_id: 8,
                key_up: false,
                extended_flags: 0,
            }];
            let (_, _, _, anomalies, _) = analyse_receipts(&receipts, &[30], 7, false);
            assert!(anomalies.unexpected_scan_code);
        }

        fn receipt(
            scan_code: u16,
            sequence_id: u32,
            key_up: bool,
            arrived: u64,
        ) -> RawInputReceipt {
            RawInputReceipt {
                arrived_ticks: QpcTicks::from_raw(arrived),
                scan_code,
                sequence_id,
                key_up,
                extended_flags: 0,
            }
        }

        #[test]
        fn collector_completion_requires_each_unique_expected_receipt() {
            let expected = [30, 31, 32, 33, 34];
            let complete: Vec<RawInputReceipt> = expected
                .iter()
                .enumerate()
                .map(|(index, scan)| receipt(*scan, 7, false, 100 + index as u64))
                .collect();
            assert!(has_expected_receipts(&complete, &expected, 7, false));

            let mut duplicate = complete[..4].to_vec();
            duplicate.push(receipt(30, 7, false, 200));
            assert!(!has_expected_receipts(&duplicate, &expected, 7, false));

            duplicate.push(receipt(34, 7, false, 201));
            assert!(has_expected_receipts(&duplicate, &expected, 7, false));
            let (_, _, _, anomalies, _) = analyse_receipts(&duplicate, &expected, 7, false);
            assert!(anomalies.duplicate_receipt);
        }

        #[test]
        fn collector_completion_rejects_wrong_direction_scan_and_stale_sequence() {
            let expected = [30, 31];
            let wrong_direction = vec![receipt(30, 7, false, 100), receipt(31, 7, true, 101)];
            assert!(!has_expected_receipts(
                &wrong_direction,
                &expected,
                7,
                false
            ));

            let unexpected_scan = vec![receipt(30, 7, false, 100), receipt(99, 7, false, 101)];
            assert!(!has_expected_receipts(
                &unexpected_scan,
                &expected,
                7,
                false
            ));

            let stale_sequence = vec![receipt(30, 6, false, 100), receipt(31, 7, false, 101)];
            assert!(!has_expected_receipts(&stale_sequence, &expected, 7, false));
        }

        #[test]
        fn packet_identity_rejects_duplicate_scan_codes() {
            assert!(matches!(
                validate_packet_scan_codes(&[0x15, 0x15]),
                Err(CalibrationError::DuplicateScanCode { scan_code: 0x15 })
            ));
            assert!(validate_packet_scan_codes(&[0x15, 0x16]).is_ok());
        }

        fn empty_shared_state() -> Arc<(Mutex<SharedCalibState>, Condvar)> {
            Arc::new((
                Mutex::new(SharedCalibState {
                    pending_receipts: SmallVec::new(),
                    active_sequence: None,
                    active_expected_scan_codes: SmallVec::new(),
                    active_expected_key_up: None,
                    pump_diagnostics: PumpDiagnostics::default(),
                    barrier_completed_generation: 0,
                    correlation_boundary_lost: false,
                    window_ready: false,
                    should_exit: false,
                    window_closed: false,
                    foreground_lost: false,
                    hwnd: 0,
                    clock_failed: false,
                    raw_input_restore_failed: false,
                    pump_thread_failed: false,
                }),
                Condvar::new(),
            ))
        }

        fn active_shared_state() -> Arc<(Mutex<SharedCalibState>, Condvar)> {
            let shared = empty_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            state.active_sequence = Some(7);
            state.active_expected_scan_codes.push(30);
            state.active_expected_key_up = Some(false);
            drop(state);
            shared
        }

        #[test]
        fn posted_barrier_handler_rejects_pending_wm_input_before_completion() {
            let mut seed = MSG {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            };
            // SAFETY: `seed` is a valid message output record. This creates a
            // queue for the current test thread before PostThreadMessageW.
            unsafe {
                PeekMessageW(&mut seed, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            }
            // SAFETY: the current thread owns the queue and the posted
            // message is a sink-style WM_INPUT used only for queue testing.
            let posted = unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW(
                    windows_sys::Win32::System::Threading::GetCurrentThreadId(),
                    WM_INPUT,
                    1,
                    0,
                )
            };
            assert_ne!(posted, 0);

            let shared = empty_shared_state();
            let ctx = PumpContext {
                shared: Arc::clone(&shared),
                input_buffer: std::cell::RefCell::new(Vec::new()),
            };
            PUMP_STATE.with(|cell| cell.set(&ctx));
            // SAFETY: `ctx` remains alive while wnd_proc reads the thread-local
            // pointer, and the sink-style message has no raw handle to read.
            unsafe { wnd_proc(std::ptr::null_mut(), WM_CALIB_BARRIER, 9, 0) };
            PUMP_STATE.with(|cell| cell.set(std::ptr::null()));

            let state = shared.0.lock().expect("shared calibration state");
            assert_eq!(state.barrier_completed_generation, 0);
            assert_eq!(state.pump_diagnostics.wm_input_seen, 1);
            assert_eq!(state.pump_diagnostics.stale_sequence, 1);
            assert!(state.correlation_boundary_lost);
            assert!(!state.pump_thread_failed);
            drop(state);

            let mut leftover = MSG {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            };
            // SAFETY: `leftover` is a valid output record and the filter only
            // checks whether the barrier left a WM_INPUT queued.
            let remaining = unsafe {
                PeekMessageW(
                    &mut leftover,
                    std::ptr::null_mut(),
                    WM_INPUT,
                    WM_INPUT,
                    PM_NOREMOVE,
                )
            };
            assert_eq!(remaining, 0);
        }

        #[test]
        fn timeout_boundary_state_forbids_next_packet_arm() {
            let shared = empty_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            assert!(can_arm_next_packet(&state));
            invalidate_correlation_boundary(&mut state);
            assert!(!can_arm_next_packet(&state));
            assert!(state.active_sequence.is_none());
        }

        #[test]
        fn idle_stale_receipt_invalidates_boundary() {
            let shared = empty_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            observe_stale_correlation_evidence(&mut state);
            assert_eq!(state.pump_diagnostics.stale_sequence, 1);
            assert!(state.correlation_boundary_lost);
            assert!(!can_arm_next_packet(&state));
        }

        #[test]
        fn mismatched_tag_invalidates_boundary() {
            let shared = empty_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            state.active_sequence = Some(9);
            assert!(!tagged_sequence_matches_active(9, Some(8)));
            observe_stale_correlation_evidence(&mut state);
            assert_eq!(state.pump_diagnostics.stale_sequence, 1);
            assert!(state.correlation_boundary_lost);
            assert!(state.active_sequence.is_none());
        }

        #[test]
        fn diagnostic_reset_does_not_restore_boundary_trust() {
            let shared = empty_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            invalidate_correlation_boundary(&mut state);
            state.pump_diagnostics = PumpDiagnostics::default();
            assert!(!can_arm_next_packet(&state));
        }

        #[test]
        fn wrong_direction_invalidates_boundary() {
            let shared = active_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            assert!(observe_incompatible_receipt(
                &mut state,
                receipt(30, 7, true, 100)
            ));
            assert_eq!(state.pump_diagnostics.wrong_direction, 1);
            assert_eq!(state.pump_diagnostics.stale_sequence, 1);
            assert!(state.correlation_boundary_lost);
            assert!(state.pending_receipts.is_empty());
        }

        #[test]
        fn unexpected_identity_invalidates_boundary() {
            let shared = active_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            assert!(observe_incompatible_receipt(
                &mut state,
                receipt(31, 7, false, 100)
            ));
            assert_eq!(state.pump_diagnostics.unexpected_identity, 1);
            assert!(state.correlation_boundary_lost);
            assert!(state.pending_receipts.is_empty());
        }

        #[test]
        fn duplicate_receipt_invalidates_boundary() {
            let shared = active_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            let existing = receipt(30, 7, false, 100);
            state.pending_receipts.push(existing);
            assert!(observe_incompatible_receipt(&mut state, existing));
            assert_eq!(state.pump_diagnostics.duplicate_receipt, 1);
            assert!(state.correlation_boundary_lost);
            assert_eq!(state.pending_receipts.len(), 1);
        }

        #[test]
        fn pending_overflow_invalidates_boundary() {
            let shared = active_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            state.pending_receipts.extend(
                (0..MAX_PENDING_RECEIPTS)
                    .map(|index| receipt(100 + index as u16, 7, false, 100 + index as u64)),
            );
            assert!(observe_incompatible_receipt(
                &mut state,
                receipt(30, 7, false, 1_000)
            ));
            assert_eq!(state.pump_diagnostics.pending_receipt_overflow, 1);
            assert!(state.correlation_boundary_lost);
            assert_eq!(state.pending_receipts.len(), MAX_PENDING_RECEIPTS);
        }

        #[test]
        fn class_mismatch_does_not_invalidate_boundary() {
            let shared = active_shared_state();
            let mut state = shared.0.lock().expect("shared calibration state");
            let receipt = receipt(30, 7, false, 100);
            assert!(!observe_incompatible_receipt(&mut state, receipt));
            state.pending_receipts.clear();
            let anomalies = SampleAnomalies {
                class_mismatch: true,
                ..SampleAnomalies::default()
            };
            assert!(anomalies.class_mismatch);
            assert!(!state.correlation_boundary_lost);
            assert_eq!(state.active_sequence, Some(7));
        }

        #[test]
        fn barrier_preserves_wm_quit_and_does_not_complete_generation() {
            let mut seed = MSG {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            };
            // SAFETY: `seed` is a valid message output record and creates the
            // current test thread's message queue.
            unsafe {
                PeekMessageW(&mut seed, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
                windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(17);
            }

            let shared = empty_shared_state();
            let ctx = PumpContext {
                shared: Arc::clone(&shared),
                input_buffer: std::cell::RefCell::new(Vec::new()),
            };
            PUMP_STATE.with(|cell| cell.set(&ctx));
            // SAFETY: `ctx` remains alive while wnd_proc reads the thread-local
            // pointer; the barrier must not reinterpret WM_QUIT as WM_INPUT.
            unsafe { wnd_proc(std::ptr::null_mut(), WM_CALIB_BARRIER, 11, 0) };
            PUMP_STATE.with(|cell| cell.set(std::ptr::null()));

            let state = shared.0.lock().expect("shared calibration state");
            assert_eq!(state.barrier_completed_generation, 0);
            assert!(state.window_closed);
            assert!(state.should_exit);
            assert!(!state.correlation_boundary_lost);
            drop(state);

            let mut quit = MSG {
                hwnd: std::ptr::null_mut(),
                message: 0,
                wParam: 0,
                lParam: 0,
                time: 0,
                pt: windows_sys::Win32::Foundation::POINT { x: 0, y: 0 },
            };
            // SAFETY: `quit` is a valid output record; consume the reposted
            // quit so it cannot affect the test harness thread.
            let found = unsafe { PeekMessageW(&mut quit, std::ptr::null_mut(), 0, 0, PM_REMOVE) };
            assert_ne!(found, 0);
            assert_eq!(quit.message, WM_QUIT);
            assert_eq!(quit.wParam, 17);
        }

        #[test]
        fn extended_e0_and_e1_flags_never_alias_instrument_keys() {
            for extended_flags in [0x02, 0x04] {
                let receipts: SmallVec<[RawInputReceipt; 15]> =
                    smallvec::smallvec![RawInputReceipt {
                        arrived_ticks: QpcTicks::from_raw(100),
                        scan_code: 30,
                        sequence_id: 7,
                        key_up: false,
                        extended_flags,
                    }];
                let (_, _, _, anomalies, _) = analyse_receipts(&receipts, &[30], 7, false);
                assert!(anomalies.unexpected_scan_code);
                assert!(anomalies.any());
            }
        }

        #[test]
        fn balanced_pair_waits_from_each_exact_completion_in_order() {
            let mut events: Vec<String> = Vec::new();
            let result = balanced_pair_measurements(
                QpcTicks::from_raw(100),
                DurationTicks::from_raw(100),
                |action| match action {
                    BalancedPairAction::Measure { key_up, target_qpc } => {
                        events.push(format!(
                            "measure_{}:{}",
                            if key_up { "up" } else { "down" },
                            target_qpc.as_u64()
                        ));
                        let mut measured = sample(SmallVec::new());
                        measured.target_ticks = target_qpc;
                        measured.call_completed_ticks = if key_up {
                            QpcTicks::from_raw(400)
                        } else {
                            QpcTicks::from_raw(250)
                        };
                        Ok(Some(measured))
                    }
                },
            )
            .unwrap();
            assert_eq!(result.0.call_completed_ticks, QpcTicks::from_raw(250));
            assert_eq!(result.1.call_completed_ticks, QpcTicks::from_raw(400));
            assert_eq!(events, vec!["measure_down:100", "measure_up:350"]);
        }

        #[test]
        fn pair_matching_uses_scan_code_and_preserves_signed_shrink() {
            let down = sample(smallvec::smallvec![
                RawInputReceipt {
                    arrived_ticks: QpcTicks::from_raw(12_000_000),
                    scan_code: 31,
                    sequence_id: 7,
                    key_up: false,
                    extended_flags: 0,
                },
                RawInputReceipt {
                    arrived_ticks: QpcTicks::from_raw(11_000_000),
                    scan_code: 30,
                    sequence_id: 7,
                    key_up: false,
                    extended_flags: 0,
                },
            ]);
            let mut up = sample(smallvec::smallvec![
                RawInputReceipt {
                    arrived_ticks: QpcTicks::from_raw(21_000_000),
                    scan_code: 31,
                    sequence_id: 7,
                    key_up: true,
                    extended_flags: 0,
                },
                RawInputReceipt {
                    arrived_ticks: QpcTicks::from_raw(22_000_000),
                    scan_code: 30,
                    sequence_id: 7,
                    key_up: true,
                    extended_flags: 0,
                },
            ]);
            up.call_completed_ticks = QpcTicks::from_raw(20_000_000);

            let pair = pair_packets(
                down,
                up,
                &[30, 31],
                sky_dispatch_core::time::DurationTicks::from_raw(1),
                sky_dispatch_core::time::DurationTicks::from_raw(1),
            )
            .unwrap();

            assert_eq!(
                pair.key_evidence
                    .iter()
                    .map(|evidence| evidence.scan_code)
                    .collect::<Vec<_>>(),
                vec![30, 31]
            );
            assert!(pair.key_evidence[0].shrink_us < 0);
            assert!(pair.key_evidence[1].shrink_us > 0);
            assert_eq!(
                pair.pair_worst_shrink_us,
                Some(pair.key_evidence[1].shrink_us)
            );
            assert!(pair.is_clean());
        }
    }
} // mod platform

// ─── Non-Windows stub ─────────────────────────────────────────────────────────

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn run_calibration(
        _config: &CalibrationConfig,
    ) -> Result<CalibrationOutput, CalibrationError> {
        Err(CalibrationError::PlatformUnsupported)
    }

    pub fn run_calibration_pair_bucket(
        _config: &CalibrationConfig,
        _class: SampleClass,
    ) -> Result<CalibrationPairBucketOutput, CalibrationError> {
        Err(CalibrationError::PlatformUnsupported)
    }

    pub fn build_host_fingerprint() -> Result<HostFingerprint, CalibrationError> {
        Ok(HostFingerprint {
            host_fingerprint_version: HOST_FINGERPRINT_VERSION,
            qpc_frequency_hz: 0,
            win32_build: None,
            processor_architecture: std::env::consts::ARCH.to_string(),
            cpu_vendor: "unknown".to_string(),
            cpu_family: 0,
            cpu_model: 0,
            cpu_stepping: 0,
            logical_processor_count: 0,
            processor_group_count: 0,
            cpu_set_efficiency_classes: Vec::new(),
            highest_efficiency_class: None,
            lowest_efficiency_class: None,
            sampled_at_us: 0,
        })
    }
}

// ─── Aggregation helpers ──────────────────────────────────────────────────────

fn quantile_stats_u64(values: &[u64]) -> Result<QuantileStats, CalibrationError> {
    if values.is_empty() {
        return Ok(QuantileStats {
            min: 0,
            p50: 0,
            p90: 0,
            p95: 0,
            p99: 0,
            max: 0,
            mean: 0,
        });
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let percentile = |p: usize| {
        let idx = ((n * p).saturating_add(99) / 100)
            .saturating_sub(1)
            .min(n - 1);
        sorted[idx]
    };
    let sum = sorted.iter().try_fold(0_u64, |sum, value| {
        sum.checked_add(*value)
            .ok_or(CalibrationError::StatisticsOverflow)
    })?;
    let mean = sum / n as u64;
    Ok(QuantileStats {
        min: sorted[0],
        p50: percentile(50),
        p90: percentile(90),
        p95: percentile(95),
        p99: percentile(99),
        max: *sorted.last().unwrap(),
        mean,
    })
}

fn quantile_stats_i64(values: &[i64]) -> Result<SignedQuantileStats, CalibrationError> {
    if values.is_empty() {
        return Ok(SignedQuantileStats {
            min: 0,
            p50: 0,
            p90: 0,
            p95: 0,
            p99: 0,
            max: 0,
            mean: 0,
        });
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let percentile = |p: usize| {
        let idx = ((n * p).saturating_add(99) / 100)
            .saturating_sub(1)
            .min(n - 1);
        sorted[idx]
    };
    let sum = sorted.iter().try_fold(0_i64, |sum, value| {
        sum.checked_add(*value)
            .ok_or(CalibrationError::StatisticsOverflow)
    })?;
    let mean = sum / n as i64;
    Ok(SignedQuantileStats {
        min: sorted[0],
        p50: percentile(50),
        p90: percentile(90),
        p95: percentile(95),
        p99: percentile(99),
        max: *sorted.last().unwrap(),
        mean,
    })
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Run a complete calibration and return JSON-serialised output.
///
/// This is the primary entry point for callers (including the PyO3 wrapper in
/// `sky_player_rs`).  It blocks the calling thread for the duration of the
/// calibration run.
///
/// # Errors
///
/// Returns [`CalibrationError::PlatformUnsupported`] on non-Windows.
pub fn run_calibration_json(config: &CalibrationConfig) -> Result<String, CalibrationError> {
    let output = platform::run_calibration(config)?;
    serde_json::to_string_pretty(&output).map_err(|_e| {
        // JSON serialisation failure is an internal bug — surface it as a
        // descriptive error.  CalibrationError does not have a generic JSON
        // variant, so reuse WindowCreateFailed with u32::MAX as a sentinel.
        CalibrationError::WindowCreateFailed(u32::MAX)
    })
}

pub fn run_calibration_pair_bucket_json(
    config: &CalibrationConfig,
    class: SampleClass,
) -> Result<String, CalibrationError> {
    let output = platform::run_calibration_pair_bucket(config, class)?;
    serde_json::to_string_pretty(&output)
        .map_err(|_e| CalibrationError::WindowCreateFailed(u32::MAX))
}

pub use platform::build_host_fingerprint;

// ─── Unit tests (non-Windows stubs and pure logic) ───────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sky_dispatch_core::time::DurationTicks;

    #[test]
    fn quantile_stats_single_value() {
        let stats = quantile_stats_u64(&[42]).unwrap();
        assert_eq!(stats.min, 42);
        assert_eq!(stats.max, 42);
        assert_eq!(stats.p50, 42);
        assert_eq!(stats.mean, 42);
    }

    #[test]
    fn quantile_stats_empty_is_zero() {
        let stats = quantile_stats_u64(&[]).unwrap();
        assert_eq!(stats.min, 0);
        assert_eq!(stats.max, 0);
    }

    #[test]
    fn signed_quantile_stats_negative() {
        let stats = quantile_stats_i64(&[-10, -5, 0, 5, 10]).unwrap();
        assert_eq!(stats.min, -10);
        assert_eq!(stats.max, 10);
        assert_eq!(stats.p50, 0);
    }

    #[test]
    fn signed_delta_us_positive() {
        let a = QpcTicks::from_raw(1_000_000);
        let b = QpcTicks::from_raw(500_000);
        // a > b so delta should be positive
        let delta = signed_delta_us(a, b).unwrap();
        assert!(delta >= 0);
    }

    #[test]
    fn signed_delta_us_negative() {
        let a = QpcTicks::from_raw(500_000);
        let b = QpcTicks::from_raw(1_000_000);
        // a < b so delta should be negative
        let delta = signed_delta_us(a, b).unwrap();
        assert!(delta <= 0);
    }

    #[test]
    #[cfg(not(windows))]
    fn non_windows_returns_unsupported() {
        let config = CalibrationConfig::default();
        let result = run_calibration_json(&config);
        assert!(matches!(result, Err(CalibrationError::PlatformUnsupported)));
    }

    #[test]
    fn default_config_polyphonies() {
        let cfg = CalibrationConfig::default();
        assert_eq!(cfg.polyphonies, vec![1, 2, 3, 5, 8, 15]);
    }

    #[test]
    fn calibration_schema_and_gap_defaults_are_single_contract() {
        let cfg = CalibrationConfig::quick();
        assert_eq!(CALIBRATION_SCHEMA_VERSION, 13);
        assert_eq!(MEASUREMENT_PROTOCOL_VERSION, 8);
        assert_eq!(HOST_FINGERPRINT_VERSION, 2);
        assert_eq!(CALIBRATION_PRECISION_HANDOFF_US, 700);
        assert_eq!(CALIBRATION_MAX_ATTEMPT_MULTIPLIER, 2);
        assert_eq!(CALIBRATION_CLEANUP_RESERVE_SECONDS, 5);
        assert_eq!(CALIBRATION_MIN_TOTAL_BUDGET_SECONDS, 6);
        assert_eq!(cfg.hot_gap_target_us, 5_000);
        assert_eq!(cfg.cold_threshold_us, 20_000);
        assert_eq!(cfg.cold_idle_gap_us, 25_000);
    }

    #[test]
    fn scheduling_aid_provenance_serializes_runtime_labels() {
        let provenance = SchedulingAidProvenance {
            mmcss_acquired: "mmcss:Games",
            mmcss_active: true,
            power_throttling_active: true,
            waiter_mode: "event+high_resolution_timer",
        };
        let value = serde_json::to_value(provenance).expect("scheduling provenance JSON");
        assert_eq!(value["mmcss_acquired"], "mmcss:Games");
        assert_eq!(value["mmcss_active"], true);
        assert_eq!(value["power_throttling_active"], true);
        assert_eq!(value["waiter_mode"], "event+high_resolution_timer");
    }

    fn timing_point(
        target: u64,
        pre_call: u64,
        completion: u64,
        receipt: u64,
    ) -> PairedTimingPoint {
        PairedTimingPoint {
            target_ticks: QpcTicks::from_raw(target),
            pre_call_ticks: QpcTicks::from_raw(pre_call),
            completion_ticks: QpcTicks::from_raw(completion),
            receipt_ticks: QpcTicks::from_raw(receipt),
        }
    }

    #[test]
    fn paired_total_shrink_matches_scheduler_component() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 300, 400, 500),
            timing_point(1_100, 1_200, 1_300, 1_400),
        )
        .unwrap();
        assert_eq!(shrink.scheduler_shrink_ticks, 100);
        assert_eq!(shrink.sendinput_shrink_ticks, 0);
        assert_eq!(shrink.delivery_shrink_ticks, 0);
        assert_eq!(shrink.total_proxy_shrink_ticks, 100);
    }

    #[test]
    fn paired_total_shrink_matches_sendinput_component() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 200, 500, 600),
            timing_point(1_100, 1_200, 1_400, 1_500),
        )
        .unwrap();
        assert_eq!(shrink.scheduler_shrink_ticks, 0);
        assert_eq!(shrink.sendinput_shrink_ticks, 100);
        assert_eq!(shrink.delivery_shrink_ticks, 0);
        assert_eq!(shrink.total_proxy_shrink_ticks, 100);
    }

    #[test]
    fn paired_total_shrink_matches_delivery_component() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 200, 300, 700),
            timing_point(1_100, 1_200, 1_300, 1_600),
        )
        .unwrap();
        assert_eq!(shrink.scheduler_shrink_ticks, 0);
        assert_eq!(shrink.sendinput_shrink_ticks, 0);
        assert_eq!(shrink.delivery_shrink_ticks, 100);
        assert_eq!(shrink.total_proxy_shrink_ticks, 100);
    }

    #[test]
    fn paired_total_shrink_has_no_shrink_case() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 110, 120, 130),
            timing_point(1_100, 1_110, 1_120, 1_130),
        )
        .unwrap();
        assert_eq!(shrink.scheduler_shrink_ticks, 0);
        assert_eq!(shrink.sendinput_shrink_ticks, 0);
        assert_eq!(shrink.delivery_shrink_ticks, 0);
        assert_eq!(shrink.total_proxy_shrink_ticks, 0);
    }

    #[test]
    fn paired_total_shrink_preserves_mixed_signed_components() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 140, 170, 230),
            timing_point(1_100, 1_120, 1_155, 1_195),
        )
        .unwrap();
        assert_eq!(shrink.scheduler_shrink_ticks, 20);
        assert_eq!(shrink.sendinput_shrink_ticks, -5);
        assert_eq!(shrink.delivery_shrink_ticks, 20);
        assert_eq!(shrink.total_proxy_shrink_ticks, 35);
    }

    #[test]
    fn paired_total_shrink_allows_receipt_before_sendinput_completion() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 200, 300, 250),
            timing_point(1_100, 1_200, 1_300, 1_600),
        )
        .unwrap();
        assert_eq!(shrink.scheduler_shrink_ticks, 0);
        assert_eq!(shrink.sendinput_shrink_ticks, 0);
        assert_eq!(shrink.delivery_shrink_ticks, -350);
        assert_eq!(shrink.total_proxy_shrink_ticks, -350);
    }

    #[test]
    fn paired_total_shrink_allows_receipt_at_sendinput_completion() {
        let shrink = paired_timing_shrink_ticks(
            timing_point(100, 200, 300, 300),
            timing_point(1_100, 1_200, 1_300, 1_600),
        )
        .unwrap();
        assert_eq!(shrink.delivery_shrink_ticks, -300);
        assert_eq!(shrink.total_proxy_shrink_ticks, -300);
    }

    #[test]
    fn calibration_precision_boundary_prepares_before_handoff_and_send() {
        let source = include_str!("calibration.rs");
        let prepare = source
            .find("PreparedTaggedCalibrationPacket::try_new")
            .expect("prepared calibration payload");
        let handoff = source
            .find("self.wait_to_precision_boundary(physical_target_qpc)")
            .expect("precision handoff");
        let send = source
            .find("prepared.send_at_target(self.qpc_clock, physical_target_qpc)")
            .expect("fused calibration sender");
        assert!(prepare < handoff && handoff < send);

        let wait_body = source
            .split("fn wait_to_precision_boundary")
            .nth(1)
            .expect("precision boundary implementation");
        assert!(wait_body.contains("CALIBRATION_PRECISION_HANDOFF_US"));
        assert!(wait_body.contains("DurationTicks::ZERO"));
        assert!(wait_body.contains("wait_until_ticks_with_metrics_typed"));
    }

    #[test]
    fn calibration_accepts_monophony_and_maximum_polyphony_boundaries() {
        for polyphony in [1, 15] {
            let config = CalibrationConfig {
                polyphonies: vec![polyphony],
                ..CalibrationConfig::default()
            };
            assert!(validate_calibration_config(&config).is_ok());
        }
    }

    #[test]
    fn paired_timing_rejects_non_monotonic_boundaries() {
        let error = paired_timing_shrink_ticks(
            timing_point(100, 200, 300, 700),
            timing_point(1_100, 1_200, 1_150, 1_600),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CalibrationError::TimestampOrder {
                field: "up completion before pre_call"
            }
        ));
    }

    #[test]
    fn paired_timing_rejects_pre_call_before_target() {
        let error = paired_timing_shrink_ticks(
            timing_point(100, 99, 300, 300),
            timing_point(1_100, 1_200, 1_300, 1_600),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            CalibrationError::TimestampOrder {
                field: "down pre_call before target"
            }
        ));
    }

    #[test]
    fn raw_input_observer_keeps_win32_cleanup_alignment_and_error_contracts() {
        let source = include_str!("calibration.rs");
        assert!(source.contains("fn complete_wm_input"));
        assert!(source.contains("DefWindowProcW(hwnd, WM_INPUT"));
        assert!(source.contains("input_buffer: std::cell::RefCell<Vec<usize>>"));
        assert!(source.contains("if r == -1"));
        assert!(source.contains("guard.pump_thread_failed = true"));
    }

    #[test]
    fn calibration_budget_keeps_one_second_for_measurement() {
        let config = CalibrationConfig {
            budget_seconds: CALIBRATION_MIN_TOTAL_BUDGET_SECONDS - 1,
            ..CalibrationConfig::default()
        };
        assert!(matches!(
            validate_calibration_config(&config),
            Err(CalibrationError::BudgetExceeded)
        ));
    }

    #[test]
    fn receipt_timeout_is_a_rejected_wait_not_budget_exceeded() {
        assert_eq!(
            receipt_wait_duration(Duration::ZERO, Some(Duration::from_millis(1))).unwrap(),
            None
        );
        assert!(matches!(
            receipt_wait_duration(Duration::from_millis(1), Some(Duration::ZERO)),
            Err(CalibrationError::BudgetExceeded)
        ));
    }

    #[test]
    fn calibration_extra_info_round_trips_24_bit_sequence() {
        for sequence_id in [1, 2, 0x00FF_FFFF] {
            let extra = make_calibration_extra_info(sequence_id).unwrap();
            assert_eq!(calibration_extra_info_sequence(extra), Some(sequence_id));
        }
    }

    #[test]
    fn calibration_extra_info_rejects_zero_overflow_and_foreign_tags() {
        assert!(make_calibration_extra_info(0).is_none());
        assert!(make_calibration_extra_info(CALIBRATION_EXTRA_SEQUENCE_MASK + 1).is_none());
        assert_eq!(
            calibration_extra_info_sequence(crate::input::SKY_PLAYER_SIGNATURE),
            None
        );
    }

    #[test]
    fn actual_idle_gap_classification_uses_hot_and_cold_boundaries() {
        let threshold = DurationTicks::from_raw(20);
        assert_eq!(
            classify_idle_gap(QpcTicks::from_raw(100), QpcTicks::from_raw(119), threshold).unwrap(),
            (SampleClass::Hot, DurationTicks::from_raw(19))
        );
        assert_eq!(
            classify_idle_gap(QpcTicks::from_raw(100), QpcTicks::from_raw(120), threshold).unwrap(),
            (SampleClass::Cold, DurationTicks::from_raw(20))
        );
        assert!(
            classify_idle_gap(QpcTicks::from_raw(100), QpcTicks::from_raw(99), threshold).is_err()
        );
    }

    #[test]
    fn hot_gap_target_must_be_shorter_than_cold_threshold() {
        let mut config = CalibrationConfig::default();
        config.hot_gap_target_us = config.cold_threshold_us;
        assert!(matches!(
            validate_calibration_config(&config),
            Err(CalibrationError::HotGapTargetTooLong { .. })
        ));
    }

    #[test]
    fn quick_config_fewer_samples() {
        let quick = CalibrationConfig::quick();
        let full = CalibrationConfig::full();
        assert!(quick.samples_per_hot_bucket < full.samples_per_hot_bucket);
    }

    #[test]
    fn sample_is_complete_when_receipts_match() {
        let sample = CalibrationSample {
            sequence_id: 1,
            target_ticks: QpcTicks::from_raw(90),
            call_started_ticks: QpcTicks::from_raw(100),
            call_completed_ticks: QpcTicks::from_raw(200),
            first_receipt_ticks: Some(QpcTicks::from_raw(250)),
            last_receipt_ticks: Some(QpcTicks::from_raw(300)),
            receipt_count: 3,
            expected_receipt_count: 3,
            win32_error: None,
            actual_idle_gap_ticks: None,
            observed_class: None,
            anomalies: SampleAnomalies::default(),
            receipts: SmallVec::new(),
        };
        assert!(sample.is_complete());
        assert!(sample.first_receipt_latency_us().unwrap().is_some());
        assert!(sample.last_receipt_latency_us().unwrap().is_some());
        assert!(sample.intra_chord_spread_us().unwrap().is_some());
    }

    #[test]
    fn sample_intra_chord_spread_none_for_monophonic() {
        let sample = CalibrationSample {
            sequence_id: 1,
            target_ticks: QpcTicks::from_raw(90),
            call_started_ticks: QpcTicks::from_raw(100),
            call_completed_ticks: QpcTicks::from_raw(200),
            first_receipt_ticks: Some(QpcTicks::from_raw(250)),
            last_receipt_ticks: Some(QpcTicks::from_raw(250)), // same tick → spread = 0 us
            receipt_count: 1,
            expected_receipt_count: 1,
            win32_error: None,
            actual_idle_gap_ticks: None,
            observed_class: None,
            anomalies: SampleAnomalies::default(),
            receipts: SmallVec::new(),
        };
        // For polyphony-1 first == last, so spread = 0 (not None).
        assert_eq!(sample.intra_chord_spread_us().unwrap(), Some(0));
    }

    #[test]
    fn quantile_sum_overflow_is_an_error() {
        assert!(matches!(
            quantile_stats_u64(&[u64::MAX, u64::MAX]),
            Err(CalibrationError::StatisticsOverflow)
        ));
    }

    #[test]
    fn calibration_sample_uses_exact_sender_tick_boundaries() {
        let result = PlatformSendResult {
            requested: 1,
            inserted: 1,
            started_ticks: QpcTicks::from_raw(101),
            completed_ticks: Some(QpcTicks::from_raw(211)),
            win32_error: 0,
            timing_error: None,
        };
        assert_eq!(
            exact_sendinput_boundaries(&result).unwrap(),
            (QpcTicks::from_raw(101), QpcTicks::from_raw(211))
        );
    }

    #[test]
    fn missing_sender_completion_is_not_a_zero_timestamp_sample() {
        let result = PlatformSendResult {
            requested: 1,
            inserted: 1,
            started_ticks: QpcTicks::from_raw(101),
            completed_ticks: None,
            win32_error: 0,
            timing_error: Some(crate::clock::QpcError::CounterUnavailable),
        };
        assert!(matches!(
            exact_sendinput_boundaries(&result),
            Err(CalibrationError::ClockFailure)
        ));
    }

    #[cfg(windows)]
    use platform::CalibrationSession;

    #[test]
    #[cfg(windows)]
    fn no_send_before_foreground_acquired() {
        set_test_foreground_override(Some(|_| false));
        let res = CalibrationSession::open();
        assert!(matches!(
            res,
            Err(CalibrationError::ForegroundAcquireFailed)
        ));
        set_test_foreground_override::<fn(isize) -> bool>(None);
    }

    #[test]
    #[cfg(windows)]
    fn foreground_loss_aborts_before_next_send() {
        set_test_foreground_override(Some(|_| true));
        let mut session = CalibrationSession::open().expect("session open");
        set_test_foreground_override(Some(|_| false));
        let res = session.ensure_foreground_owned();
        assert!(matches!(res, Err(CalibrationError::ForegroundLost)));
        set_test_foreground_override::<fn(isize) -> bool>(None);
    }

    #[test]
    #[cfg(windows)]
    fn foreground_loss_still_runs_emergency_cleanup() {
        set_test_foreground_override(Some(|_| true));
        let mut session = CalibrationSession::open().expect("session open");
        session.possibly_active_mask = 0b11;
        set_test_foreground_override(Some(|_| false));
        assert!(session.ensure_foreground_owned().is_err());
        let outcome = session.cleanup_keyboard();
        assert!(outcome.cleanup_attempted);
        assert_eq!(session.possibly_active_mask, 0);
        set_test_foreground_override::<fn(isize) -> bool>(None);
    }

    #[test]
    #[cfg(windows)]
    fn closed_window_cancels_calibration() {
        set_test_foreground_override(Some(|_| true));
        let mut session = CalibrationSession::open().expect("session open");
        {
            let mut guard = session.shared.0.lock().unwrap();
            guard.window_closed = true;
        }
        let res = session.ensure_foreground_owned();
        assert!(matches!(
            res,
            Err(CalibrationError::CalibrationWindowClosed)
        ));
        set_test_foreground_override::<fn(isize) -> bool>(None);
    }

    #[test]
    #[cfg(windows)]
    fn foreground_check_is_outside_qpc_measurement_boundary() {
        set_test_foreground_override(Some(|_| true));
        let mut session = CalibrationSession::open().expect("session open");
        let check_res = session.ensure_foreground_owned();
        assert!(check_res.is_ok());
        set_test_foreground_override::<fn(isize) -> bool>(None);
    }

    #[test]
    #[cfg(windows)]
    fn successful_run_requires_foreground_ownership() {
        set_test_foreground_override(Some(|_| true));
        let mut session = CalibrationSession::open().expect("session open");
        assert!(session.ensure_foreground_owned().is_ok());
        set_test_foreground_override::<fn(isize) -> bool>(None);
    }
}
