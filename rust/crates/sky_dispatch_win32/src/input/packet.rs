use super::outcome::{
    PacketRetryReason, PhysicalPacket, PlatformSendResult, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus,
};
use super::scan_code::{
    FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE,
};
use crate::clock::{QpcClock, QpcTicks};

pub const MAX_PACKET_EVENTS: usize = 30;

#[cfg(windows)]
const MAX_SCAN_CODE: usize = 0x36;

#[cfg(windows)]
const fn create_keyboard_input(
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
const DOWN_TEMPLATES: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_SCAN_CODE] = {
    let mut arr = [create_keyboard_input(0, false); MAX_SCAN_CODE];
    let mut index = 0;
    while index < MAX_SCAN_CODE {
        arr[index] = create_keyboard_input(index as u16, false);
        index += 1;
    }
    arr
};

#[cfg(windows)]
const UP_TEMPLATES: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_SCAN_CODE] = {
    let mut arr = [create_keyboard_input(0, true); MAX_SCAN_CODE];
    let mut index = 0;
    while index < MAX_SCAN_CODE {
        arr[index] = create_keyboard_input(index as u16, true);
        index += 1;
    }
    arr
};

#[inline]
fn valid_packet(packet: PhysicalPacket) -> bool {
    packet.up_mask & !FULL_INSTRUMENT_MASK == 0
        && packet.down_mask & !FULL_INSTRUMENT_MASK == 0
        && usize::from(packet.event_count()) <= MAX_PACKET_EVENTS
}

#[cfg(windows)]
fn build_inputs(
    packet: PhysicalPacket,
) -> (
    [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_PACKET_EVENTS],
    usize,
) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT;

    let mut inputs: [INPUT; MAX_PACKET_EVENTS] = unsafe { std::mem::zeroed() };
    let mut length = 0usize;
    let mut append_mask = |mut mask: u16, templates: &[INPUT; MAX_SCAN_CODE]| {
        while mask != 0 {
            let slot = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            inputs[length] = templates[PHYSICAL_INSTRUMENT_SCAN_CODES[slot] as usize];
            length += 1;
        }
    };
    append_mask(packet.up_mask, &UP_TEMPLATES);
    append_mask(packet.down_mask, &DOWN_TEMPLATES);
    (inputs, length)
}

fn send_once(
    packet: PhysicalPacket,
    clock: QpcClock,
) -> Result<PlatformSendResult, (Option<QpcTicks>, crate::clock::QpcError, bool)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

        let requested = packet.event_count();
        let (inputs, length) = build_inputs(packet);
        let started_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => return Err((None, error, false)),
        };
        unsafe { SetLastError(0) };
        let inserted = unsafe {
            SendInput(
                length as u32,
                inputs.as_ptr(),
                std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>()
                    as i32,
            )
        }
        .min(length as u32) as u8;
        let error = if usize::from(inserted) < length {
            unsafe { GetLastError() }
        } else {
            0
        };
        let completed_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => return Err((Some(started_ticks), error, true)),
        };
        Ok(PlatformSendResult {
            requested,
            inserted,
            started_ticks,
            completed_ticks: Some(completed_ticks),
            win32_error: error,
            timing_error: None,
        })
    }
    #[cfg(not(windows))]
    {
        let requested = packet.event_count();
        let started_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => return Err((None, error, false)),
        };
        let completed_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => return Err((Some(started_ticks), error, false)),
        };
        Ok(PlatformSendResult {
            requested,
            inserted: requested,
            started_ticks,
            completed_ticks: Some(completed_ticks),
            win32_error: 0,
            timing_error: None,
        })
    }
}

