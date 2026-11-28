use super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
    TerminalUpTransportResult,
};
#[cfg(any(test, feature = "test-support"))]
use super::outcome::{PlatformSendResult, classify_send_status};
use super::packet::invalid_packet_outcome;
#[cfg(any(test, feature = "test-support"))]
use super::physical::mask_for_scan_codes;
#[cfg(any(test, feature = "test-support"))]
use super::raw::no_syscall_boundary_with_clock;

/// Single-send note-off used by the cleanup FSM, which owns retry itself.
///
/// The cleanup state machine retries across attempts and must never call an
/// API with an internal retry (that would multiply the raw `SendInput` calls
/// by the FSM's attempt budget). This primitive performs exactly one send.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn emit_up_once_with<F>(scan_codes: &[u16], mut send_fn: F) -> SendTransactionOutcome
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
    let inserted = result.inserted as usize;
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

/// Convert one terminal Up transport result into the shared cleanup evidence
/// shape. Full insertion without a Win32 error confirms transport acceptance
/// even if a timestamp is unavailable; the independent physical probe still
/// owns obligation resolution.
pub(crate) fn terminal_up_outcome(
    packet: PhysicalPacket,
    result: TerminalUpTransportResult,
) -> SendTransactionOutcome {
    if !packet.is_up_only() || packet.up_mask == 0 {
        return invalid_packet_outcome(packet);
    }

    let expected = usize::from(packet.event_count());
    let requested = usize::from(result.requested);
    if requested != expected {
        return invalid_packet_outcome(packet);
    }
    let inserted = usize::from(result.inserted);
    // Terminal Up is an independent safety send. Its status describes the
    // actual transport result; optional QPC samples remain separate evidence
    // and cannot relabel an attempted zero-insert syscall as pre-send failure.
    let status = if inserted != requested {
        if inserted == 0 {
            SendTransactionStatus::ZeroProgress
        } else {
            SendTransactionStatus::IntegrityLost
        }
    } else if result.win32_error != 0 {
        SendTransactionStatus::IntegrityLost
    } else {
        SendTransactionStatus::Complete
    };
    let win32_error = (result.win32_error != 0).then_some(result.win32_error);
    let retry_reason = if inserted == 0 {
        PacketRetryReason::ZeroProgress
    } else if inserted < requested {
        PacketRetryReason::PartialProgress {
            inserted_count: inserted as u8,
        }
    } else {
        PacketRetryReason::None
    };

    SendTransactionOutcome {
        status,
        evidence: SendEvidence {
            requested_mask: packet.up_mask,
            confirmed_mask: if inserted == requested && result.win32_error == 0 {
                packet.up_mask
            } else {
                0
            },
            skipped_mask: 0,
            first_inserted: inserted as u8,
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason,
            first_win32_error: win32_error,
            last_win32_error: win32_error,
            started_ticks: result.started_ticks,
            completed_ticks: result.completed_ticks,
            timing_error: result.timing_error,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::QpcTicks;

    #[test]
    fn terminal_up_count_mismatch_is_not_full_transport_evidence() {
        let packet = PhysicalPacket::new(0b001, 0);
        let outcome = terminal_up_outcome(
            packet,
            TerminalUpTransportResult {
                requested: 1,
                inserted: 2,
                started_ticks: Some(QpcTicks::from_raw(10)),
                completed_ticks: Some(QpcTicks::from_raw(11)),
                win32_error: 0,
                timing_error: None,
            },
        );

        assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
        assert_eq!(outcome.evidence.first_inserted, 2);
        assert_eq!(outcome.evidence.confirmed_mask, 0);
        assert!(!outcome.has_complete_raw_transport_for(packet));
    }
}
