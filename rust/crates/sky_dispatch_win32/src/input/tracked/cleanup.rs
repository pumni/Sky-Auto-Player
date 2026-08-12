use super::super::outcome::{PacketRetryReason, ReleaseAllOutcome, SendTransactionStatus};
use super::super::physical::{
    CleanupVerification, ReconciledRelease, reconcile_release_observation,
};
use super::super::scan_code::{FULL_INSTRUMENT_MASK, scan_codes_from_mask};
use super::{ReleaseScope, TrackedKeyState, release_retry_sleep};

impl TrackedKeyState {
    pub fn release_scope(&mut self, scope: ReleaseScope, target_hwnd: isize) -> ReleaseAllOutcome {
        let requested_mask = match scope {
            ReleaseScope::Tracked => {
                self.active_mask | self.possibly_active_mask | self.failed_release_mask
            }
            ReleaseScope::FullInstrument => FULL_INSTRUMENT_MASK,
        };

        if requested_mask == 0 {
            return ReleaseAllOutcome {
                attempted_mask: 0,
                transport_anomaly: false,
                released_successfully: true,
                stuck_mask: 0,
                verification_inconclusive: false,
                attempts: 0,
            };
        }

        let mut unresolved_mask = requested_mask;
        const RELEASE_RETRY_DELAYS_MS: [u64; 3] = [15, 50, 100];

        // Independent evidence dimensions: `transport_anomaly` records that
        // the release path saw any non-clean transport outcome, while the
        // typed verification verdict records what the physical probe decides.
        let mut transport_anomaly = false;
        let mut final_verification: Option<CleanupVerification> = None;

        for attempt_idx in 0..4 {
            if attempt_idx > 0 {
                release_retry_sleep(RELEASE_RETRY_DELAYS_MS[attempt_idx - 1]);
            }
            if unresolved_mask == 0 {
                break;
            }

            let previous_unresolved = unresolved_mask;
            let send_codes = scan_codes_from_mask(unresolved_mask);
            let emitted = self.do_emit_up_once(&send_codes);
            let transport_confirmed_mask = emitted.evidence.confirmed_mask;

            transport_anomaly |= !matches!(emitted.status, SendTransactionStatus::Complete)
                || emitted.evidence.attempts > 1
                || emitted.evidence.retry_reason != PacketRetryReason::None
                || emitted.evidence.first_win32_error.is_some()
                || emitted.evidence.last_win32_error.is_some();

            let physical_state =
                self.resolve_release_probe(target_hwnd, unresolved_mask, transport_confirmed_mask);
            let reconciled = reconcile_release_observation(
                unresolved_mask,
                transport_confirmed_mask,
                physical_state,
            );

            final_verification = Some(match reconciled {
                ReconciledRelease::VerifiedAllUp => CleanupVerification::AllUp,
                ReconciledRelease::Held(held_mask) => CleanupVerification::Held(held_mask),
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    CleanupVerification::Inconclusive(unconfirmed_mask)
                }
            });

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

            match reconciled {
                ReconciledRelease::VerifiedAllUp => {
                    if self.failed_release_mask == 0 {
                        self.last_error = None;
                    }
                    return ReleaseAllOutcome {
                        attempted_mask: requested_mask,
                        transport_anomaly,
                        released_successfully: true,
                        stuck_mask: 0,
                        verification_inconclusive: false,
                        attempts: (attempt_idx + 1) as u8,
                    };
                }
                ReconciledRelease::Held(held_mask) => {
                    unresolved_mask = held_mask;
                }
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    unresolved_mask = unconfirmed_mask;
                    if unconfirmed_mask == 0 {
                        self.last_error = Some(match scope {
                            ReleaseScope::Tracked => "tracked release unverified".to_string(),
                            ReleaseScope::FullInstrument => {
                                "full-instrument release unverified".to_string()
                            }
                        });
                        return ReleaseAllOutcome {
                            attempted_mask: requested_mask,
                            transport_anomaly,
                            released_successfully: false,
                            stuck_mask: 0,
                            verification_inconclusive: true,
                            attempts: (attempt_idx + 1) as u8,
                        };
                    }
                }
            }
        }

        let verification = final_verification.unwrap_or(CleanupVerification::Held(unresolved_mask));
        let released_successfully = verification.is_success();
        let verification_inconclusive = verification.is_inconclusive();
        let stuck_mask = if released_successfully {
            0
        } else {
            unresolved_mask
        };

        let error_msg = match scope {
            ReleaseScope::Tracked => "tracked release incomplete".to_string(),
            ReleaseScope::FullInstrument => format!(
                "full-instrument release incomplete: {}/{} keys unresolved",
                scan_codes_from_mask(unresolved_mask).len(),
                scan_codes_from_mask(requested_mask).len()
            ),
        };
        if !released_successfully {
            self.last_error = Some(error_msg);
            self.failed_release_mask |= stuck_mask;
        }

        ReleaseAllOutcome {
            attempted_mask: requested_mask,
            transport_anomaly,
            released_successfully,
            stuck_mask,
            verification_inconclusive,
            attempts: 4,
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
