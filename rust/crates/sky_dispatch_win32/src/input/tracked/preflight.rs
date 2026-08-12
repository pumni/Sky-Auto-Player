use super::super::outcome::PhysicalKeyPreflightError;
use super::super::physical::{InstrumentPhysicalState, instrument_physical_state_for_mask};
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
        match instrument_physical_state_for_mask(target_hwnd, FULL_INSTRUMENT_MASK) {
            InstrumentPhysicalState::AllUp => Ok(()),
            InstrumentPhysicalState::Held(held) => {
                Err(PhysicalKeyPreflightError::UserHeld(held.into_vec()))
            }
            InstrumentPhysicalState::Inconclusive => {
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
    ) -> InstrumentPhysicalState {
        if let Some(probe) = &self.custom_probe {
            probe(unresolved_mask, transport_confirmed_mask)
        } else if self.uses_custom_emitter() {
            InstrumentPhysicalState::Inconclusive
        } else {
            #[cfg(windows)]
            {
                let _ = target_hwnd;
                instrument_physical_state_for_mask(target_hwnd, unresolved_mask)
            }
            #[cfg(not(windows))]
            {
                let _ = (target_hwnd, unresolved_mask);
                InstrumentPhysicalState::Inconclusive
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
    ) -> InstrumentPhysicalState {
        instrument_physical_state_for_mask(target_hwnd, unresolved_mask)
    }
}
