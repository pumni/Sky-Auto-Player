#[cfg(test)]
use super::super::outcome::TerminalReleaseEvidence;
use super::super::outcome::{
    GuardedReleaseOutcome, PhysicalPacket, ReleaseAllOutcome, TerminalProbeVerdict,
    TerminalSendAttemptEvidence,
};
use super::super::packet::PreparedPhysicalPacket;
use super::super::physical::{ReconciledRelease, reconcile_logical_release_observation};
use super::{CleanupAttemptAdmission, TrackedKeyState, release_retry_sleep};
use crate::input::ReleaseOwnerProof;
use smallvec::SmallVec;

impl TrackedKeyState {
    /// Terminal finalization may issue one bounded Up transport when QPC is
    /// unavailable, but it is restricted to this session's tracked mask.
    #[cfg(test)]
    pub(crate) fn release_terminal_tracked_scope_for_test(
        &mut self,
        target_hwnd: isize,
    ) -> TerminalReleaseEvidence {
        self.release_scope_with_qpc_fallback(target_hwnd, true, false, None, None, 4)
            .terminal_evidence()
    }

    /// Resumable cleanup shares its retry/probe reducer with terminal cleanup.
    /// The worker callback is sampled after every delay and again immediately
    /// before an admitted physical send, allowing Stop to upgrade this drain.
    pub fn release_tracked_with_attempt_admission_and_drain_control(
        &mut self,
        target_hwnd: isize,
        mut admission: impl FnMut(u16, &ReleaseOwnerProof) -> Result<(), String>,
        mut drain_control: impl FnMut() -> Result<super::super::outcome::CleanupDrainDecision, String>,
    ) -> GuardedReleaseOutcome {
        self.release_scope_with_qpc_fallback(
            target_hwnd,
            false,
            true,
            Some(&mut admission),
            Some(&mut drain_control),
            4,
        )
    }

    /// Terminal cleanup with explicit per-attempt origin admission.
    /// Terminal QPC fallback remains available, but never bypasses the fresh
    /// per-attempt origin admission.
    #[cfg(test)]
    pub(crate) fn release_terminal_tracked_scope_with_attempt_admission(
        &mut self,
        target_hwnd: isize,
        mut admission: impl FnMut(u16, &ReleaseOwnerProof) -> Result<(), String>,
    ) -> GuardedReleaseOutcome {
        self.release_scope_with_qpc_fallback(
            target_hwnd,
            true,
            false,
            Some(&mut admission),
            None,
            4,
        )
    }

    /// Terminal cleanup with a fresh origin/foreground decision at each drain
    /// boundary. The callback may classify a stop as background; this method
    /// then sends at most one best-effort Up and retains its residual.
    pub fn release_terminal_tracked_scope_with_attempt_admission_and_drain_control(
        &mut self,
        target_hwnd: isize,
        mut admission: impl FnMut(u16, &ReleaseOwnerProof) -> Result<(), String>,
        mut drain_control: impl FnMut() -> Result<super::super::outcome::CleanupDrainDecision, String>,
    ) -> GuardedReleaseOutcome {
        self.release_scope_with_qpc_fallback(
            target_hwnd,
            true,
            true,
            Some(&mut admission),
            Some(&mut drain_control),
            4,
        )
    }

    /// Recovery uses the same journal, sender admission and release reducer as
    /// worker cleanup, but each foreground transition is one finite attempt.
    /// The limit is deliberately capped at the worker's four-call budget and
    /// is applied to the whole drain invocation, including QPC fallback.
    pub fn release_terminal_tracked_scope_with_attempt_admission_and_drain_control_with_raw_call_budget(
        &mut self,
        target_hwnd: isize,
        mut admission: impl FnMut(u16, &ReleaseOwnerProof) -> Result<(), String>,
        mut drain_control: impl FnMut() -> Result<super::super::outcome::CleanupDrainDecision, String>,
        max_raw_up_calls: u8,
    ) -> GuardedReleaseOutcome {
        self.release_scope_with_qpc_fallback(
            target_hwnd,
            true,
            false,
            Some(&mut admission),
            Some(&mut drain_control),
            max_raw_up_calls.clamp(1, 4),
        )
    }

