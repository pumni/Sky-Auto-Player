use super::{DownChordHeaderTicks, DownIntentTicks};

pub const MAX_EXECUTOR_BYTES: usize = 512 * 1024 * 1024;

pub(crate) fn checked_executor_storage_bytes(
    chord_capacity: usize,
    intent_capacity: usize,
) -> Result<usize, String> {
    let header_bytes = chord_capacity
        .checked_mul(std::mem::size_of::<DownChordHeaderTicks>())
        .ok_or_else(|| "executor chord-header memory size overflow".to_string())?;
    let intent_bytes = intent_capacity
        .checked_mul(std::mem::size_of::<DownIntentTicks>())
        .ok_or_else(|| "executor intent-arena memory size overflow".to_string())?;
    let estimated_bytes = header_bytes
        .checked_add(intent_bytes)
        .ok_or_else(|| "executor vector memory size overflow".to_string())?;
    if estimated_bytes > MAX_EXECUTOR_BYTES {
        return Err(format!(
            "executor memory requirement ({estimated_bytes} bytes) exceeds budget of {MAX_EXECUTOR_BYTES} bytes"
        ));
    }
    Ok(estimated_bytes)
}

pub(super) fn reserve_executor_chords(
    down_chords: &mut Vec<DownChordHeaderTicks>,
    chord_count: usize,
) -> Result<(), String> {
    down_chords
        .try_reserve_exact(chord_count)
        .map_err(|error| format!("executor chord capacity allocation failed: {error}"))
}

pub(super) fn reserve_executor_intents(
    down_intents: &mut Vec<DownIntentTicks>,
    intent_count: usize,
) -> Result<(), String> {
    down_intents
        .try_reserve_exact(intent_count)
        .map_err(|error| format!("executor intent-arena capacity allocation failed: {error}"))
}
