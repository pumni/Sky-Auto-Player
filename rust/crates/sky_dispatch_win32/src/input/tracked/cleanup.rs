use super::super::outcome::{
    PacketRetryReason, ReleaseAllOutcome, TerminalProbeVerdict, TerminalReleaseEvidence,
    TerminalSendAttemptEvidence,
};
use super::super::physical::{
    LogicalInstrumentPhysicalState, ReconciledRelease, reconcile_logical_release_observation,
};
use super::super::scan_code::FULL_INSTRUMENT_MASK;
use super::{ReleaseScope, TrackedKeyState, release_retry_sleep};
use smallvec::SmallVec;

impl TrackedKeyState {
    pub fn release_scope(&mut self, scope: ReleaseScope, target_hwnd: isize) -> ReleaseAllOutcome {
        self.release_scope_with_qpc_fallback(scope, target_hwnd, false)
            .release_outcome
    }

    /// Terminal finalization may issue one bounded Up transport when QPC is
    /// unavailable, but it is restricted to this session's tracked mask.
    pub fn release_terminal_tracked_scope(
        &mut self,
        target_hwnd: isize,
    ) -> TerminalReleaseEvidence {
        self.release_scope_with_qpc_fallback(ReleaseScope::Tracked, target_hwnd, true)
    }

