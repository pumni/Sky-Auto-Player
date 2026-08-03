//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

use smallvec::SmallVec;
use std::fmt;

use crate::clock::{QpcClock, QpcTicks};

pub const SKY_PLAYER_SIGNATURE: usize = 0x5C1B9111;

pub const PHYSICAL_INSTRUMENT_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, // Y U I O P
    0x23, 0x24, 0x25, 0x26, 0x27, // H J K L ;
    0x31, 0x32, 0x33, 0x34, 0x35, // N M , . /
];

// The current instrument allowlist contains no E0/E1 extended scan codes.

fn key_mask(scan_code: u16) -> Option<u16> {
    PHYSICAL_INSTRUMENT_SCAN_CODES
        .iter()
        .position(|&code| code == scan_code)
        .map(|slot| 1u16 << slot)
}

fn scan_codes_from_mask(mask: u16) -> Vec<u16> {
    PHYSICAL_INSTRUMENT_SCAN_CODES
        .iter()
        .enumerate()
        .filter_map(|(slot, &scan_code)| (mask & (1u16 << slot) != 0).then_some(scan_code))
        .collect()
}

#[cfg(windows)]
fn keyboard_layout_for_target(
    target_hwnd: isize,
) -> Option<windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL> {
    let thread_id = if target_hwnd == 0 {
        0
    } else {
        let mut process_id = 0u32;
        // SAFETY: The HWND is supplied by the validated focus/target path;
        // the process-id output points to a local scalar for the duration of
        // this call.
        let thread_id = unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                target_hwnd as windows_sys::Win32::Foundation::HWND,
                &mut process_id,
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
    (!layout.is_null()).then_some(layout)
}

#[cfg(windows)]
fn virtual_key_for_scan_code(scan_code: u16, target_hwnd: isize) -> Option<i32> {
    let layout = keyboard_layout_for_target(target_hwnd)?;
    // SAFETY: MapVirtualKeyExW reads only the scalar scan code and borrowed
    // layout handle; it does not retain either value.
    let virtual_key = unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::MapVirtualKeyExW(
            u32::from(scan_code),
            windows_sys::Win32::UI::Input::KeyboardAndMouse::MAPVK_VSC_TO_VK_EX,
            layout,
        )
    };
    (virtual_key != 0).then_some(virtual_key as i32)
}

pub(crate) fn is_scan_code_physically_down(scan_code: u16, target_hwnd: isize) -> Option<bool> {
    #[cfg(windows)]
    {
        let virtual_key = virtual_key_for_scan_code(scan_code, target_hwnd)?;
        // SAFETY: GetAsyncKeyState accepts any virtual-key integer and does not
        // retain pointers or transfer ownership.
        let state = unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(virtual_key)
        };
        Some((state as u16 & 0x8000) != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = (scan_code, target_hwnd);
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSendResult {
    pub requested: u32,
    pub inserted: u32,
    /// QPC boundaries for the syscall. `completed_ticks` is absent only when
    /// the post-call clock query failed; in that case `timing_error` is set.
    pub started_ticks: QpcTicks,
    pub completed_ticks: Option<QpcTicks>,
    pub completed_us: u64,
    pub win32_error: u32,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSendResult {
    pub sent: SmallVec<[u16; 15]>,
    pub skipped_duplicates: SmallVec<[u16; 15]>,
    pub success: bool,
    pub error: Option<String>,
    pub send_completed_us: u64,
    pub send_started_ticks: Option<QpcTicks>,
    pub send_completed_ticks: Option<QpcTicks>,
    pub first_win32_error: Option<u32>,
    pub last_win32_error: Option<u32>,
    pub send_attempts: u8,
    pub zero_progress_retries: u8,
    pub first_inserted: u8,
    pub partial_progress: bool,
    pub retried_after_zero_progress: bool,
    pub chord_integrity_lost: bool,
    pub keys_inserted_before_failure: u8,
    pub keys_rolled_back: u8,
    pub rollback_residue_keys: u8,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownSendOutcome {
    Complete {
        completed_us: u64,
        started_ticks: Option<QpcTicks>,
        completed_ticks: Option<QpcTicks>,
        sent: SmallVec<[u16; 15]>,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        retried_after_zero_progress: bool,
        timing_error: Option<crate::clock::QpcError>,
    },
    ZeroProgress {
        error: Option<u32>,
        completed_us: u64,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
        started_ticks: Option<QpcTicks>,
        completed_ticks: Option<QpcTicks>,
        timing_error: Option<crate::clock::QpcError>,
    },
    IntegrityLost {
        inserted_before_failure: u8,
        rolled_back: u8,
        rollback_residue: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
        completed_us: u64,
        started_ticks: Option<QpcTicks>,
        completed_ticks: Option<QpcTicks>,
        sent: SmallVec<[u16; 15]>,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        timing_error: Option<crate::clock::QpcError>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    pub sent: SmallVec<[u16; 15]>,
    pub completed_us: u64,
    pub started_ticks: Option<QpcTicks>,
    pub completed_ticks: Option<QpcTicks>,
    pub success: bool,
    pub keys_dropped: u64,
    pub first_win32_error: Option<u32>,
    pub last_win32_error: Option<u32>,
    pub send_attempts: u8,
    pub zero_progress_retries: u8,
    pub first_inserted: u8,
    pub partial_progress: bool,
    pub retried_after_zero_progress: bool,
    pub chord_integrity_lost: bool,
    pub keys_inserted_before_failure: u8,
    pub keys_rolled_back: u8,
    pub rollback_residue_keys: u8,
    pub timing_error: Option<crate::clock::QpcError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAllOutcome {
    pub attempted: Vec<u16>,
    pub released_successfully: bool,
    pub stuck_keys: Vec<u16>,
    pub verification_inconclusive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhysicalKeyPreflightError {
    UserHeld(Vec<u16>),
    VerificationInconclusive,
}

impl fmt::Display for PhysicalKeyPreflightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UserHeld(keys) => write!(
                f,
                "instrument keys are physically held before playback: {keys:?}"
            ),
            Self::VerificationInconclusive => {
                f.write_str("instrument key physical-state verification was inconclusive")
            }
        }
    }
}

fn no_syscall_boundary_with_clock(
    clock: Option<QpcClock>,
) -> (
    Option<QpcTicks>,
    Option<QpcTicks>,
    u64,
    Option<crate::clock::QpcError>,
) {
    let clock = match clock {
        Some(clock) => clock,
        None => match QpcClock::initialize() {
            Ok(clock) => clock,
            Err(error) => {
                return (None, None, 0, Some(error));
            }
        },
    };
    let Ok(_) = crate::clock::qpc_frequency_checked() else {
        return (
            None,
            None,
            0,
            Some(crate::clock::QpcError::FrequencyUnavailable),
        );
    };
    match clock.now() {
        Ok(ticks) => {
            match clock.timeline_to_us(crate::clock::TimelineTicks::from_raw(ticks.as_u64())) {
                Ok(micros) => (Some(ticks), Some(ticks), micros, None),
                Err(_) => (
                    Some(ticks),
                    Some(ticks),
                    0,
                    Some(crate::clock::QpcError::ConversionOverflow),
                ),
            }
        }
        Err(error) => (None, None, 0, Some(error)),
    }
}

#[cfg(windows)]
pub const fn create_keyboard_input(
    scan_code: u16,
    key_up: bool,
) -> windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    let mut flags = KEYEVENTF_SCANCODE;
    if key_up {
        flags |= KEYEVENTF_KEYUP;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: scan_code,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: SKY_PLAYER_SIGNATURE,
            },
        },
    }
}

#[cfg(windows)]
const MAX_SCAN_CODE: usize = 0x36;

#[cfg(windows)]
const DOWN_TEMPLATES: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_SCAN_CODE] = {
    let mut arr = [create_keyboard_input(0, false); MAX_SCAN_CODE];
    let mut i = 0;
    while i < MAX_SCAN_CODE {
        arr[i] = create_keyboard_input(i as u16, false);
        i += 1;
    }
    arr
};

