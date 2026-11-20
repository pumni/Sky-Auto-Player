use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
#[cfg(test)]
use super::super::packet::send_prepared_physical_packet_once_with_start;
#[cfg(test)]
use super::super::packet::send_prepared_physical_packet_view_once_with_start;
use super::super::packet::{
    PreparedPacketView, PreparedPhysicalPacket, invalid_packet_outcome,
    send_prepared_physical_packet_once, send_prepared_terminal_up_once,
};
#[cfg(any(test, feature = "test-support"))]
use super::super::up_transaction::emit_up_once_with;
#[cfg(test)]
use super::super::up_transaction::terminal_up_outcome;
use super::{CleanupAttemptAdmission, TrackedKeyState};
#[cfg(test)]
use crate::clock::QpcTicks;

pub(super) struct CleanupSendContext<'a, 'b> {
    pub(super) terminal_qpc_fallback: &'a mut bool,
    pub(super) allow_background_terminal_up: bool,
    pub(super) admission: Option<&'a mut CleanupAttemptAdmission<'b>>,
    pub(super) drain_control:
        &'a mut dyn FnMut() -> Result<super::super::outcome::CleanupDrainDecision, String>,
    pub(super) background_terminal_drain: &'a mut bool,
    pub(super) raw_up_invocations: u8,
    pub(super) attempts: &'a mut u8,
}

fn packet_outcome_proves_zero_transport(
    packet: PhysicalPacket,
    outcome: SendTransactionOutcome,
) -> bool {
    let evidence = outcome.evidence;
    let packet_mask = packet.up_mask | packet.down_mask;
    if packet_mask == 0
        || evidence.requested_mask != packet_mask
        || evidence.confirmed_mask != 0
        || evidence.skipped_mask != 0
        || evidence.first_inserted != 0
    {
        return false;
    }

    match outcome.status {
        SendTransactionStatus::ClockFailureBeforeSend => {
            evidence.attempts == 0
                && evidence.timing_error.is_some()
                && evidence.retry_reason == PacketRetryReason::None
        }
        SendTransactionStatus::ZeroProgress => {
            evidence.attempts == 0
                && matches!(
                    evidence.retry_reason,
                    PacketRetryReason::None | PacketRetryReason::ZeroProgress
                )
        }
        SendTransactionStatus::PreparationRejected => evidence.attempts == 0,
        _ => false,
    }
}

impl TrackedKeyState {
    #[cfg(any(test, feature = "test-support"))]
    fn apply_test_packet_ambiguity(
        &mut self,
        packet: PhysicalPacket,
        mut outcome: SendTransactionOutcome,
    ) -> SendTransactionOutcome {
        let packet_mask = packet.up_mask | packet.down_mask;
        if self.prepared_packet_ambiguity_mask == Some(packet_mask)
            && outcome.status == SendTransactionStatus::Complete
        {
            self.prepared_packet_ambiguity_mask = None;
            outcome.status = SendTransactionStatus::PartialProgress;
            outcome.evidence.confirmed_mask = 0;
            outcome.evidence.first_inserted = packet.event_count().saturating_sub(1);
        }
        outcome
    }

    #[cfg(test)]
    pub(crate) fn send_prepared_physical_packet_view(
        &mut self,
        prepared: PreparedPacketView<'_>,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_view_with_commit_policy(prepared, false)
    }

