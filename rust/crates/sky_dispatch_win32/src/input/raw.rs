use super::outcome::{EmitResult, PlatformSendResult};
use super::scan_code::{
    PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE, valid_instrument_scan_code,
};
use crate::clock::{QpcClock, QpcTicks};
use smallvec::SmallVec;

pub(crate) fn no_syscall_boundary_with_clock(
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
            .any(|&scan_code| !valid_instrument_scan_code(scan_code))
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
