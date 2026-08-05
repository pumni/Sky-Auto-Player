use crate::clock::QpcTicks;
use smallvec::SmallVec;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSendResult {
    pub requested: u32,
    pub inserted: u32,
    /// QPC boundaries for the syscall. `completed_ticks` is absent only when
    /// the post-call clock query failed; in that case `timing_error` is set.
    pub started_ticks: QpcTicks,
    pub completed_ticks: Option<QpcTicks>,
    pub completed_us: u64,
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
pub enum PhysicalSendOutcome {
    Complete {
        requested: u8,
        inserted: u8,
        attempts: u8,
        started_ticks: QpcTicks,
        completed_ticks: QpcTicks,
    },
    ZeroProgress {
        requested: u8,
        attempts: u8,
        first_error: u32,
        last_error: u32,
        started_ticks: QpcTicks,
        completed_ticks: QpcTicks,
    },
    Partial {
        requested: u8,
        inserted_count: u8,
        attempts: u8,
        first_error: u32,
        last_error: u32,
        started_ticks: QpcTicks,
        completed_ticks: QpcTicks,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSendResult {
    pub sent: SmallVec<[u16; 15]>,
    pub skipped_duplicates: SmallVec<[u16; 15]>,
    pub success: bool,
    pub error: Option<String>,
    pub send_completed_us: u64,
    pub send_started_ticks: Option<QpcTicks>,
    pub send_completed_ticks: Option<QpcTicks>,
    pub first_win32_error: Option<u32>,
    pub last_win32_error: Option<u32>,
    pub send_attempts: u8,
    pub zero_progress_retries: u8,
    pub first_inserted: u8,
    pub partial_progress: bool,
    pub retried_after_zero_progress: bool,
    pub chord_integrity_lost: bool,
    pub keys_inserted_before_failure: u8,
    pub keys_rolled_back: u8,
    pub rollback_residue_keys: u8,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownSendOutcome {
    Complete {
        completed_us: u64,
        started_ticks: Option<QpcTicks>,
        completed_ticks: Option<QpcTicks>,
        sent: SmallVec<[u16; 15]>,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        retried_after_zero_progress: bool,
        timing_error: Option<crate::clock::QpcError>,
    },
    ZeroProgress {
        error: Option<u32>,
        completed_us: u64,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
        started_ticks: Option<QpcTicks>,
        completed_ticks: Option<QpcTicks>,
        timing_error: Option<crate::clock::QpcError>,
    },
    IntegrityLost {
        inserted_before_failure: u8,
        rolled_back: u8,
        rollback_residue: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
        completed_us: u64,
        started_ticks: Option<QpcTicks>,
        completed_ticks: Option<QpcTicks>,
        sent: SmallVec<[u16; 15]>,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        timing_error: Option<crate::clock::QpcError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    pub sent: SmallVec<[u16; 15]>,
    pub completed_us: u64,
    pub started_ticks: Option<QpcTicks>,
    pub completed_ticks: Option<QpcTicks>,
    pub success: bool,
    pub keys_dropped: u64,
    pub first_win32_error: Option<u32>,
    pub last_win32_error: Option<u32>,
    pub send_attempts: u8,
    pub zero_progress_retries: u8,
    pub first_inserted: u8,
    pub partial_progress: bool,
    pub retried_after_zero_progress: bool,
    pub chord_integrity_lost: bool,
    pub keys_inserted_before_failure: u8,
    pub keys_rolled_back: u8,
    pub rollback_residue_keys: u8,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAllOutcome {
    pub attempted: Vec<u16>,
    pub released_successfully: bool,
    pub stuck_keys: Vec<u16>,
    pub verification_inconclusive: bool,
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
