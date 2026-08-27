use super::outcome::{
    PacketRetryReason, PlatformSendResult, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus, classify_send_status,
};
use super::physical::mask_for_scan_codes;
use super::raw::{no_syscall_boundary_with_clock, send_input_raw};

#[cfg(test)]
pub(crate) fn emit_up_with_immediate<F>(
    scan_codes: &[u16],
    mut send_fn: F,
) -> SendTransactionOutcome
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, timing_error) = no_syscall_boundary_with_clock(None);
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
    let first = send_fn(scan_codes, true);
    let first_inserted = (first.inserted as usize).min(n);
    let first_win32_error = (first.win32_error != 0).then_some(first.win32_error);
    let status1 = classify_send_status(
        first_inserted,
        n,
        first.win32_error,
        Some(first.started_ticks),
        first.completed_ticks,
    );

    if first_inserted >= n {
        return SendTransactionOutcome {
            status: status1,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: if matches!(status1, SendTransactionStatus::Complete) {
                    requested_mask
                } else {
                    0
                },
                skipped_mask: 0,
                first_inserted: first_inserted as u8,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error,
                last_win32_error: first_win32_error,
                started_ticks: Some(first.started_ticks),
                completed_ticks: first.completed_ticks,
                timing_error: first.timing_error,
            },
        };
    }

    // Note-off (Up) is idempotent: if first_inserted < n (whether zero or partial),
    // retry the full requested set.
    let second = send_fn(scan_codes, true);
    let second_inserted = (second.inserted as usize).min(n);
    let second_win32_error = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32_error = second_win32_error.or(first_win32_error);
    let status2 = classify_send_status(
        second_inserted,
        n,
        second.win32_error,
        Some(first.started_ticks),
        second.completed_ticks,
    );

    let status = if matches!(status2, SendTransactionStatus::Complete) {
        SendTransactionStatus::Complete
    } else if first_inserted == 0 && second_inserted == 0 {
        SendTransactionStatus::ZeroProgress
    } else {
        SendTransactionStatus::PartialProgress
    };

    SendTransactionOutcome {
        status,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: if matches!(status, SendTransactionStatus::Complete) {
                requested_mask
            } else {
                0
            },
            skipped_mask: 0,
            first_inserted: first_inserted as u8,
            attempts: 2,
            zero_progress_retries: u8::from(first_inserted == 0),
            retry_reason: if first_inserted == 0 {
                PacketRetryReason::ZeroProgress
            } else {
                PacketRetryReason::PartialProgress {
                    inserted_count: first_inserted as u8,
                }
            },
            first_win32_error: first_win32_error.or(second_win32_error),
            last_win32_error,
            started_ticks: Some(first.started_ticks),
            completed_ticks: second.completed_ticks,
            timing_error: first.timing_error.or(second.timing_error),
        },
    }
}

pub fn emit_up_with<F>(scan_codes: &[u16], send_fn: F) -> SendTransactionOutcome
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    emit_up_once_with(scan_codes, send_fn)
}

pub fn emit_up(scan_codes: &[u16]) -> SendTransactionOutcome {
    emit_up_once_with(scan_codes, send_input_raw)
}

/// Single-send note-off used by the cleanup FSM, which owns retry itself.
///
/// The cleanup state machine retries across attempts and must never call an
/// API with an internal retry (that would multiply the raw `SendInput` calls
/// by the FSM's attempt budget). This primitive performs exactly one send.
pub fn emit_up_once_with<F>(scan_codes: &[u16], mut send_fn: F) -> SendTransactionOutcome
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, timing_error) = no_syscall_boundary_with_clock(None);
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
    let result = send_fn(scan_codes, true);
    let inserted = (result.inserted as usize).min(n);
    let status = classify_send_status(
        inserted,
        n,
        result.win32_error,
        Some(result.started_ticks),
        result.completed_ticks,
    );
    let win32_error = (result.win32_error != 0).then_some(result.win32_error);
    SendTransactionOutcome {
        status,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: if matches!(status, SendTransactionStatus::Complete) {
                requested_mask
            } else {
                0
            },
            skipped_mask: 0,
            first_inserted: inserted as u8,
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason: if inserted == 0 {
                PacketRetryReason::ZeroProgress
            } else if inserted < n {
                PacketRetryReason::PartialProgress {
                    inserted_count: inserted as u8,
                }
            } else {
                PacketRetryReason::None
            },
            first_win32_error: win32_error,
            last_win32_error: win32_error,
            started_ticks: Some(result.started_ticks),
            completed_ticks: result.completed_ticks,
            timing_error: result.timing_error,
        },
    }
}
