use super::profile::MaterializedInstrumentKeyProfile;
#[cfg(test)]
use super::scan_code::PHYSICAL_INSTRUMENT_SCAN_CODES;
use super::scan_code::{FULL_INSTRUMENT_MASK, key_mask};
use crate::focus::foreground_window_matches;
use sky_dispatch_core::model::MAX_KEYS;
#[cfg(any(test, feature = "test-support"))]
use smallvec::SmallVec;

#[cfg(windows)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct TargetKeyboardContext {
    layout: windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL,
}

#[cfg(windows)]
pub(crate) fn keyboard_context_for_target(target_hwnd: isize) -> Option<TargetKeyboardContext> {
    let thread_id = if target_hwnd == 0 {
        0
    } else {
        // SAFETY: The HWND is supplied by the validated focus/target path;
        // a null process-id output is permitted because only the target thread
        // ID is needed for the following keyboard-layout query.
        let thread_id = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                target_hwnd as windows_sys::Win32::Foundation::HWND,
                std::ptr::null_mut(),
            )
        };
        if thread_id == 0 {
            return None;
        }
        thread_id
    };

    // SAFETY: GetKeyboardLayout returns a borrowed layout handle and does not
    // retain pointers supplied by the caller.
    let layout =
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayout(thread_id) };
    (!layout.is_null()).then_some(TargetKeyboardContext { layout })
}

#[cfg(windows)]
#[cfg(test)]
pub(crate) fn map_instrument_virtual_keys(
    context: &TargetKeyboardContext,
    requested_mask: u16,
) -> Option<[i32; PHYSICAL_INSTRUMENT_SCAN_CODES.len()]> {
    let mut virtual_keys = [0i32; PHYSICAL_INSTRUMENT_SCAN_CODES.len()];
    for (index, &scan_code) in PHYSICAL_INSTRUMENT_SCAN_CODES.iter().enumerate() {
        if requested_mask & (1u16 << index) == 0 {
            continue;
        }
        // SAFETY: MapVirtualKeyExW reads only the scalar scan code and the
        // borrowed HKL handle; it does not retain either value.
        let virtual_key = unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyExW(
                u32::from(scan_code),
                windows_sys::Win32::UI::Input::KeyboardAndMouse::MAPVK_VSC_TO_VK_EX,
                context.layout,
            )
        };
        if virtual_key == 0 {
            return None;
        }
        virtual_keys[index] = virtual_key as i32;
    }
    Some(virtual_keys)
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstrumentPhysicalState {
    AllUp,
    Held(SmallVec<[u16; 15]>),
    Inconclusive,
}

/// Logical-mask physical evidence used by an armed session. The legacy
/// `InstrumentPhysicalState` remains available for canonical-only callers and
/// for the public preflight error shape; session reconciliation never maps
/// custom keys through the canonical scan-code registry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LogicalInstrumentPhysicalState {
    AllUp,
    Held(u16),
    Inconclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReconciledRelease {
    VerifiedAllUp,
    Held(u16),
    Inconclusive(u16),
}

/// Typed final verdict of the cleanup FSM after all retry attempts.
///
/// The cleanup path must never conflate the two independent evidence
/// dimensions: a transport anomaly (unexpected `SendInput` outcome) and a
/// physical-verification failure (probe could not confirm all-up). Mapping the
/// final observation through this enum keeps `verification_inconclusive`
/// purely probe-derived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CleanupVerification {
    AllUp,
    Held(u16),
    Inconclusive(u16),
}

impl CleanupVerification {
    pub(crate) fn is_success(&self) -> bool {
        matches!(self, Self::AllUp)
    }

    /// `true` only when the physical probe itself could not decide, never when
    /// the transport path reported an anomaly.
    pub(crate) fn is_inconclusive(&self) -> bool {
        matches!(self, Self::Inconclusive(_))
    }
}

#[cfg(test)]
pub(crate) fn reconcile_release_observation(
    requested_mask: u16,
    transport_confirmed_mask: u16,
    physical_state: InstrumentPhysicalState,
) -> ReconciledRelease {
    match physical_state {
        InstrumentPhysicalState::AllUp
            if transport_confirmed_mask & requested_mask == requested_mask =>
        {
            ReconciledRelease::VerifiedAllUp
        }
        InstrumentPhysicalState::AllUp => {
            ReconciledRelease::Inconclusive(requested_mask & !transport_confirmed_mask)
        }
        InstrumentPhysicalState::Held(held_keys) => {
            let held_mask = mask_for_scan_codes(&held_keys).unwrap_or(0);
            ReconciledRelease::Held(held_mask & requested_mask)
        }
        InstrumentPhysicalState::Inconclusive => {
            let unresolved = requested_mask & !transport_confirmed_mask;
            ReconciledRelease::Inconclusive(unresolved)
        }
    }
}

