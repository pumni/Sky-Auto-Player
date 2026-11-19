use super::outcome::{PlatformSendResult, TerminalUpTransportResult};
use super::scan_code::{
    PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE, valid_instrument_scan_code,
};
use crate::clock::{QpcClock, QpcError, QpcTicks};

/// The single raw Win32 transport primitive used by musical packets, safety
/// releases, and calibration payloads. Callers fully prepare the INPUT slice
/// and clear the thread's last-error state before their authoritative QPC
/// boundary. `inputs` must point to `requested` initialized INPUT values, and
/// the caller must keep that storage alive until return. This function performs
/// exactly one SendInput invocation.
#[cfg(windows)]
pub(crate) unsafe fn send_prepared_inputs_once(
    inputs: *const windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT,
    requested: u8,
) -> (u8, u32) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

    let requested_count = u32::from(requested);
    let raw_inserted = unsafe {
        SendInput(
            requested_count,
            inputs,
            std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
        )
    };
    // Requested packets are bounded to fifteen inputs, so saturating the
    // impossible larger count retains an unmistakable mismatch in evidence.
    let inserted = u8::try_from(raw_inserted).unwrap_or(u8::MAX);
    let win32_error = if raw_inserted < requested_count {
        unsafe { windows_sys::Win32::Foundation::GetLastError() }
    } else {
        0
    };
    (inserted, win32_error)
}

#[cfg(not(windows))]
pub(crate) fn send_prepared_inputs_once(event_count: u8) -> (u8, u32) {
    (event_count, 0)
}

pub(crate) fn no_syscall_boundary_with_clock(
    clock: Option<QpcClock>,
) -> (QpcTicks, Option<QpcTicks>, Option<crate::clock::QpcError>) {
    let clock = match clock {
        Some(clock) => clock,
        None => match QpcClock::initialize() {
            Ok(clock) => clock,
            Err(error) => {
                return (QpcTicks::ZERO, None, Some(error));
            }
        },
    };
    let Ok(_) = crate::clock::qpc_frequency_checked() else {
        return (
            QpcTicks::ZERO,
            None,
            Some(crate::clock::QpcError::FrequencyUnavailable),
        );
    };
    match clock.now() {
        Ok(ticks) => (ticks, Some(ticks), None),
        Err(error) => (QpcTicks::ZERO, None, Some(error)),
    }
}

