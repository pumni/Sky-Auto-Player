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
    match send_once_prepared(&prepared, clock, supplied_started_ticks, None) {
        Ok(result) => Ok(result),
        Err(PreparedSendFailure::Clock(start, error, called)) => Err((start, error, called)),
        Err(PreparedSendFailure::DeadlineMissed { .. }) => {
            unreachable!("prepared send cutoff is disabled for generic packets")
        }
    }
}

enum PreparedSendFailure {
    Clock(Option<QpcTicks>, crate::clock::QpcError, bool),
    DeadlineMissed { started_ticks: QpcTicks },
}

#[cfg(windows)]
struct PreparedInputView {
    requested: u8,
    length: usize,
    inputs: *const windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT,
    cb_size: i32,
}

/// Run the shared target-aware QPC/SendInput envelope for a fixed INPUT view.
///
/// The view is constructed by a trusted crate-internal caller before entering
/// this function.  Keeping this envelope shared lets the calibration packet
/// carry its correlation tag without creating a second timing implementation.
#[cfg(windows)]
fn send_input_view_at_target(
    view: PreparedInputView,
    clock: QpcClock,
    physical_target_qpc: QpcTicks,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> Result<PlatformSendResult, PreparedSendFailure> {
    use windows_sys::Win32::Foundation::{GetLastError, SetLastError};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SendInput;

    // Last-error state is preparation; it must not sit between the crossing
    // sample and SendInput.
    unsafe { SetLastError(0) };
    let started_ticks = loop {
        let ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(None, error, false));
            }
        };
        if ticks >= physical_target_qpc {
            break ticks;
        }
        std::hint::spin_loop();
    };

    if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
        return Err(PreparedSendFailure::DeadlineMissed { started_ticks });
    }

    let inserted = unsafe { SendInput(view.length as u32, view.inputs, view.cb_size) }
        .min(view.length as u32) as u8;
    let win32_error = if usize::from(inserted) < view.length {
        unsafe { GetLastError() }
    } else {
        0
    };
    let completed_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(PreparedSendFailure::Clock(Some(started_ticks), error, true));
        }
    };

    Ok(PlatformSendResult {
        requested: view.requested,
        inserted,
        started_ticks,
        completed_ticks: Some(completed_ticks),
        win32_error,
        timing_error: None,
    })
}

#[cfg(not(windows))]
fn send_input_view_at_target(
    requested: u8,
    clock: QpcClock,
    physical_target_qpc: QpcTicks,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> Result<PlatformSendResult, PreparedSendFailure> {
    let started_ticks = loop {
        let ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(None, error, false));
            }
        };
        if ticks >= physical_target_qpc {
            break ticks;
        }
        std::hint::spin_loop();
    };
    if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
        return Err(PreparedSendFailure::DeadlineMissed { started_ticks });
    }
    let completed_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(PreparedSendFailure::Clock(
                Some(started_ticks),
                error,
                false,
            ));
        }
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

fn send_once_prepared(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    supplied_started_ticks: Option<QpcTicks>,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> Result<PlatformSendResult, PreparedSendFailure> {
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
                Err(error) => return Err(PreparedSendFailure::Clock(None, error, false)),
            },
        };
        if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
            return Err(PreparedSendFailure::DeadlineMissed { started_ticks });
        }
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
            Err(error) => {
                return Err(PreparedSendFailure::Clock(Some(started_ticks), error, true));
            }
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
                Err(error) => return Err(PreparedSendFailure::Clock(None, error, false)),
            },
        };
        if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
            return Err(PreparedSendFailure::DeadlineMissed { started_ticks });
        }
        let completed_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(
                    Some(started_ticks),
                    error,
                    false,
                ));
            }
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
    DeadlineMissed(QpcTicks),
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
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> PacketSendAttempt {
    match send_once_prepared(
        prepared,
        clock,
        supplied_started_ticks,
        latest_allowed_down_qpc,
    ) {
        Ok(res) => PacketSendAttempt::Outcome(res),
        Err(PreparedSendFailure::Clock(start, err, called)) => {
            PacketSendAttempt::ClockFailure(start, err, called)
        }
        Err(PreparedSendFailure::DeadlineMissed { started_ticks }) => {
            PacketSendAttempt::DeadlineMissed(started_ticks)
        }
    }
}

