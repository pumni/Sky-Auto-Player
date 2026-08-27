use super::super::down_transaction::emit_down_once_with;
use super::super::outcome::{
    PacketRetryReason, PhysicalPacket, SendEvidence, SendTransactionOutcome, SendTransactionStatus,
};
use super::super::packet::send_prepared_physical_packet_once_at_target_with_cutoff;
use super::super::packet::send_prepared_physical_packet_once_with_start_and_cutoff;
use super::super::packet::{
    PreparedPacketView, PreparedPhysicalPacket, invalid_packet_outcome,
    send_physical_packet_once_with_start,
};
use super::super::physical::mask_for_scan_codes;
use super::super::raw::{
    no_syscall_boundary_with_clock, send_input_raw, send_input_raw_with_clock,
};
use super::super::scan_code::key_mask;
use super::super::up_transaction::emit_up_once_with;
use super::TrackedKeyState;
use crate::clock::QpcTicks;
use smallvec::SmallVec;

fn deadline_missed_before_send_outcome(
    packet: PhysicalPacket,
    started_ticks: QpcTicks,
) -> SendTransactionOutcome {
    SendTransactionOutcome {
        status: SendTransactionStatus::DeadlineMissedBeforeSend,
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
            started_ticks: Some(started_ticks),
            completed_ticks: None,
            timing_error: None,
        },
    }
}