#[cfg(windows)]
const UP_TEMPLATES: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_SCAN_CODE] = {
    let mut arr = [create_keyboard_input(0, true); MAX_SCAN_CODE];
    let mut i = 0;
    while i < MAX_SCAN_CODE {
        arr[i] = create_keyboard_input(i as u16, true);
        i += 1;
    }
    arr
};

pub fn send_input_raw(scan_codes: &[u16], key_up: bool) -> PlatformSendResult {
    let clock = match QpcClock::initialize() {
        Ok(clock) => clock,
        Err(error) => {
            return PlatformSendResult {
                requested: u32::try_from(scan_codes.len()).unwrap_or(u32::MAX),
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: None,
                completed_us: 0,
                win32_error: 0,
                timing_error: Some(error),
            };
        }
    };
    send_input_raw_with_clock(scan_codes, key_up, clock)
}

pub fn send_input_raw_with_clock(
    scan_codes: &[u16],
    key_up: bool,
    clock: QpcClock,
) -> PlatformSendResult {
    if scan_codes.len() > PHYSICAL_INSTRUMENT_SCAN_CODES.len()
        || scan_codes
            .iter()
            .any(|&scan_code| !PHYSICAL_INSTRUMENT_SCAN_CODES.contains(&scan_code))
    {
        return PlatformSendResult {
            requested: u32::try_from(scan_codes.len()).unwrap_or(u32::MAX),
            inserted: 0,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: None,
            completed_us: 0,
            win32_error: 87,
            timing_error: None,
        };
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SetLastError;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};

        if scan_codes.is_empty() {
            let (started_ticks, completed_ticks, completed_us, timing_error) =
                no_syscall_boundary_with_clock(Some(clock));
            return PlatformSendResult {
                requested: 0,
                inserted: 0,
                started_ticks: started_ticks.unwrap_or(QpcTicks::ZERO),
                completed_ticks,
                completed_us,
                win32_error: 0,
                timing_error,
            };
        }

        let mut packets: [INPUT; 15] = unsafe { std::mem::zeroed() };
        let len = scan_codes.len().min(15);
        for i in 0..len {
            let sc = scan_codes[i];
            packets[i] = if key_up {
                UP_TEMPLATES[sc as usize]
            } else {
                DOWN_TEMPLATES[sc as usize]
            };
        }
        let requested = len as u32;
        let cb_size = std::mem::size_of::<INPUT>() as i32;

        let started_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(timing_error) => {
                return PlatformSendResult {
                    requested,
                    inserted: 0,
                    started_ticks: QpcTicks::ZERO,
                    completed_ticks: None,
                    completed_us: 0,
                    win32_error: 0,
                    timing_error: Some(timing_error),
                };
            }
        };

        // SAFETY: `packets` array holds `requested` contiguous, correctly aligned INPUT
        // values and remains alive and immobile for the duration of SendInput.
        // `requested` is bounded to 15 by the validated caller.
        // SendInput does not promise to clear last-error on every path. Reset
        // it immediately before the syscall so a partial/zero result never
        // inherits an unrelated error from earlier worker activity.
        unsafe { SetLastError(0) };
        let inserted = unsafe { SendInput(requested, packets.as_ptr(), cb_size) }.min(requested);
        let win32_error = if inserted != requested {
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        } else {
            0
        };
        let (completed_ticks, completed_us, timing_error) = match clock.now() {
            Ok(ticks) => {
                match clock.timeline_to_us(crate::clock::TimelineTicks::from_raw(ticks.as_u64())) {
                    Ok(micros) => (Some(ticks), micros, None),
                    Err(_) => (
                        Some(ticks),
                        0,
                        Some(crate::clock::QpcError::ConversionOverflow),
                    ),
                }
            }
            Err(error) => (None, 0, Some(error)),
        };

        PlatformSendResult {
            requested,
            inserted,
            started_ticks,
            completed_ticks,
            completed_us,
            win32_error,
            timing_error,
        }
    }
    #[cfg(not(windows))]
    {
        let (started_ticks, completed_ticks, completed_us, timing_error) =
            no_syscall_boundary_with_clock(Some(clock));
        PlatformSendResult {
            requested: scan_codes.len() as u32,
            inserted: scan_codes.len() as u32,
            started_ticks: started_ticks.unwrap_or(QpcTicks::ZERO),
            completed_ticks,
            completed_us,
            win32_error: 0,
            timing_error,
        }
    }
}

