use super::super::outcome::PhysicalKeyPreflightError;
#[cfg(any(test, feature = "test-support"))]
use super::super::physical::InstrumentPhysicalState;
use super::super::physical::{
    LogicalInstrumentPhysicalState, instrument_logical_physical_state_for_mask,
};
use super::super::scan_code::FULL_INSTRUMENT_MASK;
use super::TrackedKeyState;

impl TrackedKeyState {
    pub fn ensure_instrument_keys_physically_up(
        &self,
        target_hwnd: isize,
    ) -> Result<(), PhysicalKeyPreflightError> {
        #[cfg(any(test, feature = "test-support"))]
        if self
            .force_preflight_failure
            .as_ref()
            .is_some_and(|flag| flag.load(std::sync::atomic::Ordering::Acquire))
        {
            return Err(PhysicalKeyPreflightError::VerificationInconclusive);
        }
        if self.uses_custom_emitter() {
            return Ok(());
        }
        if target_hwnd == 0 {
            return Err(PhysicalKeyPreflightError::VerificationInconclusive);
        }
        match instrument_logical_physical_state_for_mask(
            &self.instrument_key_profile,
            target_hwnd,
            FULL_INSTRUMENT_MASK,
        ) {
            LogicalInstrumentPhysicalState::AllUp => Ok(()),
            LogicalInstrumentPhysicalState::Held(held_mask) => {
                Err(PhysicalKeyPreflightError::UserHeld(
                    self.instrument_key_profile
                        .scan_codes_from_mask(held_mask)
                        .into_vec(),
                ))
            }
            LogicalInstrumentPhysicalState::Inconclusive => {
                Err(PhysicalKeyPreflightError::VerificationInconclusive)
            }
        }
    }

    /// Resolve the physical probe for the cleanup FSM.
    ///
    /// A test-only `custom_probe` closure provides deterministic evidence keyed
    /// on the unresolved and transport-confirmed masks. When no probe is
    /// installed, a simulated transport emitter must NOT be allowed to
    /// synthesize an AllUp/Held verdict from transport confirmation alone.
    #[cfg(any(test, feature = "test-support"))]
    #[inline]
    pub(super) fn resolve_release_probe(
        &self,
        target_hwnd: isize,
        unresolved_mask: u16,
        transport_confirmed_mask: u16,
    ) -> LogicalInstrumentPhysicalState {
        if let Some(probe) = &self.custom_probe {
            match probe(unresolved_mask, transport_confirmed_mask) {
                InstrumentPhysicalState::AllUp => LogicalInstrumentPhysicalState::AllUp,
                InstrumentPhysicalState::Held(held) => self
                    .instrument_key_profile
                    .logical_mask_for_scan_codes(&held)
                    .map_or(
                        LogicalInstrumentPhysicalState::Inconclusive,
                        LogicalInstrumentPhysicalState::Held,
                    ),
                InstrumentPhysicalState::Inconclusive => {
                    LogicalInstrumentPhysicalState::Inconclusive
                }
            }
        } else if self.uses_custom_emitter() {
            LogicalInstrumentPhysicalState::Inconclusive
        } else {
            #[cfg(windows)]
            {
                let _ = target_hwnd;
                instrument_logical_physical_state_for_mask(
                    &self.instrument_key_profile,
                    target_hwnd,
                    unresolved_mask,
                )
            }
            #[cfg(not(windows))]
            {
                let _ = (target_hwnd, unresolved_mask);
                LogicalInstrumentPhysicalState::Inconclusive
            }
        }
    }

    #[cfg(not(any(test, feature = "test-support")))]
    #[inline]
    pub(super) fn resolve_release_probe(
        &self,
        target_hwnd: isize,
        unresolved_mask: u16,
        _transport_confirmed_mask: u16,
    ) -> LogicalInstrumentPhysicalState {
        instrument_logical_physical_state_for_mask(
            &self.instrument_key_profile,
            target_hwnd,
            unresolved_mask,
        )
    }
}
