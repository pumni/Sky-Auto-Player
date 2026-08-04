use super::outcome::{EmitResult, PlatformSendResult};
use super::raw::{no_syscall_boundary_with_clock, send_input_raw};
use smallvec::SmallVec;

pub(crate) fn emit_up_with_immediate<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, completed_us, timing_error) =
            no_syscall_boundary_with_clock(None);
        return EmitResult {
            sent: SmallVec::new(),
            completed_us,
            started_ticks,
            completed_ticks,
            success: timing_error.is_none(),
            keys_dropped: 0,
            first_win32_error: None,
            last_win32_error: None,
            send_attempts: 0,
            zero_progress_retries: 0,
            first_inserted: 0,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error,
        };
    }
    let n = scan_codes.len();
    let first = send_fn(scan_codes, true);
    let first_inserted = (first.inserted as usize).min(n);
    let first_win32_error = (first.win32_error != 0).then_some(first.win32_error);
    if first_inserted >= n {
        return EmitResult {
            sent: scan_codes.iter().copied().collect(),
            completed_us: first.completed_us,
            started_ticks: Some(first.started_ticks),
            completed_ticks: first.completed_ticks,
            success: true,
            keys_dropped: 0,
            first_win32_error,
            last_win32_error: first_win32_error,
            send_attempts: 1,
            zero_progress_retries: 0,
            first_inserted: first_inserted as u8,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error: first.timing_error,
        };
    }

    // A partial Up is also uncertain: retry the entire requested set instead
    // of assuming SendInput's inserted count identifies a prefix.
    let second = send_fn(scan_codes, true);
    let second_inserted = (second.inserted as usize).min(n);
    let success = second_inserted >= n;
    let second_win32_error = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32_error = second_win32_error.or(first_win32_error);
    EmitResult {
        sent: if success {
            scan_codes.iter().copied().collect()
        } else {
            SmallVec::new()
        },
        completed_us: second.completed_us,
        started_ticks: Some(first.started_ticks),
        completed_ticks: second.completed_ticks,
        success,
        keys_dropped: u64::from(!success),
        first_win32_error: first_win32_error.or(second_win32_error),
        last_win32_error,
        send_attempts: 2,
        zero_progress_retries: u8::from(first_inserted == 0),
        first_inserted: first_inserted as u8,
        partial_progress: (first_inserted > 0 || second_inserted > 0) && !success,
        retried_after_zero_progress: first_inserted == 0,
        chord_integrity_lost: false,
        keys_inserted_before_failure: if success {
            0
        } else {
            first_inserted.max(second_inserted) as u8
        },
        keys_rolled_back: 0,
        rollback_residue_keys: 0,
        timing_error: first.timing_error.or(second.timing_error),
    }
}

pub fn emit_up_with<F>(scan_codes: &[u16], send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    emit_up_with_immediate(scan_codes, send_fn)
}

pub fn emit_up(scan_codes: &[u16]) -> EmitResult {
    emit_up_with(scan_codes, send_input_raw)
}
