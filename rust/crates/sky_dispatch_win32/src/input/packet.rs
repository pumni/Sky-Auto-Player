use super::outcome::{PhysicalPacket, PhysicalSendOutcome};
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

#[cfg(windows)]
fn send_once(packet: PhysicalPacket, clock: QpcClock) -> PhysicalSendOutcome {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

    let requested = packet.event_count();
    let (inputs, length) = build_inputs(packet);
    let started_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: super::outcome::PacketClockFailurePhase::BeforeSend,
                send_was_called: false,
                inserted_count: None,
                started_ticks: None,
                error,
            };
        }
    };
    unsafe { SetLastError(0) };
    let inserted = unsafe {
        SendInput(
            length as u32,
            inputs.as_ptr(),
            std::mem::size_of::<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>() as i32,
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
        Err(error) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: super::outcome::PacketClockFailurePhase::AfterSend,
                send_was_called: true,
                inserted_count: Some(inserted),
                started_ticks: Some(started_ticks),
                error,
            };
        }
    };
    outcome_from_counts(
        requested,
        inserted,
        1,
        error,
        started_ticks,
        completed_ticks,
    )
}

#[cfg(not(windows))]
fn send_once(packet: PhysicalPacket, clock: QpcClock) -> PhysicalSendOutcome {
    let requested = packet.event_count();
    let started_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: super::outcome::PacketClockFailurePhase::BeforeSend,
                send_was_called: false,
                inserted_count: None,
                started_ticks: None,
                error,
            };
        }
    };
    let completed_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return PhysicalSendOutcome::ClockFailure {
                phase: super::outcome::PacketClockFailurePhase::AfterSend,
                send_was_called: false,
                inserted_count: Some(requested),
                started_ticks: Some(started_ticks),
                error,
            };
        }
    };
    PhysicalSendOutcome::Complete {
        requested,
        inserted: requested,
        attempts: 1,
        started_ticks,
        completed_ticks,
    }
}

fn outcome_from_counts(
    requested: u8,
    inserted: u8,
    attempts: u8,
    error: u32,
    started_ticks: QpcTicks,
    completed_ticks: QpcTicks,
) -> PhysicalSendOutcome {
    if inserted >= requested {
        PhysicalSendOutcome::Complete {
            requested,
            inserted,
            attempts,
            started_ticks,
            completed_ticks,
        }
    } else if inserted == 0 {
        PhysicalSendOutcome::ZeroProgress {
            requested,
            attempts,
            first_error: error,
            last_error: error,
            started_ticks,
            completed_ticks,
        }
    } else {
        PhysicalSendOutcome::Partial {
            requested,
            inserted_count: inserted,
            attempts,
            first_error: error,
            last_error: error,
            started_ticks,
            completed_ticks,
        }
    }
}