    fn release_scope_with_qpc_fallback(
        &mut self,
        scope: ReleaseScope,
        target_hwnd: isize,
        terminal_qpc_fallback: bool,
    ) -> TerminalReleaseEvidence {
        let requested_mask = match scope {
            ReleaseScope::Tracked => self.release_obligation_mask(),
            ReleaseScope::FullInstrument => FULL_INSTRUMENT_MASK,
        };

        if requested_mask == 0 {
            return TerminalReleaseEvidence {
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
            };
        }

        let mut unresolved_mask = requested_mask;
        const RELEASE_RETRY_DELAYS_MS: [u64; 3] = [15, 50, 100];

        // Keep the raw physical probe verdict separate from reconciliation:
        // the report exposes what the probe observed, while ownership is only
        // discharged when transport and probe evidence agree.
        let mut transport_anomaly = false;
        let mut final_reconciliation: Option<ReconciledRelease> = None;
        let mut final_probe_verdict: Option<TerminalProbeVerdict> = None;
        let mut terminal_send_attempts = SmallVec::<[TerminalSendAttemptEvidence; 4]>::new();

        for attempt_idx in 0..4 {
            if attempt_idx > 0 {
                release_retry_sleep(RELEASE_RETRY_DELAYS_MS[attempt_idx - 1]);
            }
            if unresolved_mask == 0 {
                break;
            }

            let previous_unresolved = unresolved_mask;
            self.release_attempts_total = self.release_attempts_total.saturating_add(1);
            let emitted = self.do_emit_up_once(unresolved_mask, terminal_qpc_fallback);
            if terminal_qpc_fallback {
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
            } else if emitted.evidence.timing_error.is_some() {
                self.timing_error = emitted.evidence.timing_error;
            }
            let transport_confirmed_mask = emitted.evidence.confirmed_mask;

            let requested_event_count = previous_unresolved.count_ones() as u8;
            transport_anomaly |= emitted.evidence.attempts != 1
                || emitted.evidence.requested_mask != previous_unresolved
                || emitted.evidence.first_inserted != requested_event_count
                || transport_confirmed_mask != previous_unresolved
                || emitted.evidence.skipped_mask != 0
                || emitted.evidence.retry_reason != PacketRetryReason::None
                || emitted.evidence.first_win32_error.is_some()
                || emitted.evidence.last_win32_error.is_some();

            let physical_state =
                self.resolve_release_probe(target_hwnd, unresolved_mask, transport_confirmed_mask);
            final_probe_verdict = Some(match physical_state {
                LogicalInstrumentPhysicalState::AllUp => TerminalProbeVerdict::AllUp,
                LogicalInstrumentPhysicalState::Held(held_mask) => {
                    TerminalProbeVerdict::Held(held_mask)
                }
                LogicalInstrumentPhysicalState::Inconclusive => {
                    TerminalProbeVerdict::Inconclusive(previous_unresolved)
                }
            });
            let reconciled = reconcile_logical_release_observation(
                unresolved_mask,
                transport_confirmed_mask,
                physical_state,
            );
            // The cleanup FSM is an ownership boundary: malformed empty
            // residuals cannot discharge a nonempty request. Normalize them
            // before mutating any conservative backend mask.
            let reconciled = match reconciled {
                ReconciledRelease::Held(0) | ReconciledRelease::Inconclusive(0) => {
                    ReconciledRelease::Inconclusive(previous_unresolved)
                }
                verdict => verdict,
            };
            final_reconciliation = Some(reconciled);

            let resolved_this_transition = match reconciled {
                ReconciledRelease::VerifiedAllUp => previous_unresolved,
                ReconciledRelease::Held(held_mask) => previous_unresolved & !held_mask,
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    previous_unresolved & !unconfirmed_mask
                }
            };
            self.active_mask &= !resolved_this_transition;
            self.possibly_active_mask &= !resolved_this_transition;
            self.failed_release_mask &= !resolved_this_transition;
            self.in_flight_mask &= !resolved_this_transition;

            match reconciled {
                ReconciledRelease::VerifiedAllUp => {
                    if self.failed_release_mask == 0 {
                        self.last_error = None;
                    }
                    return TerminalReleaseEvidence {
                        release_outcome: ReleaseAllOutcome {
                            attempted_mask: requested_mask,
                            transport_anomaly,
                            released_successfully: true,
                            stuck_mask: 0,
                            verification_inconclusive: false,
                            attempts: (attempt_idx + 1) as u8,
                        },
                        send_attempts: terminal_send_attempts,
                        probe_verdict: TerminalProbeVerdict::AllUp,
                    };
                }
                ReconciledRelease::Held(held_mask) => {
                    unresolved_mask = held_mask;
                }
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    unresolved_mask = unconfirmed_mask;
                }
            }
        }

        let release_verification =
            final_reconciliation.unwrap_or(ReconciledRelease::Inconclusive(unresolved_mask));
        let released_successfully =
            matches!(release_verification, ReconciledRelease::VerifiedAllUp);
        let verification_inconclusive =
            matches!(release_verification, ReconciledRelease::Inconclusive(_));
        let stuck_mask = if released_successfully {
            0
        } else {
            unresolved_mask
        };

        let error_msg = match scope {
            ReleaseScope::Tracked => "tracked release incomplete".to_string(),
            ReleaseScope::FullInstrument => format!(
                "full-instrument release incomplete: {}/{} keys unresolved",
                self.instrument_key_profile
                    .scan_codes_from_mask(unresolved_mask)
                    .len(),
                self.instrument_key_profile
                    .scan_codes_from_mask(requested_mask)
                    .len()
            ),
        };
        if !released_successfully {
            self.last_error = Some(error_msg);
            self.failed_release_mask |= stuck_mask;
        }

        let probe_verdict =
            final_probe_verdict.unwrap_or(TerminalProbeVerdict::Inconclusive(unresolved_mask));

        TerminalReleaseEvidence {
            release_outcome: ReleaseAllOutcome {
                attempted_mask: requested_mask,
                transport_anomaly,
                released_successfully,
                stuck_mask,
                verification_inconclusive,
                attempts: 4,
            },
            send_attempts: terminal_send_attempts,
            probe_verdict,
        }
    }

    pub fn release_all(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        self.release_scope(ReleaseScope::Tracked, target_hwnd)
    }

    pub fn release_all_full_instrument(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.full_instrument_release_calls =
                self.full_instrument_release_calls.saturating_add(1);
            if let Some(counter) = &self.full_instrument_release_counter {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        self.release_scope(ReleaseScope::FullInstrument, target_hwnd)
    }
}