pub fn emit_down_with<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, completed_us, timing_error) =
            no_syscall_boundary_with_clock(None);
        return EmitResult {
            sent: SmallVec::new(),
            completed_us,
            started_ticks,
            completed_ticks,
            success: timing_error.is_none(),
            keys_dropped: 0,
            first_win32_error: None,
            last_win32_error: None,
            send_attempts: 0,
            zero_progress_retries: 0,
            first_inserted: 0,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error,
        };
    }
    let n = scan_codes.len();
    let res1 = send_fn(scan_codes, false);
    let landed1 = (res1.inserted as usize).min(n);
    let first_win32_error = (res1.win32_error != 0).then_some(res1.win32_error);

    if landed1 >= n {
        let sent: SmallVec<[u16; 15]> = scan_codes.iter().copied().collect();
        return EmitResult {
            sent,
            completed_us: res1.completed_us,
            started_ticks: Some(res1.started_ticks),
            completed_ticks: res1.completed_ticks,
            success: true,
            keys_dropped: 0,
            first_win32_error,
            last_win32_error: first_win32_error,
            send_attempts: 1,
            zero_progress_retries: 0,
            first_inserted: landed1 as u8,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error: res1.timing_error,
        };
    }

    // A non-zero partial insertion has already destroyed chord integrity. Do
    // not infer which keys landed and do not send a remainder as Down.
    // Roll back the entire requested chord immediately; any residue is tracked
    // as uncertain and the worker's terminal cleanup handles it fail-closed.
    if landed1 > 0 {
        let rollback = send_fn(scan_codes, true);
        let rollback_inserted = (rollback.inserted as usize).min(n);
        let rollback_error = (rollback.win32_error != 0).then_some(rollback.win32_error);
        return EmitResult {
            sent: SmallVec::new(),
            completed_us: rollback.completed_us,
            started_ticks: Some(res1.started_ticks),
            completed_ticks: rollback.completed_ticks,
            success: false,
            keys_dropped: (n - landed1) as u64,
            first_win32_error,
            last_win32_error: rollback_error.or(first_win32_error),
            send_attempts: 2,
            zero_progress_retries: 0,
            first_inserted: landed1 as u8,
            partial_progress: true,
            retried_after_zero_progress: false,
            chord_integrity_lost: true,
            keys_inserted_before_failure: landed1 as u8,
            keys_rolled_back: rollback_inserted as u8,
            rollback_residue_keys: n.saturating_sub(rollback_inserted) as u8,
            timing_error: res1.timing_error.or(rollback.timing_error),
        };
    }

    // Zero progress is the only case where an immediate retry is safe: the
    // first call inserted no packet, so the chord has not been split yet.
    let retry = send_fn(scan_codes, false);
    let retry_inserted = (retry.inserted as usize).min(n);
    let retry_error = (retry.win32_error != 0).then_some(retry.win32_error);
    if retry_inserted >= n {
        return EmitResult {
            sent: scan_codes.iter().copied().collect(),
            completed_us: retry.completed_us,
            started_ticks: Some(res1.started_ticks),
            completed_ticks: retry.completed_ticks,
            success: true,
            keys_dropped: 0,
            first_win32_error,
            last_win32_error: retry_error.or(first_win32_error),
            send_attempts: 2,
            zero_progress_retries: 1,
            first_inserted: 0,
            partial_progress: false,
            retried_after_zero_progress: true,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error: retry.timing_error.or(res1.timing_error),
        };
    }

    let mut completed_us = retry.completed_us;
    let started_ticks = Some(res1.started_ticks);
    let mut completed_ticks = retry.completed_ticks;
    let mut send_attempts = 2;
    let mut last_win32_error = retry_error.or(first_win32_error);
    let mut rollback_timing_error = None;
    let mut keys_rolled_back = 0u8;
    let mut rollback_residue_keys = 0u8;
    if retry_inserted > 0 {
        let rollback = send_fn(scan_codes, true);
        let rollback_inserted = (rollback.inserted as usize).min(n);
        completed_us = rollback.completed_us;
        completed_ticks = rollback.completed_ticks;
        send_attempts = 3;
        last_win32_error = (rollback.win32_error != 0)
            .then_some(rollback.win32_error)
            .or(last_win32_error);
        rollback_timing_error = rollback.timing_error;
        keys_rolled_back = rollback_inserted as u8;
        rollback_residue_keys = n.saturating_sub(rollback_inserted) as u8;
    }
    let timing_error = retry
        .timing_error
        .or(res1.timing_error)
        .or(rollback_timing_error);
    EmitResult {
        sent: SmallVec::new(),
        completed_us,
        started_ticks,
        completed_ticks,
        success: false,
        keys_dropped: (n - retry_inserted) as u64,
        first_win32_error,
        last_win32_error,
        send_attempts,
        zero_progress_retries: 1,
        first_inserted: 0,
        partial_progress: retry_inserted > 0,
        retried_after_zero_progress: true,
        chord_integrity_lost: retry_inserted > 0,
        keys_inserted_before_failure: retry_inserted as u8,
        keys_rolled_back,
        rollback_residue_keys,
        timing_error,
    }
}

pub fn emit_down(scan_codes: &[u16]) -> EmitResult {
    emit_down_with(scan_codes, send_input_raw)
}

