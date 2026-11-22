use super::outcome::{
    PacketPreparationError, PacketRetryReason, PhysicalPacket, PlatformSendResult, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus, classify_send_status,
};
use super::profile::MaterializedInstrumentKeyProfile;
use super::raw::{send_prepared_inputs_once, terminal_up_send_with};
use super::scan_code::{
    FULL_INSTRUMENT_MASK, PHYSICAL_INSTRUMENT_SCAN_CODES, SKY_PLAYER_SIGNATURE,
};
use super::up_transaction::terminal_up_outcome;
use crate::clock::{QpcClock, QpcError, QpcTicks};
use sky_dispatch_core::model::MAX_KEYS;
#[cfg(windows)]
use std::mem::MaybeUninit;

pub const MAX_PACKET_EVENTS: usize = MAX_KEYS;

/// Fixed-capacity physical work prepared before the precision boundary.
///
/// The Win32 payload is intentionally opaque to the scheduler/core crates.
/// Production callers retain this value until the single `SendInput` call.
pub struct PreparedPhysicalPacket {
    packet: PhysicalPacket,
    length: u8,
    up_length: u8,
    #[cfg(windows)]
    inputs:
        [MaybeUninit<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>; MAX_PACKET_EVENTS],
}

/// Borrowed view over an already materialized physical packet.
///
/// The view never owns or rebuilds Win32 `INPUT` values. It is used for the
/// final sender boundary to borrow the prepared packet without allocating.
#[derive(Clone, Copy)]
pub struct PreparedPacketView<'a> {
    packet: PhysicalPacket,
    length: u8,
    #[cfg(windows)]
    inputs: &'a [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT],
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
        Self::try_new_internal(packet, None)
    }

    pub fn try_new_with_profile(
        packet: PhysicalPacket,
        profile: &MaterializedInstrumentKeyProfile,
    ) -> Result<Self, PacketPreparationError> {
        Self::try_new_internal(packet, Some(profile))
    }

    fn try_new_internal(
        packet: PhysicalPacket,
        profile: Option<&MaterializedInstrumentKeyProfile>,
    ) -> Result<Self, PacketPreparationError> {
        let overlap_mask = packet.up_mask & packet.down_mask;
        if overlap_mask != 0 {
            return Err(PacketPreparationError::OverlappingDirections { overlap_mask });
        }
        if !valid_packet(packet) {
            return Err(PacketPreparationError::InvalidMask);
        }
        if packet.event_count() == 0 {
            return Err(PacketPreparationError::Empty);
        }
        #[cfg(windows)]
        let (inputs, length) = build_inputs(packet, profile);
        #[cfg(not(windows))]
        let length = packet.event_count() as usize;
        Ok(Self {
            packet,
            length: length as u8,
            up_length: packet.up_mask.count_ones() as u8,
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

    pub fn as_view(&self) -> PreparedPacketView<'_> {
        #[cfg(windows)]
        {
            PreparedPacketView {
                packet: self.packet,
                length: self.length,
                inputs: self.initialized_inputs(),
            }
        }
        #[cfg(not(windows))]
        {
            PreparedPacketView {
                packet: self.packet,
                length: self.length,
            }
        }
    }

    /// Borrow the already initialized Up prefix of a prepared packet.
    ///
    /// Physical packet construction writes all Up events before any Down
    /// events, so this view shares the original storage and needs no second
    /// INPUT construction. The stored Up length was computed and validated
    /// when the packet was prepared, before the dispatch deadline.
    pub fn up_prefix_view(&self) -> Option<PreparedPacketView<'_>> {
        if self.up_length == 0 {
            return None;
        }

        #[cfg(windows)]
        {
            let inputs = self
                .initialized_inputs()
                .get(..usize::from(self.up_length))?;
            Some(PreparedPacketView {
                packet: PhysicalPacket::new(self.packet.up_mask, 0),
                length: self.up_length,
                inputs,
            })
        }
        #[cfg(not(windows))]
        {
            Some(PreparedPacketView {
                packet: PhysicalPacket::new(self.packet.up_mask, 0),
                length: self.up_length,
            })
        }
    }

    #[cfg(windows)]
    pub(super) fn initialized_inputs(
        &self,
    ) -> &[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT] {
        let length = usize::from(self.length);
        // SAFETY: `try_new` sets `length` only from `build_inputs`, which
        // writes every element in the prefix. The length is bounded by the
        // fixed array capacity, so no uninitialized element is exposed.
        unsafe { std::slice::from_raw_parts(self.inputs.as_ptr().cast(), length) }
    }
}

