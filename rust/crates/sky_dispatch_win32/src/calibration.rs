//! Chord-aware `SendInput` delivery-proxy calibration harness.
//!
//! # Evidence scope
//!
//! This module measures **injected Raw Input delivery proxy** latency only.
//! Concretely it captures the time between `SendInput` completion and the
//! moment a `WM_INPUT` message from the app-owned calibration window arrives.
//!
//! The measured boundary is:
//! ```text
//! call_started_ticks   — QPC immediately before SendInput
//! call_completed_ticks — QPC immediately after SendInput returns
//! first_receipt_ticks  — QPC when the first WM_INPUT for this packet arrives
//! last_receipt_ticks   — QPC when the last WM_INPUT for this packet arrives
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
//! Each calibration packet is tagged with a `sequence_id` embedded in the high
//! 32 bits of `dwExtraInfo`, which allows precise correlation without relying
//! on scan code matching across overlapping samples.
//!
//! The window message pump runs on a dedicated thread so it does not interfere
//! with the calling thread's timing measurements.
//!
//! # Non-Windows
//!
//! On non-Windows targets the public surface compiles but every function
//! returns [`CalibrationError::PlatformUnsupported`].

use crate::clock::{QpcClock, QpcTicks, qpc_now_ticks_checked, qpc_ticks_to_us};
use crate::input::PlatformSendResult;
use serde::{Deserialize, Serialize};
use sky_dispatch_core::time::SEND_COLD_THRESHOLD_US;
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

    #[error("sequence {sequence_id}: timeout waiting for {expected} receipts (got {received})")]
    ReceiptTimeout {
        sequence_id: u32,
        expected: u8,
        received: u8,
    },

    #[error("scan code {scan_code} is not an instrument key")]
    InvalidScanCode { scan_code: u16 },

    #[error("polyphony {0} exceeds maximum of 15")]
    PolyphonyTooLarge(usize),

    #[error("sample count must be at least 1")]
    ZeroSamples,

    #[error("QPC failed during calibration")]
    ClockFailure,

    #[error("calibration statistics overflowed")]
    StatisticsOverflow,

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

    #[error("calibration sequence id exhausted")]
    SequenceOverflow,

    #[error("{phase} packet {sequence_id} was not fully clean: {received}/{expected} receipts")]
    PacketIntegrity {
        phase: &'static str,
        sequence_id: u32,
        expected: u8,
        received: u8,
        win32_error: Option<u32>,
    },

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

// ─── Sample record ────────────────────────────────────────────────────────────

/// A single calibration sample for one polyphony/direction bucket.
///
/// All times are QPC ticks at the time of collection and are converted to
/// microseconds only when building the output JSON so that internal logic
/// stays in tick-domain.
#[derive(Debug, Clone)]
pub struct CalibrationSample {
    pub sequence_id: u32,
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

/// Serializable, tick-free evidence for one measured sample.
///
/// Chunked calibration processes cannot share their QPC clock state, so the
/// native process converts each sample to the same microsecond values used by
/// `aggregate_samples` before it exits. Keeping these observations in the
/// chunk artifact lets the finalizer merge quantiles exactly instead of
/// approximating a global quantile from per-chunk quantiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationSampleEvidence {
    pub clean: bool,
    pub call_duration_us: u64,
    pub first_receipt_us: Option<i64>,
    pub last_receipt_us: Option<i64>,
    pub intra_chord_spread_us: Option<u64>,
    pub receipt_count: u8,
    pub expected_receipt_count: u8,
    pub win32_error: Option<u32>,
    pub anomalies: SampleAnomalies,
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
}

impl SampleAnomalies {
    pub fn any(&self) -> bool {
        self.duplicate_receipt
            || self.reordered_receipt
            || self.unexpected_scan_code
            || self.timeout
            || self.partial_send
            || self.class_mismatch
    }
}

// ─── Bucket-level statistics ──────────────────────────────────────────────────

/// Aggregated statistics for one (kind, polyphony, class) bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub error_count: u64,
    pub timeout_count: u64,
    pub anomaly_count: u64,
    pub class_mismatch_count: u64,
    pub call_duration_us: QuantileStats,
    /// Latency from `call_completed` to first Raw Input receipt.
    pub first_receipt_us: Option<SignedQuantileStats>,
    /// Latency from `call_completed` to last Raw Input receipt.
    pub last_receipt_us: Option<SignedQuantileStats>,
    /// Spread between first and last receipt (zero for polyphony-1 buckets).
    pub intra_chord_spread_us: Option<QuantileStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuantileStats {
    pub min: u64,
    pub p50: u64,
    pub p90: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
    pub mean: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Direction of the injected packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PacketKind {
    Down,
    Up,
}

pub const MEASUREMENT_PROTOCOL_VERSION: u32 = 3;
pub const CALIBRATION_SCHEMA_VERSION: u32 = 8;
pub const CALIBRATION_CLEANUP_RESERVE_SECONDS: u64 = 5;
pub const CALIBRATION_MIN_MEASUREMENT_SECONDS: u64 = 1;
pub const CALIBRATION_MIN_TOTAL_BUDGET_SECONDS: u64 =
    CALIBRATION_CLEANUP_RESERVE_SECONDS + CALIBRATION_MIN_MEASUREMENT_SECONDS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CalibrationStep {
    MeasureDown,
    PrepareDown,
    HotGap,
    ColdGapAfterPrepare,
    MeasureUp,
    CleanupUp,
}

pub(crate) fn calibration_protocol(
    kind: PacketKind,
    class: SampleClass,
) -> &'static [CalibrationStep] {
    match (kind, class) {
        (PacketKind::Down, _) => &[CalibrationStep::MeasureDown, CalibrationStep::CleanupUp],
        (PacketKind::Up, SampleClass::Hot) => &[
            CalibrationStep::PrepareDown,
            CalibrationStep::HotGap,
            CalibrationStep::MeasureUp,
        ],
        (PacketKind::Up, SampleClass::Cold) => &[
            CalibrationStep::PrepareDown,
            CalibrationStep::ColdGapAfterPrepare,
            CalibrationStep::MeasureUp,
        ],
    }
}

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
    /// Number of measured samples per hot bucket.
    pub samples_per_hot_bucket: u32,
    /// Number of measured samples per cold bucket.
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
            samples_per_hot_bucket: 20,
            samples_per_cold_bucket: 20,
            warmup_samples: 4,
            ..Self::default()
        }
    }

    /// Full calibration preset (tuning estimator / release gate).
    pub fn full() -> Self {
        Self {
            samples_per_hot_bucket: 5_000,
            samples_per_cold_bucket: 5_000,
            warmup_samples: 50,
            ..Self::default()
        }
    }
}

// ─── Output schema ────────────────────────────────────────────────────────────

/// The complete output of one calibration run.
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
    pub configuration: CalibrationConfig,
    /// Buckets keyed by (kind, polyphony, class).
    /// Serialised as nested maps: `down.1.hot`, `up.3.cold`, …
    pub buckets: CalibrationBuckets,
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

