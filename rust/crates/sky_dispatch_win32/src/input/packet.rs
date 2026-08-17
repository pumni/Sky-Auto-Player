use super::outcome::{
    PacketPreparationError, PacketRetryReason, PhysicalPacket, PlatformSendResult, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, classify_send_status,
};
use super::scan_code::{
    FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE,
};
use crate::clock::{QpcClock, QpcTicks};

pub const MAX_PACKET_EVENTS: usize = 30;

/// Fixed-capacity physical work prepared before the precision boundary.
///
/// The Win32 payload is intentionally opaque to the scheduler/core crates.
/// Production callers retain this value until the single `SendInput` call.
pub struct PreparedPhysicalPacket {
    packet: PhysicalPacket,
    length: u8,
    #[cfg(windows)]
    inputs: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; MAX_PACKET_EVENTS],
}

impl std::fmt::Debug for PreparedPhysicalPacket {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedPhysicalPacket")
            .field("packet", &self.packet)
            .field("length", &self.length)
            .finish()
    }
}

impl PreparedPhysicalPacket {
    pub fn try_new(packet: PhysicalPacket) -> Result<Self, PacketPreparationError> {
        if !valid_packet(packet) {
            return Err(PacketPreparationError::InvalidMask);
        }
        if packet.event_count() == 0 {
            return Err(PacketPreparationError::Empty);
        }
        #[cfg(windows)]
        let (inputs, length) = build_inputs(packet);
        #[cfg(not(windows))]
        let length = packet.event_count() as usize;
        Ok(Self {
            packet,
            length: length as u8,
            #[cfg(windows)]
            inputs,
        })
    }

    #[inline]
    pub fn packet(&self) -> PhysicalPacket {
        self.packet
    }

    #[inline]
    pub fn event_count(&self) -> u8 {
        self.length
    }
}

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
    supplied_started_ticks: Option<QpcTicks>,
) -> Result<PlatformSendResult, (Option<QpcTicks>, crate::clock::QpcError, bool)> {
    let prepared = PreparedPhysicalPacket::try_new(packet)
        .expect("send_once receives a validated physical packet");
    send_once_prepared(&prepared, clock, supplied_started_ticks)
}