pub fn send_physical_packet_with_clock(
    packet: PhysicalPacket,
    clock: QpcClock,
) -> PhysicalSendOutcome {
    if !valid_packet(packet) || packet.event_count() == 0 {
        return PhysicalSendOutcome::ZeroProgress {
            requested: packet.event_count(),
            attempts: 0,
            first_error: 87,
            last_error: 87,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: QpcTicks::ZERO,
        };
    }

    let first = send_once(packet, clock);
    let should_retry = (packet.is_up_only()
        && matches!(
            first,
            PhysicalSendOutcome::ZeroProgress { .. } | PhysicalSendOutcome::Partial { .. }
        ))
        || matches!(first, PhysicalSendOutcome::ZeroProgress { .. });
    if !should_retry {
        return first;
    }
    let second = send_once(packet, clock);
    match second {
        PhysicalSendOutcome::Complete {
            requested,
            inserted,
            started_ticks: _,
            completed_ticks,
            ..
        } => PhysicalSendOutcome::Complete {
            requested,
            inserted,
            attempts: 2,
            started_ticks: match first {
                PhysicalSendOutcome::Complete { started_ticks, .. }
                | PhysicalSendOutcome::ZeroProgress { started_ticks, .. }
                | PhysicalSendOutcome::Partial { started_ticks, .. } => started_ticks,
                PhysicalSendOutcome::ClockFailure { .. } => {
                    unreachable!("clock failures are never retried")
                }
            },
            completed_ticks,
        },
        PhysicalSendOutcome::ZeroProgress {
            requested,
            first_error,
            last_error,
            started_ticks: _,
            completed_ticks,
            ..
        } => PhysicalSendOutcome::ZeroProgress {
            requested,
            attempts: 2,
            first_error: match first {
                PhysicalSendOutcome::ZeroProgress { first_error, .. }
                | PhysicalSendOutcome::Partial { first_error, .. } => first_error,
                PhysicalSendOutcome::Complete { .. } => first_error,
                PhysicalSendOutcome::ClockFailure { .. } => {
                    unreachable!("clock failures are never retried")
                }
            },
            last_error,
            started_ticks: match first {
                PhysicalSendOutcome::Complete { started_ticks, .. }
                | PhysicalSendOutcome::ZeroProgress { started_ticks, .. }
                | PhysicalSendOutcome::Partial { started_ticks, .. } => started_ticks,
                PhysicalSendOutcome::ClockFailure { .. } => {
                    unreachable!("clock failures are never retried")
                }
            },
            completed_ticks,
        },
        PhysicalSendOutcome::Partial {
            requested,
            inserted_count,
            first_error: second_first_error,
            last_error,
            completed_ticks,
            ..
        } => PhysicalSendOutcome::Partial {
            requested,
            inserted_count,
            attempts: 2,
            first_error: match first {
                PhysicalSendOutcome::Complete { .. } => second_first_error,
                PhysicalSendOutcome::ZeroProgress { first_error, .. }
                | PhysicalSendOutcome::Partial { first_error, .. } => first_error,
                PhysicalSendOutcome::ClockFailure { .. } => {
                    unreachable!("clock failures are never retried")
                }
            },
            last_error,
            started_ticks: match first {
                PhysicalSendOutcome::Complete { started_ticks, .. }
                | PhysicalSendOutcome::ZeroProgress { started_ticks, .. }
                | PhysicalSendOutcome::Partial { started_ticks, .. } => started_ticks,
                PhysicalSendOutcome::ClockFailure { .. } => {
                    unreachable!("clock failures are never retried")
                }
            },
            completed_ticks,
        },
        PhysicalSendOutcome::ClockFailure { .. } => second,
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
    fn outcome_classification_covers_every_inserted_count() {
        for inserted in 0..=11 {
            let outcome = outcome_from_counts(
                11,
                inserted,
                1,
                5,
                QpcTicks::from_raw(10),
                QpcTicks::from_raw(20),
            );
            match (inserted, outcome) {
                (0, PhysicalSendOutcome::ZeroProgress { .. })
                | (11, PhysicalSendOutcome::Complete { .. })
                | (1..=10, PhysicalSendOutcome::Partial { .. }) => {}
                (count, other) => panic!("unexpected outcome for inserted={count}: {other:?}"),
            }
        }
    }

    #[test]
    fn invalid_mask_fails_before_any_send() {
        let clock = QpcClock::initialize().expect("QPC available for test");
        let outcome = send_physical_packet_with_clock(
            PhysicalPacket::new(FULL_INSTRUMENT_MASK | (1 << 15), 0),
            clock,
        );
        assert!(matches!(
            outcome,
            PhysicalSendOutcome::ZeroProgress {
                attempts: 0,
                first_error: 87,
                ..
            }
        ));
    }

    #[cfg(windows)]
    #[test]
    fn input_builder_places_all_up_events_before_down_events() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP;

        let (inputs, len) = build_inputs(PhysicalPacket::new(0b1, 0b10));
        assert_eq!(len, 2);
        // `INPUT::Anonymous` is a Windows SDK union; the builder above
        // initializes its keyboard arm for every populated slot.
        unsafe {
            assert_eq!(inputs[0].Anonymous.ki.wScan, 0x15);
            assert_ne!(inputs[0].Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
            assert_eq!(inputs[1].Anonymous.ki.wScan, 0x16);
            assert_eq!(inputs[1].Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
        }
    }
}