impl TrackedKeyState {
    fn do_emit_down(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_down_once_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_down_once_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_down_once_with(scan_codes, send_input_raw)
        }
    }

    fn do_emit_up(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_up_once_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_up_once_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_up_once_with(scan_codes, send_input_raw)
        }
    }

    /// Single-send note-off for operator-owned cleanup retries. The cleanup FSM
    /// bounds the raw `SendInput` count itself; this must never perform an
    /// internal retry.
    pub(super) fn do_emit_up_once(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_up_once_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_up_once_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_up_once_with(scan_codes, send_input_raw)
        }
    }

    pub fn key_down(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        let requested_mask = mask_for_scan_codes(scan_codes).unwrap_or(0);
        if scan_codes.is_empty() || requested_mask == 0 {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
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

        let mut to_send: SmallVec<[u16; 15]> = SmallVec::new();
        let mut duplicates: SmallVec<[u16; 15]> = SmallVec::new();

        for &sc in scan_codes {
            if self.active_mask & key_mask(sc).unwrap_or(0) != 0 {
                duplicates.push(sc);
            } else {
                to_send.push(sc);
            }
        }

        let skipped_mask = mask_for_scan_codes(&duplicates).unwrap_or(0);

        if to_send.is_empty() {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask,
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

        for &sc in &to_send {
            self.possibly_active_mask |= key_mask(sc).unwrap_or(0);
        }

        let emitted = self.do_emit_down(&to_send);
        self.timing_error = emitted.evidence.timing_error;

        if matches!(
            emitted.status,
            SendTransactionStatus::PartialProgress | SendTransactionStatus::IntegrityLost
        ) {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
        }
        if matches!(emitted.status, SendTransactionStatus::ZeroProgress) {
            self.sendinput_zero_progress_failures =
                self.sendinput_zero_progress_failures.saturating_add(1);
        }

        let to_send_mask = mask_for_scan_codes(&to_send).unwrap_or(0);

        match emitted.status {
            SendTransactionStatus::Complete => {
                self.active_mask |= to_send_mask;
                self.possibly_active_mask &= !to_send_mask;
            }
            SendTransactionStatus::IntegrityLost => {
                self.active_mask &= !to_send_mask;
                self.possibly_active_mask |= to_send_mask;
                self.chord_split_events = self.chord_split_events.saturating_add(1);
            }
            SendTransactionStatus::ClockFailureAfterSend => {
                self.active_mask &= !to_send_mask;
                self.possibly_active_mask |= to_send_mask;
            }
            SendTransactionStatus::ZeroProgress
            | SendTransactionStatus::DeadlineMissedBeforeSend
            | SendTransactionStatus::ClockFailureBeforeSend => {
                self.possibly_active_mask &= !to_send_mask;
            }
            SendTransactionStatus::PartialProgress => {
                self.active_mask &= !to_send_mask;
                self.possibly_active_mask |= to_send_mask;
                self.chord_split_events = self.chord_split_events.saturating_add(1);
            }
        }

        if !emitted.is_success() {
            self.chords_rejected = self.chords_rejected.saturating_add(1);
            self.authored_keys_rejected = self
                .authored_keys_rejected
                .saturating_add(to_send.len() as u64);
        }

        if emitted.is_success() {
            if self.failed_release_mask == 0 {
                self.last_error = None;
            }
        } else {
            self.last_error = Some(format!("note-on rejected; status={:?}", emitted.status,));
        }

        SendTransactionOutcome {
            status: emitted.status,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: if emitted.is_success() {
                    requested_mask
                } else {
                    0
                },
                skipped_mask,
                ..emitted.evidence
            },
        }
    }

    /// Send one validated physical packet. The packet builder is the sole
    /// authored/release transport: it emits all Up events before all Down
    /// events in one `SendInput` call.
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
    pub fn send_physical_packet_with_start(
        &mut self,
        packet: PhysicalPacket,
        started_ticks: QpcTicks,
    ) -> SendTransactionOutcome {
        if let Err(error) = PreparedPhysicalPacket::try_new(packet) {
            self.last_error = Some(format!("physical packet preparation failed: {error}"));
            return self.apply_packet_outcome(packet, invalid_packet_outcome(packet));
        }
        let outcome = {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(emitter) = self.custom_packet_emitter.as_ref() {
                let mut outcome = emitter(packet);
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
                send_physical_packet_once_with_start(packet, clock, started_ticks)
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
                send_physical_packet_once_with_start(packet, clock, started_ticks)
            }
        };

        self.apply_packet_outcome(packet, outcome)
    }

    /// Send a packet whose fixed Win32 payload was built before the precision
    /// boundary.  State reconciliation remains identical to the legacy packet
    /// wrapper; only payload construction moves out of the final path.
    pub fn send_prepared_physical_packet_with_start(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        started_ticks: QpcTicks,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_with_start_and_cutoff(prepared, started_ticks, None)
    }

    /// Send a prepared packet with a caller-controlled authoritative start
    /// timestamp and the same pre-syscall Down cutoff as production.
    pub fn send_prepared_physical_packet_with_start_and_cutoff(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        started_ticks: QpcTicks,
        latest_allowed_down_qpc: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_view_with_start_and_cutoff(
            prepared.as_view(),
            started_ticks,
            latest_allowed_down_qpc,
        )
    }

    /// Send a prepared borrowed-view packet with a caller-controlled
    /// authoritative start timestamp and the same pre-syscall Down cutoff.
    pub fn send_prepared_physical_packet_view_with_start_and_cutoff(
        &mut self,
        prepared: PreparedPacketView<'_>,
        started_ticks: QpcTicks,
        latest_allowed_down_qpc: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
            return self.apply_packet_outcome(
                packet,
                deadline_missed_before_send_outcome(packet, started_ticks),
            );
        }
        let outcome = {
            #[cfg(any(test, feature = "test-support"))]
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
                super::super::packet::send_prepared_physical_packet_view_once_with_start_and_cutoff(
                    prepared,
                    clock,
                    started_ticks,
                    latest_allowed_down_qpc,
                )
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
                super::super::packet::send_prepared_physical_packet_view_once_with_start_and_cutoff(
                    prepared,
                    clock,
                    started_ticks,
                    latest_allowed_down_qpc,
                )
            }
        };
        self.apply_packet_outcome(packet, outcome)
    }

    /// Send a trusted prepared packet on the production precision path.
    ///
    /// The target-crossing loop and the one SendInput attempt are owned by
    /// the Win32 packet primitive. Test support may provide an already
    /// controlled crossing sample or a deterministic packet emitter.
    pub fn send_prepared_physical_packet_at_target_with_cutoff(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        qpc_clock: crate::clock::QpcClock,
        physical_target_qpc: QpcTicks,
        latest_allowed_down_qpc: Option<QpcTicks>,
        _test_started_ticks: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        #[cfg(any(test, feature = "test-support"))]
        {
            let started_ticks = if let Some(started_ticks) = _test_started_ticks {
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
                                        retry_reason: PacketRetryReason::None,
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
                if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
                    return self.apply_packet_outcome(
                        packet,
                        deadline_missed_before_send_outcome(packet, started_ticks),
                    );
                }
                if self.custom_packet_emitter.is_some() {
                    let mut outcome = {
                        let emitter = self
                            .custom_packet_emitter
                            .as_ref()
                            .expect("packet emitter checked above");
                        emitter(packet)
                    };
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
                if _test_started_ticks.is_some() {
                    let outcome = send_prepared_physical_packet_once_with_start_and_cutoff(
                        prepared,
                        qpc_clock,
                        started_ticks,
                        latest_allowed_down_qpc,
                    );
                    return self.apply_packet_outcome(packet, outcome);
                }
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

    /// Phase-A benchmark seam: the direct test boundary supplies a controlled
    /// caller-owned crossing sample while retaining the production sender's
    /// immediate one-attempt path.
    #[cfg(any(test, feature = "test-support"))]
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

    /// Send a trusted prepared packet on the production precision path.
    ///
    /// The Win32 primitive samples the authoritative pre-call QPC after the
    /// prepared payload pointer and length are resolved and immediately
    /// before the single `SendInput` call. Test-only timestamp injection is
    /// deliberately kept in `send_prepared_physical_packet_with_start`.
    pub fn send_prepared_physical_packet(
        &mut self,
        prepared: &PreparedPhysicalPacket,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_with_cutoff(prepared, None)
    }

    /// Send a trusted prepared packet and enforce an optional Down-only hard
    /// cutoff against the sender's authoritative pre-call QPC sample.
    pub fn send_prepared_physical_packet_with_cutoff(
        &mut self,
        prepared: &PreparedPhysicalPacket,
        latest_allowed_down_qpc: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        self.send_prepared_physical_packet_view_with_cutoff(
            prepared.as_view(),
            latest_allowed_down_qpc,
        )
    }

    /// Send a trusted borrowed prepared packet and enforce an optional
    /// Down-only hard cutoff against the sender's authoritative QPC sample.
    pub fn send_prepared_physical_packet_view_with_cutoff(
        &mut self,
        prepared: PreparedPacketView<'_>,
        latest_allowed_down_qpc: Option<QpcTicks>,
    ) -> SendTransactionOutcome {
        let packet = prepared.packet();
        #[cfg(any(test, feature = "test-support"))]
        let outcome = if let Some(emitter) = self.custom_packet_emitter.as_ref() {
            emitter(packet)
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
            super::super::packet::send_prepared_physical_packet_view_once_with_cutoff(
                prepared,
                clock,
                latest_allowed_down_qpc,
            )
        };
        #[cfg(not(any(test, feature = "test-support")))]
        let outcome = {
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
            super::super::packet::send_prepared_physical_packet_view_once_with_cutoff(
                prepared,
                clock,
                latest_allowed_down_qpc,
            )
        };
        self.apply_packet_outcome(packet, outcome)
    }

    fn apply_packet_outcome(
        &mut self,
        packet: PhysicalPacket,
        outcome: SendTransactionOutcome,
    ) -> SendTransactionOutcome {
        let confirmed_mask = outcome.evidence.confirmed_mask;
        match outcome.status {
            SendTransactionStatus::Complete => {
                let union = packet.up_mask | packet.down_mask;
                self.active_mask = (self.active_mask & !packet.up_mask) | packet.down_mask;
                self.possibly_active_mask &= !union;
                self.failed_release_mask &= !packet.up_mask;
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
                    let uncertain_mask = packet.up_mask | packet.down_mask;
                    self.active_mask &= !uncertain_mask;
                    self.possibly_active_mask |= uncertain_mask;
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
            SendTransactionStatus::DeadlineMissedBeforeSend => {
                // This is a typed no-syscall timing result, not a transport
                // rejection. The worker owns Production missed-Down recovery
                // and records the boundary there. Keep backend rejection
                // health clean because SendInput was never called.
                self.last_error = None;
            }
        }
        if packet.up_mask != 0 && !outcome.is_success() {
            self.active_mask &= !confirmed_mask;
            self.possibly_active_mask &= !confirmed_mask;
            self.failed_release_mask &= !confirmed_mask;
            self.failed_release_mask |= packet.up_mask & !confirmed_mask;
        }
        outcome
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn key_down_physical_packet(&mut self, packet: PhysicalPacket) -> SendTransactionOutcome {
        self.send_physical_packet(packet)
    }

    pub fn key_up(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        let requested_mask = mask_for_scan_codes(scan_codes).unwrap_or(0);
        if scan_codes.is_empty() || requested_mask == 0 {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
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

        let mut to_release: SmallVec<[u16; 15]> = SmallVec::new();
        let mut already_released: SmallVec<[u16; 15]> = SmallVec::new();

        for &sc in scan_codes {
            let bit = key_mask(sc).unwrap_or(0);
            if self.active_mask & bit != 0
                || self.possibly_active_mask & bit != 0
                || self.failed_release_mask & bit != 0
            {
                to_release.push(sc);
            } else {
                already_released.push(sc);
            }
        }

        let skipped_mask = mask_for_scan_codes(&already_released).unwrap_or(0);

        if to_release.is_empty() {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask,
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

        let emitted = self.do_emit_up(&to_release);
        self.timing_error = emitted.evidence.timing_error;

        if matches!(emitted.status, SendTransactionStatus::PartialProgress) {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
        }

        let confirmed_mask = emitted.evidence.confirmed_mask;
        self.active_mask &= !confirmed_mask;
        self.possibly_active_mask &= !confirmed_mask;
        self.failed_release_mask &= !confirmed_mask;

        if !emitted.is_success() {
            let unconfirmed_released =
                mask_for_scan_codes(&to_release).unwrap_or(0) & !confirmed_mask;
            self.failed_release_mask |= unconfirmed_released;
            self.last_error = Some("partial note-off".to_string());
        } else if self.failed_release_mask == 0 {
            self.last_error = None;
        }

        SendTransactionOutcome {
            status: emitted.status,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask,
                skipped_mask,
                ..emitted.evidence
            },
        }
    }
}
