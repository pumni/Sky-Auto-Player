//! Shared transport evidence predicate for production observer records.

use sky_dispatch_win32::input::{PacketRetryReason, SendTransactionStatus};

/// Transport/timing evidence captured for one physical dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DispatchObservationEvidence {
    pub status: SendTransactionStatus,
    pub attempts: u8,
    pub retry_reason: PacketRetryReason,
    pub requested_count: usize,
    pub confirmed_count: usize,
    pub skipped_count: usize,
    pub timing_valid: bool,
    pub transport_anomaly: bool,
    pub recovery_used: bool,
    pub chord_integrity_lost: bool,
}

/// Canonical clean-dispatch predicate shared by Down, Mixed, and Up evidence.
pub fn is_clean_dispatch_observation(evidence: DispatchObservationEvidence) -> bool {
    evidence.status == SendTransactionStatus::Complete
        && evidence.attempts == 1
        && evidence.retry_reason == PacketRetryReason::None
        && evidence.skipped_count == 0
        && evidence.confirmed_count == evidence.requested_count
        && evidence.timing_valid
        && !evidence.transport_anomaly
        && !evidence.recovery_used
        && !evidence.chord_integrity_lost
}
