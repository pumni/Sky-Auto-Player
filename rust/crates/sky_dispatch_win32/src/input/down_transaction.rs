use super::outcome::{EmitResult, PlatformSendResult};
use super::raw::{no_syscall_boundary_with_clock, send_input_raw};
use smallvec::SmallVec;

pub fn emit_down_with<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
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
    let res1 = send_fn(scan_codes, false);
    let landed1 = (res1.inserted as usize).min(n);
    let first_win32_error = (res1.win32_error != 0).then_some(res1.win32_error);

    if landed1 >= n {
        let sent: SmallVec<[u16; 15]> = scan_codes.iter().copied().collect();
        return EmitResult {
            sent,
            completed_us: res1.completed_us,
            started_ticks: Some(res1.started_ticks),
            completed_ticks: res1.completed_ticks,
            success: true,
            keys_dropped: 0,
            first_win32_error,
            last_win32_error: first_win32_error,
            send_attempts: 1,
            zero_progress_retries: 0,
            first_inserted: landed1 as u8,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error: res1.timing_error,
        };
    }

    // A non-zero partial insertion has already destroyed chord integrity. Do
    // not infer which keys landed and do not send a remainder as Down.
    // Roll back the entire requested chord immediately; any residue is tracked
    // as uncertain and the worker's terminal cleanup handles it fail-closed.
    if landed1 > 0 {
        let rollback = send_fn(scan_codes, true);
        let rollback_inserted = (rollback.inserted as usize).min(n);
        let rollback_error = (rollback.win32_error != 0).then_some(rollback.win32_error);
        return EmitResult {
            sent: SmallVec::new(),
            completed_us: rollback.completed_us,
            started_ticks: Some(res1.started_ticks),
            completed_ticks: rollback.completed_ticks,
            success: false,
            keys_dropped: (n - landed1) as u64,
            first_win32_error,
            last_win32_error: rollback_error.or(first_win32_error),
            send_attempts: 2,
            zero_progress_retries: 0,
            first_inserted: landed1 as u8,
            partial_progress: true,
            retried_after_zero_progress: false,
            chord_integrity_lost: true,
            keys_inserted_before_failure: landed1 as u8,
            keys_rolled_back: rollback_inserted as u8,
            rollback_residue_keys: n.saturating_sub(rollback_inserted) as u8,
            timing_error: res1.timing_error.or(rollback.timing_error),
        };
    }

    // Zero progress is the only case where an immediate retry is safe: the
    // first call inserted no packet, so the chord has not been split yet.
    let retry = send_fn(scan_codes, false);
    let retry_inserted = (retry.inserted as usize).min(n);
    let retry_error = (retry.win32_error != 0).then_some(retry.win32_error);
    if retry_inserted >= n {
        return EmitResult {
            sent: scan_codes.iter().copied().collect(),
            completed_us: retry.completed_us,
            started_ticks: Some(res1.started_ticks),
            completed_ticks: retry.completed_ticks,
            success: true,
            keys_dropped: 0,
            first_win32_error,
            last_win32_error: retry_error.or(first_win32_error),
            send_attempts: 2,
            zero_progress_retries: 1,
            first_inserted: 0,
            partial_progress: false,
            retried_after_zero_progress: true,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error: retry.timing_error.or(res1.timing_error),
        };
    }

    let mut completed_us = retry.completed_us;
    let started_ticks = Some(res1.started_ticks);
    let mut completed_ticks = retry.completed_ticks;
    let mut send_attempts = 2;
    let mut last_win32_error = retry_error.or(first_win32_error);
    let mut rollback_timing_error = None;
    let mut keys_rolled_back = 0u8;
    let mut rollback_residue_keys = 0u8;
    if retry_inserted > 0 {
        let rollback = send_fn(scan_codes, true);
        let rollback_inserted = (rollback.inserted as usize).min(n);
        completed_us = rollback.completed_us;
        completed_ticks = rollback.completed_ticks;
        send_attempts = 3;
        last_win32_error = (rollback.win32_error != 0)
            .then_some(rollback.win32_error)
            .or(last_win32_error);
        rollback_timing_error = rollback.timing_error;
        keys_rolled_back = rollback_inserted as u8;
        rollback_residue_keys = n.saturating_sub(rollback_inserted) as u8;
    }
    let timing_error = retry
        .timing_error
        .or(res1.timing_error)
        .or(rollback_timing_error);
    EmitResult {
        sent: SmallVec::new(),
        completed_us,
        started_ticks,
        completed_ticks,
        success: false,
        keys_dropped: (n - retry_inserted) as u64,
        first_win32_error,
        last_win32_error,
        send_attempts,
        zero_progress_retries: 1,
        first_inserted: 0,
        partial_progress: retry_inserted > 0,
        retried_after_zero_progress: true,
        chord_integrity_lost: retry_inserted > 0,
        keys_inserted_before_failure: retry_inserted as u8,
        keys_rolled_back,
        rollback_residue_keys,
        timing_error,
    }
}

pub fn emit_down(scan_codes: &[u16]) -> EmitResult {
    emit_down_with(scan_codes, send_input_raw)
}
