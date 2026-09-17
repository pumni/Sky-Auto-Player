use super::super::outcome::{
    PacketRetryReason, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use super::super::packet::{PreparedPacketView, PreparedPhysicalPacket};
use super::TrackedKeyState;
use crate::clock::QpcTicks;

impl TrackedKeyState {
    /// Send a trusted borrowed prepared packet without strict latest-start
    /// policy. Normal prepared playback uses this entry point; strict/dynamic
    /// callers continue through the cutoff-bearing entry point.
    pub fn send_prepared_physical_packet_view_without_cutoff(
        &mut self,
        prepared: PreparedPacketView<'_>,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        #[cfg(any(test, feature = "test-support"))]
        if let Some(emitter) = self.custom_packet_emitter.as_ref() {
            let started_ticks = if let Some(clock) = self.qpc_clock {
                match clock.now() {
                    Ok(ticks) => Some(ticks),
                    Err(error) => {
                        self.timing_error = Some(error);
                        return self.apply_packet_outcome(
                            packet,
                            super::packet_send::clock_failure_before_send_outcome(packet, error),
                        );
                    }
                }
            } else {
                None
            };
            let mut outcome = emitter(packet);
            if let Some(started_ticks) = started_ticks {
                outcome.evidence.started_ticks = Some(started_ticks);
                if outcome
                    .evidence
                    .completed_ticks
                    .is_some_and(|completed| completed < started_ticks)
                {
                    outcome.evidence.completed_ticks = Some(started_ticks);
                }
            }
            return self.apply_packet_outcome(packet, outcome);
        }

        let Some(clock) = self.qpc_clock else {
            self.last_error = Some("packet sender has no QPC clock".to_string());
            return self.apply_packet_outcome(
                packet,
                SendTransactionOutcome {
                    status: SendTransactionStatus::ZeroProgress,
                    evidence: SendEvidence {
                        requested_mask: packet.up_mask | packet.down_mask,
                        confirmed_mask: 0,
                        skipped_mask: 0,
                        first_inserted: 0,
                        attempts: 0,
                        zero_progress_retries: 0,
                        retry_reason: PacketRetryReason::None,
                        first_win32_error: None,
                        last_win32_error: None,
                        started_ticks: None,
                        completed_ticks: None,
                        timing_error: None,
                    },
                },
            );
        };

        let outcome =
            super::super::packet::send_prepared_physical_packet_view_once(prepared, clock);
        self.apply_packet_outcome(packet, outcome)
    }

    /// Final normal prepared boundary with no strict latest-start policy.
    /// Test support may still provide a deterministic pre-call timestamp.
    pub fn send_prepared_physical_packet_at_final_boundary_without_cutoff(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        test_started_ticks: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(test_started_ticks) = test_started_ticks {
            return self.send_prepared_physical_packet_with_start(prepared, test_started_ticks);
        }
        #[cfg(not(any(test, feature = "test-support")))]
        let _ = test_started_ticks;
        self.send_prepared_physical_packet(prepared)
    }
}