/// Run one terminal Up send with optional timing samples around the transport.
/// A failed pre-call sample is evidence loss only: it must not suppress the
/// caller's bounded safety send. The cleanup state machine owns all retries.
pub(crate) fn terminal_up_send_with<Qpc, Send>(
    requested: u8,
    mut sample_qpc: Qpc,
    mut send_input: Send,
) -> TerminalUpTransportResult
where
    Qpc: FnMut() -> Result<QpcTicks, QpcError>,
    Send: FnMut() -> (u8, u32),
{
    let (started_ticks, start_error) = match sample_qpc() {
        Ok(ticks) => (Some(ticks), None),
        Err(error) => (None, Some(error)),
    };
    let (inserted, win32_error) = send_input();
    let (completed_ticks, completion_error) = match sample_qpc() {
        Ok(ticks) => (Some(ticks), None),
        Err(error) => (None, Some(error)),
    };

    TerminalUpTransportResult {
        requested,
        inserted,
        started_ticks,
        completed_ticks,
        win32_error,
        timing_error: start_error.or(completion_error),
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

pub(crate) fn send_input_raw(scan_codes: &[u16], key_up: bool) -> PlatformSendResult {
    let clock = match QpcClock::initialize() {
        Ok(clock) => clock,
        Err(error) => {
            return PlatformSendResult {
                requested: u8::try_from(scan_codes.len()).unwrap_or(u8::MAX),
                inserted: 0,
                started_ticks: QpcTicks::ZERO,
                completed_ticks: None,
                win32_error: 0,
                timing_error: Some(error),
            };
        }
    };
    send_input_raw_with_clock(scan_codes, key_up, clock)
}

pub(crate) fn send_input_raw_with_clock(
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
            requested: u8::try_from(scan_codes.len()).unwrap_or(u8::MAX),
            inserted: 0,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: None,
            win32_error: 87,
            timing_error: None,
        };
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SetLastError;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT;

        if scan_codes.is_empty() {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(Some(clock));
            return PlatformSendResult {
                requested: 0,
                inserted: 0,
                started_ticks,
                completed_ticks,
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
        // Clear stale thread-local state before the authoritative start sample;
        // the reset must not sit inside the measured SendInput envelope.
        unsafe { SetLastError(0) };
        let started_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(timing_error) => {
                return PlatformSendResult {
                    requested: len as u8,
                    inserted: 0,
                    started_ticks: QpcTicks::ZERO,
                    completed_ticks: None,
                    win32_error: 0,
                    timing_error: Some(timing_error),
                };
            }
        };

        // SAFETY: `packets[..len]` is fully initialized immediately above and
        // remains alive until this synchronous transport call returns.
        let (inserted, win32_error) =
            unsafe { send_prepared_inputs_once(packets.as_ptr(), len as u8) };
        let (completed_ticks, timing_error) = match clock.now() {
            Ok(ticks) => (Some(ticks), None),
            Err(error) => (None, Some(error)),
        };

        PlatformSendResult {
            requested: len as u8,
            inserted,
            started_ticks,
            completed_ticks,
            win32_error,
            timing_error,
        }
    }
    #[cfg(not(windows))]
    {
        let (started_ticks, completed_ticks, timing_error) =
            no_syscall_boundary_with_clock(Some(clock));
        PlatformSendResult {
            requested: scan_codes.len() as u8,
            inserted: scan_codes.len() as u8,
            started_ticks,
            completed_ticks,
            win32_error: 0,
            timing_error,
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn clears_last_error_before_authoritative_start_sample() {
        let source = include_str!("raw.rs").replace("\r\n", "\n");
        let sender = source
            .split("pub(crate) fn send_input_raw_with_clock")
            .nth(1)
            .expect("raw sender implementation");
        let reset = sender
            .find("SetLastError(0)")
            .expect("raw sender last-error reset");
        let start = sender
            .find("let started_ticks = match clock.now()")
            .expect("raw sender authoritative start sample");
        assert!(reset < start);
        assert!(start < sender.find("send_prepared_inputs_once").unwrap());
        let shared_primitive = source
            .split("pub(crate) unsafe fn send_prepared_inputs_once")
            .nth(1)
            .expect("shared Win32 SendInput primitive");
        assert!(shared_primitive.contains("SendInput("));
    }
}

#[cfg(test)]
mod terminal_up_tests {
    use super::terminal_up_send_with;
    use crate::clock::{QpcError, QpcTicks};
    use std::cell::Cell;

    #[test]
    fn pre_call_qpc_failure_does_not_skip_terminal_up_send() {
        let sample_count = Cell::new(0);
        let send_count = Cell::new(0);
        let result = terminal_up_send_with(
            2,
            || {
                let sample = sample_count.get();
                sample_count.set(sample + 1);
                if sample == 0 {
                    Err(QpcError::CounterUnavailable)
                } else {
                    Ok(QpcTicks::from_raw(17))
                }
            },
            || {
                send_count.set(send_count.get() + 1);
                (2, 0)
            },
        );

        assert_eq!(send_count.get(), 1);
        assert_eq!(result.requested, 2);
        assert_eq!(result.inserted, 2);
        assert_eq!(result.started_ticks, None);
        assert_eq!(result.completed_ticks, Some(QpcTicks::from_raw(17)));
        assert_eq!(result.timing_error, Some(QpcError::CounterUnavailable));
    }

    #[test]
    fn post_send_qpc_failure_keeps_start_and_marks_completion_unavailable() {
        let sample_count = Cell::new(0);
        let send_count = Cell::new(0);
        let result = terminal_up_send_with(
            1,
            || {
                let sample = sample_count.get();
                sample_count.set(sample + 1);
                if sample == 0 {
                    Ok(QpcTicks::from_raw(23))
                } else {
                    Err(QpcError::CounterUnavailable)
                }
            },
            || {
                send_count.set(send_count.get() + 1);
                (1, 0)
            },
        );

        assert_eq!(send_count.get(), 1);
        assert_eq!(result.started_ticks, Some(QpcTicks::from_raw(23)));
        assert_eq!(result.completed_ticks, None);
        assert_eq!(result.timing_error, Some(QpcError::CounterUnavailable));
    }
}
