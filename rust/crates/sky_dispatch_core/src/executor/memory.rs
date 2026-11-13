use super::DownChordTicks;

pub const MAX_EXECUTOR_BYTES: usize = 512 * 1024 * 1024;

pub(super) fn checked_executor_storage_bytes(chord_count: usize) -> Result<usize, String> {
    let estimated_bytes = chord_count
        .checked_mul(std::mem::size_of::<DownChordTicks>())
        .ok_or_else(|| "executor memory size overflow".to_string())?;
    if estimated_bytes > MAX_EXECUTOR_BYTES {
        return Err(format!(
            "executor memory requirement ({estimated_bytes} bytes) exceeds budget of {MAX_EXECUTOR_BYTES} bytes"
        ));
    }
    Ok(estimated_bytes)
}

pub(super) fn reserve_executor_chords(
    down_chords: &mut Vec<DownChordTicks>,
    chord_count: usize,
) -> Result<(), String> {
    down_chords
        .try_reserve_exact(chord_count)
        .map_err(|error| format!("executor chord capacity allocation failed: {error}"))
}