/// Emit a note-off without delaying the real-time worker.
///
/// A partial `SendInput` result gets one immediate retry of the whole
/// requested set. Any delayed retry belongs to the coordinator, which can then enter an
/// interruptible recovery pause instead of blocking command handling inside
/// the platform seam.
fn emit_up_with_immediate<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, completed_us, timing_error) =
            no_syscall_boundary_with_clock(None);
        return EmitResult {
            sent: SmallVec::new(),
            completed_us,
            started_ticks,
            completed_ticks,
            success: timing_error.is_none(),
            keys_dropped: 0,
            first_win32_error: None,
            last_win32_error: None,
            send_attempts: 0,
            zero_progress_retries: 0,
            first_inserted: 0,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error,
        };
    }
    let n = scan_codes.len();
    let first = send_fn(scan_codes, true);
    let first_inserted = (first.inserted as usize).min(n);
    let first_win32_error = (first.win32_error != 0).then_some(first.win32_error);
    if first_inserted >= n {
        return EmitResult {
            sent: scan_codes.iter().copied().collect(),
            completed_us: first.completed_us,
            started_ticks: Some(first.started_ticks),
            completed_ticks: first.completed_ticks,
            success: true,
            keys_dropped: 0,
            first_win32_error,
            last_win32_error: first_win32_error,
            send_attempts: 1,
            zero_progress_retries: 0,
            first_inserted: first_inserted as u8,
            partial_progress: false,
            retried_after_zero_progress: false,
            chord_integrity_lost: false,
            keys_inserted_before_failure: 0,
            keys_rolled_back: 0,
            rollback_residue_keys: 0,
            timing_error: first.timing_error,
        };
    }

    // A partial Up is also uncertain: retry the entire requested set instead
    // of assuming SendInput's inserted count identifies a prefix.
    let second = send_fn(scan_codes, true);
    let second_inserted = (second.inserted as usize).min(n);
    let success = second_inserted >= n;
    let second_win32_error = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32_error = second_win32_error.or(first_win32_error);
    EmitResult {
        sent: if success {
            scan_codes.iter().copied().collect()
        } else {
            SmallVec::new()
        },
        completed_us: second.completed_us,
        started_ticks: Some(first.started_ticks),
        completed_ticks: second.completed_ticks,
        success,
        keys_dropped: u64::from(!success),
        first_win32_error: first_win32_error.or(second_win32_error),
        last_win32_error,
        send_attempts: 2,
        zero_progress_retries: u8::from(first_inserted == 0),
        first_inserted: first_inserted as u8,
        partial_progress: (first_inserted > 0 || second_inserted > 0) && !success,
        retried_after_zero_progress: first_inserted == 0,
        chord_integrity_lost: false,
        keys_inserted_before_failure: if success {
            0
        } else {
            first_inserted.max(second_inserted) as u8
        },
        keys_rolled_back: 0,
        rollback_residue_keys: 0,
        timing_error: first.timing_error.or(second.timing_error),
    }
}

pub fn emit_up_with<F>(scan_codes: &[u16], send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    emit_up_with_immediate(scan_codes, send_fn)
}

pub fn emit_up(scan_codes: &[u16]) -> EmitResult {
    emit_up_with(scan_codes, send_input_raw)
}

pub type CustomEmitterFn = Box<dyn Fn(&[u16], bool) -> PlatformSendResult + Send + Sync>;

#[derive(Default)]
pub struct TrackedKeyState {
    pub active_mask: u16,
    pub possibly_active_mask: u16,
    pub failed_release_mask: u16,
    pub last_error: Option<String>,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_keys_rejected: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    pub timing_error: Option<crate::clock::QpcError>,
    pub custom_emitter: Option<CustomEmitterFn>,
    qpc_clock: Option<QpcClock>,
}

impl fmt::Debug for TrackedKeyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackedKeyState")
            .field("active_mask", &self.active_mask)
            .field("possibly_active_mask", &self.possibly_active_mask)
            .field("failed_release_mask", &self.failed_release_mask)
            .field("last_error", &self.last_error)
            .field("keys_dropped", &self.keys_dropped)
            .field("chord_split_events", &self.chord_split_events)
            .field("sendinput_partial_events", &self.sendinput_partial_events)
            .field(
                "sendinput_zero_progress_failures",
                &self.sendinput_zero_progress_failures,
            )
            .field("chords_rejected", &self.chords_rejected)
            .field("authored_keys_rejected", &self.authored_keys_rejected)
            .field(
                "keys_inserted_before_failure",
                &self.keys_inserted_before_failure,
            )
            .field("keys_rolled_back", &self.keys_rolled_back)
            .field("rollback_residue_keys", &self.rollback_residue_keys)
            .field("timing_error", &self.timing_error)
            .field("custom_emitter", &self.custom_emitter.is_some())
            .field("qpc_clock_configured", &self.qpc_clock.is_some())
            .finish()
    }
}