fn send_once_prepared(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    supplied_started_ticks: Option<QpcTicks>,
) -> Result<PlatformSendResult, (Option<QpcTicks>, crate::clock::QpcError, bool)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

        let requested = prepared.event_count();
        let length = usize::from(prepared.event_count());
        let started_ticks = match supplied_started_ticks {
            Some(ticks) => ticks,
            None => match clock.now() {
                Ok(ticks) => ticks,
                Err(error) => return Err((None, error, false)),
            },
        };
        let inserted = unsafe {
            SendInput(
                length as u32,
                prepared.inputs.as_ptr(),
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
        let requested = prepared.event_count();
        let started_ticks = match supplied_started_ticks {
            Some(ticks) => ticks,
            None => match clock.now() {
                Ok(ticks) => ticks,
                Err(error) => return Err((None, error, false)),
            },
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

/// One low-level `SendInput`/QPC attempt for a physical packet. Kept as a
/// first-class value so the retry policy is testable through an injectable
/// seam without invoking the Win32 syscall.
#[allow(dead_code)]
enum PacketSendAttempt {
    Outcome(PlatformSendResult),
    ClockFailure(Option<QpcTicks>, crate::clock::QpcError, bool),
}

fn run_send_attempt(
    packet: PhysicalPacket,
    clock: QpcClock,
    supplied_started_ticks: Option<QpcTicks>,
) -> PacketSendAttempt {
    match send_once(packet, clock, supplied_started_ticks) {
        Ok(res) => PacketSendAttempt::Outcome(res),
        Err((start, err, called)) => PacketSendAttempt::ClockFailure(start, err, called),
    }
}

fn run_prepared_send_attempt(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    supplied_started_ticks: Option<QpcTicks>,
) -> PacketSendAttempt {
    match send_once_prepared(prepared, clock, supplied_started_ticks) {
        Ok(res) => PacketSendAttempt::Outcome(res),
        Err((start, err, called)) => PacketSendAttempt::ClockFailure(start, err, called),
    }
}

/// One physical packet attempt with no retry policy.
fn send_physical_packet_once_impl(
    packet: PhysicalPacket,
    mut send_one: impl FnMut(PhysicalPacket) -> PacketSendAttempt,
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

    let first = match send_one(packet) {
        PacketSendAttempt::Outcome(res) => res,
        PacketSendAttempt::ClockFailure(start, err, called) => {
            return SendTransactionOutcome {
                status: if called {
                    SendTransactionStatus::ClockFailureAfterSend
                } else {
                    SendTransactionStatus::ClockFailureBeforeSend
                },
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: u8::from(called),
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
    let requested = usize::from(first.requested);
    let inserted = usize::from(first.inserted).min(requested);
    let status = classify_send_status(
        inserted,
        requested,
        first.win32_error,
        Some(first.started_ticks),
        first.completed_ticks,
    );
    let win32_error = (first.win32_error != 0).then_some(first.win32_error);
    let retry_reason = if inserted == 0 && !matches!(status, SendTransactionStatus::Complete) {
        PacketRetryReason::ZeroProgress
    } else if inserted < requested {
        PacketRetryReason::PartialProgress {
            inserted_count: inserted as u8,
        }
    } else {
        PacketRetryReason::None
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
            first_inserted: inserted as u8,
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason,
            first_win32_error: win32_error,
            last_win32_error: win32_error,
            started_ticks: Some(first.started_ticks),
            completed_ticks: first.completed_ticks,
            timing_error: first.timing_error,
        },
    }
}

/// Test-only retry policy retained to exercise the fail-closed transport
/// matrix. Production callers use `send_physical_packet_once_with_clock`; the
/// worker/recovery state machines own any retry decision.
#[cfg(test)]
fn send_physical_packet_retry_policy_impl(
    packet: PhysicalPacket,
    mut send_one: impl FnMut(PhysicalPacket) -> PacketSendAttempt,
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

    let first = match send_one(packet) {
        PacketSendAttempt::Outcome(res) => res,
        PacketSendAttempt::ClockFailure(start, err, called) => {
            return SendTransactionOutcome {
                status: if called {
                    SendTransactionStatus::ClockFailureAfterSend
                } else {
                    SendTransactionStatus::ClockFailureBeforeSend
                },
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: u8::from(called),
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
    let status1 = classify_send_status(
        first.inserted as usize,
        first.requested as usize,
        first.win32_error,
        Some(first.started_ticks),
        first.completed_ticks,
    );

    if first.inserted == first.requested {
        return SendTransactionOutcome {
            status: status1,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: if matches!(status1, SendTransactionStatus::Complete) {
                    requested_mask
                } else {
                    0
                },
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
    let up_only = packet.is_up_only();

    // A partial insertion of a Down/Mixed packet has already split the chord.
    // Never issue a second whole-packet send: the Down that landed would be
    // duplicated and the physical stream would violate chord integrity.
    if !up_only && first.inserted > 0 {
        let status = if first.completed_ticks.is_none() {
            SendTransactionStatus::ClockFailureAfterSend
        } else {
            SendTransactionStatus::IntegrityLost
        };
        return SendTransactionOutcome {
            status,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: 0,
                skipped_mask: 0,
                first_inserted: first.inserted,
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason,
                first_win32_error: first_win32,
                last_win32_error: first_win32,
                started_ticks: Some(first.started_ticks),
                completed_ticks: first.completed_ticks,
                timing_error: None,
            },
        };
    }

    // Safe retry: UpOnly (partial or zero) is idempotent; a Down/Mixed packet
    // is only retried after guaranteed zero progress (no chord was split).
    let second = match send_one(packet) {
        PacketSendAttempt::Outcome(res) => res,
        PacketSendAttempt::ClockFailure(start, err, called) => {
            return SendTransactionOutcome {
                status: SendTransactionStatus::ClockFailureAfterSend,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: first.inserted,
                    attempts: u8::from(called).saturating_add(1),
                    zero_progress_retries: u8::from(first.inserted == 0),
                    retry_reason,
                    first_win32_error: first_win32,
                    last_win32_error: first_win32,
                    started_ticks: start,
                    completed_ticks: None,
                    timing_error: Some(err),
                },
            };
        }
    };
    let second_win32 = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32 = second_win32.or(first_win32);
    let status2 = classify_send_status(
        second.inserted as usize,
        second.requested as usize,
        second.win32_error,
        Some(first.started_ticks),
        second.completed_ticks,
    );

    SendTransactionOutcome {
        status: status2,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: if matches!(status2, SendTransactionStatus::Complete) {
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

pub fn send_physical_packet_once_with_clock(
    packet: PhysicalPacket,
    clock: QpcClock,
) -> SendTransactionOutcome {
    send_physical_packet_once_impl(packet, |packet| run_send_attempt(packet, clock, None))
}

/// One packet transaction using a start boundary sampled by the caller after
/// all control, focus, target, and lease gates have passed.
pub fn send_physical_packet_once_with_start(
    packet: PhysicalPacket,
    clock: QpcClock,
    started_ticks: QpcTicks,
) -> SendTransactionOutcome {
    send_physical_packet_once_impl(packet, |packet| {
        run_send_attempt(packet, clock, Some(started_ticks))
    })
}

/// One packet transaction using a payload built before the final admission
/// boundary and a caller-owned authoritative start timestamp.
pub fn send_prepared_physical_packet_once_with_start(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    started_ticks: QpcTicks,
) -> SendTransactionOutcome {
    send_physical_packet_once_impl(prepared.packet(), |_| {
        run_prepared_send_attempt(prepared, clock, Some(started_ticks))
    })
}

#[cfg(test)]
fn send_physical_packet_retry_policy_scripted(
    packet: PhysicalPacket,
    mut send_one: impl FnMut(PhysicalPacket) -> PlatformSendResult,
) -> SendTransactionOutcome {
    send_physical_packet_retry_policy_impl(packet, |packet| {
        PacketSendAttempt::Outcome(send_one(packet))
    })
}

#[cfg(test)]
fn send_physical_packet_once_scripted(
    packet: PhysicalPacket,
    mut send_one: impl FnMut(PhysicalPacket) -> PlatformSendResult,
) -> SendTransactionOutcome {
    send_physical_packet_once_impl(packet, |packet| {
        PacketSendAttempt::Outcome(send_one(packet))
    })
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
        let outcome = send_physical_packet_once_with_clock(
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

    fn scripted_attempt(requested: u8, inserted: u8) -> PlatformSendResult {
        PlatformSendResult {
            requested,
            inserted,
            started_ticks: QpcTicks::ZERO,
            completed_ticks: Some(QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    }

    #[test]
    fn partial_down_packet_never_issues_a_second_send() {
        let mut calls = 0;
        let outcome =
            send_physical_packet_retry_policy_scripted(PhysicalPacket::new(0, 0b111), |_| {
                calls += 1;
                // A hypothetical second call would be a full success; it must
                // never happen for a Down-bearing partial insertion.
                scripted_attempt(3, if calls == 1 { 1 } else { 3 })
            });
        assert_eq!(calls, 1);
        assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
        assert_eq!(outcome.evidence.first_inserted, 1);
        assert_eq!(outcome.evidence.attempts, 1);
        assert_eq!(outcome.evidence.confirmed_mask, 0);
        assert!(!outcome.is_success());
    }

    #[test]
    fn partial_mixed_packet_never_issues_a_second_packet_call() {
        let mut calls = 0;
        let outcome =
            send_physical_packet_retry_policy_scripted(PhysicalPacket::new(0b001, 0b110), |_| {
                calls += 1;
                scripted_attempt(3, 2)
            });
        assert_eq!(calls, 1);
        assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
        assert_eq!(outcome.evidence.first_inserted, 2);
        assert_eq!(outcome.evidence.attempts, 1);
        assert_eq!(outcome.evidence.confirmed_mask, 0);
    }

    #[test]
    fn every_mixed_insertion_prefix_fails_closed_without_retry() {
        let packet = PhysicalPacket::new(FULL_INSTRUMENT_MASK, FULL_INSTRUMENT_MASK);
        let requested = packet.event_count();
        assert_eq!(requested, MAX_PACKET_EVENTS as u8);

        for inserted in 1..requested {
            let mut calls = 0;
            let outcome = send_physical_packet_retry_policy_scripted(packet, |_| {
                calls += 1;
                scripted_attempt(requested, inserted)
            });

            assert_eq!(calls, 1, "partial prefix {inserted} was retried");
            assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
            assert_eq!(outcome.evidence.first_inserted, inserted);
            assert_eq!(outcome.evidence.attempts, 1);
            assert_eq!(outcome.evidence.confirmed_mask, 0);
            assert_eq!(outcome.evidence.requested_mask, FULL_INSTRUMENT_MASK);
        }

        let mut calls = 0;
        let complete = send_physical_packet_retry_policy_scripted(packet, |_| {
            calls += 1;
            scripted_attempt(requested, requested)
        });
        assert_eq!(calls, 1);
        assert_eq!(complete.status, SendTransactionStatus::Complete);
        assert_eq!(complete.evidence.confirmed_mask, FULL_INSTRUMENT_MASK);
    }

    #[test]
    fn single_attempt_packet_does_not_retry_zero_progress() {
        let mut calls = 0;
        let outcome = send_physical_packet_once_scripted(PhysicalPacket::new(0, 0b111), |_| {
            calls += 1;
            scripted_attempt(3, 0)
        });
        assert_eq!(calls, 1);
        assert_eq!(outcome.status, SendTransactionStatus::ZeroProgress);
        assert_eq!(outcome.evidence.attempts, 1);
        assert_eq!(outcome.evidence.zero_progress_retries, 0);
        assert_eq!(
            outcome.evidence.retry_reason,
            PacketRetryReason::ZeroProgress
        );
    }

    #[test]
    fn up_only_partial_packet_retries_and_can_complete() {
        let mut calls = 0;
        let outcome =
            send_physical_packet_retry_policy_scripted(PhysicalPacket::new(0b111, 0), |_| {
                calls += 1;
                scripted_attempt(3, if calls == 1 { 1 } else { 3 })
            });
        assert_eq!(calls, 2);
        assert_eq!(outcome.status, SendTransactionStatus::Complete);
        assert_eq!(outcome.evidence.attempts, 2);
        assert_eq!(outcome.evidence.first_inserted, 1);
    }

    #[test]
    fn down_zero_progress_retries_whole_packet_without_splitting() {
        let mut calls = 0;
        let outcome =
            send_physical_packet_retry_policy_scripted(PhysicalPacket::new(0, 0b111), |_| {
                calls += 1;
                scripted_attempt(3, if calls == 1 { 0 } else { 3 })
            });
        assert_eq!(calls, 2);
        assert_eq!(outcome.status, SendTransactionStatus::Complete);
        assert_eq!(outcome.evidence.zero_progress_retries, 1);
        assert_eq!(outcome.evidence.attempts, 2);
    }

    #[test]
    fn down_zero_progress_then_partial_second_is_integrity_lost() {
        let mut calls = 0;
        let outcome =
            send_physical_packet_retry_policy_scripted(PhysicalPacket::new(0, 0b111), |_| {
                calls += 1;
                scripted_attempt(3, if calls == 1 { 0 } else { 1 })
            });
        assert_eq!(calls, 2);
        assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
        assert_eq!(outcome.evidence.first_inserted, 0);
        assert_eq!(outcome.evidence.zero_progress_retries, 1);
    }

    #[cfg(not(windows))]
    #[test]
    fn physical_packet_send_seam_compiles_and_returns_complete_for_valid_packet() {
        let clock = QpcClock::initialize().expect("clock");
        let packet = PhysicalPacket::new(1, 0);
        let res = send_physical_packet_once_with_clock(packet, clock);
        assert_eq!(res.status, SendTransactionStatus::Complete);
    }
}
