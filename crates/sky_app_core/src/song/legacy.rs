use super::{KEY_NAMES, Note, SKY_SCAN_CODES, Song, SongError, truncated_f64_to_i64};
use serde_json::Value;

pub fn parse_song_json(bytes: &[u8], fallback_name: &str) -> Result<Song, SongError> {
    let value: Value =
        serde_json::from_slice(bytes).map_err(|error| SongError::InvalidJson(error.to_string()))?;
    let object = match value {
        Value::Array(values) => values.into_iter().next().ok_or(SongError::EmptySongList)?,
        other => other,
    };
    let object = object.as_object().ok_or(SongError::InvalidRoot)?;
    let notes_value = object.get("songNotes").ok_or(SongError::MissingNotes)?;
    let notes = notes_value.as_array().ok_or(SongError::InvalidNotes)?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(fallback_name)
        .to_owned();
    let mut parsed = Vec::with_capacity(notes.len());
    for (index, raw) in notes.iter().enumerate() {
        let note = raw.as_object().ok_or(SongError::InvalidNote { index })?;
        let raw_time = note.get("time").ok_or(SongError::MissingTime { index })?;
        let time_ms =
            parse_legacy_timestamp_ms(raw_time).ok_or(SongError::InvalidTime { index })?;
        if time_ms < 0 {
            return Err(SongError::NegativeTime { index });
        }
        let raw_key = note.get("key").ok_or(SongError::MissingKey { index })?;
        let key = raw_key
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| SongError::UnmappedKey {
                index,
                key: format_legacy_song_value(raw_key),
            })?;
        if scan_code_for_key(&key).is_none() {
            return Err(SongError::UnmappedKey { index, key });
        }
        parsed.push(Note {
            time_ms,
            key,
            source_index: index,
        });
    }
    parsed.sort_by_key(|note| note.time_ms);
    Ok(Song {
        name,
        notes: parsed,
    })
}

fn parse_legacy_timestamp_ms(value: &Value) -> Option<i64> {
    match value {
        Value::Bool(value) => Some(i64::from(*value)),
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .or_else(|| number.as_f64().and_then(truncated_f64_to_i64)),
        Value::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn format_legacy_song_value(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(value) => value.to_string().to_ascii_lowercase(),
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Array(values) => format!(
            "[{}]",
            values
                .iter()
                .map(format_legacy_song_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(_) => "{...}".into(),
    }
}

pub fn scan_code_for_key(key: &str) -> Option<u16> {
    let key = key
        .strip_prefix("1Key")
        .or_else(|| key.strip_prefix("2Key"))
        .or_else(|| key.strip_prefix("3Key"))
        .map(|suffix| format!("Key{suffix}"))
        .unwrap_or_else(|| key.to_owned());
    KEY_NAMES
        .iter()
        .position(|candidate| *candidate == key)
        .map(|index| SKY_SCAN_CODES[index])
}