pub(crate) fn reconcile_logical_release_observation(
    requested_mask: u16,
    transport_confirmed_mask: u16,
    physical_state: LogicalInstrumentPhysicalState,
) -> ReconciledRelease {
    match physical_state {
        LogicalInstrumentPhysicalState::AllUp
            if transport_confirmed_mask & requested_mask == requested_mask =>
        {
            ReconciledRelease::VerifiedAllUp
        }
        LogicalInstrumentPhysicalState::AllUp => {
            ReconciledRelease::Inconclusive(requested_mask & !transport_confirmed_mask)
        }
        LogicalInstrumentPhysicalState::Held(held_mask) => {
            ReconciledRelease::Held(held_mask & requested_mask)
        }
        LogicalInstrumentPhysicalState::Inconclusive => {
            let unresolved = requested_mask & !transport_confirmed_mask;
            ReconciledRelease::Inconclusive(unresolved)
        }
    }
}

#[cfg(test)]
pub(crate) fn classify_async_key_states(
    requested_mask: u16,
    key_states: &[i16; 15],
) -> InstrumentPhysicalState {
    if requested_mask & !FULL_INSTRUMENT_MASK != 0 {
        return InstrumentPhysicalState::Inconclusive;
    }
    let mut held = SmallVec::new();
    for (index, &state) in key_states.iter().enumerate() {
        if requested_mask & (1u16 << index) == 0 {
            continue;
        }
        if (state as u16 & 0x8000) != 0 {
            held.push(PHYSICAL_INSTRUMENT_SCAN_CODES[index]);
        }
    }
    if held.is_empty() {
        InstrumentPhysicalState::AllUp
    } else {
        InstrumentPhysicalState::Held(held)
    }
}

pub(crate) fn classify_logical_async_key_states(
    requested_mask: u16,
    key_states: &[i16; MAX_KEYS],
) -> LogicalInstrumentPhysicalState {
    if requested_mask & !FULL_INSTRUMENT_MASK != 0 {
        return LogicalInstrumentPhysicalState::Inconclusive;
    }
    let mut held_mask = 0u16;
    for (index, &state) in key_states.iter().enumerate() {
        if requested_mask & (1u16 << index) != 0 && (state as u16 & 0x8000) != 0 {
            held_mask |= 1u16 << index;
        }
    }
    if held_mask == 0 {
        LogicalInstrumentPhysicalState::AllUp
    } else {
        LogicalInstrumentPhysicalState::Held(held_mask)
    }
}

fn query_async_key_state(_index: usize, virtual_key: i32) -> i16 {
    #[cfg(windows)]
    {
        // SAFETY: GetAsyncKeyState accepts the validated virtual-key scalar
        // and does not retain pointers or transfer ownership.
        unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(virtual_key) }
    }
    #[cfg(not(windows))]
    {
        let _ = (_index, virtual_key);
        0
    }
}

#[cfg(test)]
fn instrument_physical_state_for_mask_with<
    Context,
    Foreground,
    ContextResolver,
    VirtualKeyMapper,
    KeyStateQuery,
>(
    target_hwnd: isize,
    requested_mask: u16,
    mut foreground_matches: Foreground,
    mut context_for_target: ContextResolver,
    mut map_virtual_keys: VirtualKeyMapper,
    mut query_key_state: KeyStateQuery,
) -> InstrumentPhysicalState
where
    Foreground: FnMut(isize) -> bool,
    ContextResolver: FnMut(isize) -> Option<Context>,
    VirtualKeyMapper: FnMut(&Context, u16) -> Option<[i32; PHYSICAL_INSTRUMENT_SCAN_CODES.len()]>,
    KeyStateQuery: FnMut(usize, i32) -> i16,
{
    if target_hwnd == 0 || !foreground_matches(target_hwnd) {
        return InstrumentPhysicalState::Inconclusive;
    }
    if requested_mask == 0 {
        return InstrumentPhysicalState::AllUp;
    }
    if requested_mask & !FULL_INSTRUMENT_MASK != 0 {
        return InstrumentPhysicalState::Inconclusive;
    }

    let Some(context) = context_for_target(target_hwnd) else {
        return InstrumentPhysicalState::Inconclusive;
    };
    let Some(virtual_keys) = map_virtual_keys(&context, requested_mask) else {
        return InstrumentPhysicalState::Inconclusive;
    };
    let mut key_states = [0i16; PHYSICAL_INSTRUMENT_SCAN_CODES.len()];
    for (index, &virtual_key) in virtual_keys.iter().enumerate() {
        if requested_mask & (1u16 << index) == 0 {
            continue;
        }
        key_states[index] = query_key_state(index, virtual_key);
    }
    if !foreground_matches(target_hwnd) {
        return InstrumentPhysicalState::Inconclusive;
    }
    classify_async_key_states(requested_mask, &key_states)
}

