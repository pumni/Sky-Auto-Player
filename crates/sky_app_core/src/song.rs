//! Pure song decoding, deterministic Piano planning, scheduling, and bounded risk analysis.
//!
//! This module deliberately owns no filesystem, delivery, Win32, or
//! realtime-player concerns. A caller supplies bytes and receives an
//! immutable plan that an outer native adapter may hand to the qualified
//! realtime engine.

use serde::{Deserialize, Serialize};

pub mod decode;
pub use decode::*;
mod legacy;
mod planner;
mod risk;
mod schedule;
pub use legacy::{parse_song_json, scan_code_for_key};
pub use planner::*;
pub use risk::{
    DenseCluster, RiskReport, analyze_schedule, analyze_schedule_with_context,
    analyze_schedule_with_notes,
};
pub(crate) use schedule::build_schedule_from_scaled_onsets;
pub use schedule::{
    OnsetChord, ScheduleMetadata, build_schedule, build_schedule_with_policy, frame_us,
};
mod numeric;
pub(crate) use numeric::round_scaled_onset_checked;
use numeric::{bounded_tempo_scale_recommendation, checked_note_gap_us, truncated_f64_to_i64};

pub const SKY_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
];
pub const KEY_NAMES: [&str; 15] = [
    "Key0", "Key1", "Key2", "Key3", "Key4", "Key5", "Key6", "Key7", "Key8", "Key9", "Key10",
    "Key11", "Key12", "Key13", "Key14",
];
pub const VALID_FPS: [u16; 7] = [30, 60, 90, 120, 144, 165, 240];
pub const HOLD_FRAMES: [f64; 3] = [1.0, 1.25, 1.5];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub time_ms: i64,
    pub key: String,
    pub source_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Song {
    pub name: String,
    pub notes: Vec<Note>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SongError {
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("song root must be an object")]
    InvalidRoot,
    #[error("song list is empty")]
    EmptySongList,
    #[error("songNotes is missing")]
    MissingNotes,
    #[error("songNotes must be an array")]
    InvalidNotes,
    #[error("note {index} must be an object")]
    InvalidNote { index: usize },
    #[error("note {index} is missing time")]
    MissingTime { index: usize },
    #[error("note {index} is missing key")]
    MissingKey { index: usize },
    #[error("note {index} has invalid time")]
    InvalidTime { index: usize },
    #[error("note {index} has negative time")]
    NegativeTime { index: usize },
    #[error("note {index} has an unmapped key: {key}")]
    UnmappedKey { index: usize, key: String },
    #[error("schedule tempo must be finite and positive")]
    InvalidTempo,
    #[error("schedule FPS is unsupported")]
    InvalidFps,
    #[error("schedule hold frame value is unsupported")]
    InvalidHold,
    #[error("schedule timing margin is invalid")]
    InvalidTimingMargin,
    #[error("schedule timing value overflowed")]
    TimingOverflow,
    #[error("same-key repeat is infeasible: {interval_us}us")]
    ImpossibleRepeat { interval_us: u64 },
    #[error("schedule contains no actions")]
    EmptySchedule,
}

#[cfg(test)]
#[path = "song/tests.rs"]
mod tests;
