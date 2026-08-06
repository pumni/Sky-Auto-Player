use super::outcome::{
    PacketRetryReason, PlatformSendResult, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus,
};
use super::physical::mask_for_scan_codes;
use super::raw::{no_syscall_boundary_with_clock, send_input_raw};

pub fn emit_down_with<F>(scan_codes: &[u16], mut send_fn: F) -> SendTransactionOutcome
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, _completed_us, timing_error) =
            no_syscall_boundary_with_clock(None);
        return SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: 0,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 0,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(started_ticks),
                completed_ticks,
                timing_error,
            },
        };
    }

    let requested_mask = mask_for_scan_codes(scan_codes).unwrap_or(0);
    let n = scan_codes.len();
    let res1 = send_fn(scan_codes, false);
    let landed1 = (res1.inserted as usize).min(n);
    let first_win32_error = (res1.win32_error != 0).then_some(res1.win32_error);

    if landed1 >= n {
        return SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: requested_mask,
                skipped_mask: 0,
                first_inserted: landed1 as u8,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error,
                last_win32_error: first_win32_error,
                started_ticks: Some(res1.started_ticks),
                completed_ticks: res1.completed_ticks,
                timing_error: res1.timing_error,
            },
        };
    }

    // A non-zero partial insertion has already destroyed chord integrity. Do
    // not infer which keys landed and do not send a remainder as Down.
    // Roll back the entire requested chord immediately; any residue is tracked
    // as uncertain and the worker's terminal cleanup handles it fail-closed.
    if landed1 > 0 {
        let rollback = send_fn(scan_codes, true);
        let rollback_error = (rollback.win32_error != 0).then_some(rollback.win32_error);
        return SendTransactionOutcome {
            status: SendTransactionStatus::IntegrityLost,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: landed1 as u8,
                attempts: 2,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::PartialProgress {
                    inserted_count: landed1 as u8,
                },
                first_win32_error,
                last_win32_error: rollback_error.or(first_win32_error),
                started_ticks: Some(res1.started_ticks),
                completed_ticks: rollback.completed_ticks,
                timing_error: res1.timing_error.or(rollback.timing_error),
            },
        };
    }

    // Zero progress is the only case where an immediate retry is safe: the
    // first call inserted no packet, so the chord has not been split yet.
    let retry = send_fn(scan_codes, false);
    let retry_inserted = (retry.inserted as usize).min(n);
    let retry_error = (retry.win32_error != 0).then_some(retry.win32_error);
    if retry_inserted >= n {
        return SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: requested_mask,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 2,
                zero_progress_retries: 1,
                retry_reason: PacketRetryReason::ZeroProgress,
                first_win32_error,
                last_win32_error: retry_error.or(first_win32_error),
                started_ticks: Some(res1.started_ticks),
                completed_ticks: retry.completed_ticks,
                timing_error: retry.timing_error.or(res1.timing_error),
            },
        };
    }

    let mut completed_ticks = retry.completed_ticks;
    let mut attempts = 2;
    let mut last_win32_error = retry_error.or(first_win32_error);
    let mut rollback_timing_error = None;
    let mut status = SendTransactionStatus::ZeroProgress;

    if retry_inserted > 0 {
        let rollback = send_fn(scan_codes, true);
        completed_ticks = rollback.completed_ticks;
        attempts = 3;
        last_win32_error = (rollback.win32_error != 0)
            .then_some(rollback.win32_error)
            .or(last_win32_error);
        rollback_timing_error = rollback.timing_error;
        status = SendTransactionStatus::IntegrityLost;
    }

    let timing_error = retry
        .timing_error
        .or(res1.timing_error)
        .or(rollback_timing_error);

    SendTransactionOutcome {
        status,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: 0,
            skipped_mask: 0,
            first_inserted: 0,
            attempts,
            zero_progress_retries: 1,
            retry_reason: PacketRetryReason::ZeroProgress,
            first_win32_error,
            last_win32_error,
            started_ticks: Some(res1.started_ticks),
            completed_ticks,
            timing_error,
        },
    }
}

pub fn emit_down(scan_codes: &[u16]) -> SendTransactionOutcome {
    emit_down_with(scan_codes, send_input_raw)
}
