//! Fixed modifier-state observation used immediately before musical Down.

use super::physical::query_async_key_state;

const VK_LWIN: i32 = 0x5B;
const VK_RWIN: i32 = 0x5C;
const VK_CONTROL: i32 = 0x11;
const VK_SHIFT: i32 = 0x10;
const VK_MENU: i32 = 0x12;

const MODIFIER_VKS: [(i32, u8); 5] = [
    (VK_LWIN, ModifierMask::LWIN.bits()),
    (VK_RWIN, ModifierMask::RWIN.bits()),
    (VK_CONTROL, ModifierMask::CONTROL.bits()),
    (VK_SHIFT, ModifierMask::SHIFT.bits()),
    (VK_MENU, ModifierMask::ALT.bits()),
];

/// Bit positions reported by the final modifier guard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModifierMask(u8);

impl ModifierMask {
    pub const LWIN: Self = Self(1 << 0);
    pub const RWIN: Self = Self(1 << 1);
    pub const CONTROL: Self = Self(1 << 2);
    pub const SHIFT: Self = Self(1 << 3);
    pub const ALT: Self = Self(1 << 4);

    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Final modifier evidence. Zero API results mean only that no held bit was observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModifierKeyObservation {
    NoHeldObserved,
    HeldObserved(ModifierMask),
}

/// Query exactly the five supported modifiers. `GetAsyncKeyState` has no
/// usable error channel for distinguishing key-up from its documented zero
/// failure result, so zero samples intentionally remain `NoHeldObserved`.
#[inline]
pub fn observe_current_modifier_keys() -> ModifierKeyObservation {
    observe_modifier_keys_with(|virtual_key| query_async_key_state(0, virtual_key))
}

#[inline]
pub(crate) fn observe_modifier_keys_with(
    mut query: impl FnMut(i32) -> i16,
) -> ModifierKeyObservation {
    let mut held_mask = 0u8;
    for (virtual_key, mask) in MODIFIER_VKS {
        if (query(virtual_key) as u16 & 0x8000) != 0 {
            held_mask |= mask;
        }
    }
    if held_mask == 0 {
        ModifierKeyObservation::NoHeldObserved
    } else {
        ModifierKeyObservation::HeldObserved(ModifierMask(held_mask))
    }
}

#[cfg(test)]
mod tests {
    use super::{MODIFIER_VKS, ModifierKeyObservation, ModifierMask, observe_modifier_keys_with};

    #[test]
    fn each_modifier_high_bit_is_reported_and_queries_remain_fixed() {
        for (held_index, (_, expected_mask)) in MODIFIER_VKS.iter().copied().enumerate() {
            let mut calls = Vec::with_capacity(MODIFIER_VKS.len());
            let observation = observe_modifier_keys_with(|virtual_key| {
                calls.push(virtual_key);
                if calls.len() - 1 == held_index {
                    i16::MIN
                } else {
                    0
                }
            });
            assert_eq!(
                observation,
                ModifierKeyObservation::HeldObserved(ModifierMask(expected_mask))
            );
            assert_eq!(calls, MODIFIER_VKS.map(|(virtual_key, _)| virtual_key));
        }
    }

    #[test]
    fn multiple_modifiers_preserve_the_exact_observed_mask() {
        let mut calls = 0;
        let observation = observe_modifier_keys_with(|_| {
            let held = matches!(calls, 0 | 2 | 4);
            calls += 1;
            if held { i16::MIN } else { 0 }
        });
        assert_eq!(calls, 5);
        assert_eq!(
            observation,
            ModifierKeyObservation::HeldObserved(ModifierMask(
                ModifierMask::LWIN.bits() | ModifierMask::CONTROL.bits() | ModifierMask::ALT.bits()
            ))
        );
    }

    #[test]
    fn zero_results_are_named_no_held_observed_and_query_count_is_packet_independent() {
        for _down_chord_size in [1, 5, 15] {
            let mut calls = 0;
            let observation = observe_modifier_keys_with(|_| {
                calls += 1;
                0
            });
            assert_eq!(observation, ModifierKeyObservation::NoHeldObserved);
            assert_eq!(calls, 5);
        }
    }
}