impl<'a> PreparedPacketView<'a> {
    #[inline]
    pub fn packet(self) -> PhysicalPacket {
        self.packet
    }

    #[inline]
    pub fn event_count(self) -> u8 {
        self.length
    }
}

/// Fixed-capacity calibration payload prepared before the precision handoff.
///
/// The correlation tag belongs only to the calibration Raw Input observer and
/// never enters production packet identity or reconciliation state.
pub(crate) struct PreparedTaggedCalibrationPacket {
    requested: u8,
    #[cfg(windows)]
    inputs: [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; 15],
}

impl PreparedTaggedCalibrationPacket {
    /// Materialize the complete tagged INPUT array before the final wait
    /// handoff. The caller validates instrument scan codes; this constructor
    /// still rejects an empty or oversized packet so the seam is fail-closed.
    pub(crate) fn try_new(scan_codes: &[u16], key_up: bool, extra: usize) -> Option<Self> {
        let requested = u8::try_from(scan_codes.len()).ok()?;
        if requested == 0 || scan_codes.len() > 15 {
            return None;
        }

        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
            };

            let mut inputs: [INPUT; 15] = unsafe { std::mem::zeroed() };
            let mut flags = KEYEVENTF_SCANCODE;
            if key_up {
                flags |= KEYEVENTF_KEYUP;
            }
            for (index, &scan_code) in scan_codes.iter().enumerate() {
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
            Some(Self { requested, inputs })
        }

        #[cfg(not(windows))]
        {
            let _ = (key_up, extra);
            Some(Self { requested })
        }
    }

    /// Enter the shared target-crossing envelope with an already materialized
    /// tagged payload. No INPUT construction or allocation is allowed here.
    pub(crate) fn send_at_target(
        &self,
        clock: QpcClock,
        physical_target_qpc: QpcTicks,
    ) -> PlatformSendResult {
        #[cfg(windows)]
        let result = send_input_view_at_target(
            PreparedInputView {
                requested: self.requested,
                inputs: self.inputs.as_ptr(),
            },
            clock,
            physical_target_qpc,
        );

        #[cfg(not(windows))]
        let result = send_input_view_at_target(self.requested, clock, physical_target_qpc);

        match result {
            Ok(result) => result,
            Err(PreparedSendFailure::Clock(failure)) => PlatformSendResult {
                requested: self.requested,
                inserted: failure.inserted,
                started_ticks: failure.started_ticks.unwrap_or(QpcTicks::ZERO),
                completed_ticks: None,
                win32_error: failure.win32_error,
                timing_error: Some(failure.error),
            },
        }
    }
}

#[cfg(windows)]
const MAX_SCAN_CODE: usize = 0x36;

#[cfg(windows)]
pub(crate) const fn create_keyboard_input(
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
    packet.up_mask & packet.down_mask == 0
        && packet.up_mask & !FULL_INSTRUMENT_MASK == 0
        && packet.down_mask & !FULL_INSTRUMENT_MASK == 0
        && usize::from(packet.event_count()) <= MAX_PACKET_EVENTS
}

#[cfg(windows)]
fn build_inputs(
    packet: PhysicalPacket,
    profile: Option<&MaterializedInstrumentKeyProfile>,
) -> (
    [MaybeUninit<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT>; MAX_PACKET_EVENTS],
    usize,
) {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT;

    let mut inputs: [MaybeUninit<INPUT>; MAX_PACKET_EVENTS] =
        [const { MaybeUninit::uninit() }; MAX_PACKET_EVENTS];
    let mut length = 0usize;
    let mut append_mask = |mut mask: u16, templates: &[INPUT; MAX_SCAN_CODE], down: bool| {
        while mask != 0 {
            let slot = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            let input = match profile {
                Some(profile) => {
                    if down {
                        profile.down_template(slot)
                    } else {
                        profile.up_template(slot)
                    }
                }
                None => templates[PHYSICAL_INSTRUMENT_SCAN_CODES[slot] as usize],
            };
            inputs[length].write(input);
            length += 1;
        }
    };
    append_mask(packet.up_mask, &UP_TEMPLATES, false);
    append_mask(packet.down_mask, &DOWN_TEMPLATES, true);
    (inputs, length)
}