#[cfg(windows)]
pub(crate) fn map_profile_virtual_keys(
    context: &TargetKeyboardContext,
    profile: &MaterializedInstrumentKeyProfile,
    requested_mask: u16,
) -> Option<[i32; MAX_KEYS]> {
    let mut virtual_keys = [0i32; MAX_KEYS];
    for (slot, virtual_key_slot) in virtual_keys.iter_mut().enumerate() {
        if requested_mask & (1u16 << slot) == 0 {
            continue;
        }
        let key = profile.physical_key(slot);
        if key.extended {
            return None;
        }
        // SAFETY: MapVirtualKeyExW reads only the validated scan-code scalar
        // and borrowed HKL handle; it does not retain either value.
        let virtual_key = unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyExW(
                u32::from(key.scan_code),
                windows_sys::Win32::UI::Input::KeyboardAndMouse::MAPVK_VSC_TO_VK_EX,
                context.layout,
            )
        };
        if virtual_key == 0 {
            return None;
        }
        *virtual_key_slot = virtual_key as i32;
    }
    Some(virtual_keys)
}

fn instrument_logical_physical_state_for_mask_with<
    Context,
    Foreground,
    ContextResolver,
    VirtualKeyMapper,
    KeyStateQuery,
>(
    profile: &MaterializedInstrumentKeyProfile,
    target_hwnd: isize,
    requested_mask: u16,
    mut foreground_matches: Foreground,
    mut context_for_target: ContextResolver,
    mut map_virtual_keys: VirtualKeyMapper,
    mut query_key_state: KeyStateQuery,
) -> LogicalInstrumentPhysicalState
where
    Foreground: FnMut(isize) -> bool,
    ContextResolver: FnMut(isize) -> Option<Context>,
    VirtualKeyMapper:
        FnMut(&Context, &MaterializedInstrumentKeyProfile, u16) -> Option<[i32; MAX_KEYS]>,
    KeyStateQuery: FnMut(usize, i32) -> i16,
{
    if target_hwnd == 0 || !foreground_matches(target_hwnd) {
        return LogicalInstrumentPhysicalState::Inconclusive;
    }
    if requested_mask == 0 {
        return LogicalInstrumentPhysicalState::AllUp;
    }
    if requested_mask & !FULL_INSTRUMENT_MASK != 0 {
        return LogicalInstrumentPhysicalState::Inconclusive;
    }

    let Some(context) = context_for_target(target_hwnd) else {
        return LogicalInstrumentPhysicalState::Inconclusive;
    };
    let Some(virtual_keys) = map_virtual_keys(&context, profile, requested_mask) else {
        return LogicalInstrumentPhysicalState::Inconclusive;
    };
    let mut key_states = [0i16; MAX_KEYS];
    for (index, &virtual_key) in virtual_keys.iter().enumerate() {
        if requested_mask & (1u16 << index) == 0 {
            continue;
        }
        key_states[index] = query_key_state(index, virtual_key);
    }
    if !foreground_matches(target_hwnd) {
        return LogicalInstrumentPhysicalState::Inconclusive;
    }
    classify_logical_async_key_states(requested_mask, &key_states)
}

