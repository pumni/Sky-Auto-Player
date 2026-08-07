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

impl SendEvidence {
    pub fn duration_ticks(
        &self,
    ) -> Result<crate::clock::DurationTicks, sky_dispatch_core::time::TimeArithmeticError> {
        match (self.started_ticks, self.completed_ticks) {
            (Some(start), Some(end)) => end.checked_duration_since(start),
            _ => Err(sky_dispatch_core::time::TimeArithmeticError::NegativeOrder),
        }
    }
}

pub fn classify_send_status(
    inserted: usize,
    requested: usize,
    win32_error: u32,
    started_ticks: Option<QpcTicks>,
    completed_ticks: Option<QpcTicks>,
) -> SendTransactionStatus {
    let clock_missing = started_ticks.is_none() || completed_ticks.is_none();
    if clock_missing {
        if inserted > 0 {
            SendTransactionStatus::ClockFailureAfterSend
        } else {
            SendTransactionStatus::ClockFailureBeforeSend
        }
    } else if inserted < requested {
        if inserted == 0 {
            SendTransactionStatus::ZeroProgress
        } else {
            SendTransactionStatus::IntegrityLost
        }
    } else if win32_error != 0 {
        SendTransactionStatus::IntegrityLost
    } else {
        SendTransactionStatus::Complete
    }
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

    pub fn sent_scan_codes(&self) -> SmallVec<[u16; 15]> {
        scan_codes_from_mask(self.evidence.confirmed_mask)
    }

    pub fn skipped_duplicates(&self) -> SmallVec<[u16; 15]> {
        scan_codes_from_mask(self.evidence.skipped_mask)
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
            .into_iter()
            .collect()
    }

    pub fn stuck_keys(&self) -> Vec<u16> {
        scan_codes_from_mask(self.stuck_mask).into_iter().collect()
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