pub(crate) fn invalid_packet_outcome(packet: PhysicalPacket) -> SendTransactionOutcome {
    SendTransactionOutcome {
        status: SendTransactionStatus::PreparationRejected,
        evidence: SendEvidence {
            requested_mask: packet.up_mask | packet.down_mask,
            confirmed_mask: 0,
            skipped_mask: 0,
            first_inserted: 0,
            attempts: 0,
            zero_progress_retries: 0,
            retry_reason: PacketRetryReason::None,
            first_win32_error: None,
            last_win32_error: None,
            started_ticks: None,
            completed_ticks: None,
            timing_error: None,
        },
    }
}

/// Send one already-validated Up-only packet for terminal cleanup. Timing is
/// sampled when available, but a missing pre-call QPC sample cannot block this
/// bounded safety send. The caller retains ownership of retry and verification.
pub(crate) fn send_prepared_terminal_up_once(
    prepared: &PreparedPhysicalPacket,
    clock: Option<QpcClock>,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    if !packet.is_up_only() || packet.up_mask == 0 {
        return invalid_packet_outcome(packet);
    }

    let clock = clock.or_else(|| QpcClock::initialize().ok());
    let initialization_error = clock.is_none().then_some(QpcError::FrequencyUnavailable);
    let mut sample_qpc = || match clock {
        Some(clock) => clock.now(),
        None => Err(initialization_error.unwrap_or(QpcError::FrequencyUnavailable)),
    };

    #[cfg(windows)]
    let transport = {
        use windows_sys::Win32::Foundation::SetLastError;

        let requested = prepared.event_count();
        terminal_up_send_with(requested, &mut sample_qpc, || {
            // Clear stale thread-local state after the optional start
            // sample and immediately before the authoritative syscall.
            unsafe { SetLastError(0) };
            // SAFETY: PreparedPhysicalPacket initializes exactly this fixed
            // prefix and owns the storage until the one-call sender returns.
            unsafe { send_prepared_inputs_once(prepared.initialized_inputs().as_ptr(), requested) }
        })
    };

    #[cfg(not(windows))]
    let transport = terminal_up_send_with(prepared.event_count(), &mut sample_qpc, || {
        send_prepared_inputs_once(prepared.event_count())
    });

    terminal_up_outcome(packet, transport)
}

#[derive(Clone, Copy, Debug)]
struct SendClockFailure {
    started_ticks: Option<QpcTicks>,
    error: crate::clock::QpcError,
    sendinput_called: bool,
    inserted: u8,
    win32_error: u32,
}

impl SendClockFailure {
    fn before_send(error: crate::clock::QpcError) -> Self {
        Self {
            started_ticks: None,
            error,
            sendinput_called: false,
            inserted: 0,
            win32_error: 0,
        }
    }

    fn after_send(
        started_ticks: QpcTicks,
        error: crate::clock::QpcError,
        inserted: u8,
        win32_error: u32,
    ) -> Self {
        Self {
            started_ticks: Some(started_ticks),
            error,
            sendinput_called: true,
            inserted,
            win32_error,
        }
    }

    #[cfg(not(windows))]
    fn after_pre_call_without_send(started_ticks: QpcTicks, error: crate::clock::QpcError) -> Self {
        Self {
            started_ticks: Some(started_ticks),
            error,
            sendinput_called: false,
            inserted: 0,
            win32_error: 0,
        }
    }
}

enum PreparedSendFailure {
    Clock(SendClockFailure),
}