fn prepared_send_outcome(
    packet: PhysicalPacket,
    first: Result<PlatformSendResult, PreparedSendFailure>,
) -> SendTransactionOutcome {
    let requested_mask = packet.up_mask | packet.down_mask;
    let first = match first {
        Ok(res) => res,
        Err(PreparedSendFailure::DeadlineMissed { started_ticks }) => {
            return SendTransactionOutcome {
                status: SendTransactionStatus::DeadlineMissedBeforeSend,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks: None,
                    timing_error: None,
                },
            };
        }
        Err(PreparedSendFailure::Clock(start, error, called)) => {
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
                    timing_error: Some(error),
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
        PacketSendAttempt::DeadlineMissed(_) => {
            unreachable!("generic packet send has no prepared-packet cutoff")
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

/// Trusted fast path for a packet that was validated and materialized before
/// the precision boundary. This deliberately does not inspect the packet
/// masks or recount events after the caller's final QPC sample.
///
/// In the production path `started_ticks` is `None`, so the authoritative
/// `pre_call_qpc` is sampled here, after the prepared payload length and
/// pointer have been resolved and immediately before `SendInput`. Test
/// support may provide a controlled timestamp without changing production
/// behavior.
fn send_prepared_physical_packet_once_impl(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    started_ticks: Option<QpcTicks>,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    let first =
        match run_prepared_send_attempt(prepared, clock, started_ticks, latest_allowed_down_qpc) {
            PacketSendAttempt::Outcome(res) => Ok(res),
            PacketSendAttempt::DeadlineMissed(started_ticks) => {
                Err(PreparedSendFailure::DeadlineMissed { started_ticks })
            }
            PacketSendAttempt::ClockFailure(start, error, called) => {
                Err(PreparedSendFailure::Clock(start, error, called))
            }
        };
    prepared_send_outcome(packet, first)
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
        PacketSendAttempt::DeadlineMissed(_) => {
            unreachable!("retry policy has no prepared-packet cutoff")
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
        PacketSendAttempt::DeadlineMissed(_) => {
            unreachable!("retry policy has no prepared-packet cutoff")
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
    send_prepared_physical_packet_once_impl(prepared, clock, Some(started_ticks), None)
}

/// One trusted prepared packet attempt using a caller-supplied start boundary
/// and a Down-only hard-late cutoff. The cutoff is checked against the same
/// start timestamp that is used as the packet's authoritative pre-call
/// evidence, before the Win32 syscall.
pub fn send_prepared_physical_packet_once_with_start_and_cutoff(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    started_ticks: QpcTicks,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(
        prepared,
        clock,
        Some(started_ticks),
        latest_allowed_down_qpc,
    )
}

/// One trusted prepared packet attempt whose authoritative pre-call QPC is
/// sampled inside the Win32 sender immediately before `SendInput`.
pub fn send_prepared_physical_packet_once(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(prepared, clock, None, None)
}

/// One trusted prepared packet attempt whose authoritative pre-call QPC is
/// sampled inside the Win32 sender and checked against the optional Down
/// cutoff before `SendInput`.
pub fn send_prepared_physical_packet_once_with_cutoff(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(prepared, clock, None, latest_allowed_down_qpc)
}

/// One trusted prepared packet attempt whose target-crossing QPC sample is
/// taken inside the precision sender. All payload metadata needed by the
/// syscall is resolved before the loop; the crossing sample is reused as the
/// authoritative pre-call timestamp.
pub fn send_prepared_physical_packet_once_at_target_with_cutoff(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    physical_target_qpc: QpcTicks,
    latest_allowed_down_qpc: Option<QpcTicks>,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    #[cfg(windows)]
    let view = {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT;
        let length = usize::from(prepared.event_count());
        PreparedInputView {
            requested: prepared.event_count(),
            length,
            inputs: prepared.inputs.as_ptr(),
            cb_size: std::mem::size_of::<INPUT>() as i32,
        }
    };
    #[cfg(not(windows))]
    let requested = prepared.event_count();

    #[cfg(windows)]
    let first =
        send_input_view_at_target(view, clock, physical_target_qpc, latest_allowed_down_qpc);
    #[cfg(not(windows))]
    let first = send_input_view_at_target(
        requested,
        clock,
        physical_target_qpc,
        latest_allowed_down_qpc,
    );
    prepared_send_outcome(packet, first)
}

/// Crate-private tagged calibration packet entry point. Packet materialization
/// is completed before the shared target-crossing envelope begins; the
/// correlation tag never enters the production packet identity.
pub(crate) fn send_tagged_packet_at_target(
    scan_codes: &[u16],
    key_up: bool,
    extra: usize,
    clock: QpcClock,
    physical_target_qpc: QpcTicks,
) -> PlatformSendResult {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        };

        let requested = scan_codes.len().min(15) as u8;
        let mut inputs: [INPUT; 15] = unsafe { std::mem::zeroed() };
        let mut flags = KEYEVENTF_SCANCODE;
        if key_up {
            flags |= KEYEVENTF_KEYUP;
        }
        for (index, &scan_code) in scan_codes.iter().take(15).enumerate() {
            inputs[index] = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: 0,
                        wScan: scan_code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: extra,
                    },
                },
            };
        }
        let result = send_input_view_at_target(
            PreparedInputView {
                requested,
                length: usize::from(requested),
                inputs: inputs.as_ptr(),
                cb_size: std::mem::size_of::<INPUT>() as i32,
            },
            clock,
            physical_target_qpc,
            None,
        );
        match result {
            Ok(result) => result,
            Err(PreparedSendFailure::Clock(started, error, _)) => PlatformSendResult {
                requested,
                inserted: 0,
                started_ticks: started.unwrap_or(QpcTicks::ZERO),
                completed_ticks: None,
                win32_error: 0,
                timing_error: Some(error),
            },
            Err(PreparedSendFailure::DeadlineMissed { .. }) => {
                unreachable!("tagged calibration packets do not use a Down cutoff")
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = (scan_codes, key_up, extra);
        match send_input_view_at_target(
            u8::try_from(scan_codes.len()).unwrap_or(u8::MAX),
            clock,
            physical_target_qpc,
            None,
        ) {
            Ok(result) => result,
            Err(PreparedSendFailure::Clock(started, error, _)) => PlatformSendResult {
                requested: u8::try_from(scan_codes.len()).unwrap_or(u8::MAX),
                inserted: 0,
                started_ticks: started.unwrap_or(QpcTicks::ZERO),
                completed_ticks: None,
                win32_error: 0,
                timing_error: Some(error),
            },
            Err(PreparedSendFailure::DeadlineMissed { .. }) => unreachable!(),
        }
    }
}

#[cfg(test)]
fn send_prepared_physical_packet_once_at_target_scripted(
    prepared: &PreparedPhysicalPacket,
    physical_target_qpc: QpcTicks,
    latest_allowed_down_qpc: Option<QpcTicks>,
    mut qpc_now: impl FnMut() -> Result<QpcTicks, crate::clock::QpcError>,
    mut send_one: impl FnMut(QpcTicks) -> Result<PlatformSendResult, crate::clock::QpcError>,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    let started_ticks = loop {
        let ticks = match qpc_now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return prepared_send_outcome(
                    packet,
                    Err(PreparedSendFailure::Clock(None, error, false)),
                );
            }
        };
        if ticks >= physical_target_qpc {
            break ticks;
        }
        std::hint::spin_loop();
    };
    if latest_allowed_down_qpc.is_some_and(|latest| started_ticks > latest) {
        return prepared_send_outcome(
            packet,
            Err(PreparedSendFailure::DeadlineMissed { started_ticks }),
        );
    }
    let first = send_one(started_ticks)
        .map_err(|error| PreparedSendFailure::Clock(Some(started_ticks), error, true));
    prepared_send_outcome(packet, first)
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
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    #[test]
    fn prepared_send_path_is_trusted_after_final_qpc_boundary() {
        let source = include_str!("packet.rs");
        let trusted = source
            .split("fn send_prepared_physical_packet_once_impl")
            .nth(1)
            .expect("trusted prepared-send primitive");
        let body = trusted
            .split("/// Test-only retry policy")
            .next()
            .expect("trusted primitive body");
        assert!(body.contains("run_prepared_send_attempt"));
        assert!(!body.contains("send_physical_packet_once_impl"));
        assert!(!body.contains("valid_packet("));
        assert!(!body.contains("event_count() == 0"));
    }

    #[test]
    fn prepared_cutoff_is_checked_before_the_sendinput_call() {
        let source = include_str!("packet.rs");
        let trusted = source
            .split("fn send_once_prepared(")
            .nth(1)
            .expect("trusted prepared-send primitive");
        let body = trusted
            .split("/// One low-level")
            .next()
            .expect("prepared-send primitive body");
        let cutoff = body
            .find("latest_allowed_down_qpc.is_some_and")
            .expect("authoritative cutoff check");
        let syscall = body.find("SendInput(").expect("direct SendInput call");
        assert!(cutoff < syscall);
    }

    #[test]
    fn target_crossing_sample_is_reused_without_a_second_pre_call_read() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).expect("prepared");
        let samples = Rc::new(RefCell::new(VecDeque::from([
            Ok(QpcTicks::from_raw(9)),
            Ok(QpcTicks::from_raw(9)),
            Ok(QpcTicks::from_raw(10)),
        ])));
        let completion_samples = Rc::new(RefCell::new(VecDeque::from([QpcTicks::from_raw(11)])));
        let send_calls = Rc::new(Cell::new(0));
        let observed_start = Rc::new(Cell::new(QpcTicks::ZERO));
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(10),
            Some(QpcTicks::from_raw(20)),
            {
                let samples = Rc::clone(&samples);
                move || samples.borrow_mut().pop_front().expect("scripted QPC")
            },
            {
                let completion_samples = Rc::clone(&completion_samples);
                let send_calls = Rc::clone(&send_calls);
                let observed_start = Rc::clone(&observed_start);
                move |started_ticks| {
                    send_calls.set(send_calls.get() + 1);
                    observed_start.set(started_ticks);
                    Ok(PlatformSendResult {
                        requested: 1,
                        inserted: 1,
                        started_ticks,
                        completed_ticks: completion_samples.borrow_mut().pop_front(),
                        win32_error: 0,
                        timing_error: None,
                    })
                }
            },
        );

        assert_eq!(result.status, SendTransactionStatus::Complete);
        assert_eq!(result.evidence.started_ticks, Some(QpcTicks::from_raw(10)));
        assert_eq!(
            result.evidence.completed_ticks,
            Some(QpcTicks::from_raw(11))
        );
        assert_eq!(observed_start.get(), QpcTicks::from_raw(10));
        assert_eq!(send_calls.get(), 1);
        assert!(samples.borrow().is_empty());
        assert!(completion_samples.borrow().is_empty());
    }

    #[test]
    fn target_crossing_past_down_cutoff_makes_zero_send_attempts() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).expect("prepared");
        let send_calls = Rc::new(Cell::new(0));
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
            Some(QpcTicks::from_raw(100)),
            || Ok(QpcTicks::from_raw(101)),
            {
                let send_calls = Rc::clone(&send_calls);
                move |_| {
                    send_calls.set(send_calls.get() + 1);
                    Ok(scripted_attempt(1, 1))
                }
            },
        );

        assert_eq!(
            result.status,
            SendTransactionStatus::DeadlineMissedBeforeSend
        );
        assert_eq!(result.evidence.started_ticks, Some(QpcTicks::from_raw(101)));
        assert_eq!(result.evidence.attempts, 0);
        assert_eq!(send_calls.get(), 0);
    }

    #[test]
    fn target_crossing_at_down_cutoff_is_allowed() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).expect("prepared");
        let send_calls = Rc::new(Cell::new(0));
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
            Some(QpcTicks::from_raw(100)),
            || Ok(QpcTicks::from_raw(100)),
            {
                let send_calls = Rc::clone(&send_calls);
                move |started_ticks| {
                    send_calls.set(send_calls.get() + 1);
                    Ok(PlatformSendResult {
                        requested: 1,
                        inserted: 1,
                        started_ticks,
                        completed_ticks: Some(QpcTicks::from_raw(101)),
                        win32_error: 0,
                        timing_error: None,
                    })
                }
            },
        );

        assert_eq!(result.status, SendTransactionStatus::Complete);
        assert_eq!(result.evidence.attempts, 1);
        assert_eq!(send_calls.get(), 1);
    }

    #[test]
    fn target_crossing_inside_down_cutoff_is_allowed() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).expect("prepared");
        let send_calls = Rc::new(Cell::new(0));
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
            Some(QpcTicks::from_raw(102)),
            || Ok(QpcTicks::from_raw(101)),
            {
                let send_calls = Rc::clone(&send_calls);
                move |started_ticks| {
                    send_calls.set(send_calls.get() + 1);
                    Ok(PlatformSendResult {
                        requested: 1,
                        inserted: 1,
                        started_ticks,
                        completed_ticks: Some(QpcTicks::from_raw(102)),
                        win32_error: 0,
                        timing_error: None,
                    })
                }
            },
        );

        assert_eq!(result.status, SendTransactionStatus::Complete);
        assert_eq!(result.evidence.started_ticks, Some(QpcTicks::from_raw(101)));
        assert_eq!(result.evidence.attempts, 1);
        assert_eq!(send_calls.get(), 1);
    }

    #[test]
    fn target_aware_sender_keeps_up_only_release_eligible_when_late() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0b001, 0)).expect("prepared");
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
            None,
            || Ok(QpcTicks::from_raw(101)),
            |started_ticks| {
                Ok(PlatformSendResult {
                    requested: 1,
                    inserted: 1,
                    started_ticks,
                    completed_ticks: Some(QpcTicks::from_raw(102)),
                    win32_error: 0,
                    timing_error: None,
                })
            },
        );

        assert_eq!(result.status, SendTransactionStatus::Complete);
        assert_eq!(result.evidence.attempts, 1);
    }

    #[test]
    fn target_sender_has_no_qpc_read_between_crossing_and_sendinput() {
        let source = include_str!("packet.rs");
        let trusted = source
            .split("pub fn send_prepared_physical_packet_once_at_target_with_cutoff")
            .nth(1)
            .expect("target-aware sender");
        let crossing = trusted
            .find("if ticks >= physical_target_qpc")
            .expect("target crossing");
        let syscall = trusted.find("SendInput(").expect("SendInput call");
        assert_eq!(
            trusted[crossing..syscall].matches("clock.now()").count(),
            0,
            "no QPC read may occur between crossing and SendInput"
        );
        assert!(
            trusted[..crossing].contains("prepared.inputs.as_ptr()"),
            "payload pointer must be resolved before target crossing"
        );
    }

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
