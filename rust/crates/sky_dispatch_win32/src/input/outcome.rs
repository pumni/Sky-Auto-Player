use super::scan_code::scan_codes_from_mask;
use crate::clock::QpcTicks;
use smallvec::SmallVec;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformSendResult {
    pub requested: u8,
    pub inserted: u8,
    pub started_ticks: QpcTicks,
    pub completed_ticks: Option<QpcTicks>,
    pub win32_error: u32,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalPacket {
    pub up_mask: u16,
    pub down_mask: u16,
}

impl PhysicalPacket {
    pub const fn new(up_mask: u16, down_mask: u16) -> Self {
        Self { up_mask, down_mask }
    }

    pub const fn event_count(self) -> u8 {
        (self.up_mask.count_ones() + self.down_mask.count_ones()) as u8
    }

    pub const fn is_up_only(self) -> bool {
        self.down_mask == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketRetryReason {
    None,
    ZeroProgress,
    PartialProgress { inserted_count: u8 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendTransactionStatus {
    Complete,
    ZeroProgress,
    PartialProgress,
    IntegrityLost,
    ClockFailureBeforeSend,
    ClockFailureAfterSend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendEvidence {
    pub requested_mask: u16,
    pub confirmed_mask: u16,
    pub skipped_mask: u16,
    pub first_inserted: u8,
    pub attempts: u8,
    pub zero_progress_retries: u8,
    pub retry_reason: PacketRetryReason,
    pub first_win32_error: Option<u32>,
    pub last_win32_error: Option<u32>,
    pub started_ticks: Option<QpcTicks>,
    pub completed_ticks: Option<QpcTicks>,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendTransactionOutcome {
    pub status: SendTransactionStatus,
    pub evidence: SendEvidence,
}

impl SendTransactionOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self.status, SendTransactionStatus::Complete)
    }

    pub fn completed_us(&self) -> u64 {
        match (self.evidence.started_ticks, self.evidence.completed_ticks) {
            (Some(start), Some(end)) => crate::clock::qpc_ticks_to_us(QpcTicks::from_raw(
                end.as_u64().saturating_sub(start.as_u64()),
            ))
            .unwrap_or(0),
            _ => 0,
        }
    }

    pub fn sent_scan_codes(&self) -> SmallVec<[u16; 15]> {
        scan_codes_from_mask(self.evidence.confirmed_mask).into()
    }

    pub fn skipped_duplicates(&self) -> SmallVec<[u16; 15]> {
        scan_codes_from_mask(self.evidence.skipped_mask).into()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseAllOutcome {
    pub attempted_mask: u16,
    pub transport_anomaly: bool,
    pub released_successfully: bool,
    pub stuck_mask: u16,
    pub verification_inconclusive: bool,
    pub attempts: u8,
}

impl ReleaseAllOutcome {
    pub fn attempted(&self) -> Vec<u16> {
        scan_codes_from_mask(self.attempted_mask)
    }

    pub fn stuck_keys(&self) -> Vec<u16> {
        scan_codes_from_mask(self.stuck_mask)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhysicalKeyPreflightError {
    UserHeld(Vec<u16>),
    VerificationInconclusive,
}

impl fmt::Display for PhysicalKeyPreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserHeld(keys) => write!(
                f,
                "instrument keys are physically held before playback: {keys:?}"
            ),
            Self::VerificationInconclusive => {
                f.write_str("instrument key physical-state verification was inconclusive")
            }
        }
    }
}