#[cfg(windows)]
struct PreparedInputView {
    requested: u8,
    inputs: *const windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT,
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
) -> Result<PlatformSendResult, PreparedSendFailure> {
    use windows_sys::Win32::Foundation::SetLastError;

    // Last-error state is preparation; it must not sit between the crossing
    // sample and SendInput.
    unsafe { SetLastError(0) };
    let started_ticks = loop {
        let ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(SendClockFailure::before_send(
                    error,
                )));
            }
        };
        if ticks >= physical_target_qpc {
            break ticks;
        }
        std::hint::spin_loop();
    };

    // SAFETY: the internal calibration caller supplies a fixed, initialized
    // array prefix and retains it for this synchronous call.
    let (inserted, win32_error) = unsafe { send_prepared_inputs_once(view.inputs, view.requested) };
    let completed_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(PreparedSendFailure::Clock(SendClockFailure::after_send(
                started_ticks,
                error,
                inserted,
                win32_error,
            )));
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
) -> Result<PlatformSendResult, PreparedSendFailure> {
    let started_ticks = loop {
        let ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(SendClockFailure::before_send(
                    error,
                )));
            }
        };
        if ticks >= physical_target_qpc {
            break ticks;
        }
        std::hint::spin_loop();
    };
    let completed_ticks = match clock.now() {
        Ok(ticks) => ticks,
        Err(error) => {
            return Err(PreparedSendFailure::Clock(
                SendClockFailure::after_pre_call_without_send(started_ticks, error),
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

fn send_once_prepared_view(
    prepared: PreparedPacketView<'_>,
    clock: QpcClock,
    supplied_started_ticks: Option<QpcTicks>,
) -> Result<PlatformSendResult, PreparedSendFailure> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SetLastError;

        let requested = prepared.event_count();
        let inputs = prepared.inputs.as_ptr();
        // Reset the thread-local error before the final correctness sample so
        // short insertion evidence cannot inherit an unrelated prior error.
        unsafe { SetLastError(0) };
        let started_ticks = match supplied_started_ticks {
            Some(ticks) => ticks,
            None => match clock.now() {
                Ok(ticks) => ticks,
                Err(error) => {
                    return Err(PreparedSendFailure::Clock(SendClockFailure::before_send(
                        error,
                    )));
                }
            },
        };
        // SAFETY: PreparedPacketView is backed by an initialized prefix of the
        // still-live PreparedPhysicalPacket borrowed by this function.
        let (inserted, win32_error) =
            unsafe { send_prepared_inputs_once(inputs, prepared.event_count()) };
        let completed_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(SendClockFailure::after_send(
                    started_ticks,
                    error,
                    inserted,
                    win32_error,
                )));
            }
        };
        Ok(PlatformSendResult {
            requested,
            inserted,
            started_ticks,
            completed_ticks: Some(completed_ticks),
            win32_error,
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
                Err(error) => {
                    return Err(PreparedSendFailure::Clock(SendClockFailure::before_send(
                        error,
                    )));
                }
            },
        };
        let completed_ticks = match clock.now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return Err(PreparedSendFailure::Clock(
                    SendClockFailure::after_pre_call_without_send(started_ticks, error),
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

/// One low-level `SendInput`/QPC attempt for a prepared physical packet.
enum PacketSendAttempt {
    Outcome(PlatformSendResult),
    ClockFailure(SendClockFailure),
}

fn run_prepared_send_attempt(
    prepared: PreparedPacketView<'_>,
    clock: QpcClock,
    supplied_started_ticks: Option<QpcTicks>,
) -> PacketSendAttempt {
    match send_once_prepared_view(prepared, clock, supplied_started_ticks) {
        Ok(res) => PacketSendAttempt::Outcome(res),
        Err(PreparedSendFailure::Clock(failure)) => PacketSendAttempt::ClockFailure(failure),
    }
}

fn prepared_send_outcome(
    packet: PhysicalPacket,
    first: Result<PlatformSendResult, PreparedSendFailure>,
) -> SendTransactionOutcome {
    let requested_mask = packet.up_mask | packet.down_mask;
    let first = match first {
        Ok(res) => res,
        Err(PreparedSendFailure::Clock(failure)) => {
            return SendTransactionOutcome {
                status: if failure.sendinput_called {
                    SendTransactionStatus::ClockFailureAfterSend
                } else {
                    SendTransactionStatus::ClockFailureBeforeSend
                },
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: failure.inserted,
                    attempts: u8::from(failure.sendinput_called),
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: (failure.win32_error != 0).then_some(failure.win32_error),
                    last_win32_error: (failure.win32_error != 0).then_some(failure.win32_error),
                    started_ticks: failure.started_ticks,
                    completed_ticks: None,
                    timing_error: Some(failure.error),
                },
            };
        }
    };
    let requested = usize::from(first.requested);
    let inserted = usize::from(first.inserted);
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
/// `pre_call_qpc` is sampled here, after the prepared payload length, pointer,
/// and ABI size have been resolved and immediately before `SendInput`. Test
/// support may provide a controlled timestamp without changing production
/// behavior.
fn send_prepared_physical_packet_once_impl(
    prepared: PreparedPacketView<'_>,
    clock: QpcClock,
    started_ticks: Option<QpcTicks>,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    let first = match run_prepared_send_attempt(prepared, clock, started_ticks) {
        PacketSendAttempt::Outcome(res) => Ok(res),
        PacketSendAttempt::ClockFailure(failure) => Err(PreparedSendFailure::Clock(failure)),
    };
    prepared_send_outcome(packet, first)
}

/// One packet transaction using a payload built before the final admission
/// boundary and a caller-owned authoritative start timestamp.
#[cfg(test)]
pub(crate) fn send_prepared_physical_packet_once_with_start(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    started_ticks: QpcTicks,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(prepared.as_view(), clock, Some(started_ticks))
}

/// One trusted borrowed prepared-packet attempt using a caller-supplied start
/// boundary.
#[cfg(test)]
pub(crate) fn send_prepared_physical_packet_view_once_with_start(
    prepared: PreparedPacketView<'_>,
    clock: QpcClock,
    started_ticks: QpcTicks,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(prepared, clock, Some(started_ticks))
}

/// One trusted prepared packet attempt whose authoritative pre-call QPC is
/// sampled inside the Win32 sender immediately before `SendInput`.
pub(crate) fn send_prepared_physical_packet_once(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(prepared.as_view(), clock, None)
}

/// One trusted borrowed prepared-packet attempt at the caller's final boundary.
pub(crate) fn send_prepared_physical_packet_view_once(
    prepared: PreparedPacketView<'_>,
    clock: QpcClock,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_once_impl(prepared, clock, None)
}

/// One trusted prepared packet attempt whose target-crossing QPC sample is
/// taken inside the precision sender. All payload metadata needed by the
/// syscall is resolved before the loop; the crossing sample is reused as the
/// authoritative pre-call timestamp.
pub(crate) fn send_prepared_physical_packet_once_at_target(
    prepared: &PreparedPhysicalPacket,
    clock: QpcClock,
    physical_target_qpc: QpcTicks,
) -> SendTransactionOutcome {
    send_prepared_physical_packet_view_once_at_target(
        prepared.as_view(),
        clock,
        physical_target_qpc,
    )
}

/// One trusted borrowed prepared-packet attempt whose target-crossing QPC
/// sample is taken immediately before `SendInput`.
pub(crate) fn send_prepared_physical_packet_view_once_at_target(
    prepared: PreparedPacketView<'_>,
    clock: QpcClock,
    physical_target_qpc: QpcTicks,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    #[cfg(windows)]
    let view = {
        PreparedInputView {
            requested: prepared.event_count(),
            inputs: prepared.inputs.as_ptr(),
        }
    };
    #[cfg(not(windows))]
    let requested = prepared.event_count();

    #[cfg(windows)]
    let first = send_input_view_at_target(view, clock, physical_target_qpc);
    #[cfg(not(windows))]
    let first = send_input_view_at_target(requested, clock, physical_target_qpc);
    prepared_send_outcome(packet, first)
}

#[cfg(test)]
fn send_prepared_physical_packet_once_at_target_scripted(
    prepared: &PreparedPhysicalPacket,
    physical_target_qpc: QpcTicks,
    mut qpc_now: impl FnMut() -> Result<QpcTicks, crate::clock::QpcError>,
    mut send_one: impl FnMut(QpcTicks) -> Result<PlatformSendResult, (u8, u32, crate::clock::QpcError)>,
) -> SendTransactionOutcome {
    let packet = prepared.packet();
    let started_ticks = loop {
        let ticks = match qpc_now() {
            Ok(ticks) => ticks,
            Err(error) => {
                return prepared_send_outcome(
                    packet,
                    Err(PreparedSendFailure::Clock(SendClockFailure::before_send(
                        error,
                    ))),
                );
            }
        };
        if ticks >= physical_target_qpc {
            break ticks;
        }
        std::hint::spin_loop();
    };
    let first = send_one(started_ticks).map_err(|(inserted, win32_error, error)| {
        PreparedSendFailure::Clock(SendClockFailure::after_send(
            started_ticks,
            error,
            inserted,
            win32_error,
        ))
    });
    prepared_send_outcome(packet, first)
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
            .split("/// One packet transaction using a payload built before the final admission")
            .next()
            .expect("trusted primitive body");
        assert!(body.contains("run_prepared_send_attempt"));
        assert!(!body.contains("valid_packet("));
        assert!(!body.contains("event_count() == 0"));
    }

    #[test]
    fn tagged_calibration_payload_is_materialized_before_sender_handoff() {
        let source = include_str!("packet.rs").replace("\r\n", "\n");
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production packet implementation");
        let constructor = production
            .split("pub(crate) fn try_new(")
            .nth(1)
            .expect("prepared tagged constructor");
        let sender = production
            .split("pub(crate) fn send_at_target(")
            .nth(1)
            .expect("prepared tagged sender")
            .split("#[cfg(windows)]\nconst MAX_SCAN_CODE")
            .next()
            .expect("prepared tagged sender body");
        assert!(constructor.contains("INPUT {"));
        assert!(constructor.contains("std::mem::zeroed"));
        assert!(!sender.contains("inputs[index] = INPUT {"));
        assert!(!sender.contains("KEYBDINPUT"));
        assert!(!sender.contains("std::mem::zeroed"));
        assert!(!sender.contains("Vec::"));
    }

    #[test]
    fn immediate_sender_samples_true_pre_call() {
        let source = include_str!("packet.rs");
        let body = source
            .split("fn send_once_prepared_view(")
            .nth(1)
            .expect("immediate prepared sender")
            .split("enum PacketSendAttempt")
            .next()
            .expect("immediate prepared sender body");
        assert!(body.contains("clock.now()"));
        assert!(body.contains("send_prepared_inputs_once"));
        assert!(!body.contains("latest_start"));
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
    fn post_send_qpc_failure_preserves_insert_count_and_win32_error() {
        let prepared = PreparedPhysicalPacket::try_new(PhysicalPacket::new(0b001, 0b010))
            .expect("prepared Mixed packet");

        for (inserted, win32_error) in [(0, 5), (1, 5), (2, 0)] {
            let send_calls = Cell::new(0);
            let outcome = send_prepared_physical_packet_once_at_target_scripted(
                &prepared,
                QpcTicks::from_raw(100),
                || Ok(QpcTicks::from_raw(100)),
                |started_ticks| {
                    send_calls.set(send_calls.get() + 1);
                    assert_eq!(started_ticks, QpcTicks::from_raw(100));
                    Err((inserted, win32_error, QpcError::CounterUnavailable))
                },
            );

            assert_eq!(outcome.status, SendTransactionStatus::ClockFailureAfterSend);
            assert_eq!(outcome.evidence.attempts, 1);
            assert_eq!(outcome.evidence.first_inserted, inserted);
            assert_eq!(
                outcome.evidence.first_win32_error,
                (win32_error != 0).then_some(win32_error)
            );
            assert_eq!(
                outcome.evidence.last_win32_error,
                (win32_error != 0).then_some(win32_error)
            );
            assert_eq!(
                outcome.evidence.started_ticks,
                Some(QpcTicks::from_raw(100))
            );
            assert_eq!(outcome.evidence.completed_ticks, None);
            assert_eq!(
                outcome.evidence.timing_error,
                Some(QpcError::CounterUnavailable)
            );
            assert_eq!(send_calls.get(), 1);
        }
    }

    #[test]
    fn contradictory_send_count_is_integrity_lost_without_transport_confirmation() {
        let packet = PhysicalPacket::new(0, 0b001);
        let outcome = prepared_send_outcome(
            packet,
            Ok(PlatformSendResult {
                requested: 1,
                inserted: 2,
                started_ticks: QpcTicks::from_raw(10),
                completed_ticks: Some(QpcTicks::from_raw(11)),
                win32_error: 0,
                timing_error: None,
            }),
        );

        assert_eq!(outcome.status, SendTransactionStatus::IntegrityLost);
        assert_eq!(outcome.evidence.first_inserted, 2);
        assert_eq!(outcome.evidence.confirmed_mask, 0);
        assert_eq!(outcome.evidence.attempts, 1);
        assert!(!outcome.has_complete_raw_transport_for(packet));
    }

    #[test]
    fn pre_send_qpc_failure_never_calls_the_scripted_sender() {
        let prepared = PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001))
            .expect("prepared Down packet");
        let outcome = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
            || Err(QpcError::CounterUnavailable),
            |_| panic!("pre-call QPC failure must not reach the sender"),
        );

        assert_eq!(
            outcome.status,
            SendTransactionStatus::ClockFailureBeforeSend
        );
        assert_eq!(outcome.evidence.attempts, 0);
        assert_eq!(outcome.evidence.first_inserted, 0);
        assert_eq!(outcome.evidence.first_win32_error, None);
        assert_eq!(outcome.evidence.last_win32_error, None);
        assert_eq!(outcome.evidence.started_ticks, None);
        assert_eq!(outcome.evidence.completed_ticks, None);
        assert_eq!(
            outcome.evidence.timing_error,
            Some(QpcError::CounterUnavailable)
        );
    }

    #[test]
    fn target_crossing_late_down_is_still_sent() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).expect("prepared");
        let send_calls = Rc::new(Cell::new(0));
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
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
    fn target_crossing_late_down_keeps_one_attempt() {
        let prepared =
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 0b001)).expect("prepared");
        let send_calls = Rc::new(Cell::new(0));
        let result = send_prepared_physical_packet_once_at_target_scripted(
            &prepared,
            QpcTicks::from_raw(100),
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
    fn physical_packet_accepts_fifteen_events_per_direction() {
        for (up_mask, down_mask) in [(FULL_INSTRUMENT_MASK, 0), (0, FULL_INSTRUMENT_MASK)] {
            let prepared = PreparedPhysicalPacket::try_new(PhysicalPacket::new(up_mask, down_mask))
                .expect("fifteen-key packet prepares");
            assert_eq!(prepared.event_count(), MAX_PACKET_EVENTS as u8);
        }
    }

    #[test]
    fn prepared_mixed_up_prefix_is_a_borrowed_initialized_prefix() {
        for up_count in [1_u32, 5, 14] {
            let up_mask = (1_u16 << up_count) - 1;
            let down_count = 15 - up_count;
            let down_mask = ((1_u16 << down_count) - 1) << up_count;
            let packet = PhysicalPacket::new(up_mask, down_mask);
            let prepared = PreparedPhysicalPacket::try_new(packet).expect("valid Mixed packet");
            let full = prepared.as_view();
            let prefix = prepared.up_prefix_view().expect("non-empty Up prefix");

            assert_eq!(full.packet(), packet);
            assert_eq!(full.event_count(), 15);
            assert_eq!(prefix.packet(), PhysicalPacket::new(up_mask, 0));
            assert_eq!(prefix.event_count(), up_count as u8);
            assert_eq!(prefix.packet().event_count(), prefix.event_count());
            #[cfg(windows)]
            assert_eq!(prefix.inputs.as_ptr(), full.inputs.as_ptr());
        }

        let down_only = PreparedPhysicalPacket::try_new(PhysicalPacket::new(0, 1))
            .expect("valid Down-only packet");
        assert!(down_only.up_prefix_view().is_none());
    }

    #[test]
    fn prepared_packet_rejects_mask_outside_physical_allowlist() {
        assert!(
            PreparedPhysicalPacket::try_new(PhysicalPacket::new(
                FULL_INSTRUMENT_MASK | (1 << 15),
                0,
            ))
            .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn input_builder_places_all_up_events_before_down_events() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::KEYEVENTF_KEYUP;

        let (inputs, len) = build_inputs(PhysicalPacket::new(0b1, 0b10), None);
        assert_eq!(len, 2);
        unsafe {
            let first = inputs[0].assume_init_ref();
            let second = inputs[1].assume_init_ref();
            assert_eq!(first.Anonymous.ki.wScan, 0x15);
            assert_ne!(first.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
            assert_eq!(second.Anonymous.ki.wScan, 0x16);
            assert_eq!(second.Anonymous.ki.dwFlags & KEYEVENTF_KEYUP, 0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn prepared_packet_rejects_overlapping_directions() {
        let error = PreparedPhysicalPacket::try_new(PhysicalPacket::new(0b001, 0b001))
            .expect_err("same-key Up+Down packet must be rejected");
        assert_eq!(
            error,
            PacketPreparationError::OverlappingDirections {
                overlap_mask: 0b001
            }
        );
        assert!(PreparedPhysicalPacket::try_new(PhysicalPacket::new(0b001, 0b010)).is_ok());
    }
}
