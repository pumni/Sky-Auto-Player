//! Strict, versioned source decoding for JSON Sky sheet files.
//!
//! `parse_song_json` remains the legacy catalog API. This module exposes the
//! richer source-preserving path used by the compatibility planner work.

mod legacy;
mod modern;
mod tables;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub const MAX_SHEET_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_SOURCE_NOTES: usize = 200_000;
pub const MAX_SOURCE_TRACKS: usize = 16;
pub const MAX_SOURCE_COLUMNS: usize = 200_000;

const TEMPO_CHANGERS: [f64; 4] = [1.0, 0.5, 0.25, 0.125];
const PITCH_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceGame {
    Sky,
    Genshin,
}

impl SourceGame {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "Sky" => Some(Self::Sky),
            "Genshin" => Some(Self::Genshin),
            _ => None,
        }
    }

    fn as_storage_id(self) -> &'static str {
        match self {
            Self::Sky => "Sky",
            Self::Genshin => "Genshin",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SongFormat {
    LegacyFlat,
    Composed { version: u8 },
    Recorded { version: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pitchability {
    Pitched,
    Unpitched,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum CanonicalPitch {
    /// The old flat format already names one of Sky's fifteen physical keys.
    SkyKeyIndex(u8),
    /// An absolute semitone Note Number after applying the source format's
    /// instrument and effective pitch rules.
    NoteNumber {
        number: i32,
        source_pitchability: Pitchability,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum RawNoteIdentity {
    LegacyKeyIndex(u8),
    LegacyIndex(u32),
    NominalId(i32),
    AbsoluteNoteNumber(i32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourcePosition {
    FlatSongNote {
        note_index: usize,
    },
    ComposedColumn {
        column_index: usize,
        note_index: usize,
    },
    RecordedNote {
        note_index: usize,
    },
    TrackNote {
        track_index: usize,
        note_index: usize,
        column_index: Option<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceTrack {
    pub index: usize,
    pub instrument: Option<String>,
    /// Effective source pitch; a track override has already been resolved
    /// against the song pitch.
    pub effective_pitch: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalEvent {
    /// Stable ordinal of the serialized source note entry. Layer expansion
    /// keeps the same source index for each emitted track event.
    pub source_index: usize,
    pub source_position: SourcePosition,
    pub track_index: usize,
    pub onset_us: u64,
    /// Zero means the source format authored a one-shot note without sustain.
    pub duration_us: u64,
    pub raw_identity: RawNoteIdentity,
    pub pitch: CanonicalPitch,
    pub layer_mask: Option<u16>,
    pub span_columns: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecodeWarning {
    EmptyLayerMask {
        source_index: usize,
    },
    PotentialPhysicalKeyMerge {
        onset_us: u64,
        key_index: u8,
        occurrences: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodeDiagnostics {
    /// Serialized note tuples/objects before any layer expansion.
    pub source_note_count: usize,
    pub decoded_event_count: usize,
    pub source_track_count: usize,
    pub source_column_count: usize,
    pub empty_layer_mask_count: usize,
    pub authored_sustain_count: usize,
    /// Number of extra old-flat events that share an onset and physical key.
    /// Events remain intact for the later planner to diagnose and merge.
    pub potential_physical_key_merge_count: usize,
    pub warnings: Vec<DecodeWarning>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalSong {
    pub name: String,
    pub format: SongFormat,
    pub source_game: SourceGame,
    /// Exact stored `data.appName`, when present.
    pub source_app_name: Option<String>,
    pub source_pitch: String,
    pub bpm: Option<f64>,
    pub tracks: Vec<SourceTrack>,
    /// Stable ascending-onset order; equal onsets retain serialized source order.
    pub events: Vec<CanonicalEvent>,
    pub diagnostics: DecodeDiagnostics,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SongDecodeError {
    #[error("sheet payload exceeds {limit_bytes} bytes (got {actual_bytes})")]
    PayloadTooLarge {
        actual_bytes: usize,
        limit_bytes: usize,
    },
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("sheet root must be an object or an array containing one song")]
    InvalidRoot,
    #[error("song array is empty")]
    EmptySongArray,
    #[error("song arrays with {count} entries are unsupported; provide one song per file")]
    MultipleSongsUnsupported { count: usize },
    #[error("encrypted sheets are unsupported")]
    Encrypted,
    #[error("unsupported sheet format: {reason}")]
    UnsupportedFormat { reason: String },
    #[error("unsupported {kind} sheet version {version}")]
    UnsupportedVersion { kind: String, version: u64 },
    #[error("required field `{path}` is missing")]
    MissingField { path: String },
    #[error("invalid field `{path}`: {reason}")]
    InvalidField { path: String, reason: String },
    #[error("source game is missing or ambiguous; `data.appName` must be `Sky` or `Genshin`")]
    AmbiguousSourceGame,
    #[error("unsupported source game `{value}`; expected `Sky` or `Genshin`")]
    UnsupportedSourceGame { value: String },
    #[error("unsupported {game} source instrument `{instrument}`")]
    UnsupportedInstrument { game: String, instrument: String },
    #[error("resource limit exceeded for {resource}: {actual} (maximum {maximum})")]
    ResourceLimit {
        resource: String,
        actual: usize,
        maximum: usize,
    },
    #[error("timestamp or duration conversion overflow at `{path}`")]
    TimeOverflow { path: String },
    #[error("source note number conversion overflow at `{path}`")]
    NoteNumberOverflow { path: String },
}

/// Detects the authoritative source schema without decoding its notes.
///
/// A versioned schema always wins over a coexisting `songNotes` projection.
pub fn detect_song_format(bytes: &[u8]) -> Result<SongFormat, SongDecodeError> {
    if bytes.len() > MAX_SHEET_BYTES {
        return Err(SongDecodeError::PayloadTooLarge {
            actual_bytes: bytes.len(),
            limit_bytes: MAX_SHEET_BYTES,
        });
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| SongDecodeError::InvalidJson(error.to_string()))?;
    let root = one_song_root(&value)?;
    let object = root.as_object().ok_or(SongDecodeError::InvalidRoot)?;
    reject_encrypted(object)?;
    detect_format(object)
}

/// Decodes JSON bytes into source-preserving canonical musical events.
///
/// The filename extension belongs to the filesystem adapter; `.txt` uses this
/// same JSON-only decoder and does not enable ABC or other text imports.
pub fn decode_song_json(
    bytes: &[u8],
    fallback_name: &str,
) -> Result<CanonicalSong, SongDecodeError> {
    if bytes.len() > MAX_SHEET_BYTES {
        return Err(SongDecodeError::PayloadTooLarge {
            actual_bytes: bytes.len(),
            limit_bytes: MAX_SHEET_BYTES,
        });
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| SongDecodeError::InvalidJson(error.to_string()))?;
    let root = one_song_root(&value)?;
    let object = root.as_object().ok_or(SongDecodeError::InvalidRoot)?;
    reject_encrypted(object)?;
    let format = detect_format(object)?;
    let name = decode_name(object, fallback_name)?;
    match format {
        SongFormat::LegacyFlat => legacy::decode_legacy_flat(object, format, name),
        SongFormat::Composed { version } if version <= 3 => {
            legacy::decode_composed_legacy(object, version, format, name)
        }
        SongFormat::Composed { .. } => modern::decode_composed_tracks(object, format, name),
        SongFormat::Recorded { version } if version <= 2 => {
            legacy::decode_recorded_legacy(object, version, format, name)
        }
        SongFormat::Recorded { .. } => modern::decode_recorded_tracks(object, format, name),
    }
}

fn one_song_root(value: &Value) -> Result<&Value, SongDecodeError> {
    match value {
        Value::Array(items) if items.is_empty() => Err(SongDecodeError::EmptySongArray),
        Value::Array(items) if items.len() == 1 => Ok(&items[0]),
        Value::Array(items) => {
            Err(SongDecodeError::MultipleSongsUnsupported { count: items.len() })
        }
        Value::Object(_) => Ok(value),
        _ => Err(SongDecodeError::InvalidRoot),
    }
}

fn detect_format(object: &Map<String, Value>) -> Result<SongFormat, SongDecodeError> {
    match (object.get("type"), object.get("version")) {
        (None, None) if object.contains_key("songNotes") => Ok(SongFormat::LegacyFlat),
        (None, None) => Err(SongDecodeError::UnsupportedFormat {
            reason: "expected versioned `type`/`version` fields or legacy `songNotes`".into(),
        }),
        (None, Some(_)) => Err(SongDecodeError::MissingField {
            path: "type".into(),
        }),
        (Some(_), None) => Err(SongDecodeError::MissingField {
            path: "version".into(),
        }),
        (Some(kind), Some(version)) => {
            let kind = kind
                .as_str()
                .ok_or_else(|| invalid("type", "must be a string"))?;
            let version = version
                .as_u64()
                .ok_or_else(|| invalid("version", "must be a non-negative integer"))?;
            let (minimum, maximum) = match kind {
                "composed" => (1, 5),
                "recorded" => (1, 4),
                _ => {
                    return Err(SongDecodeError::UnsupportedFormat {
                        reason: format!("unknown song type `{kind}`"),
                    });
                }
            };
            if !(minimum..=maximum).contains(&version) {
                return Err(SongDecodeError::UnsupportedVersion {
                    kind: kind.into(),
                    version,
                });
            }
            let version = version as u8;
            match kind {
                "composed" => Ok(SongFormat::Composed { version }),
                "recorded" => Ok(SongFormat::Recorded { version }),
                _ => unreachable!("kind was validated above"),
            }
        }
    }
}

fn reject_encrypted(object: &Map<String, Value>) -> Result<(), SongDecodeError> {
    for (field, value) in [
        ("isEncrypted", object.get("isEncrypted")),
        (
            "data.isEncrypted",
            object.get("data").and_then(|data| data.get("isEncrypted")),
        ),
    ] {
        if let Some(value) = value {
            match value.as_bool() {
                Some(true) => return Err(SongDecodeError::Encrypted),
                Some(false) => {}
                None if value.is_null() => {}
                None => return Err(invalid(field, "must be a boolean")),
            }
        }
    }
    Ok(())
}

fn decode_name(
    object: &Map<String, Value>,
    fallback_name: &str,
) -> Result<String, SongDecodeError> {
    match object.get("name") {
        Some(Value::String(name)) => Ok(name.clone()),
        Some(Value::Null) | None if !fallback_name.trim().is_empty() => Ok(fallback_name.into()),
        Some(Value::Null) | None => Ok("Untitled".into()),
        Some(_) => Err(invalid("name", "must be a string")),
    }
}

fn decode_source_game(
    object: &Map<String, Value>,
    legacy_flat: bool,
) -> Result<(SourceGame, Option<String>), SongDecodeError> {
    let app_name = object
        .get("data")
        .map(|value| {
            value
                .as_object()
                .ok_or_else(|| invalid("data", "must be an object"))
        })
        .transpose()?
        .and_then(|data| data.get("appName"));
    let source_app_name = match app_name {
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => return Err(invalid("data.appName", "must be a string")),
        None if legacy_flat => None,
        None => return Err(SongDecodeError::AmbiguousSourceGame),
    };
    match source_app_name.as_deref() {
        Some(value) => {
            let game =
                SourceGame::parse(value).ok_or_else(|| SongDecodeError::UnsupportedSourceGame {
                    value: value.into(),
                })?;
            if legacy_flat && game != SourceGame::Sky {
                return Err(SongDecodeError::UnsupportedFormat {
                    reason: "legacy flat `KeyN` data is in Sky's fifteen-key coordinate space"
                        .into(),
                });
            }
            Ok((game, source_app_name))
        }
        None => Ok((SourceGame::Sky, None)),
    }
}

fn decode_source_pitch(
    object: &Map<String, Value>,
    legacy_flat: bool,
) -> Result<String, SongDecodeError> {
    if legacy_flat
        && object.get("pitch").is_none()
        && let Some(value) = object.get("pitchLevel")
    {
        let level = value
            .as_u64()
            .filter(|level| *level < PITCH_NAMES.len() as u64)
            .ok_or_else(|| invalid("pitchLevel", "must be an integer from 0 through 11"))?;
        return Ok(PITCH_NAMES[level as usize].into());
    }
    match object.get("pitch") {
        Some(Value::String(pitch)) if pitch_index(pitch).is_some() => Ok(pitch.clone()),
        Some(Value::String(_)) => Err(invalid("pitch", "must be a known source pitch")),
        Some(_) => Err(invalid("pitch", "must be a string")),
        None => Ok("C".into()),
    }
}

fn decode_bpm(object: &Map<String, Value>, required: bool) -> Result<Option<f64>, SongDecodeError> {
    let Some(value) = object.get("bpm") else {
        if required {
            return Err(SongDecodeError::MissingField { path: "bpm".into() });
        }
        return Ok(None);
    };
    let bpm = value
        .as_f64()
        .filter(|bpm| bpm.is_finite() && *bpm > 0.0)
        .ok_or_else(|| invalid("bpm", "must be a finite positive number"))?;
    Ok(Some(bpm))
}

fn source_track(
    game: SourceGame,
    instrument: &str,
    effective_pitch: Option<String>,
    pitch_offset: i32,
    index: usize,
) -> Result<SourceTrack, SongDecodeError> {
    if !tables::instrument_supported(game, instrument) {
        return Err(unsupported_instrument(game, instrument));
    }
    let effective_pitch =
        effective_pitch.unwrap_or_else(|| PITCH_NAMES[pitch_offset as usize].into());
    Ok(SourceTrack {
        index,
        instrument: Some(instrument.into()),
        effective_pitch: Some(effective_pitch),
    })
}

fn optional_pitch(
    value: Option<&Value>,
    song_pitch_offset: i32,
    path: &str,
) -> Result<(String, i32), SongDecodeError> {
    match value {
        None | Some(Value::Null) => Ok((
            PITCH_NAMES[song_pitch_offset as usize].into(),
            song_pitch_offset,
        )),
        Some(Value::String(value)) if value.is_empty() => Ok((
            PITCH_NAMES[song_pitch_offset as usize].into(),
            song_pitch_offset,
        )),
        Some(Value::String(value)) => {
            let offset = pitch_index(value).ok_or_else(|| {
                invalid(format!("{path}.pitch"), "must be empty or a known pitch")
            })?;
            Ok((value.clone(), offset as i32))
        }
        Some(_) => Err(invalid(format!("{path}.pitch"), "must be a string or null")),
    }
}

fn track_pitch_offset(track: &SourceTrack, song_pitch: &str) -> i32 {
    track
        .effective_pitch
        .as_deref()
        .and_then(pitch_index)
        .or_else(|| pitch_index(song_pitch))
        .unwrap_or(0) as i32
}

fn composed_boundaries(tempo_ids: &[u8], bpm: f64) -> Result<Vec<u64>, SongDecodeError> {
    let mut boundaries = Vec::with_capacity(tempo_ids.len() + 1);
    boundaries.push(0);
    let mut exact_us = 0.0f64;
    for (index, tempo_id) in tempo_ids.iter().copied().enumerate() {
        let changer = TEMPO_CHANGERS[tempo_id as usize];
        let step_us = (60_000_000.0 / bpm) * changer;
        exact_us += step_us;
        if !step_us.is_finite()
            || !exact_us.is_finite()
            || exact_us < 0.0
            || exact_us >= u64::MAX as f64
        {
            return Err(SongDecodeError::TimeOverflow {
                path: format!("columns[{index}]"),
            });
        }
        boundaries.push(exact_us.round() as u64);
    }
    Ok(boundaries)
}

fn tempo_id(value: &Value, path: &str) -> Result<u8, SongDecodeError> {
    let id = value
        .as_u64()
        .filter(|id| *id < TEMPO_CHANGERS.len() as u64)
        .ok_or_else(|| invalid(path, "must be a known tempo id from 0 through 3"))?;
    Ok(id as u8)
}
fn recorded_v1_layer_mask(value: Option<&Value>, path: &str) -> Result<u16, SongDecodeError> {
    let Some(value) = value else {
        // The upstream v1 decoder uses `note[2] || 1` before interpreting the
        // resulting decimal string as hexadecimal.
        return Ok(1);
    };
    if value.is_null() || value.as_u64() == Some(0) {
        return Ok(1);
    }
    let number = value
        .as_u64()
        .ok_or_else(|| invalid(path, "recorded v1 layer must be a non-negative integer"))?;
    let text = number.to_string();
    let mask = u32::from_str_radix(&text, 16)
        .map_err(|_| invalid(path, "recorded v1 decimal-as-hex layer is invalid"))?;
    validate_layer_mask(mask as u64, path)
}

fn layer_mask_integer(value: &Value, path: &str) -> Result<u16, SongDecodeError> {
    let mask = value
        .as_u64()
        .ok_or_else(|| invalid(path, "must be a non-negative integer bitmask"))?;
    validate_layer_mask(mask, path)
}

fn binary_layer_mask(value: &Value, path: &str) -> Result<u16, SongDecodeError> {
    let bits = value
        .as_str()
        .ok_or_else(|| invalid(path, "composed v1 layer mask must be a binary string"))?;
    if bits.is_empty()
        || bits.len() > MAX_SOURCE_TRACKS
        || !bits.bytes().all(|byte| byte == b'0' || byte == b'1')
    {
        return Err(invalid(path, "must contain 1 through 16 binary digits"));
    }
    let reversed = bits.chars().rev().collect::<String>();
    let mask = u32::from_str_radix(&reversed, 2)
        .map_err(|_| invalid(path, "invalid reversed binary layer mask"))?;
    validate_layer_mask(mask as u64, path)
}

fn hex_layer_mask(value: &Value, path: &str) -> Result<u16, SongDecodeError> {
    let digits = value
        .as_str()
        .ok_or_else(|| invalid(path, "hexadecimal layer mask must be a string"))?;
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid(path, "must contain 1 through 4 hexadecimal digits"));
    }
    let mask = u32::from_str_radix(digits, 16)
        .map_err(|_| invalid(path, "invalid hexadecimal layer mask"))?;
    validate_layer_mask(mask as u64, path)
}

fn validate_layer_mask(mask: u64, path: &str) -> Result<u16, SongDecodeError> {
    if mask > u16::MAX as u64 {
        return Err(invalid(path, "selects more than 16 source tracks"));
    }
    Ok(mask as u16)
}

fn decode_flat_key(key: &str) -> Option<(u8, u16)> {
    for index in 0..15u8 {
        let suffix = format!("Key{index}");
        if key == suffix {
            return Some((index, 1));
        }
        for (prefix, mask) in [("1", 1), ("2", 2), ("3", 3)] {
            if key == format!("{prefix}{suffix}") {
                return Some((index, mask));
            }
        }
    }
    None
}

fn milliseconds_to_microseconds(value: &Value, path: &str) -> Result<u64, SongDecodeError> {
    let milliseconds = value
        .as_f64()
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| invalid(path, "must be a finite non-negative millisecond value"))?;
    let microseconds = milliseconds * 1_000.0;
    if !microseconds.is_finite() || microseconds >= u64::MAX as f64 {
        return Err(SongDecodeError::TimeOverflow { path: path.into() });
    }
    Ok(microseconds.round() as u64)
}

fn nonnegative_index(value: &Value, path: &str) -> Result<usize, SongDecodeError> {
    let index = value
        .as_u64()
        .ok_or_else(|| invalid(path, "must be a non-negative integer"))?;
    usize::try_from(index).map_err(|_| invalid(path, "is outside the supported index range"))
}

fn integer_i32(value: &Value, path: &str) -> Result<i32, SongDecodeError> {
    let integer = value
        .as_i64()
        .ok_or_else(|| invalid(path, "must be a finite integer note number"))?;
    i32::try_from(integer).map_err(|_| invalid(path, "is outside the supported note-number range"))
}

fn pitch_index(pitch: &str) -> Option<usize> {
    PITCH_NAMES.iter().position(|name| *name == pitch)
}

fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a Vec<Value>, SongDecodeError> {
    required_value(object, key, path)?
        .as_array()
        .ok_or_else(|| invalid(path, "must be an array"))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    prefix: &str,
) -> Result<&'a str, SongDecodeError> {
    required_value(object, key, prefix)?
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("{prefix}.{key}"), "must be a non-empty string"))
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    prefix: &str,
) -> Result<&'a Value, SongDecodeError> {
    object
        .get(key)
        .ok_or_else(|| SongDecodeError::MissingField {
            path: if prefix.is_empty() {
                key.into()
            } else {
                format!("{prefix}.{key}")
            },
        })
}

fn highest_track(mask: u16) -> usize {
    (u16::BITS - mask.leading_zeros()) as usize
}

fn flat_key_collisions(events: &[CanonicalEvent]) -> (usize, Vec<DecodeWarning>) {
    let mut counts = BTreeMap::<(u64, u8), usize>::new();
    for event in events {
        if let CanonicalPitch::SkyKeyIndex(index) = event.pitch {
            *counts.entry((event.onset_us, index)).or_default() += 1;
        }
    }
    let mut merged = 0usize;
    let mut warnings = Vec::new();
    for ((onset_us, key_index), occurrences) in counts {
        if occurrences > 1 {
            merged += occurrences - 1;
            warnings.push(DecodeWarning::PotentialPhysicalKeyMerge {
                onset_us,
                key_index,
                occurrences,
            });
        }
    }
    (merged, warnings)
}

struct DiagnosticCounts {
    source_note_count: usize,
    source_track_count: usize,
    source_column_count: usize,
    empty_layer_mask_count: usize,
    potential_physical_key_merge_count: usize,
}

fn diagnostics(
    counts: DiagnosticCounts,
    events: &[CanonicalEvent],
    warnings: Vec<DecodeWarning>,
) -> DecodeDiagnostics {
    DecodeDiagnostics {
        source_note_count: counts.source_note_count,
        decoded_event_count: events.len(),
        source_track_count: counts.source_track_count,
        source_column_count: counts.source_column_count,
        empty_layer_mask_count: counts.empty_layer_mask_count,
        authored_sustain_count: events.iter().filter(|event| event.duration_us > 0).count(),
        potential_physical_key_merge_count: counts.potential_physical_key_merge_count,
        warnings,
    }
}

fn check_limit(resource: &str, actual: usize, maximum: usize) -> Result<(), SongDecodeError> {
    if actual > maximum {
        Err(resource_limit(resource, actual, maximum))
    } else {
        Ok(())
    }
}

fn resource_limit(resource: &str, actual: usize, maximum: usize) -> SongDecodeError {
    SongDecodeError::ResourceLimit {
        resource: resource.into(),
        actual,
        maximum,
    }
}

fn unsupported_instrument(game: SourceGame, instrument: &str) -> SongDecodeError {
    SongDecodeError::UnsupportedInstrument {
        game: game.as_storage_id().into(),
        instrument: instrument.into(),
    }
}

fn invalid(path: impl Into<String>, reason: impl Into<String>) -> SongDecodeError {
    SongDecodeError::InvalidField {
        path: path.into(),
        reason: reason.into(),
    }
}