impl TrackedKeyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_emitter<F>(emitter: F) -> Self
    where
        F: Fn(&[u16], bool) -> PlatformSendResult + Send + Sync + 'static,
    {
        Self {
            custom_emitter: Some(Box::new(emitter)),
            ..Default::default()
        }
    }

    pub fn with_qpc_clock(clock: QpcClock) -> Self {
        Self {
            qpc_clock: Some(clock),
            ..Default::default()
        }
    }

    /// Admit a real playback start/resume only when the user is not holding an
    /// instrument key. Mock emitters do not represent physical keyboard state,
    /// so they are explicitly exempt from this host preflight.
    pub fn ensure_instrument_keys_physically_up(
        &self,
        target_hwnd: isize,
    ) -> Result<(), PhysicalKeyPreflightError> {
        if self.custom_emitter.is_some() {
            return Ok(());
        }
        if target_hwnd == 0 {
            return Err(PhysicalKeyPreflightError::VerificationInconclusive);
        }
        #[cfg(windows)]
        {
            let mut held = Vec::new();
            for &scan_code in &PHYSICAL_INSTRUMENT_SCAN_CODES {
                match is_scan_code_physically_down(scan_code, target_hwnd) {
                    Some(true) => held.push(scan_code),
                    Some(false) => {}
                    None => return Err(PhysicalKeyPreflightError::VerificationInconclusive),
                }
            }
            if held.is_empty() {
                Ok(())
            } else {
                Err(PhysicalKeyPreflightError::UserHeld(held))
            }
        }
        #[cfg(not(windows))]
        {
            let _ = target_hwnd;
            Err(PhysicalKeyPreflightError::VerificationInconclusive)
        }
    }

    fn do_emit_down(&mut self, scan_codes: &[u16]) -> EmitResult {
        if let Some(ref emitter) = self.custom_emitter {
            emit_down_with(scan_codes, |sc, key_up| emitter(sc, key_up))
        } else {
            if let Some(clock) = self.qpc_clock {
                emit_down_with(scan_codes, |sc, key_up| {
                    send_input_raw_with_clock(sc, key_up, clock)
                })
            } else {
                emit_down(scan_codes)
            }
        }
    }

    fn do_emit_up(&mut self, scan_codes: &[u16]) -> EmitResult {
        if let Some(ref emitter) = self.custom_emitter {
            emit_up_with(scan_codes, |sc, key_up| emitter(sc, key_up))
        } else {
            if let Some(clock) = self.qpc_clock {
                emit_up_with(scan_codes, |sc, key_up| {
                    send_input_raw_with_clock(sc, key_up, clock)
                })
            } else {
                emit_up(scan_codes)
            }
        }
    }

    pub fn key_down(&mut self, scan_codes: &[u16]) -> DownSendOutcome {
        if scan_codes.is_empty() {
            let (started_ticks, completed_ticks, completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return DownSendOutcome::Complete {
                completed_us,
                started_ticks,
                completed_ticks,
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                send_attempts: 0,
                zero_progress_retries: 0,
                retried_after_zero_progress: false,
                timing_error,
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

        if to_send.is_empty() {
            let (started_ticks, completed_ticks, completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return DownSendOutcome::Complete {
                completed_us,
                started_ticks,
                completed_ticks,
                sent: SmallVec::new(),
                skipped_duplicates: duplicates,
                send_attempts: 0,
                zero_progress_retries: 0,
                retried_after_zero_progress: false,
                timing_error,
            };
        }

        for &sc in &to_send {
            self.possibly_active_mask |= key_mask(sc).unwrap_or(0);
        }

        let emitted = self.do_emit_down(&to_send);
        self.timing_error = emitted.timing_error;
        self.keys_dropped += emitted.keys_dropped;
        if emitted.partial_progress {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
        }
        if !emitted.success && emitted.retried_after_zero_progress && emitted.sent.is_empty() {
            self.sendinput_zero_progress_failures =
                self.sendinput_zero_progress_failures.saturating_add(1);
        }
        self.keys_inserted_before_failure = self
            .keys_inserted_before_failure
            .saturating_add(emitted.keys_inserted_before_failure as u64);
        self.keys_rolled_back = self
            .keys_rolled_back
            .saturating_add(emitted.keys_rolled_back as u64);
        self.rollback_residue_keys = self
            .rollback_residue_keys
            .saturating_add(emitted.rollback_residue_keys as u64);

        if emitted.chord_integrity_lost {
            // SendInput's inserted count is not a trustworthy prefix receipt.
            // Keep the complete chord possibly active until full cleanup has
            // verified that every requested key is up.
            for &sc in &to_send {
                let bit = key_mask(sc).unwrap_or(0);
                self.active_mask &= !bit;
                self.possibly_active_mask |= bit;
            }
        } else {
            for &sc in &emitted.sent {
                self.active_mask |= key_mask(sc).unwrap_or(0);
            }

            for &sc in &to_send {
                self.possibly_active_mask &= !key_mask(sc).unwrap_or(0);
            }
        }

        if !emitted.success {
            self.chords_rejected = self.chords_rejected.saturating_add(1);
            self.authored_keys_rejected = self
                .authored_keys_rejected
                .saturating_add(to_send.len() as u64);
        }
        if emitted.chord_integrity_lost {
            self.chord_split_events += 1;
        }
        if emitted.success {
            if self.failed_release_mask == 0 {
                self.last_error = None;
            }
        } else {
            self.last_error = Some(format!(
                "note-on rejected: {} of {} keys dropped; chord integrity lost={}",
                emitted.keys_dropped,
                to_send.len(),
                emitted.chord_integrity_lost,
            ));
        }

        if emitted.chord_integrity_lost {
            DownSendOutcome::IntegrityLost {
                inserted_before_failure: emitted.keys_inserted_before_failure,
                rolled_back: emitted.keys_rolled_back,
                rollback_residue: emitted.rollback_residue_keys,
                first_error: emitted.first_win32_error,
                last_error: emitted.last_win32_error,
                completed_us: emitted.completed_us,
                started_ticks: emitted.started_ticks,
                completed_ticks: emitted.completed_ticks,
                sent: emitted.sent,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                timing_error: emitted.timing_error,
            }
        } else if !emitted.success {
            DownSendOutcome::ZeroProgress {
                error: emitted.last_win32_error.or(emitted.first_win32_error),
                completed_us: emitted.completed_us,
                started_ticks: emitted.started_ticks,
                completed_ticks: emitted.completed_ticks,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                first_error: emitted.first_win32_error,
                last_error: emitted.last_win32_error,
                timing_error: emitted.timing_error,
            }
        } else {
            DownSendOutcome::Complete {
                completed_us: emitted.completed_us,
                started_ticks: emitted.started_ticks,
                completed_ticks: emitted.completed_ticks,
                sent: emitted.sent,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                retried_after_zero_progress: emitted.retried_after_zero_progress,
                timing_error: emitted.timing_error,
            }
        }
    }

    pub fn key_up(&mut self, scan_codes: &[u16]) -> InputSendResult {
        if scan_codes.is_empty() {
            let (send_started_ticks, send_completed_ticks, send_completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                success: timing_error.is_none(),
                error: None,
                send_completed_us,
                send_started_ticks,
                send_completed_ticks,
                first_win32_error: None,
                last_win32_error: None,
                send_attempts: 0,
                zero_progress_retries: 0,
                first_inserted: 0,
                partial_progress: false,
                retried_after_zero_progress: false,
                chord_integrity_lost: false,
                keys_inserted_before_failure: 0,
                keys_rolled_back: 0,
                rollback_residue_keys: 0,
                timing_error,
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

        if to_release.is_empty() {
            let (send_started_ticks, send_completed_ticks, send_completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: already_released,
                success: timing_error.is_none(),
                error: None,
                send_completed_us,
                send_started_ticks,
                send_completed_ticks,
                first_win32_error: None,
                last_win32_error: None,
                send_attempts: 0,
                zero_progress_retries: 0,
                first_inserted: 0,
                partial_progress: false,
                retried_after_zero_progress: false,
                chord_integrity_lost: false,
                keys_inserted_before_failure: 0,
                keys_rolled_back: 0,
                rollback_residue_keys: 0,
                timing_error,
            };
        }

        let emitted = self.do_emit_up(&to_release);
        self.timing_error = emitted.timing_error;

        if emitted.partial_progress {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
        }
        self.keys_inserted_before_failure = self
            .keys_inserted_before_failure
            .saturating_add(emitted.keys_inserted_before_failure as u64);
        self.keys_rolled_back = self
            .keys_rolled_back
            .saturating_add(emitted.keys_rolled_back as u64);
        self.rollback_residue_keys = self
            .rollback_residue_keys
            .saturating_add(emitted.rollback_residue_keys as u64);

        for &sc in &emitted.sent {
            let bit = key_mask(sc).unwrap_or(0);
            self.active_mask &= !bit;
            self.possibly_active_mask &= !bit;
            self.failed_release_mask &= !bit;
        }

        if !emitted.success {
            for &sc in &to_release {
                if !emitted.sent.contains(&sc) {
                    self.failed_release_mask |= key_mask(sc).unwrap_or(0);
                }
            }
            self.last_error = Some(format!(
                "partial note-off: {}/{}",
                emitted.sent.len(),
                to_release.len()
            ));
        } else if self.failed_release_mask == 0 {
            self.last_error = None;
        }

        InputSendResult {
            sent: emitted.sent,
            skipped_duplicates: already_released,
            success: emitted.success,
            error: if emitted.success {
                None
            } else {
                Some("partial note-off".to_string())
            },
            send_completed_us: emitted.completed_us,
            send_started_ticks: emitted.started_ticks,
            send_completed_ticks: emitted.completed_ticks,
            first_win32_error: emitted.first_win32_error,
            last_win32_error: emitted.last_win32_error,
            send_attempts: emitted.send_attempts,
            zero_progress_retries: emitted.zero_progress_retries,
            first_inserted: emitted.first_inserted,
            partial_progress: emitted.partial_progress,
            retried_after_zero_progress: emitted.retried_after_zero_progress,
            chord_integrity_lost: emitted.chord_integrity_lost,
            keys_inserted_before_failure: emitted.keys_inserted_before_failure,
            keys_rolled_back: emitted.keys_rolled_back,
            rollback_residue_keys: emitted.rollback_residue_keys,
            timing_error: emitted.timing_error,
        }
    }

    pub fn release_all(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        let tracked_mask = self.active_mask | self.possibly_active_mask | self.failed_release_mask;
        if tracked_mask == 0 {
            return ReleaseAllOutcome {
                attempted: Vec::new(),
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive: false,
            };
        }

        let attempted = scan_codes_from_mask(tracked_mask);

        let mut released_successfully = false;

        for pass_idx in 0..3 {
            let emitted = self.do_emit_up(&attempted);
            if emitted.success && emitted.sent.len() == attempted.len() {
                released_successfully = true;
                break;
            }
            if pass_idx < 2 {
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        }

        let mut verification_inconclusive = false;
        let mut stuck = Vec::new();
        if self.custom_emitter.is_none() && target_hwnd == 0 {
            verification_inconclusive = true;
        } else if self.custom_emitter.is_none() {
            for &scan_code in &attempted {
                match is_scan_code_physically_down(scan_code, target_hwnd) {
                    Some(true) => stuck.push(scan_code),
                    Some(false) => {}
                    None => verification_inconclusive = true,
                }
            }
        }

        if !stuck.is_empty() {
            for delay_ms in [50, 100] {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let _ = self.do_emit_up(&stuck);
                stuck.retain(|&scan_code| {
                    is_scan_code_physically_down(scan_code, target_hwnd).unwrap_or_else(|| {
                        verification_inconclusive = true;
                        true
                    })
                });
                if stuck.is_empty() {
                    released_successfully = true;
                    break;
                }
            }
        }

        if released_successfully && stuck.is_empty() && !verification_inconclusive {
            self.active_mask = 0;
            self.possibly_active_mask = 0;
            self.failed_release_mask = 0;
            self.last_error = None;
            return ReleaseAllOutcome {
                attempted,
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive,
            };
        }

        if stuck.is_empty() {
            self.failed_release_mask |= attempted
                .iter()
                .filter_map(|&scan_code| key_mask(scan_code))
                .fold(0, |mask, bit| mask | bit);
        } else {
            self.failed_release_mask |= stuck
                .iter()
                .filter_map(|&scan_code| key_mask(scan_code))
                .fold(0, |mask, bit| mask | bit);
        }
        self.last_error = Some("tracked release incomplete".to_string());
        let reported_stuck = scan_codes_from_mask(self.failed_release_mask);
        ReleaseAllOutcome {
            attempted,
            released_successfully: false,
            stuck_keys: reported_stuck,
            verification_inconclusive: verification_inconclusive || stuck.is_empty(),
        }
    }

    pub fn release_all_full_instrument(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        let _tracked_outcome = self.release_all(target_hwnd);
        let attempted = PHYSICAL_INSTRUMENT_SCAN_CODES.to_vec();
        let emitted = self.do_emit_up(&attempted);
        let sent = emitted.sent;
        let send_success = emitted.success;
        let mut release_successful = send_success;
        let mut verification_inconclusive = false;
        let mut stuck: Vec<u16> = attempted
            .iter()
            .copied()
            .filter(|scan_code| !sent.contains(scan_code))
            .collect();

        if self.custom_emitter.is_none() && target_hwnd == 0 {
            verification_inconclusive = true;
        } else if self.custom_emitter.is_none() {
            for &scan_code in &attempted {
                match is_scan_code_physically_down(scan_code, target_hwnd) {
                    Some(true) if !stuck.contains(&scan_code) => stuck.push(scan_code),
                    Some(_) => {}
                    None => verification_inconclusive = true,
                }
            }
        }

        if !stuck.is_empty() {
            for delay_ms in [50, 100] {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let retry_sent = self.do_emit_up(&stuck).sent;
                stuck.retain(|scan_code| {
                    if !retry_sent.contains(scan_code) {
                        return true;
                    }
                    if self.custom_emitter.is_some() {
                        return false;
                    }
                    is_scan_code_physically_down(*scan_code, target_hwnd).unwrap_or_else(|| {
                        verification_inconclusive = true;
                        true
                    })
                });
                if stuck.is_empty() {
                    release_successful = true;
                    break;
                }
            }
        }

        if release_successful && stuck.is_empty() && !verification_inconclusive {
            self.active_mask = 0;
            self.possibly_active_mask = 0;
            self.failed_release_mask = 0;
            self.last_error = None;
            return ReleaseAllOutcome {
                attempted,
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive,
            };
        }

        self.failed_release_mask |= stuck
            .iter()
            .filter_map(|&scan_code| key_mask(scan_code))
            .fold(0, |mask, bit| mask | bit);
        self.last_error = Some(format!(
            "full-instrument release incomplete: {}/{} keys unresolved",
            stuck.len(),
            attempted.len()
        ));
        let reported_stuck = scan_codes_from_mask(self.failed_release_mask);
        ReleaseAllOutcome {
            attempted,
            released_successfully: false,
            stuck_keys: reported_stuck,
            verification_inconclusive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn scripted_result(requested: usize, inserted: u32, completed_us: u64) -> PlatformSendResult {
        PlatformSendResult {
            requested: requested as u32,
            inserted,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            completed_us,
            win32_error: 0,
            timing_error: None,
        }
    }

    #[test]
    fn down_retry_matrix_is_exact_and_clamped() {
        for (script, expected_sent, expected_dropped, expected_calls, expected_success) in [
            (vec![3], 3, 0, 1, true),
            // A partial first insertion is rolled back; the remainder is
            // never emitted as a second note-on chord.
            (vec![2, 1], 0, 1, 2, false),
            (vec![0, 0], 0, 3, 2, false),
            (vec![1, 1], 0, 2, 2, false),
            (vec![99], 3, 0, 1, true),
        ] {
            let mut returns = VecDeque::from(script);
            let mut calls = 0;
            let emitted = emit_down_with(&[2, 3, 4], |codes, _| {
                calls += 1;
                scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
            });
            assert_eq!(emitted.sent.len(), expected_sent);
            assert_eq!(calls, expected_calls);
            assert_eq!(emitted.success, expected_success);
            assert_eq!(emitted.keys_dropped, expected_dropped);
        }
    }

    #[test]
    fn partial_note_on_marks_integrity_loss_and_rolls_back_uncertain_chord() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls += 1;
            assert_eq!(key_up, calls == 2);
            scripted_result(codes.len(), 2, calls)
        });

        assert!(!emitted.success);
        assert!(emitted.partial_progress);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(emitted.first_inserted, 2);
        assert_eq!(emitted.sent.len(), 0);
        assert_eq!(emitted.send_attempts, 2);
    }

    #[test]
    fn partial_note_on_rolls_back_the_uncertain_whole_chord() {
        let mut calls = Vec::new();
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls.push((codes.to_vec(), key_up));
            scripted_result(
                codes.len(),
                if key_up { codes.len() as u32 } else { 1 },
                calls.len() as u64,
            )
        });

        assert!(!emitted.success);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(calls, vec![(vec![2, 3, 4], false), (vec![2, 3, 4], true)]);
    }

    #[test]
    fn partial_note_on_after_zero_retry_rolls_back_the_uncertain_whole_chord() {
        let mut calls = Vec::new();
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls.push((codes.to_vec(), key_up));
            let inserted = match calls.len() {
                1 => 0,
                2 if !key_up => 1,
                _ if key_up => 2,
                _ => 0,
            };
            scripted_result(codes.len(), inserted, calls.len() as u64)
        });

        assert!(!emitted.success);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(
            calls,
            vec![
                (vec![2, 3, 4], false),
                (vec![2, 3, 4], false),
                (vec![2, 3, 4], true),
            ]
        );
        assert_eq!(emitted.keys_inserted_before_failure, 1);
        assert_eq!(emitted.keys_rolled_back, 2);
        assert_eq!(emitted.rollback_residue_keys, 1);
    }

    #[test]
    fn zero_progress_can_retry_whole_chord_without_splitting() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls += 1;
            assert!(!key_up);
            scripted_result(codes.len(), if calls == 1 { 0 } else { 3 }, calls)
        });

        assert!(emitted.success);
        assert_eq!(emitted.sent.len(), 3);
        assert_eq!(emitted.send_attempts, 2);
        assert_eq!(emitted.zero_progress_retries, 1);
        assert!(emitted.retried_after_zero_progress);
        assert!(!emitted.chord_integrity_lost);
    }

    #[test]
    fn complete_rollback_reports_no_residue_after_zero_retry() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3, 4], |codes, key_up| {
            calls += 1;
            let inserted = match calls {
                1 => 0,
                2 if !key_up => 1,
                3 if key_up => codes.len() as u32,
                _ => 0,
            };
            scripted_result(codes.len(), inserted, calls)
        });

        assert!(!emitted.success);
        assert_eq!(emitted.rollback_residue_keys, 0);
    }

    #[test]
    fn up_retry_matrix_is_immediate_and_bounded() {
        for (script, expected_sent, expected_calls, expected_success) in [
            (vec![3], 3, 1, true),
            (vec![1, 3], 3, 2, true),
            (vec![0, 0], 0, 2, false),
            (vec![99], 3, 1, true),
        ] {
            let mut returns = VecDeque::from(script);
            let mut calls = 0;
            let emitted = emit_up_with_immediate(&[2, 3, 4], |codes, _| {
                calls += 1;
                scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
            });
            assert_eq!(emitted.sent.len(), expected_sent);
            assert_eq!(calls, expected_calls);
            assert_eq!(emitted.success, expected_success);
        }
    }

    #[test]
    fn partial_note_off_retries_the_entire_requested_set() {
        let mut calls = Vec::new();
        let emitted = emit_up_with_immediate(&[2, 3, 4], |codes, key_up| {
            assert!(key_up);
            calls.push(codes.to_vec());
            scripted_result(
                codes.len(),
                if calls.len() == 1 { 1 } else { 3 },
                calls.len() as u64,
            )
        });

        assert!(emitted.success);
        assert_eq!(calls, vec![vec![2, 3, 4], vec![2, 3, 4]]);
    }

    #[test]
    fn structured_win32_errors_survive_down_retry() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3], |codes, _| {
            calls += 1;
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 1,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: Some(QpcTicks::ZERO),
                completed_us: calls,
                win32_error: if calls == 1 { 5 } else { 0 },
                timing_error: None,
            }
        });

        assert!(!emitted.success);
        assert!(emitted.chord_integrity_lost);
        assert_eq!(emitted.first_win32_error, Some(5));
        assert_eq!(emitted.last_win32_error, Some(5));
        assert_eq!(emitted.send_attempts, 2);
        assert_eq!(emitted.zero_progress_retries, 0);
    }

    #[test]
    fn structured_win32_errors_survive_up_zero_progress_retries() {
        let mut calls = 0;
        let emitted = emit_up_with_immediate(&[2], |codes, _| {
            calls += 1;
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: Some(QpcTicks::ZERO),
                completed_us: calls,
                win32_error: if calls == 1 { 5 } else { 1460 },
                timing_error: None,
            }
        });

        assert!(!emitted.success);
        assert_eq!(emitted.first_win32_error, Some(5));
        assert_eq!(emitted.last_win32_error, Some(1460));
        assert_eq!(emitted.send_attempts, 2);
        assert_eq!(emitted.zero_progress_retries, 1);
        assert!(!emitted.partial_progress);
    }

    #[test]
    fn zero_progress_note_on_counts_rejection_without_counting_a_split() {
        let mut state = TrackedKeyState::with_emitter(|codes, key_up| {
            assert!(!key_up);
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: Some(QpcTicks::ZERO),
                completed_us: 1,
                win32_error: 5,
                timing_error: None,
            }
        });

        let result = state.key_down(&[2, 3]);
        assert!(matches!(result, DownSendOutcome::ZeroProgress { .. }));
        assert_eq!(state.chords_rejected, 1);
        assert_eq!(state.authored_keys_rejected, 2);
        assert_eq!(state.sendinput_partial_events, 0);
        assert_eq!(state.sendinput_zero_progress_failures, 1);
        assert_eq!(state.chord_split_events, 0);
    }

    #[test]
    fn instrument_scan_codes_are_the_physical_allowlist() {
        assert_eq!(
            PHYSICAL_INSTRUMENT_SCAN_CODES,
            [
                0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34,
                0x35,
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn current_layout_maps_all_instrument_scan_codes() {
        assert!(keyboard_layout_for_target(0).is_some());
        for &scan_code in &PHYSICAL_INSTRUMENT_SCAN_CODES {
            assert_ne!(
                virtual_key_for_scan_code(scan_code, 0),
                None,
                "scan code {scan_code:#x} must map in the current layout"
            );
        }
    }

    #[test]
    fn raw_sender_rejects_unknown_scan_code_before_backend_call() {
        let result = send_input_raw(&[0xffff], false);
        assert_eq!(result.requested, 1);
        assert_eq!(result.inserted, 0);
        assert_eq!(result.win32_error, 87);
        assert_eq!(result.timing_error, None);

        let oversized = send_input_raw(&[0x15; 16], false);
        assert_eq!(oversized.inserted, 0);
        assert_eq!(oversized.win32_error, 87);
    }

    #[test]
    fn full_instrument_release_reports_unreleased_keys() {
        let mut state = TrackedKeyState::with_emitter(|codes, key_up| PlatformSendResult {
            requested: codes.len() as u32,
            inserted: if key_up { 0 } else { codes.len() as u32 },
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            completed_us: 10,
            win32_error: 5,
            timing_error: None,
        });
        let outcome = state.release_all_full_instrument(0);
        assert!(!outcome.released_successfully);
        assert_eq!(outcome.attempted, PHYSICAL_INSTRUMENT_SCAN_CODES);
        assert_eq!(outcome.stuck_keys, PHYSICAL_INSTRUMENT_SCAN_CODES);
        assert_eq!(state.failed_release_mask.count_ones(), 15);
    }
}