/// Output for exactly one physical calibration bucket.  The Python runner
/// stores this as the durable checkpoint unit; no native process is allowed to
/// produce the complete 24-bucket matrix in one run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationBucketOutput {
    pub version: u32,
    pub measurement_protocol_version: u32,
    pub source_git_sha: &'static str,
    pub native_build_id: &'static str,
    pub dirty_worktree: bool,
    pub native_source_fingerprint: &'static str,
    pub rustc_version: &'static str,
    pub evidence_kind: &'static str,
    pub host_fingerprint: HostFingerprint,
    pub configuration: CalibrationConfig,
    pub kind: PacketKind,
    pub class: SampleClass,
    pub polyphony: u8,
    pub attempted: u64,
    pub setup_attempted: u64,
    pub setup_anomalous: u64,
    pub setup_timed_out: u64,
    pub warmup_attempted: u64,
    pub warmup_anomalous: u64,
    pub warmup_timed_out: u64,
    pub total_attempted: u64,
    pub total_anomalous: u64,
    pub total_timed_out: u64,
    pub bucket: BucketStats,
    /// Bounded diagnostic evidence retained after aggregation.  The full raw
    /// sample stream is never serialized into an artifact.
    pub worst_samples: Vec<CalibrationSampleEvidence>,
    /// Every anomalous sample is retained because anomalies are acceptance
    /// blockers and are expected to be rare.
    pub anomalous_samples: Vec<CalibrationSampleEvidence>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CalibrationBuckets {
    pub down: HashMap<u8, HashMap<String, BucketStats>>,
    pub up: HashMap<u8, HashMap<String, BucketStats>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostFingerprint {
    pub qpc_frequency_hz: u64,
    pub win32_build: Option<String>,
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

// ─── Internal raw-input receipt state (shared between pump and collector) ─────

/// A single Raw Input receipt delivered by the message pump.
#[derive(Debug, Clone)]
struct RawInputReceipt {
    arrived_ticks: QpcTicks,
    scan_code: u16,
    sequence_id: u32,
}

/// Shared state between the calibration thread (sends packets and awaits
/// receipts) and the window pump thread (delivers `WM_INPUT` events).
struct SharedCalibState {
    /// Receipts delivered for the currently active packet sequence.
    pending_receipts: Vec<RawInputReceipt>,
    /// Sequence ID of the currently expected packet, `None` when idle.
    active_sequence: Option<u32>,
    /// Set by the pump thread when the window is ready.
    window_ready: bool,
    /// Set to signal the pump thread to exit gracefully (checked on resume).
    should_exit: bool,
    /// HWND of the calibration window (as `isize`).
    hwnd: isize,
    clock_failed: bool,
    raw_input_restore_failed: bool,
    pump_thread_failed: bool,
}

// ─── Platform-specific implementation ────────────────────────────────────────

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, send_input_raw};
    use crate::mmcss::{MmcssGuard, PriorityMode};

    use std::sync::{Arc, Condvar, Mutex};
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::Input::{
        GetRawInputData, GetRegisteredRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE,
        RAWINPUTHEADER, RID_INPUT, RIDEV_INPUTSINK, RegisterRawInputDevices,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        HWND_MESSAGE, MSG, PostMessageW, RegisterClassExW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
        SWP_NOZORDER, SetWindowPos, TranslateMessage, WM_INPUT, WM_USER, WNDCLASSEXW,
        WS_OVERLAPPEDWINDOW,
    };

    // HID_USAGE_PAGE_GENERIC = 0x01 (USB HID spec, no feature flag needed)
    const HID_USAGE_PAGE_GENERIC: u16 = 0x01;

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

    // ── Window procedure ──────────────────────────────────────────────────────

    // Thread-local pointer to the shared state, set before the message loop
    // starts and cleared after it exits.
    thread_local! {
        static PUMP_STATE: std::cell::Cell<*const PumpContext> =
            const { std::cell::Cell::new(std::ptr::null()) };
    }

    struct PumpContext {
        shared: Arc<(Mutex<SharedCalibState>, Condvar)>,
        input_buffer: std::cell::RefCell<Vec<u8>>,
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
                let arrived = match qpc_now_ticks_checked() {
                    Ok(ticks) => ticks,
                    Err(_) => {
                        let (lock, cvar) = ctx.shared.as_ref();
                        if let Ok(mut guard) = lock.lock() {
                            guard.clock_failed = true;
                            cvar.notify_all();
                        }
                        return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                    }
                };
                let hri = lparam as HRAWINPUT;
                let mut size: u32 = 0;
                // SAFETY: querying size with null buffer is the documented
                // pattern for GetRawInputData.
                unsafe {
                    GetRawInputData(
                        hri,
                        RID_INPUT,
                        std::ptr::null_mut(),
                        &mut size,
                        std::mem::size_of::<RAWINPUTHEADER>() as u32,
                    )
                };
                if size == 0 || size > 4096 {
                    // SAFETY: forward to default handler.
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                let mut buf = ctx.input_buffer.borrow_mut();
                buf.resize(size as usize, 0);
                // SAFETY: buf has the capacity reported by the previous call.
                let read = unsafe {
                    GetRawInputData(
                        hri,
                        RID_INPUT,
                        buf.as_mut_ptr().cast(),
                        &mut size,
                        std::mem::size_of::<RAWINPUTHEADER>() as u32,
                    )
                };
                if read == u32::MAX || read < std::mem::size_of::<RAWINPUTHEADER>() as u32 {
                    // SAFETY: forward on parse failure.
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                // SAFETY: buf is at least sizeof(RAWINPUT) and was filled by
                // GetRawInputData.
                let raw: &RAWINPUT = unsafe { &*(buf.as_ptr().cast()) };
                let rtype = raw.header.dwType;
                // RIM_TYPEKEYBOARD = 1
                if rtype != 1 {
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                }
                let keyboard = unsafe { &raw.data.keyboard };
                let scan_code = keyboard.MakeCode;
                let extra = keyboard.ExtraInformation as usize;
                let Some(seq_id) = calibration_extra_info_sequence(extra) else {
                    // Not one of our injected packets — ignore.
                    return unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) };
                };

                let receipt = RawInputReceipt {
                    arrived_ticks: arrived,
                    scan_code,
                    sequence_id: seq_id,
                };

                let (lock, cvar) = ctx.shared.as_ref();
                if let Ok(mut guard) = lock.lock() {
                    #[allow(clippy::collapsible_if)]
                    if guard.active_sequence == Some(seq_id) {
                        guard.pending_receipts.push(receipt);
                        cvar.notify_one();
                    }
                    // Receipts for stale sequence IDs are silently discarded.
                }
                0
            }
            WM_CALIB_EXIT => {
                // SAFETY: PostQuitMessage is safe to call from within a window
                // procedure.
                unsafe { windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
                0
            }
            WM_CALIB_ACTIVATE => {
                // Trigger a SetWindowPos to flush any pending messages.
                // SAFETY: hwnd is a valid live window handle.
                unsafe {
                    SetWindowPos(
                        hwnd,
                        HWND_MESSAGE,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                };
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
            let (lock, _cvar) = shared.as_ref();
            if let Ok(mut g) = lock.lock() {
                g.window_ready = true; // signal failure with hwnd = 0
                drop(g);
            }
            eprintln!("[calibration] RegisterClassExW failed: {err}");
            return;
        }

        let window_name: Vec<u16> = "SkyCalib\0".encode_utf16().collect();
        // SAFETY: the class name and window name are NUL-terminated UTF-16 slices
        // valid for the duration of the call.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                1,
                1,
                HWND_MESSAGE,
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
            if r == 0 || r == -1 {
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
        shared: Arc<(Mutex<SharedCalibState>, Condvar)>,
        hwnd: isize,
        pump_thread: Option<std::thread::JoinHandle<()>>,
        qpc_clock: QpcClock,
        next_sequence: u32,
        possibly_active_mask: u16,
        last_send_completed_ticks: Option<QpcTicks>,
        measurement_deadline: Option<QpcTicks>,
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

    impl CalibrationSession {
        pub fn open() -> Result<Self, CalibrationError> {
            let qpc_clock = QpcClock::initialize().map_err(|_| CalibrationError::ClockFailure)?;
            let initial = SharedCalibState {
                pending_receipts: Vec::new(),
                active_sequence: None,
                window_ready: false,
                should_exit: false,
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

            Ok(Self {
                shared,
                hwnd,
                pump_thread: Some(handle),
                qpc_clock,
                next_sequence: 1,
                possibly_active_mask: 0,
                last_send_completed_ticks: None,
                measurement_deadline: None,
            })
        }

        pub fn set_measurement_deadline(&mut self, deadline: QpcTicks) {
            self.measurement_deadline = Some(deadline);
        }

        /// Return a QPC deadline after reserving time for final cleanup.
        pub fn measurement_deadline(
            &self,
            budget_seconds: u64,
        ) -> Result<QpcTicks, CalibrationError> {
            let budget_us = budget_seconds
                .checked_mul(1_000_000)
                .ok_or(CalibrationError::ClockFailure)?;
            let cleanup_reserve_us = CALIBRATION_CLEANUP_RESERVE_SECONDS.saturating_mul(1_000_000);
            let measurement_us = budget_us.saturating_sub(cleanup_reserve_us);
            let duration = self
                .qpc_clock
                .duration_from_us(measurement_us)
                .map_err(|_| CalibrationError::ClockFailure)?;
            let now = self
                .qpc_clock
                .now()
                .map_err(|_| CalibrationError::ClockFailure)?;
            now.checked_add_duration(duration)
                .map_err(|_| CalibrationError::ClockFailure)
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

        fn sleep_with_budget(&self, duration: Duration) -> Result<(), CalibrationError> {
            let Some(deadline) = self.measurement_deadline else {
                std::thread::sleep(duration);
                return Ok(());
            };
            let now = self
                .qpc_clock
                .now()
                .map_err(|_| CalibrationError::ClockFailure)?;
            let remaining = deadline
                .checked_duration_since(now)
                .map_err(|_| CalibrationError::BudgetExceeded)?;
            let remaining_us = self
                .qpc_clock
                .duration_to_us(remaining)
                .map_err(|_| CalibrationError::ClockFailure)?;
            let requested_us = duration.as_micros().min(u128::from(u64::MAX)) as u64;
            if remaining_us == 0 || requested_us > remaining_us {
                return Err(CalibrationError::BudgetExceeded);
            }
            std::thread::sleep(duration);
            self.ensure_budget()
        }

        fn cleanup_keyboard(&mut self) -> CleanupOutcome {
            let attempted = PHYSICAL_INSTRUMENT_SCAN_CODES.to_vec();
            let expected = attempted.len() as u32;
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
                match crate::input::is_scan_code_physically_down(scan_code) {
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
        pub fn measure_packet(
            &mut self,
            scan_codes: &[u16],
            key_up: bool,
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            self.ensure_budget()?;
            let n = scan_codes.len();
            if n == 0 || n > 15 {
                return Err(CalibrationError::PolyphonyTooLarge(n));
            }
            for &sc in scan_codes {
                if !PHYSICAL_INSTRUMENT_SCAN_CODES.contains(&sc) {
                    return Err(CalibrationError::InvalidScanCode { scan_code: sc });
                }
            }

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
                g.pending_receipts.clear();
            }

            // The tagged sender owns both QPC boundaries. Do not add outer
            // timestamps here: those would include calibration bookkeeping and
            // would mislabel it as the SendInput syscall duration.
            let psr = send_input_raw_tagged(scan_codes, key_up, extra, self.qpc_clock);
            let (call_started, call_completed) = match exact_sendinput_boundaries(&psr) {
                Ok(boundaries) => boundaries,
                Err(error) => {
                    let (lock, cvar) = self.shared.as_ref();
                    let mut guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                    guard.active_sequence = None;
                    cvar.notify_all();
                    return Err(error);
                }
            };
            self.last_send_completed_ticks = Some(call_completed);
            let expected = scan_codes.len() as u8;
            let partial_send = psr.inserted < expected as u32;
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
                guard.active_sequence = None;
                cvar.notify_all();
                return Err(CalibrationError::PacketIntegrity {
                    phase: "partial_send",
                    sequence_id: seq,
                    expected,
                    received: psr.inserted.min(expected as u32) as u8,
                    win32_error: (psr.win32_error != 0).then_some(psr.win32_error),
                });
            }

            let expected_receipts = (psr.inserted as usize).min(scan_codes.len()) as u8;

            // Wait for expected receipts.
            let receipt_deadline = std::time::Instant::now() + receipt_timeout;
            let (first, last, count, anomalies) = {
                let (lock, cvar) = self.shared.as_ref();
                let mut guard = lock.lock().map_err(|_| CalibrationError::StateLockFailed)?;
                loop {
                    if guard.clock_failed {
                        guard.active_sequence = None;
                        cvar.notify_all();
                        return Err(CalibrationError::ClockFailure);
                    }
                    let n_received = guard.pending_receipts.len();
                    if n_received >= expected_receipts as usize {
                        break;
                    }
                    let mut remaining =
                        receipt_deadline.saturating_duration_since(std::time::Instant::now());
                    if let Some(budget_deadline) = self.measurement_deadline {
                        let now = self
                            .qpc_clock
                            .now()
                            .map_err(|_| CalibrationError::ClockFailure)?;
                        let budget_remaining = budget_deadline
                            .checked_duration_since(now)
                            .map_err(|_| CalibrationError::BudgetExceeded)?;
                        let budget_remaining_us = self
                            .qpc_clock
                            .duration_to_us(budget_remaining)
                            .map_err(|_| CalibrationError::ClockFailure)?;
                        remaining = remaining.min(Duration::from_micros(budget_remaining_us));
                    }
                    if remaining.is_zero() {
                        guard.active_sequence = None;
                        cvar.notify_all();
                        if self.measurement_deadline.is_some() {
                            return Err(CalibrationError::BudgetExceeded);
                        }
                        break;
                    }
                    guard = cvar
                        .wait_timeout(guard, remaining)
                        .map_err(|_| CalibrationError::StateLockFailed)?
                        .0;
                }

                let receipts = std::mem::take(&mut guard.pending_receipts);
                guard.active_sequence = None;
                cvar.notify_all();
                drop(guard);

                analyse_receipts(&receipts, scan_codes, seq, expected_receipts)
            };

            if key_up && psr.inserted == expected as u32 {
                self.possibly_active_mask = 0;
            }

            Ok(CalibrationSample {
                sequence_id: seq,
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
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            let previous_completion = self
                .last_send_completed_ticks
                .ok_or(CalibrationError::ClockFailure)?;
            let mut sample = self.measure_packet(scan_codes, key_up, receipt_timeout)?;
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

        fn require_clean_packet(
            sample: CalibrationSample,
            phase: &'static str,
        ) -> Result<CalibrationSample, CalibrationError> {
            if sample.is_complete() && !sample.anomalies.any() {
                return Ok(sample);
            }
            Err(CalibrationError::PacketIntegrity {
                phase,
                sequence_id: sample.sequence_id,
                expected: sample.expected_receipt_count,
                received: sample.receipt_count,
                win32_error: sample.win32_error,
            })
        }

        /// Inject the setup packet used to establish a physical Down state.
        /// Setup evidence is never admitted to timing quantiles.
        pub fn prepare_keys_down(
            &mut self,
            scan_codes: &[u16],
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            let sample = self.measure_packet(scan_codes, false, receipt_timeout)?;
            Self::require_clean_packet(sample, "setup down")
        }

        /// Release a measured packet and verify that its receipt was complete.
        /// Cleanup packets are bookkeeping, not measured evidence.
        pub fn cleanup_keys_up(
            &mut self,
            scan_codes: &[u16],
            receipt_timeout: Duration,
        ) -> Result<CalibrationSample, CalibrationError> {
            let sample = self.measure_packet(scan_codes, true, receipt_timeout)?;
            Self::require_clean_packet(sample, "cleanup up")
        }

        /// Wait from an exact SendInput completion boundary until the physical
        /// cold threshold has elapsed. The completion tick, rather than a
        /// receipt or authored timestamp, is the classification anchor.
        pub fn wait_cold_gap_after(
            &self,
            completed_ticks: QpcTicks,
            gap_us: u64,
        ) -> Result<(), CalibrationError> {
            let gap_ticks = self
                .qpc_clock
                .duration_from_us(gap_us)
                .map_err(|_| CalibrationError::ClockFailure)?;
            let deadline = completed_ticks
                .checked_add_duration(gap_ticks)
                .map_err(|_| CalibrationError::ClockFailure)?;
            loop {
                self.ensure_budget()?;
                let now = self
                    .qpc_clock
                    .now()
                    .map_err(|_| CalibrationError::ClockFailure)?;
                if now >= deadline {
                    return Ok(());
                }
                let remaining = deadline
                    .as_u64()
                    .checked_sub(now.as_u64())
                    .ok_or(CalibrationError::ClockFailure)?;
                let remaining_us = self
                    .qpc_clock
                    .duration_to_us(crate::clock::DurationTicks::from_raw(remaining))
                    .map_err(|_| CalibrationError::ClockFailure)?;
                if remaining_us > 100 {
                    self.sleep_with_budget(Duration::from_micros(remaining_us.min(1_000)))?;
                } else {
                    std::hint::spin_loop();
                }
            }
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

    /// Variant of `send_input_raw` that injects a custom `dwExtraInfo` so we
    /// can correlate Raw Input receipts with the correct sequence.
    fn send_input_raw_tagged(
        scan_codes: &[u16],
        key_up: bool,
        extra: usize,
        clock: QpcClock,
    ) -> crate::input::PlatformSendResult {
        use smallvec::SmallVec;
        use windows_sys::Win32::Foundation::SetLastError;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, SendInput,
        };

        if scan_codes.is_empty() {
            let completed_ticks = match clock.now() {
                Ok(ticks) => ticks,
                Err(error) => {
                    return crate::input::PlatformSendResult {
                        requested: 0,
                        inserted: 0,
                        started_ticks: QpcTicks::ZERO,
                        completed_ticks: None,
                        completed_us: 0,
                        win32_error: 0,
                        timing_error: Some(error),
                    };
                }
            };
            let (completed_us, timing_error) = match clock
                .timeline_to_us(sky_dispatch_core::time::TimelineTicks::from_raw(
                    completed_ticks.as_u64(),
                ))
                .map_err(|_| crate::clock::QpcError::ConversionOverflow)
            {
                Ok(micros) => (micros, None),
                Err(error) => (0, Some(error)),
            };
            return crate::input::PlatformSendResult {
                requested: 0,
                inserted: 0,
                started_ticks: completed_ticks,
                completed_ticks: Some(completed_ticks),
                completed_us,
                win32_error: 0,
                timing_error,
            };
        }

        let started_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return crate::input::PlatformSendResult {
                    requested: scan_codes.len() as u32,
                    inserted: 0,
                    started_ticks: QpcTicks::ZERO,
                    completed_ticks: None,
                    completed_us: 0,
                    win32_error: 0,
                    timing_error: Some(error),
                };
            }
        };

        let mut flags = KEYEVENTF_SCANCODE;
        if key_up {
            flags |= KEYEVENTF_KEYUP;
        }

        let packets: SmallVec<[INPUT; 15]> = scan_codes
            .iter()
            .map(|&sc| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: sc,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: extra,
                    },
                },
            })
            .collect();

        let requested = packets.len() as u32;
        let cb_size = std::mem::size_of::<INPUT>() as i32;

        // SAFETY: packets is contiguous, correctly aligned, and remains valid
        // for the duration of SendInput.
        unsafe { SetLastError(0) };
        let inserted = unsafe { SendInput(requested, packets.as_ptr(), cb_size) }.min(requested);
        let win32_error = if inserted != requested {
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        } else {
            0
        };
        let (completed_ticks, completed_us, timing_error) = match clock.now() {
            Ok(ticks) => match clock
                .timeline_to_us(sky_dispatch_core::time::TimelineTicks::from_raw(
                    ticks.as_u64(),
                ))
                .map_err(|_| crate::clock::QpcError::ConversionOverflow)
            {
                Ok(micros) => (Some(ticks), micros, None),
                Err(error) => (Some(ticks), 0, Some(error)),
            },
            Err(error) => (None, 0, Some(error)),
        };

        crate::input::PlatformSendResult {
            requested,
            inserted,
            started_ticks,
            completed_ticks,
            completed_us,
            win32_error,
            timing_error,
        }
    }

    fn analyse_receipts(
        receipts: &[RawInputReceipt],
        expected_scan_codes: &[u16],
        expected_seq: u32,
        receipt_count_for_completion: u8,
    ) -> (Option<QpcTicks>, Option<QpcTicks>, u8, SampleAnomalies) {
        let mut anomalies = SampleAnomalies::default();
        let count = receipts.len().min(u8::MAX as usize) as u8;

        if count == 0 {
            anomalies.timeout = true;
            return (None, None, 0, anomalies);
        }
        if count < receipt_count_for_completion {
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
        }

        // Detect duplicates: same scan code appearing more than once.
        for i in 0..receipts.len() {
            for j in (i + 1)..receipts.len() {
                if receipts[i].scan_code == receipts[j].scan_code {
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

        let last = if count >= receipt_count_for_completion {
            last
        } else {
            None
        };

        (first, last, count, anomalies)
    }

    // ── Host fingerprint ──────────────────────────────────────────────────────

    pub fn build_host_fingerprint() -> Result<HostFingerprint, CalibrationError> {
        use crate::clock::qpc_frequency;
        let freq = qpc_frequency();
        let sampled_at_us = qpc_now_ticks_checked()
            .map_err(|_| CalibrationError::ClockFailure)
            .and_then(|ticks| qpc_ticks_to_us(ticks).map_err(|_| CalibrationError::ClockFailure))?;
        let win32_build = windows_build_string();
        Ok(HostFingerprint {
            qpc_frequency_hz: freq,
            win32_build,
            sampled_at_us,
        })
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
    pub fn run_calibration(
        config: &CalibrationConfig,
    ) -> Result<CalibrationOutput, CalibrationError> {
        super::validate_calibration_config(config)?;

        let _mmcss = MmcssGuard::acquire(PriorityMode::Auto);
        let mut session = CalibrationSession::open()?;
        let measurement_deadline = session.measurement_deadline(config.budget_seconds)?;
        session.set_measurement_deadline(measurement_deadline);

        let receipt_timeout = Duration::from_millis(config.receipt_timeout_ms as u64);
        let hot_gap_sleep = Duration::from_micros(config.hot_gap_target_us);
        let cold_threshold_ticks = session
            .qpc_clock
            .duration_from_us(config.cold_threshold_us)
            .map_err(|_| CalibrationError::ClockFailure)?;

        // We use the first N scan codes from the canonical instrument list for
        // each polyphony level.  This is deterministic and canonical per the
        // plan.
        let mut all_raw: HashMap<(PacketKind, u8, SampleClass), Vec<CalibrationSample>> =
            HashMap::new();
        let mut warmup_attempted: u64 = 0;
        let mut measured_attempted: u64 = 0;
        let mut setup_attempted: u64 = 0;
        let mut setup_anomalous: u64 = 0;
        let mut setup_timed_out: u64 = 0;
        let mut total_attempted: u64 = 0;
        let mut warmup_anomalous: u64 = 0;
        let mut measured_anomalous: u64 = 0;
        let mut total_anomalous: u64 = 0;
        let mut warmup_timed_out: u64 = 0;
        let mut measured_timed_out: u64 = 0;
        let mut total_timed_out: u64 = 0;
        let mut measured_class_mismatch: u64 = 0;

        let mut record_attempt = |sample: &CalibrationSample, warmup: bool, setup: bool| {
            let attempted = if setup {
                &mut setup_attempted
            } else if warmup {
                &mut warmup_attempted
            } else {
                &mut measured_attempted
            };
            let anomalous = if setup {
                &mut setup_anomalous
            } else if warmup {
                &mut warmup_anomalous
            } else {
                &mut measured_anomalous
            };
            let timed_out = if setup {
                &mut setup_timed_out
            } else if warmup {
                &mut warmup_timed_out
            } else {
                &mut measured_timed_out
            };
            checked_increment(attempted)?;
            checked_increment(&mut total_attempted)?;
            if sample.anomalies.any() {
                checked_increment(anomalous)?;
                checked_increment(&mut total_anomalous)?;
            }
            if sample.anomalies.timeout {
                checked_increment(timed_out)?;
                checked_increment(&mut total_timed_out)?;
            }
            if sample.anomalies.class_mismatch && !warmup && !setup {
                checked_increment(&mut measured_class_mismatch)?;
            }
            Ok::<(), CalibrationError>(())
        };

        for &poly in &config.polyphonies {
            let scan_codes = &PHYSICAL_INSTRUMENT_SCAN_CODES[..poly as usize];

            for kind in [PacketKind::Down, PacketKind::Up] {
                // Warm-up is deliberately excluded from both measured
                // classes. It is not evidence of either hot or cold state.
                for _ in 0..config.warmup_samples {
                    if session.budget_expired(measurement_deadline)? {
                        return Err(CalibrationError::BudgetExceeded);
                    }
                    let sample = match kind {
                        PacketKind::Down => {
                            let s = session.measure_packet(scan_codes, false, receipt_timeout)?;
                            session.cleanup_keys_up(scan_codes, receipt_timeout)?;
                            s
                        }
                        PacketKind::Up => {
                            let setup = session.prepare_keys_down(scan_codes, receipt_timeout)?;
                            record_attempt(&setup, true, true)?;
                            session.measure_packet(scan_codes, true, receipt_timeout)?
                        }
                    };
                    record_attempt(&sample, true, false)?;
                    if !hot_gap_sleep.is_zero() {
                        session.sleep_with_budget(hot_gap_sleep)?;
                    }
                }

                for (class, sample_count, gap) in [
                    (
                        SampleClass::Hot,
                        config.samples_per_hot_bucket,
                        hot_gap_sleep,
                    ),
                    (
                        SampleClass::Cold,
                        config.samples_per_cold_bucket,
                        Duration::from_micros(config.cold_idle_gap_us),
                    ),
                ] {
                    for _ in 0..sample_count {
                        if session.budget_expired(measurement_deadline)? {
                            return Err(CalibrationError::BudgetExceeded);
                        }
                        let protocol = calibration_protocol(kind, class);
                        // Up/Cold must establish the physical Down state first;
                        // its cold wait is anchored to that setup completion.
                        if !protocol.contains(&CalibrationStep::ColdGapAfterPrepare)
                            && !gap.is_zero()
                        {
                            session.sleep_with_budget(gap)?;
                        }
                        let sample = match kind {
                            PacketKind::Down => {
                                let s = session.measure_classified_packet(
                                    scan_codes,
                                    false,
                                    class,
                                    cold_threshold_ticks,
                                    receipt_timeout,
                                )?;
                                session.cleanup_keys_up(scan_codes, receipt_timeout)?;
                                s
                            }
                            PacketKind::Up => {
                                let setup =
                                    session.prepare_keys_down(scan_codes, receipt_timeout)?;
                                record_attempt(&setup, false, true)?;
                                if !protocol.contains(&CalibrationStep::HotGap) {
                                    session.wait_cold_gap_after(
                                        setup.call_completed_ticks,
                                        config.cold_idle_gap_us,
                                    )?;
                                }
                                session.measure_classified_packet(
                                    scan_codes,
                                    true,
                                    class,
                                    cold_threshold_ticks,
                                    receipt_timeout,
                                )?
                            }
                        };

                        record_attempt(&sample, false, false)?;
                        all_raw.entry((kind, poly, class)).or_default().push(sample);
                    }
                }
            }
        }

        let cleanup = session.close();
        if !cleanup.cleanup_success || cleanup.cleanup_verification_inconclusive {
            return Err(CalibrationError::CleanupFailed {
                stuck_keys: cleanup.cleanup_stuck_keys,
            });
        }

        // Aggregate into buckets.
        let mut buckets = CalibrationBuckets::default();
        for ((kind, poly, class), samples) in &all_raw {
            let stats = aggregate_samples(samples)?;
            let class_key = match class {
                SampleClass::Hot => "hot",
                SampleClass::Cold => "cold",
            };
            match kind {
                PacketKind::Down => {
                    buckets
                        .down
                        .entry(*poly)
                        .or_default()
                        .insert(class_key.to_string(), stats);
                }
                PacketKind::Up => {
                    buckets
                        .up
                        .entry(*poly)
                        .or_default()
                        .insert(class_key.to_string(), stats);
                }
            }
        }

        Ok(CalibrationOutput {
            version: CALIBRATION_SCHEMA_VERSION,
            measurement_protocol_version: MEASUREMENT_PROTOCOL_VERSION,
            source_git_sha: env!("SKY_NATIVE_BUILD_COMMIT"),
            native_build_id: env!("SKY_NATIVE_BUILD_COMMIT"),
            dirty_worktree: env!("SKY_NATIVE_DIRTY_WORKTREE") == "true",
            native_source_fingerprint: env!("SKY_NATIVE_SOURCE_FINGERPRINT"),
            rustc_version: env!("SKY_RUSTC_VERSION"),
            evidence_kind: "injected_raw_input_delivery_proxy",
            host_fingerprint: build_host_fingerprint()?,
            configuration: config.clone(),
            buckets,
            warmup_attempted,
            measured_attempted,
            setup_attempted,
            setup_anomalous,
            setup_timed_out,
            total_attempted,
            warmup_anomalous,
            measured_anomalous,
            total_anomalous,
            warmup_timed_out,
            measured_timed_out,
            measured_class_mismatch,
            total_timed_out,
            cleanup,
        })
    }

    struct BucketStepFailure {
        sample_index: u32,
        phase: &'static str,
        source: CalibrationError,
    }

    fn failure_phase(source: &CalibrationError, fallback: &'static str) -> &'static str {
        match source {
            CalibrationError::BudgetExceeded => "budget",
            CalibrationError::PacketIntegrity { phase, .. } if phase.starts_with("cleanup") => {
                "cleanup"
            }
            CalibrationError::PacketIntegrity {
                win32_error: Some(_),
                ..
            } => fallback,
            CalibrationError::PacketIntegrity { .. } => "receipt",
            CalibrationError::CleanupFailed { .. } => "cleanup",
            _ => fallback,
        }
    }

    fn bucket_failure(
        kind: PacketKind,
        class: SampleClass,
        polyphony: u8,
        failure: BucketStepFailure,
        cleanup: CleanupOutcome,
    ) -> CalibrationError {
        CalibrationError::BucketFailed {
            report: Box::new(CalibrationFailureReport {
                kind: format!("{kind:?}").to_lowercase(),
                class: format!("{class:?}").to_lowercase(),
                polyphony,
                sample_index: failure.sample_index,
                phase: failure.phase.to_string(),
                exact_error: failure.source.to_string(),
                win32_error: failure.source.win32_error(),
                cleanup_success: cleanup.cleanup_success,
                cleanup_stuck_keys: cleanup.cleanup_stuck_keys,
                cleanup_verification_inconclusive: cleanup.cleanup_verification_inconclusive,
                raw_input_restore_failed: cleanup.raw_input_restore_failed,
                pump_thread_failed: cleanup.pump_thread_failed,
            }),
        }
    }

    /// Run exactly one `(kind, class, polyphony)` bucket.  This is intentionally
    /// separate from `run_calibration`: the process boundary is part of the
    /// evidence contract and makes a failed bucket unable to erase completed
    /// buckets from the checkpoint.
    pub fn run_calibration_bucket(
        config: &CalibrationConfig,
        kind: PacketKind,
        class: SampleClass,
    ) -> Result<CalibrationBucketOutput, CalibrationError> {
        super::validate_calibration_config(config)?;
        if config.polyphonies.len() != 1 {
            return Err(CalibrationError::PolyphonyTooLarge(
                config.polyphonies.len(),
            ));
        }

        let polyphony = config.polyphonies[0];
        let _mmcss = MmcssGuard::acquire(PriorityMode::Auto);
        let mut session = CalibrationSession::open()?;
        let measurement_deadline = session.measurement_deadline(config.budget_seconds)?;
        session.set_measurement_deadline(measurement_deadline);
        let scan_codes = &PHYSICAL_INSTRUMENT_SCAN_CODES[..polyphony as usize];
        let receipt_timeout = Duration::from_millis(config.receipt_timeout_ms as u64);
        let cold_threshold_ticks = session
            .qpc_clock
            .duration_from_us(config.cold_threshold_us)
            .map_err(|_source| CalibrationError::BucketFailed {
                report: Box::new(CalibrationFailureReport {
                    kind: format!("{kind:?}").to_lowercase(),
                    class: format!("{class:?}").to_lowercase(),
                    polyphony,
                    sample_index: 0,
                    phase: "setup down".to_string(),
                    exact_error: "QPC duration conversion failed".to_string(),
                    win32_error: None,
                    cleanup_success: false,
                    cleanup_stuck_keys: Vec::new(),
                    cleanup_verification_inconclusive: true,
                    raw_input_restore_failed: true,
                    pump_thread_failed: false,
                }),
            })?;

        let hot_gap = Duration::from_micros(config.hot_gap_target_us);
        let expected_samples = match class {
            SampleClass::Hot => config.samples_per_hot_bucket,
            SampleClass::Cold => config.samples_per_cold_bucket,
        };
        let mut measured = Vec::with_capacity(expected_samples as usize);
        let mut warmup_attempted = 0u64;
        let mut warmup_anomalous = 0u64;
        let mut warmup_timed_out = 0u64;
        let mut setup_attempted = 0u64;
        let mut setup_anomalous = 0u64;
        let mut setup_timed_out = 0u64;
        let mut step_result: Result<(), BucketStepFailure> = Ok(());

        let mut record = |sample: &CalibrationSample, warmup: bool, setup: bool| {
            let attempted = if setup {
                &mut setup_attempted
            } else if warmup {
                &mut warmup_attempted
            } else {
                // Measured attempts are represented by `measured` and the
                // aggregate below, so no separate counter is needed here.
                return;
            };
            *attempted = attempted.saturating_add(1);
            let anomalous = if setup {
                &mut setup_anomalous
            } else {
                &mut warmup_anomalous
            };
            if sample.anomalies.any() {
                *anomalous = anomalous.saturating_add(1);
            }
            let timed_out = if setup {
                &mut setup_timed_out
            } else {
                &mut warmup_timed_out
            };
            if sample.anomalies.timeout {
                *timed_out = timed_out.saturating_add(1);
            }
        };

        eprintln!(
            "[calibration] bucket={}/{}/{} sample=0/{} phase=warmup",
            format!("{kind:?}").to_lowercase(),
            format!("{class:?}").to_lowercase(),
            polyphony,
            expected_samples
        );
        for sample_index in 1..=config.warmup_samples {
            if session.budget_expired(measurement_deadline)? {
                step_result = Err(BucketStepFailure {
                    sample_index,
                    phase: "budget",
                    source: CalibrationError::BudgetExceeded,
                });
                break;
            }
            match kind {
                PacketKind::Down => {
                    let measured_down =
                        match session.measure_packet(scan_codes, false, receipt_timeout) {
                            Ok(sample) => sample,
                            Err(source) => {
                                step_result = Err(BucketStepFailure {
                                    sample_index,
                                    phase: failure_phase(&source, "measured send"),
                                    source,
                                });
                                break;
                            }
                        };
                    record(&measured_down, true, false);
                    if let Err(source) = session.cleanup_keys_up(scan_codes, receipt_timeout) {
                        step_result = Err(BucketStepFailure {
                            sample_index,
                            phase: failure_phase(&source, "cleanup"),
                            source,
                        });
                        break;
                    }
                }
                PacketKind::Up => {
                    let setup = match session.prepare_keys_down(scan_codes, receipt_timeout) {
                        Ok(sample) => sample,
                        Err(source) => {
                            step_result = Err(BucketStepFailure {
                                sample_index,
                                phase: failure_phase(&source, "setup down"),
                                source,
                            });
                            break;
                        }
                    };
                    record(&setup, true, true);
                    let measured_up =
                        match session.measure_packet(scan_codes, true, receipt_timeout) {
                            Ok(sample) => sample,
                            Err(source) => {
                                step_result = Err(BucketStepFailure {
                                    sample_index,
                                    phase: failure_phase(&source, "measured send"),
                                    source,
                                });
                                break;
                            }
                        };
                    record(&measured_up, true, false);
                }
            }
            if step_result.is_err() {
                break;
            }
            if !hot_gap.is_zero()
                && let Err(source) = session.sleep_with_budget(hot_gap)
            {
                step_result = Err(BucketStepFailure {
                    sample_index,
                    phase: "budget",
                    source,
                });
                break;
            }
        }

        if step_result.is_ok() {
            for sample_index in 1..=expected_samples {
                if session.budget_expired(measurement_deadline)? {
                    step_result = Err(BucketStepFailure {
                        sample_index,
                        phase: "budget",
                        source: CalibrationError::BudgetExceeded,
                    });
                    break;
                }
                eprintln!(
                    "[calibration] bucket={}/{}/{} sample={}/{} phase=starting",
                    format!("{kind:?}").to_lowercase(),
                    format!("{class:?}").to_lowercase(),
                    polyphony,
                    sample_index,
                    expected_samples
                );

                let protocol = calibration_protocol(kind, class);
                if kind == PacketKind::Down {
                    let previous = session.last_send_completed_ticks.ok_or(BucketStepFailure {
                        sample_index,
                        phase: "gap",
                        source: CalibrationError::ClockFailure,
                    });
                    let Ok(previous) = previous else {
                        step_result = previous.map(|_| ());
                        break;
                    };
                    if class == SampleClass::Cold {
                        if let Err(source) =
                            session.wait_cold_gap_after(previous, config.cold_idle_gap_us)
                        {
                            step_result = Err(BucketStepFailure {
                                sample_index,
                                phase: "gap",
                                source,
                            });
                            break;
                        }
                    } else if !hot_gap.is_zero()
                        && let Err(source) = session.sleep_with_budget(hot_gap)
                    {
                        step_result = Err(BucketStepFailure {
                            sample_index,
                            phase: "budget",
                            source,
                        });
                        break;
                    }
                }

                let sample = match kind {
                    PacketKind::Down => session.measure_classified_packet(
                        scan_codes,
                        false,
                        class,
                        cold_threshold_ticks,
                        receipt_timeout,
                    ),
                    PacketKind::Up => {
                        let setup = session.prepare_keys_down(scan_codes, receipt_timeout);
                        let setup = match setup {
                            Ok(setup) => setup,
                            Err(source) => {
                                step_result = Err(BucketStepFailure {
                                    sample_index,
                                    phase: failure_phase(&source, "setup down"),
                                    source,
                                });
                                break;
                            }
                        };
                        record(&setup, false, true);
                        if protocol.contains(&CalibrationStep::ColdGapAfterPrepare)
                            && let Err(source) = session.wait_cold_gap_after(
                                setup.call_completed_ticks,
                                config.cold_idle_gap_us,
                            )
                        {
                            step_result = Err(BucketStepFailure {
                                sample_index,
                                phase: "gap",
                                source,
                            });
                            break;
                        }
                        session.measure_classified_packet(
                            scan_codes,
                            true,
                            class,
                            cold_threshold_ticks,
                            receipt_timeout,
                        )
                    }
                };
                let sample = match sample {
                    Ok(sample) => sample,
                    Err(source) => {
                        step_result = Err(BucketStepFailure {
                            sample_index,
                            phase: failure_phase(&source, "measured send"),
                            source,
                        });
                        break;
                    }
                };
                measured.push(sample);
                if kind == PacketKind::Down
                    && let Err(source) = session.cleanup_keys_up(scan_codes, receipt_timeout)
                {
                    step_result = Err(BucketStepFailure {
                        sample_index,
                        phase: failure_phase(&source, "cleanup"),
                        source,
                    });
                    break;
                }
            }
        }

        let cleanup = session.close();
        if let Err(failure) = step_result {
            return Err(bucket_failure(kind, class, polyphony, failure, cleanup));
        }
        if measured.len() != expected_samples as usize {
            return Err(bucket_failure(
                kind,
                class,
                polyphony,
                BucketStepFailure {
                    sample_index: measured.len() as u32 + 1,
                    phase: "measured send",
                    source: CalibrationError::StatisticsOverflow,
                },
                cleanup,
            ));
        }
        if !cleanup.cleanup_success || cleanup.cleanup_verification_inconclusive {
            return Err(bucket_failure(
                kind,
                class,
                polyphony,
                BucketStepFailure {
                    sample_index: expected_samples,
                    phase: "cleanup",
                    source: CalibrationError::CleanupFailed {
                        stuck_keys: cleanup.cleanup_stuck_keys.clone(),
                    },
                },
                cleanup,
            ));
        }

        let bucket = aggregate_samples(&measured)?;
        let total_attempted = warmup_attempted
            .saturating_add(setup_attempted)
            .saturating_add(measured.len() as u64);
        let total_anomalous = warmup_anomalous
            .saturating_add(setup_anomalous)
            .saturating_add(bucket.anomaly_count);
        let total_timed_out = warmup_timed_out
            .saturating_add(setup_timed_out)
            .saturating_add(bucket.timeout_count);

        Ok(CalibrationBucketOutput {
            version: CALIBRATION_SCHEMA_VERSION,
            measurement_protocol_version: MEASUREMENT_PROTOCOL_VERSION,
            source_git_sha: env!("SKY_NATIVE_BUILD_COMMIT"),
            native_build_id: env!("SKY_NATIVE_BUILD_COMMIT"),
            dirty_worktree: env!("SKY_NATIVE_DIRTY_WORKTREE") == "true",
            native_source_fingerprint: env!("SKY_NATIVE_SOURCE_FINGERPRINT"),
            rustc_version: env!("SKY_RUSTC_VERSION"),
            evidence_kind: "injected_raw_input_delivery_proxy",
            host_fingerprint: build_host_fingerprint()?,
            configuration: config.clone(),
            kind,
            class,
            polyphony,
            attempted: measured.len() as u64,
            setup_attempted,
            setup_anomalous,
            setup_timed_out,
            warmup_attempted,
            warmup_anomalous,
            warmup_timed_out,
            total_attempted,
            total_anomalous,
            total_timed_out,
            bucket,
            worst_samples: compact_worst_samples(&measured, 16)?,
            anomalous_samples: anomalous_sample_evidence(&measured)?,
            cleanup,
        })
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

    pub fn run_calibration_bucket(
        _config: &CalibrationConfig,
        _kind: PacketKind,
        _class: SampleClass,
    ) -> Result<CalibrationBucketOutput, CalibrationError> {
        Err(CalibrationError::PlatformUnsupported)
    }

    pub fn build_host_fingerprint() -> Result<HostFingerprint, CalibrationError> {
        Ok(HostFingerprint {
            qpc_frequency_hz: 0,
            win32_build: None,
            sampled_at_us: 0,
        })
    }
}

// ─── Aggregation helpers ──────────────────────────────────────────────────────

fn checked_increment(value: &mut u64) -> Result<(), CalibrationError> {
    *value = value
        .checked_add(1)
        .ok_or(CalibrationError::StatisticsOverflow)?;
    Ok(())
}

fn sample_evidence(
    sample: &CalibrationSample,
) -> Result<CalibrationSampleEvidence, CalibrationError> {
    Ok(CalibrationSampleEvidence {
        clean: sample.is_complete() && !sample.anomalies.any(),
        call_duration_us: sample.call_duration_us()?,
        first_receipt_us: sample.first_receipt_latency_us()?,
        last_receipt_us: sample.last_receipt_latency_us()?,
        intra_chord_spread_us: sample.intra_chord_spread_us()?,
        receipt_count: sample.receipt_count,
        expected_receipt_count: sample.expected_receipt_count,
        win32_error: sample.win32_error,
        anomalies: sample.anomalies.clone(),
    })
}

fn evidence_score(evidence: &CalibrationSampleEvidence) -> u64 {
    let first = evidence.first_receipt_us.unwrap_or_default().unsigned_abs();
    let last = evidence.last_receipt_us.unwrap_or_default().unsigned_abs();
    evidence
        .call_duration_us
        .saturating_add(first)
        .saturating_add(last)
        .saturating_add(evidence.intra_chord_spread_us.unwrap_or_default())
}

fn compact_worst_samples(
    samples: &[CalibrationSample],
    limit: usize,
) -> Result<Vec<CalibrationSampleEvidence>, CalibrationError> {
    let mut evidence = samples
        .iter()
        .map(sample_evidence)
        .collect::<Result<Vec<_>, _>>()?;
    evidence.sort_by_key(|sample| std::cmp::Reverse(evidence_score(sample)));
    evidence.truncate(limit);
    Ok(evidence)
}

fn anomalous_sample_evidence(
    samples: &[CalibrationSample],
) -> Result<Vec<CalibrationSampleEvidence>, CalibrationError> {
    samples
        .iter()
        .filter(|sample| sample.anomalies.any())
        .map(sample_evidence)
        .collect()
}

fn aggregate_samples(samples: &[CalibrationSample]) -> Result<BucketStats, CalibrationError> {
    let n = samples.len() as u64;
    let clean = samples
        .iter()
        .filter(|sample| sample.is_complete() && !sample.anomalies.any())
        .count() as u64;
    let partial_send = samples
        .iter()
        .filter(|sample| sample.anomalies.partial_send)
        .count() as u64;
    let error_count = samples
        .iter()
        .filter(|s| !s.is_complete() || s.anomalies.any())
        .count() as u64;
    let timeout_count = samples.iter().filter(|s| s.anomalies.timeout).count() as u64;
    let anomaly_count = samples.iter().filter(|s| s.anomalies.any()).count() as u64;
    let class_mismatch_count = samples
        .iter()
        .filter(|s| s.anomalies.class_mismatch)
        .count() as u64;

    let clean_samples: Vec<&CalibrationSample> = samples
        .iter()
        .filter(|sample| sample.is_complete() && !sample.anomalies.any())
        .collect();
    let call_durations: Vec<u64> = clean_samples
        .iter()
        .map(|sample| sample.call_duration_us())
        .collect::<Result<_, _>>()?;
    let call_duration_us = quantile_stats_u64(&call_durations)?;

    let first_latencies: Vec<i64> = clean_samples
        .iter()
        .map(|sample| sample.first_receipt_latency_us())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let first_receipt_us = if first_latencies.is_empty() {
        None
    } else {
        Some(quantile_stats_i64(&first_latencies)?)
    };

    let last_latencies: Vec<i64> = clean_samples
        .iter()
        .map(|sample| sample.last_receipt_latency_us())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let last_receipt_us = if last_latencies.is_empty() {
        None
    } else {
        Some(quantile_stats_i64(&last_latencies)?)
    };

    let spreads: Vec<u64> = clean_samples
        .iter()
        .map(|sample| sample.intra_chord_spread_us())
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();
    let intra_chord_spread_us = if spreads.is_empty() {
        None
    } else {
        Some(quantile_stats_u64(&spreads)?)
    };

    Ok(BucketStats {
        attempted: n,
        clean,
        clean_sample_count: clean,
        rejected: n - clean,
        partial_send,
        sample_count: n,
        error_count,
        timeout_count,
        anomaly_count,
        class_mismatch_count,
        call_duration_us,
        first_receipt_us,
        last_receipt_us,
        intra_chord_spread_us,
    })
}

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

pub fn run_calibration_bucket_json(
    config: &CalibrationConfig,
    kind: PacketKind,
    class: SampleClass,
) -> Result<String, CalibrationError> {
    let output = platform::run_calibration_bucket(config, kind, class)?;
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
        assert_eq!(CALIBRATION_SCHEMA_VERSION, 8);
        assert_eq!(CALIBRATION_CLEANUP_RESERVE_SECONDS, 5);
        assert_eq!(CALIBRATION_MIN_TOTAL_BUDGET_SECONDS, 6);
        assert_eq!(cfg.hot_gap_target_us, 5_000);
        assert_eq!(cfg.cold_threshold_us, 20_000);
        assert_eq!(cfg.cold_idle_gap_us, 25_000);
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
        };
        // For polyphony-1 first == last, so spread = 0 (not None).
        assert_eq!(sample.intra_chord_spread_us().unwrap(), Some(0));
    }

    #[test]
    fn aggregation_uses_only_clean_samples_for_timing_quantiles() {
        let clean = CalibrationSample {
            sequence_id: 1,
            call_started_ticks: QpcTicks::from_raw(0),
            call_completed_ticks: QpcTicks::from_raw(1_000_000_000),
            first_receipt_ticks: Some(QpcTicks::from_raw(1_000_000_100)),
            last_receipt_ticks: Some(QpcTicks::from_raw(1_000_000_100)),
            receipt_count: 1,
            expected_receipt_count: 1,
            win32_error: None,
            actual_idle_gap_ticks: None,
            observed_class: None,
            anomalies: SampleAnomalies::default(),
        };
        let rejected = CalibrationSample {
            sequence_id: 2,
            call_started_ticks: QpcTicks::from_raw(0),
            call_completed_ticks: QpcTicks::from_raw(2_000_000_000),
            first_receipt_ticks: None,
            last_receipt_ticks: None,
            receipt_count: 0,
            expected_receipt_count: 1,
            win32_error: None,
            actual_idle_gap_ticks: None,
            observed_class: None,
            anomalies: SampleAnomalies {
                timeout: true,
                class_mismatch: true,
                ..SampleAnomalies::default()
            },
        };

        let stats = aggregate_samples(&[clean, rejected]).unwrap();
        assert_eq!(stats.attempted, 2);
        assert_eq!(stats.clean_sample_count, 1);
        assert_eq!(stats.class_mismatch_count, 1);
        assert_eq!(stats.rejected, 1);
        assert_eq!(
            stats.call_duration_us.max,
            qpc_ticks_to_us(QpcTicks::from_raw(1_000_000_000)).unwrap()
        );
        assert_eq!(stats.timeout_count, 1);
    }

    #[test]
    fn quantile_sum_overflow_is_an_error() {
        assert!(matches!(
            quantile_stats_u64(&[u64::MAX, u64::MAX]),
            Err(CalibrationError::StatisticsOverflow)
        ));
    }

    #[test]
    fn up_cold_protocol_waits_after_setup_down() {
        assert_eq!(
            calibration_protocol(PacketKind::Up, SampleClass::Cold),
            &[
                CalibrationStep::PrepareDown,
                CalibrationStep::ColdGapAfterPrepare,
                CalibrationStep::MeasureUp,
            ]
        );
        assert_eq!(
            calibration_protocol(PacketKind::Up, SampleClass::Hot),
            &[
                CalibrationStep::PrepareDown,
                CalibrationStep::HotGap,
                CalibrationStep::MeasureUp,
            ]
        );
    }

    #[test]
    fn down_protocol_measures_down_then_cleans_up() {
        assert_eq!(
            calibration_protocol(PacketKind::Down, SampleClass::Cold),
            &[CalibrationStep::MeasureDown, CalibrationStep::CleanupUp]
        );
    }

    #[test]
    fn calibration_sample_uses_exact_sender_tick_boundaries() {
        let result = PlatformSendResult {
            requested: 1,
            inserted: 1,
            started_ticks: QpcTicks::from_raw(101),
            completed_ticks: Some(QpcTicks::from_raw(211)),
            completed_us: 0,
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
            completed_us: 0,
            win32_error: 0,
            timing_error: Some(crate::clock::QpcError::CounterUnavailable),
        };
        assert!(matches!(
            exact_sendinput_boundaries(&result),
            Err(CalibrationError::ClockFailure)
        ));
    }
}