pub(crate) fn instrument_logical_physical_state_for_mask(
    profile: &MaterializedInstrumentKeyProfile,
    target_hwnd: isize,
    requested_mask: u16,
) -> LogicalInstrumentPhysicalState {
    #[cfg(windows)]
    {
        instrument_logical_physical_state_for_mask_with(
            profile,
            target_hwnd,
            requested_mask,
            foreground_window_matches,
            keyboard_context_for_target,
            map_profile_virtual_keys,
            query_async_key_state,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (profile, target_hwnd, requested_mask);
        LogicalInstrumentPhysicalState::Inconclusive
    }
}

#[cfg(test)]
pub(crate) fn instrument_physical_state_for_mask(
    target_hwnd: isize,
    requested_mask: u16,
) -> InstrumentPhysicalState {
    #[cfg(windows)]
    {
        instrument_physical_state_for_mask_with(
            target_hwnd,
            requested_mask,
            foreground_window_matches,
            keyboard_context_for_target,
            map_instrument_virtual_keys,
            query_async_key_state,
        )
    }
    #[cfg(not(windows))]
    {
        let _ = (target_hwnd, requested_mask, foreground_window_matches);
        InstrumentPhysicalState::Inconclusive
    }
}

pub(crate) fn mask_for_scan_codes(scan_codes: &[u16]) -> Option<u16> {
    scan_codes.iter().try_fold(0u16, |mask, &scan_code| {
        key_mask(scan_code).map(|bit| mask | bit)
    })
}

/// Single-scan verification retained for the calibration harness. Playback
/// preflight and cleanup use `instrument_physical_state_for_mask` so they
/// resolve the target keyboard context only once per pass.
pub fn is_scan_code_physically_down(scan_code: u16, target_hwnd: isize) -> Option<bool> {
    #[cfg(windows)]
    {
        let context = keyboard_context_for_target(target_hwnd)?;
        // SAFETY: MapVirtualKeyExW reads only the validated scalar and borrowed
        // HKL handle; it does not retain either value.
        let virtual_key = unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyExW(
                u32::from(scan_code),
                windows_sys::Win32::UI::Input::KeyboardAndMouse::MAPVK_VSC_TO_VK_EX,
                context.layout,
            )
        };
        if virtual_key == 0 {
            return None;
        }
        // SAFETY: GetAsyncKeyState accepts the mapped virtual-key scalar and
        // does not retain pointers or transfer ownership.
        let state = unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(virtual_key as i32)
        };
        Some((state as u16 & 0x8000) != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = (scan_code, target_hwnd);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        InstrumentPhysicalState, LogicalInstrumentPhysicalState, classify_logical_async_key_states,
        instrument_logical_physical_state_for_mask_with, instrument_physical_state_for_mask_with,
    };
    use crate::input::{
        InstrumentKeyProfile, InstrumentKeyProfileSpec, MaterializedInstrumentKeyProfile,
        PhysicalKey,
    };

    #[test]
    fn focus_transition_after_key_reads_is_inconclusive() {
        let mut foreground_checks = 0;
        let mut key_reads = 0;
        let state = instrument_physical_state_for_mask_with(
            42,
            1,
            |target| {
                assert_eq!(target, 42);
                foreground_checks += 1;
                foreground_checks == 1
            },
            |_| Some(()),
            |_, _| Some([0; super::PHYSICAL_INSTRUMENT_SCAN_CODES.len()]),
            |index, _| {
                assert_eq!(index, 0);
                key_reads += 1;
                i16::MIN
            },
        );

        assert_eq!(state, InstrumentPhysicalState::Inconclusive);
        assert_eq!(foreground_checks, 2);
        assert_eq!(key_reads, 1);
    }

    #[test]
    fn logical_classification_preserves_profile_slot_identity() {
        let mut key_states = [0i16; super::MAX_KEYS];
        key_states[0] = i16::MIN;
        key_states[14] = i16::MIN;
        assert_eq!(
            classify_logical_async_key_states(0x7fff, &key_states),
            super::LogicalInstrumentPhysicalState::Held((1 << 0) | (1 << 14))
        );
    }

    #[test]
    fn profile_aware_snapshot_maps_non_canonical_key_to_logical_slot() {
        let mut spec = InstrumentKeyProfileSpec::canonical();
        spec.keys[0] = PhysicalKey {
            scan_code: 0x02,
            extended: false,
        };
        let profile = MaterializedInstrumentKeyProfile::from_validated(
            InstrumentKeyProfile::try_from_spec(spec).expect("profile"),
        );
        let mut mapped_scan_code = None;
        let mut queried_virtual_key = None;
        let state = instrument_logical_physical_state_for_mask_with(
            &profile,
            42,
            1,
            |_| true,
            |_| Some(()),
            |_, profile, requested_mask| {
                assert_eq!(requested_mask, 1);
                mapped_scan_code = Some(profile.physical_key(0).scan_code);
                let mut virtual_keys = [0; super::MAX_KEYS];
                virtual_keys[0] = 123;
                Some(virtual_keys)
            },
            |index, virtual_key| {
                assert_eq!(index, 0);
                queried_virtual_key = Some(virtual_key);
                i16::MIN
            },
        );

        assert_eq!(mapped_scan_code, Some(0x02));
        assert_eq!(queried_virtual_key, Some(123));
        assert_eq!(state, LogicalInstrumentPhysicalState::Held(1));
    }

    #[test]
    fn profile_aware_snapshot_focus_race_is_inconclusive() {
        let profile = MaterializedInstrumentKeyProfile::canonical();
        let mut foreground_checks = 0;
        let mut key_reads = 0;
        let state = instrument_logical_physical_state_for_mask_with(
            &profile,
            42,
            1,
            |target| {
                assert_eq!(target, 42);
                foreground_checks += 1;
                foreground_checks == 1
            },
            |_| Some(()),
            |_, _, _| {
                let mut virtual_keys = [0; super::MAX_KEYS];
                virtual_keys[0] = 123;
                Some(virtual_keys)
            },
            |index, virtual_key| {
                assert_eq!(index, 0);
                assert_eq!(virtual_key, 123);
                key_reads += 1;
                i16::MIN
            },
        );

        assert_eq!(state, LogicalInstrumentPhysicalState::Inconclusive);
        assert_eq!(foreground_checks, 2);
        assert_eq!(key_reads, 1);
    }
}
