use super::super::super::clock::QpcTicks;
use super::super::outcome::{SendEvidence, SendTransactionOutcome, SendTransactionStatus};
use super::super::packet::PreparedPhysicalPacket;
use super::super::packet::send_prepared_physical_packet_once_at_target_with_cutoff;
use super::TrackedKeyState;
use super::packet_send::deadline_missed_before_send_outcome;

impl TrackedKeyState {
    /// Test-only target-crossing seam. Production dispatch uses the packet
    /// primitive directly; tests may provide a controlled crossing sample or
    /// a deterministic custom emitter while retaining the sender cutoff.
    pub fn send_prepared_physical_packet_at_target_with_cutoff(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        qpc_clock: crate::clock::QpcClock,
        physical_target_qpc: QpcTicks,
        latest_allowed_down_qpc: Option<QpcTicks>,
        test_started_ticks: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        let started_ticks = if let Some(started_ticks) = test_started_ticks {
            Some(started_ticks)
        } else if self.custom_packet_emitter.is_some() {
            Some(loop {
                let ticks = match qpc_clock.now() {
                    Ok(ticks) => ticks,
                    Err(error) => {
                        self.timing_error = Some(error);
                        return self.apply_packet_outcome(
                            packet,
                            SendTransactionOutcome {
                                status: SendTransactionStatus::ClockFailureBeforeSend,
                                evidence: SendEvidence {
                                    requested_mask: packet.up_mask | packet.down_mask,
                                    confirmed_mask: 0,
                                    skipped_mask: 0,
                                    first_inserted: 0,
                                    attempts: 0,
                                    zero_progress_retries: 0,
                                    retry_reason: super::super::outcome::PacketRetryReason::None,
                                    first_win32_error: None,
                                    last_win32_error: None,
                                    started_ticks: None,
                                    completed_ticks: None,
                                    timing_error: Some(error),
                                },
                            },
                        );
                    }
                };
                if ticks >= physical_target_qpc {
                    break ticks;
                }
                std::hint::spin_loop();
            })
        } else {
            None
        };
        if let Some(started_ticks) = started_ticks {
            if super::super::packet::down_cutoff_missed(
                packet.down_mask,
                started_ticks,
                latest_allowed_down_qpc,
            ) {
                return self.apply_packet_outcome(
                    packet,
                    deadline_missed_before_send_outcome(packet, started_ticks),
                );
            }
            if let Some(emitter) = self.custom_packet_emitter.as_ref() {
                let mut outcome = emitter(packet);
                outcome.evidence.started_ticks = Some(started_ticks);
                if outcome
                    .evidence
                    .completed_ticks
                    .is_some_and(|completed| completed < started_ticks)
                {
                    outcome.evidence.completed_ticks = Some(started_ticks);
                }
                return self.apply_packet_outcome(packet, outcome);
            }
            if test_started_ticks.is_some() {
                let outcome =
                    super::super::packet::send_prepared_physical_packet_once_with_start_and_cutoff(
                        prepared,
                        qpc_clock,
                        started_ticks,
                        latest_allowed_down_qpc,
                    );
                return self.apply_packet_outcome(packet, outcome);
            }
        }
        let outcome = send_prepared_physical_packet_once_at_target_with_cutoff(
            prepared,
            qpc_clock,
            physical_target_qpc,
            latest_allowed_down_qpc,
        );
        self.apply_packet_outcome(packet, outcome)
    }

    /// Phase-A test seam with a controlled caller-owned crossing sample.
    pub fn send_phase_a_benchmark_boundary(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        qpc_clock: crate::clock::QpcClock,
        physical_target_qpc: QpcTicks,
        latest_allowed_down_qpc: Option<QpcTicks>,
        started_ticks: QpcTicks,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_at_target_with_cutoff(
            prepared,
            qpc_clock,
            physical_target_qpc,
            latest_allowed_down_qpc,
            Some(started_ticks),
        )
    }
}
