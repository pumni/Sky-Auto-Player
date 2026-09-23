//! P7a-only physical-key probes for benchmark and deterministic study builds.
//!
//! This module is compiled only with `test-support`. It records evidence and
//! never makes an admission decision.

use super::physical::query_async_key_state;
use super::profile::MaterializedInstrumentKeyProfile;
use super::scan_code::FULL_INSTRUMENT_MASK;
use sky_dispatch_core::model::MAX_KEYS;

/// Aggregate Shift, Control, and Alt keys plus both Windows keys.
///
/// Microsoft documents the aggregate VK_SHIFT/VK_CONTROL/VK_MENU values as
/// covering both sides. Windows keys have separate VK values.
pub const MODIFIER_GUARD_VKS: [i32; 5] = [0x5b, 0x5c, 0x11, 0x10, 0x12];
pub const MODIFIER_GUARD_NAMES: [&str; 5] = ["LWin", "RWin", "Ctrl", "Shift", "Alt"];
pub const CANONICAL_NOTE_NAMES: [&str; MAX_KEYS] = [
    "Y", "U", "I", "O", "P", "H", "J", "K", "L", ";", "N", "M", ",", ".", "/",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalStudyCandidate {
    PendingDownOnly,
    PendingDownAndModifiers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalStudyEvidence {
    HeldObserved {
        instrument_mask: u16,
        modifier_mask: u8,
    },
    NoHeldObserved,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalStudyObservation {
    /// `None` means the packet had no Down work, so the study made no claim.
    pub evidence: Option<PhysicalStudyEvidence>,
    pub query_count: u8,
}

/// Keyboard-layout mapping captured outside the candidate sender suffix.
#[derive(Clone, Copy, Debug)]
pub struct PhysicalStudyPlan {
    note_virtual_keys: [i32; MAX_KEYS],
}

impl PhysicalStudyPlan {
    /// Resolve the target thread's layout and canonical note VKs before a
    /// benchmark starts. A focus or mapping failure is reported by the caller
    /// as `Inconclusive`; the plan is never rebuilt from the send suffix.
    pub fn materialize(
        target_hwnd: isize,
        profile: &MaterializedInstrumentKeyProfile,
    ) -> Option<Self> {
        #[cfg(windows)]
        {
            if target_hwnd == 0 || !crate::focus::foreground_window_matches(target_hwnd) {
                return None;
            }
            let context = super::physical::keyboard_context_for_target(target_hwnd)?;
            let note_virtual_keys =
                super::physical::map_profile_virtual_keys(&context, profile, FULL_INSTRUMENT_MASK)?;
            if !crate::focus::foreground_window_matches(target_hwnd) {
                return None;
            }
            Some(Self { note_virtual_keys })
        }
        #[cfg(not(windows))]
        {
            let _ = (target_hwnd, profile);
            None
        }
    }

    pub fn virtual_key_for_slot(&self, slot: usize) -> Option<i32> {
        self.note_virtual_keys.get(slot).copied()
    }

    /// Execute one candidate using only pre-materialized VK values and the
    /// supplied async-state query. A zero result is deliberately represented
    /// as `NoHeldObserved`; Win32 also returns zero when the API call fails.
    pub fn observe_with<F>(
        &self,
        candidate: PhysicalStudyCandidate,
        pending_down_mask: u16,
        mut query: F,
    ) -> PhysicalStudyObservation
    where
        F: FnMut(usize, i32) -> i16,
    {
        if pending_down_mask & !FULL_INSTRUMENT_MASK != 0 {
            return PhysicalStudyObservation {
                evidence: Some(PhysicalStudyEvidence::Inconclusive),
                query_count: 0,
            };
        }
        if pending_down_mask == 0 {
            return PhysicalStudyObservation {
                evidence: None,
                query_count: 0,
            };
        }

        let mut instrument_mask = 0u16;
        let mut modifier_mask = 0u8;
        let mut query_count = 0u8;
        for (slot, virtual_key) in self.note_virtual_keys.iter().copied().enumerate() {
            if pending_down_mask & (1u16 << slot) == 0 {
                continue;
            }
            query_count += 1;
            if query(slot, virtual_key) as u16 & 0x8000 != 0 {
                instrument_mask |= 1u16 << slot;
            }
        }
        if matches!(candidate, PhysicalStudyCandidate::PendingDownAndModifiers) {
            for (modifier_slot, virtual_key) in MODIFIER_GUARD_VKS.iter().copied().enumerate() {
                query_count += 1;
                if query(MAX_KEYS + modifier_slot, virtual_key) as u16 & 0x8000 != 0 {
                    modifier_mask |= 1u8 << modifier_slot;
                }
            }
        }

        let evidence = if instrument_mask == 0 && modifier_mask == 0 {
            PhysicalStudyEvidence::NoHeldObserved
        } else {
            PhysicalStudyEvidence::HeldObserved {
                instrument_mask,
                modifier_mask,
            }
        };
        PhysicalStudyObservation {
            evidence: Some(evidence),
            query_count,
        }
    }

    pub fn observe_native(
        &self,
        candidate: PhysicalStudyCandidate,
        pending_down_mask: u16,
    ) -> PhysicalStudyObservation {
        self.observe_with(candidate, pending_down_mask, query_async_key_state)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MODIFIER_GUARD_VKS, PhysicalStudyCandidate, PhysicalStudyEvidence, PhysicalStudyPlan,
    };
    use crate::input::physical::{
        LogicalInstrumentPhysicalState, classify_logical_async_key_states,
    };
    use sky_dispatch_core::model::MAX_KEYS;

    fn plan() -> PhysicalStudyPlan {
        PhysicalStudyPlan {
            note_virtual_keys: std::array::from_fn(|slot| 0x41 + slot as i32),
        }
    }

    #[test]
    fn zero_states_are_no_held_observed_while_the_existing_classifier_says_all_up() {
        let mut queried = 0;
        let observation =
            plan().observe_with(PhysicalStudyCandidate::PendingDownOnly, 0b101, |_, _| {
                queried += 1;
                0
            });
        let empty = [0; MAX_KEYS];
        assert_eq!(
            classify_logical_async_key_states(0b101, &empty),
            LogicalInstrumentPhysicalState::AllUp
        );
        assert_eq!(queried, 2);
        assert_eq!(observation.query_count, 2);
        assert_eq!(
            observation.evidence,
            Some(PhysicalStudyEvidence::NoHeldObserved)
        );
    }

    #[test]
    fn a_held_key_outside_the_pending_down_mask_is_not_queried_or_reported() {
        let observation = plan().observe_with(
            PhysicalStudyCandidate::PendingDownOnly,
            0b001,
            |_, virtual_key| {
                if virtual_key == 0x42 { i16::MIN } else { 0 }
            },
        );
        assert_eq!(observation.query_count, 1);
        assert_eq!(
            observation.evidence,
            Some(PhysicalStudyEvidence::NoHeldObserved)
        );
    }

    #[test]
    fn modifier_candidate_queries_the_exact_five_virtual_keys() {
        let mut queried = Vec::new();
        let observation = plan().observe_with(
            PhysicalStudyCandidate::PendingDownAndModifiers,
            0b10,
            |_, virtual_key| {
                queried.push(virtual_key);
                if virtual_key == 0x5c { i16::MIN } else { 0 }
            },
        );
        assert_eq!(
            &queried[1..],
            &MODIFIER_GUARD_VKS,
            "modifier candidate must query LWin, RWin, aggregate Ctrl, Shift, and Alt"
        );
        assert_eq!(queried[0], 0x42);
        assert_eq!(observation.query_count, 6);
        assert_eq!(
            observation.evidence,
            Some(PhysicalStudyEvidence::HeldObserved {
                instrument_mask: 0,
                modifier_mask: 0b10,
            })
        );
    }

    #[test]
    fn uponly_and_invalid_masks_make_no_queries() {
        let uponly = plan().observe_with(
            PhysicalStudyCandidate::PendingDownAndModifiers,
            0,
            |_, _| panic!("UpOnly must not query Down physical state"),
        );
        assert_eq!(uponly.evidence, None);
        assert_eq!(uponly.query_count, 0);

        let invalid = plan().observe_with(
            PhysicalStudyCandidate::PendingDownOnly,
            1 << MAX_KEYS,
            |_, _| panic!("invalid mask must fail closed before querying"),
        );
        assert_eq!(invalid.evidence, Some(PhysicalStudyEvidence::Inconclusive));
        assert_eq!(invalid.query_count, 0);
    }
}