    fn send_prepared_physical_packet_view_with_commit_policy(
        &mut self,
        prepared: PreparedPacketView<'_>,
        defer_journal_commit: bool,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        #[cfg(any(test, feature = "test-support"))]
        if let Some(emitter) = self.custom_packet_emitter.as_ref() {
            #[cfg(any(test, feature = "test-support"))]
            self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::BeforeSenderAuthority);
            let previous_in_flight_mask = self.in_flight_mask;
            self.in_flight_mask |= packet.up_mask | packet.down_mask;
            #[cfg(any(test, feature = "test-support"))]
            self.panic_if_prepared_send_at(
                super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
            );
            // A custom sender is a test-support boundary replacement. Its
            // timestamps are injected evidence and must pass through unchanged
            // so missing or contradictory QPC values remain observable.
            let outcome = emitter(packet);
            self.timing_error = outcome.evidence.timing_error;
            #[cfg(any(test, feature = "test-support"))]
            self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterSenderReturn);
            #[cfg(any(test, feature = "test-support"))]
            let outcome = self.apply_test_packet_ambiguity(packet, outcome);
            let outcome =
                self.apply_packet_outcome_with_commit_policy(packet, outcome, defer_journal_commit);
            if defer_journal_commit
                && packet_outcome_proves_zero_transport(packet, outcome)
                && (packet.down_mask == 0 || outcome.evidence.attempts == 0)
            {
                self.in_flight_mask = previous_in_flight_mask;
            }
            #[cfg(any(test, feature = "test-support"))]
            self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterStateCommit);
            return outcome;
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
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::BeforeSenderAuthority);
        let previous_in_flight_mask = self.in_flight_mask;
        self.in_flight_mask |= packet.up_mask | packet.down_mask;
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterJournalBeforeSyscall);
        let outcome =
            super::super::packet::send_prepared_physical_packet_view_once(prepared, clock);
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterSenderReturn);
        #[cfg(any(test, feature = "test-support"))]
        let outcome = self.apply_test_packet_ambiguity(packet, outcome);
        let outcome =
            self.apply_packet_outcome_with_commit_policy(packet, outcome, defer_journal_commit);
        if defer_journal_commit
            && packet_outcome_proves_zero_transport(packet, outcome)
            && (packet.down_mask == 0 || outcome.evidence.attempts == 0)
        {
            self.in_flight_mask = previous_in_flight_mask;
        }
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterStateCommit);
        outcome
    }

    /// Single-send note-off for operator-owned cleanup retries. The cleanup FSM
    /// bounds the raw `SendInput` count itself; this must never perform an
    /// internal retry.
    pub(super) fn do_emit_up_once(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        mut context: CleanupSendContext<'_, '_>,
    ) -> Result<SendTransactionOutcome, String> {
        let packet = prepared.packet();
        let logical_mask = packet.up_mask;
        if !packet.is_up_only() || logical_mask == 0 {
            return Ok(invalid_packet_outcome(packet));
        }

        if let Some(admission) = context.admission.as_deref_mut() {
            let release_owner = self
                .release_owner_proof_for_mask(logical_mask)?
                .ok_or_else(|| {
                    "tracked release owner proof is missing before safety Up".to_string()
                })?;
            admission(logical_mask, release_owner)?;
        }

        let drain_decision = (context.drain_control)()?;
        *context.terminal_qpc_fallback |= drain_decision.terminal_requested;
        let background_terminal = context.allow_background_terminal_up
            && drain_decision.terminal_requested
            && !drain_decision.origin_foreground;
        if !drain_decision.origin_foreground && !background_terminal {
            return Err("tracked cleanup origin is not foreground".into());
        }
        *context.background_terminal_drain |= background_terminal;
        if background_terminal && context.raw_up_invocations != 0 {
            return Err("background terminal Up budget already used".into());
        }

        // Count after admission and immediately before entering the sender. If
        // the sender unwinds, quarantine diagnostics retain the admitted try.
        self.release_attempts_total = self.release_attempts_total.saturating_add(1);
        *context.attempts = context.attempts.saturating_add(1);

        #[cfg(test)]
        if *context.terminal_qpc_fallback
            && let Some(emitter) = self.custom_terminal_up_emitter.as_ref()
        {
            return Ok(terminal_up_outcome(packet, emitter(packet)));
        }

        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            let scan_codes = self
                .instrument_key_profile
                .scan_codes_from_mask(logical_mask);
            let mut outcome = emit_up_once_with(&scan_codes, |sc, key_up| emitter(sc, key_up));
            outcome.evidence.requested_mask = logical_mask;
            outcome.evidence.skipped_mask = 0;
            outcome.evidence.confirmed_mask = if outcome.is_success() {
                logical_mask
            } else {
                logical_prefix_mask(logical_mask, outcome.evidence.first_inserted)
            };
            return Ok(outcome);
        }

        #[cfg(any(test, feature = "test-support"))]
        if let Some(emitter) = self.custom_packet_emitter.as_ref() {
            return Ok(emitter(packet));
        }

        Ok(if *context.terminal_qpc_fallback {
            send_prepared_terminal_up_once(prepared, self.qpc_clock)
        } else {
            match self.qpc_clock {
                Some(clock) => send_prepared_physical_packet_once(prepared, clock),
                None => invalid_packet_outcome(packet),
            }
        })
    }

    /// Send one validated physical packet. The packet builder is the sole
    /// authored/release transport: it emits all Up events before all Down
    /// events in one `SendInput` call.
    #[cfg(test)]
    pub fn send_physical_packet(&mut self, packet: PhysicalPacket) -> SendTransactionOutcome {
        let Some(clock) = self.qpc_clock else {
            return self.send_physical_packet_with_start(packet, QpcTicks::ZERO);
        };
        let started_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                self.timing_error = Some(error);
                return SendTransactionOutcome {
                    status: SendTransactionStatus::ClockFailureBeforeSend,
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
                        timing_error: Some(error),
                    },
                };
            }
        };
        self.send_physical_packet_with_start(packet, started_ticks)
    }

    /// Send one validated physical packet using the caller's authoritative
    /// QPC start boundary. The timestamp is reused by the transport; it is
    /// never resampled after the final admission gate.
    #[cfg(test)]
    fn send_physical_packet_with_start(
        &mut self,
        packet: PhysicalPacket,
        started_ticks: QpcTicks,
    ) -> SendTransactionOutcome {
        let prepared = match PreparedPhysicalPacket::try_new_with_profile(
            packet,
            &self.instrument_key_profile,
        ) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.last_error = Some(format!("physical packet preparation failed: {error}"));
                return self.apply_packet_outcome(packet, invalid_packet_outcome(packet));
            }
        };
        let outcome = {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(emitter) = self.custom_packet_emitter.as_ref() {
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::BeforeSenderAuthority,
                );
                self.in_flight_mask |= packet.up_mask | packet.down_mask;
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
                );
                let mut outcome = emitter(packet);
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterSenderReturn);
                outcome.evidence.started_ticks = Some(started_ticks);
                outcome
            } else {
                let Some(clock) = self.qpc_clock else {
                    self.last_error = Some("packet sender has no QPC clock".to_string());
                    return SendTransactionOutcome {
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
                    };
                };
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::BeforeSenderAuthority,
                );
                self.in_flight_mask |= packet.up_mask | packet.down_mask;
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
                );
                send_prepared_physical_packet_once_with_start(&prepared, clock, started_ticks)
            }
            #[cfg(not(any(test, feature = "test-support")))]
            {
                let Some(clock) = self.qpc_clock else {
                    self.last_error = Some("packet sender has no QPC clock".to_string());
                    return SendTransactionOutcome {
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
                    };
                };
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::BeforeSenderAuthority,
                );
                self.in_flight_mask |= packet.up_mask | packet.down_mask;
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
                );
                send_prepared_physical_packet_once_with_start(&prepared, clock, started_ticks)
            }
        };
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterSenderReturn);
        #[cfg(any(test, feature = "test-support"))]
        let outcome = self.apply_test_packet_ambiguity(packet, outcome);
        let outcome = self.apply_packet_outcome(packet, outcome);
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterStateCommit);
        outcome
    }

    /// Send a packet whose fixed Win32 payload was built before the precision
    /// boundary.  State reconciliation remains identical to the legacy packet
    /// wrapper; only payload construction moves out of the final path.
    #[cfg(test)]
    pub(crate) fn send_prepared_physical_packet_with_start(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        started_ticks: QpcTicks,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_view_with_start(prepared.as_view(), started_ticks)
    }

    #[cfg(test)]
    fn send_prepared_physical_packet_view_with_start(
        &mut self,
        prepared: PreparedPacketView<'_>,
        started_ticks: QpcTicks,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::BeforeSenderAuthority);
        let outcome = {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(emitter) = self.custom_packet_emitter.as_ref() {
                self.in_flight_mask |= packet.up_mask | packet.down_mask;
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
                );
                let mut outcome = emitter(packet);
                outcome.evidence.started_ticks = Some(started_ticks);
                if outcome
                    .evidence
                    .completed_ticks
                    .is_some_and(|completed| completed < started_ticks)
                {
                    outcome.evidence.completed_ticks = Some(started_ticks);
                }
                outcome
            } else {
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
                self.in_flight_mask |= packet.up_mask | packet.down_mask;
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
                );
                send_prepared_physical_packet_view_once_with_start(prepared, clock, started_ticks)
            }
            #[cfg(not(any(test, feature = "test-support")))]
            {
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
                self.in_flight_mask |= packet.up_mask | packet.down_mask;
                #[cfg(any(test, feature = "test-support"))]
                self.panic_if_prepared_send_at(
                    super::PreparedSendPanicPoint::AfterJournalBeforeSyscall,
                );
                send_prepared_physical_packet_view_once_with_start(prepared, clock, started_ticks)
            }
        };
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterSenderReturn);
        let outcome = self.apply_packet_outcome(packet, outcome);
        #[cfg(any(test, feature = "test-support"))]
        self.panic_if_prepared_send_at(super::PreparedSendPanicPoint::AfterStateCommit);
        outcome
    }

    /// Send a trusted prepared packet on the production precision path.
    ///
    /// The Win32 primitive samples the authoritative pre-call QPC after the
    /// prepared payload pointer and length are resolved and immediately
    /// before the single `SendInput` call. Test-only timestamp injection is
    /// deliberately kept in `send_prepared_physical_packet_with_start`.
    #[cfg(test)]
    pub(crate) fn send_prepared_physical_packet(
        &mut self,
        prepared: &PreparedPhysicalPacket,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_view(prepared.as_view())
    }

    /// Production scheduler path: retain the in-flight transport journal until
    /// the scheduler has committed the matching physical packet.
    #[cfg(test)]
    pub(crate) fn send_prepared_physical_packet_at_final_boundary_deferred_commit(
        &mut self,
        prepared: &PreparedPhysicalPacket,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_at_final_boundary_deferred_commit_inner(prepared)
    }

    fn send_prepared_physical_packet_at_final_boundary_deferred_commit_inner(
        &mut self,
        prepared: &PreparedPhysicalPacket,
    ) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if prepared.packet().down_mask != 0
            && let Some(hook) = self.physical_study_hook.as_ref()
        {
            hook(prepared.packet().down_mask);
        }
        self.send_prepared_physical_packet_view_with_commit_policy(prepared.as_view(), true)
    }

    /// Worker-owned final-boundary sender which freezes the release origin
    /// before the syscall can create an in-flight or uncertain Down obligation.
    pub fn send_prepared_physical_packet_at_final_boundary_deferred_commit_for_target(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        target_hwnd: isize,
        target_generation: u64,
    ) -> SendTransactionOutcome {
        let owner_matches_target =
            self.session_release_owner_proof()
                .is_some_and(|release_owner| {
                    release_owner.target_hwnd() == target_hwnd
                        && release_owner.target_generation() == target_generation
                });
        if !owner_matches_target {
            self.last_error = Some(
                "prepared packet target does not match the session release owner proof".to_string(),
            );
            return invalid_packet_outcome(prepared.packet());
        }
        self.send_prepared_physical_packet_at_final_boundary_deferred_commit_inner(prepared)
    }

    /// Complete the transport half of a packet transaction after the
    /// scheduler state commit has succeeded.
    pub fn commit_physical_packet(&mut self, packet_mask: u16) -> Result<(), String> {
        if packet_mask == 0 || self.in_flight_mask != packet_mask {
            return Err(format!(
                "physical packet journal commit does not match in-flight mask: requested={packet_mask:#06x}, in_flight={:#06x}",
                self.in_flight_mask
            ));
        }
        self.in_flight_mask = 0;
        Ok(())
    }

    pub(crate) fn apply_packet_outcome(
        &mut self,
        packet: PhysicalPacket,
        outcome: SendTransactionOutcome,
    ) -> SendTransactionOutcome {
        self.apply_packet_outcome_with_commit_policy(packet, outcome, false)
    }

    fn apply_packet_outcome_with_commit_policy(
        &mut self,
        packet: PhysicalPacket,
        mut outcome: SendTransactionOutcome,
        defer_journal_commit: bool,
    ) -> SendTransactionOutcome {
        if outcome.is_success() && !outcome.has_complete_evidence_for(packet) {
            // The raw packet has a single-send contract. Treat contradictory
            // Complete evidence as an integrity failure and keep the whole
            // packet uncertain until terminal cleanup reconciles it.
            outcome.status = SendTransactionStatus::IntegrityLost;
            outcome.evidence.confirmed_mask = 0;
        }
        let confirmed_mask = outcome.evidence.confirmed_mask;
        let complete_transport_without_timing = outcome.has_complete_raw_transport_for(packet)
            && !outcome.has_valid_sender_timing_evidence();
        match outcome.status {
            SendTransactionStatus::Complete => {
                let union = packet.up_mask | packet.down_mask;
                if complete_transport_without_timing {
                    // Raw count and timing are separate evidence dimensions.
                    // A full packet with unusable QPC is terminal to music but
                    // remains journaled for cleanup: prior Ups need a probe,
                    // while Downs remain physical Up duties.
                    self.active_mask &= !union;
                    self.possibly_active_mask &= !union;
                    self.failed_release_mask &= !union;
                    self.needs_probe_mask |= packet.up_mask;
                    self.possibly_active_mask |= packet.down_mask;
                    self.in_flight_mask &= !union;
                } else {
                    self.active_mask = (self.active_mask & !packet.up_mask) | packet.down_mask;
                    self.possibly_active_mask &= !union;
                    self.failed_release_mask &= !packet.up_mask;
                }
                if self.failed_release_mask == 0 {
                    self.last_error = None;
                }
            }
            SendTransactionStatus::ZeroProgress => {
                self.sendinput_zero_progress_failures =
                    self.sendinput_zero_progress_failures.saturating_add(1);
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet made zero progress: {} events requested",
                    packet.event_count()
                ));
                if outcome.evidence.attempts != 0 && packet.down_mask != 0 {
                    // A zero raw insertion count does not prove a Down was not
                    // observed by the target. Keep a conservative release
                    // obligation until the independent safety probe reconciles it.
                    self.active_mask &= !packet.down_mask;
                    self.possibly_active_mask |= packet.down_mask;
                }
            }
            SendTransactionStatus::PreparationRejected => {
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet rejected before SendInput: {} events requested",
                    packet.event_count()
                ));
            }
            SendTransactionStatus::PartialProgress | SendTransactionStatus::IntegrityLost => {
                let uncertain_mask = packet.up_mask | packet.down_mask;
                self.active_mask &= !uncertain_mask;
                self.possibly_active_mask |= uncertain_mask;
                self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
                self.chord_split_events = self.chord_split_events.saturating_add(1);
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet partially inserted: {} of {} events",
                    outcome.evidence.first_inserted,
                    packet.event_count()
                ));
            }
            SendTransactionStatus::ClockFailureBeforeSend
            | SendTransactionStatus::ClockFailureAfterSend => {
                self.timing_error = outcome.evidence.timing_error;
                if matches!(outcome.status, SendTransactionStatus::ClockFailureAfterSend) {
                    if outcome.evidence.attempts != 0 && outcome.evidence.first_inserted == 0 {
                        self.sendinput_zero_progress_failures =
                            self.sendinput_zero_progress_failures.saturating_add(1);
                    } else if outcome.evidence.attempts != 0
                        && u16::from(outcome.evidence.first_inserted)
                            < u16::from(packet.event_count())
                    {
                        self.sendinput_partial_events =
                            self.sendinput_partial_events.saturating_add(1);
                        self.chord_split_events = self.chord_split_events.saturating_add(1);
                    }
                    if outcome.has_complete_raw_transport_for(packet) {
                        // Full transport without a valid completion boundary is
                        // still a terminal musical anomaly. Preserve the
                        // already-sent Up as a probe obligation, while any Down
                        // in the same atomic packet becomes a fresh Up duty.
                        let union = packet.up_mask | packet.down_mask;
                        self.active_mask &= !union;
                        self.possibly_active_mask &= !union;
                        self.failed_release_mask &= !union;
                        self.needs_probe_mask |= packet.up_mask;
                        self.possibly_active_mask |= packet.down_mask;
                        self.in_flight_mask &= !union;
                    } else if !packet_outcome_proves_zero_transport(packet, outcome) {
                        let uncertain_mask = packet.up_mask | packet.down_mask;
                        self.active_mask &= !uncertain_mask;
                        self.possibly_active_mask |= uncertain_mask;
                    }
                }
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet QPC failure ({:?})",
                    outcome.status
                ));
            }
        }
        if packet.up_mask != 0 && !outcome.is_success() {
            let raw_complete_qpc_failure =
                matches!(outcome.status, SendTransactionStatus::ClockFailureAfterSend)
                    && outcome.has_complete_raw_transport_for(packet);
            if !raw_complete_qpc_failure {
                self.active_mask &= !confirmed_mask;
                self.possibly_active_mask &= !confirmed_mask;
                self.failed_release_mask &= !confirmed_mask;
                self.failed_release_mask |= packet.up_mask & !confirmed_mask;
            }
        }
        if !defer_journal_commit {
            self.in_flight_mask &= !(packet.up_mask | packet.down_mask);
        }
        outcome
    }
}

#[cfg(any(test, feature = "test-support"))]
fn logical_prefix_mask(mask: u16, count: u8) -> u16 {
    let mut remaining = mask;
    let mut prefix = 0u16;
    for _ in 0..count {
        if remaining == 0 {
            break;
        }
        let bit = 1u16 << remaining.trailing_zeros();
        prefix |= bit;
        remaining &= !bit;
    }
    prefix
}
