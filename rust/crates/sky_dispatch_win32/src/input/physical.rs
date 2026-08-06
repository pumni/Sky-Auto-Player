use super::scan_code::{FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, key_mask};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstrumentPhysicalState {
    AllUp,
    Held(SmallVec<[u16; 15]>),
    Inconclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReconciledRelease {
    VerifiedAllUp,
    Held(u16),
    Inconclusive(u16),
}

pub(crate) fn reconcile_release_observation(
    requested_mask: u16,
    transport_confirmed_mask: u16,
    physical_state: InstrumentPhysicalState,
) -> ReconciledRelease {
    match physical_state {
        InstrumentPhysicalState::AllUp => ReconciledRelease::VerifiedAllUp,
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

pub(crate) fn instrument_physical_state_for_mask(
    target_hwnd: isize,
    requested_mask: u16,
) -> InstrumentPhysicalState {
    if requested_mask == 0 {
        return InstrumentPhysicalState::AllUp;
    }
    if requested_mask & !FULL_INSTRUMENT_MASK != 0 {
        return InstrumentPhysicalState::Inconclusive;
    }
    #[cfg(windows)]
    {
        if target_hwnd == 0 {
            return InstrumentPhysicalState::Inconclusive;
        }
        let Some(context) = keyboard_context_for_target(target_hwnd) else {
            return InstrumentPhysicalState::Inconclusive;
        };
        let Some(virtual_keys) = map_instrument_virtual_keys(&context, requested_mask) else {
            return InstrumentPhysicalState::Inconclusive;
        };
        let mut held = SmallVec::new();
        for (index, &virtual_key) in virtual_keys.iter().enumerate() {
            if requested_mask & (1u16 << index) == 0 {
                continue;
            }
            // SAFETY: GetAsyncKeyState accepts the validated virtual-key
            // scalar and does not retain pointers or transfer ownership.
            let state = unsafe {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(virtual_key)
            };
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
    #[cfg(not(windows))]
    {
        let _ = (target_hwnd, requested_mask);
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