    fn release_scope_with_qpc_fallback(
        &mut self,
        target_hwnd: isize,
        mut terminal_qpc_fallback: bool,
        allow_background_terminal_up: bool,
        mut admission: Option<&mut CleanupAttemptAdmission<'_>>,
        mut drain_control: Option<
            &mut dyn FnMut() -> Result<super::super::outcome::CleanupDrainDecision, String>,
        >,
        max_raw_up_calls: u8,
    ) -> GuardedReleaseOutcome {
        let requested_mask = self.release_obligation_mask();

        if requested_mask == 0 {
            return GuardedReleaseOutcome {
                release_outcome: ReleaseAllOutcome {
                    attempted_mask: 0,
                    transport_anomaly: false,
                    released_successfully: true,
                    stuck_mask: 0,
                    verification_inconclusive: false,
                    attempts: 0,
                },
                send_attempts: SmallVec::new(),
                probe_verdict: TerminalProbeVerdict::NotRequired,
                withheld_reason: None,
                needs_up_mask: 0,
                needs_probe_mask: 0,
            };
        }

        let mut needs_probe = self.needs_probe_mask & requested_mask;
        let mut needs_up = self.needs_up_mask() & requested_mask;
        let mut prepared_up = if needs_up == 0 {
            None
        } else {
            PreparedPhysicalPacket::try_new_with_profile(
                PhysicalPacket::new(needs_up, 0),
                &self.instrument_key_profile,
            )
            .ok()
        };
        if needs_up != 0 && prepared_up.is_none() {
            self.last_error = Some("tracked Up packet preparation failed".into());
            return GuardedReleaseOutcome {
                release_outcome: ReleaseAllOutcome {
                    attempted_mask: 0,
                    transport_anomaly: true,
                    released_successfully: false,
                    stuck_mask: needs_up | needs_probe,
                    verification_inconclusive: true,
                    attempts: 0,
                },
                send_attempts: SmallVec::new(),
                probe_verdict: TerminalProbeVerdict::Inconclusive(needs_up | needs_probe),
                withheld_reason: Some("tracked Up packet preparation failed".into()),
                needs_up_mask: needs_up,
                needs_probe_mask: needs_probe,
            };
        }

        const RELEASE_RETRY_DELAYS_MS: [u64; 3] = [15, 50, 100];

        let mut transport_anomaly = false;
        let mut final_probe_verdict = if needs_probe == 0 {
            TerminalProbeVerdict::NotRequired
        } else {
            TerminalProbeVerdict::Inconclusive(needs_probe)
        };
        let mut terminal_send_attempts = SmallVec::<[TerminalSendAttemptEvidence; 4]>::new();
        let mut withheld_reason = None;
        let mut attempted_mask = 0u16;
        let mut attempts = 0u8;
        let mut raw_up_invocations = 0u8;
        let mut background_terminal_drain = false;

        for attempt_idx in 0..usize::from(max_raw_up_calls) {
            if attempt_idx > 0 {
                release_retry_sleep(RELEASE_RETRY_DELAYS_MS[attempt_idx - 1]);
            }

            let mut default_drain_control =
                || Ok(super::super::outcome::CleanupDrainDecision::default());
            let drain_decision = match drain_control.as_mut() {
                Some(control) => match control() {
                    Ok(decision) => decision,
                    Err(reason) => {
                        withheld_reason = Some(reason);
                        break;
                    }
                },
                None => default_drain_control().expect("default drain control is infallible"),
            };
            terminal_qpc_fallback |= drain_decision.terminal_requested;
            let background_terminal = allow_background_terminal_up
                && drain_decision.terminal_requested
                && !drain_decision.origin_foreground;
            if !drain_decision.origin_foreground && !background_terminal {
                withheld_reason = Some("tracked cleanup origin is not foreground".into());
                break;
            }
            background_terminal_drain |= background_terminal;
            if background_terminal && raw_up_invocations != 0 {
                break;
            }
            let raw_attempt_limit = if background_terminal {
                1
            } else {
                max_raw_up_calls
            };
            if needs_up != 0 && raw_up_invocations >= raw_attempt_limit {
                break;
            }
            // A background Stop's one safety Up is best-effort transport only;
            // never use a background keyboard observation to discharge it.
            if background_terminal && needs_up == 0 {
                break;
            }

            if needs_up != 0 {
                let previous_needs_up = needs_up;
                let admission_required = admission.is_some();
                let mut reborrowed_admission =
                    |mask, proof: &ReleaseOwnerProof| match admission.as_mut() {
                        Some(check) => check(mask, proof),
                        None => Ok(()),
                    };
                let attempt_admission = admission_required.then_some(
                    &mut reborrowed_admission
                        as &mut dyn FnMut(u16, &ReleaseOwnerProof) -> Result<(), String>,
                );
                let Some(prepared) = prepared_up.as_ref() else {
                    unreachable!("nonempty needs_up was prepared before the drain")
                };
                let drain_control_callback: &mut dyn FnMut() -> Result<
                    super::super::outcome::CleanupDrainDecision,
                    String,
                > = match drain_control.as_mut() {
                    Some(control) => control,
                    None => &mut default_drain_control,
                };
                let emitted = match self.do_emit_up_once(
                    prepared,
                    super::packet_send::CleanupSendContext {
                        terminal_qpc_fallback: &mut terminal_qpc_fallback,
                        allow_background_terminal_up,
                        admission: attempt_admission,
                        drain_control: drain_control_callback,
                        background_terminal_drain: &mut background_terminal_drain,
                        raw_up_invocations,
                        attempts: &mut attempts,
                    },
                ) {
                    Ok(emitted) => emitted,
                    Err(reason) => {
                        withheld_reason = Some(reason);
                        break;
                    }
                };
                raw_up_invocations =
                    raw_up_invocations.saturating_add(u8::from(emitted.evidence.attempts != 0));
                if emitted.evidence.attempts != 0 {
                    attempted_mask |= previous_needs_up;
                }
                let packet = prepared.packet();
                let raw_transport_complete = emitted.has_complete_raw_transport_for(packet);
                transport_anomaly |= !emitted.has_complete_evidence_for(packet)
                    || emitted.evidence.timing_error.is_some();
                terminal_send_attempts.push(TerminalSendAttemptEvidence {
                    raw_calls: emitted.evidence.attempts,
                    requested_events: emitted.evidence.requested_mask.count_ones() as u8,
                    inserted_events: emitted.evidence.first_inserted,
                    win32_error: emitted
                        .evidence
                        .last_win32_error
                        .or(emitted.evidence.first_win32_error),
                    started_ticks: emitted.evidence.started_ticks,
                    completed_ticks: emitted.evidence.completed_ticks,
                    qpc_error: emitted.evidence.timing_error,
                });
                if !terminal_qpc_fallback && emitted.evidence.timing_error.is_some() {
                    self.timing_error = emitted.evidence.timing_error;
                }
                if emitted.evidence.timing_error.is_some() {
                    // A resumable drain whose QPC evidence fails may still
                    // perform bounded safety cleanup on a later iteration;
                    // its caller will reject the resume barrier and terminate.
                    terminal_qpc_fallback = true;
                }

                if background_terminal_drain && emitted.evidence.attempts != 0 {
                    // Background terminal transport is retained as sender
                    // evidence only. It cannot discharge the original Up
                    // obligation or create a foreground-qualified probe-only
                    // residual.
                    self.active_mask &= !previous_needs_up;
                    self.possibly_active_mask &= !previous_needs_up;
                    self.failed_release_mask |= previous_needs_up;
                    self.in_flight_mask &= !previous_needs_up;
                    self.needs_probe_mask &= !previous_needs_up;
                } else if raw_transport_complete {
                    self.active_mask &= !previous_needs_up;
                    self.possibly_active_mask &= !previous_needs_up;
                    self.failed_release_mask &= !previous_needs_up;
                    self.in_flight_mask &= !previous_needs_up;
                    self.needs_probe_mask |= previous_needs_up;
                    needs_up &= !previous_needs_up;
                    needs_probe |= previous_needs_up;
                    prepared_up = None;
                }

                if background_terminal_drain && raw_up_invocations != 0 {
                    break;
                }
            }

            if needs_probe != 0 {
                let physical_state =
                    self.resolve_release_probe(target_hwnd, needs_probe, needs_probe);
                let reconciled =
                    reconcile_logical_release_observation(needs_probe, needs_probe, physical_state);
                match reconciled {
                    ReconciledRelease::VerifiedAllUp => {
                        self.needs_probe_mask &= !needs_probe;
                        needs_probe = 0;
                        final_probe_verdict = TerminalProbeVerdict::AllUp;
                    }
                    ReconciledRelease::Held(held_mask)
                        if held_mask != 0 && held_mask & !needs_probe == 0 =>
                    {
                        final_probe_verdict = TerminalProbeVerdict::Held(held_mask);
                    }
                    ReconciledRelease::Held(_) | ReconciledRelease::Inconclusive(_) => {
                        final_probe_verdict = TerminalProbeVerdict::Inconclusive(needs_probe);
                    }
                }
            }

            if needs_up == 0 && needs_probe == 0 {
                if self.failed_release_mask == 0 {
                    self.last_error = None;
                }
                return GuardedReleaseOutcome {
                    release_outcome: ReleaseAllOutcome {
                        attempted_mask,
                        transport_anomaly,
                        released_successfully: true,
                        stuck_mask: 0,
                        verification_inconclusive: false,
                        attempts,
                    },
                    send_attempts: terminal_send_attempts,
                    probe_verdict: if attempted_mask == 0 {
                        TerminalProbeVerdict::AllUp
                    } else {
                        final_probe_verdict
                    },
                    withheld_reason: None,
                    needs_up_mask: 0,
                    needs_probe_mask: 0,
                };
            }
        }

        let stuck_mask = needs_up | needs_probe;
        let error_msg = "tracked release incomplete".to_string();
        if stuck_mask != 0 {
            self.last_error = Some(error_msg);
            self.failed_release_mask |= needs_up;
        }
        self.needs_probe_mask = (self.needs_probe_mask & !requested_mask) | needs_probe;
        if needs_probe != 0 && !matches!(final_probe_verdict, TerminalProbeVerdict::Held(_)) {
            final_probe_verdict = TerminalProbeVerdict::Inconclusive(needs_probe);
        }
        let verification_inconclusive =
            matches!(final_probe_verdict, TerminalProbeVerdict::Inconclusive(_));

        GuardedReleaseOutcome {
            release_outcome: ReleaseAllOutcome {
                attempted_mask,
                transport_anomaly,
                released_successfully: stuck_mask == 0,
                stuck_mask,
                verification_inconclusive,
                attempts,
            },
            send_attempts: terminal_send_attempts,
            probe_verdict: final_probe_verdict,
            withheld_reason,
            needs_up_mask: needs_up,
            needs_probe_mask: needs_probe,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn release_all(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        self.release_scope_with_qpc_fallback(target_hwnd, false, false, None, None, 4)
            .release_outcome
    }
}