pub fn send_physical_packet_with_clock(
    packet: PhysicalPacket,
    clock: QpcClock,
) -> SendTransactionOutcome {
    let requested_mask = packet.up_mask | packet.down_mask;
    if !valid_packet(packet) || packet.event_count() == 0 {
        return SendTransactionOutcome {
            status: SendTransactionStatus::ZeroProgress,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: 0,
                attempts: 0,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: Some(87),
                last_win32_error: Some(87),
                started_ticks: Some(QpcTicks::ZERO),
                completed_ticks: Some(QpcTicks::ZERO),
                timing_error: None,
            },
        };
    }

    let first = match send_once(packet, clock) {
        Ok(res) => res,
        Err((start, err, send_called)) => {
            return SendTransactionOutcome {
                status: if send_called {
                    SendTransactionStatus::ClockFailureAfterSend
                } else {
                    SendTransactionStatus::ClockFailureBeforeSend
                },
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: if send_called { 1 } else { 0 },
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: start,
                    completed_ticks: None,
                    timing_error: Some(err),
                },
            };
        }
    };

    let first_win32 = (first.win32_error != 0).then_some(first.win32_error);

    if first.inserted == first.requested {
        return SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: requested_mask,
                skipped_mask: 0,
                first_inserted: first.inserted,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: first_win32,
                last_win32_error: first_win32,
                started_ticks: Some(first.started_ticks),
                completed_ticks: first.completed_ticks,
                timing_error: None,
            },
        };
    }

    let retry_reason = if first.inserted == 0 {
        PacketRetryReason::ZeroProgress
    } else {
        PacketRetryReason::PartialProgress {
            inserted_count: first.inserted,
        }
    };

    let second = match send_once(packet, clock) {
        Ok(res) => res,
        Err((_start, err, _send_called)) => {
            return SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: first.inserted,
                    attempts: 1,
                    zero_progress_retries: u8::from(first.inserted == 0),
                    retry_reason,
                    first_win32_error: first_win32,
                    last_win32_error: first_win32,
                    started_ticks: Some(first.started_ticks),
                    completed_ticks: first.completed_ticks,
                    timing_error: Some(err),
                },
            };
        }
    };

    let second_win32 = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32 = second_win32.or(first_win32);

    let status = if second.inserted == second.requested {
        SendTransactionStatus::Complete
    } else if first.inserted == 0 && second.inserted == 0 {
        SendTransactionStatus::ZeroProgress
    } else if !packet.is_up_only() && (first.inserted > 0 || second.inserted > 0) {
        SendTransactionStatus::IntegrityLost
    } else {
        SendTransactionStatus::PartialProgress
    };

    SendTransactionOutcome {
        status,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: if matches!(status, SendTransactionStatus::Complete) {
                requested_mask
            } else {
                0
            },
            skipped_mask: 0,
            first_inserted: first.inserted,
            attempts: 2,
            zero_progress_retries: u8::from(first.inserted == 0),
            retry_reason,
            first_win32_error: first_win32.or(second_win32),
            last_win32_error: last_win32,
            started_ticks: Some(first.started_ticks),
            completed_ticks: second.completed_ticks,
            timing_error: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_packet_accepts_at_most_thirty_events() {
        let packet = PhysicalPacket::new(FULL_INSTRUMENT_MASK, FULL_INSTRUMENT_MASK);
        assert_eq!(packet.event_count(), MAX_PACKET_EVENTS as u8);
    }

    #[test]
    fn invalid_mask_fails_before_any_send() {
        let clock = QpcClock::initialize().expect("QPC available for test");
        let outcome = send_physical_packet_with_clock(
            PhysicalPacket::new(FULL_INSTRUMENT_MASK | (1 << 15), 0),
            clock,
        );
        assert_eq!(outcome.status, SendTransactionStatus::ZeroProgress);
        assert_eq!(outcome.evidence.attempts, 0);
        assert_eq!(outcome.evidence.first_win32_error, Some(87));
    }

    #[cfg(windows)]
    #[test]
    fn input_builder_places_all_up_events_before_down_events() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP;

        let (inputs, len) = build_inputs(PhysicalPacket::new(0b1, 0b10));
        assert_eq!(len, 2);
        unsafe {
            assert_eq!(inputs[0].Anonymous.ki.wScan, 0x15);
            assert_ne!(inputs[0].Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
            assert_eq!(inputs[1].Anonymous.ki.wScan, 0x16);
            assert_eq!(inputs[1].Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
        }
    }
}
