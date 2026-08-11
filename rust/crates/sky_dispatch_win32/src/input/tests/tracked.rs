use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use super::super::tracked::TrackedKeyState;

#[test]
fn same_key_retrigger_packet_contains_two_physical_events() {
    let mut state = TrackedKeyState::with_packet_emitter(|packet| {
        assert_eq!(packet.up_mask, 0b001);
        assert_eq!(packet.down_mask, 0b001);
        assert_eq!(packet.event_count(), 2);
        SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: 0b001,
                confirmed_mask: 0b001,
                skipped_mask: 0,
                first_inserted: 2,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: Some(crate::clock::QpcTicks::from_raw(10)),
                completed_ticks: Some(crate::clock::QpcTicks::from_raw(20)),
                timing_error: None,
            },
        }
    });
    let outcome = state.key_down_physical_packet(PhysicalPacket::new(0b001, 0b001));
    assert_eq!(outcome.status, SendTransactionStatus::Complete);
}
