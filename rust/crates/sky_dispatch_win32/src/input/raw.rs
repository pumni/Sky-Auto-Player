use super::outcome::TerminalUpTransportResult;
#[cfg(any(test, feature = "test-support"))]
use crate::clock::QpcClock;
use crate::clock::{QpcError, QpcTicks};

/// The single raw Win32 transport primitive used by musical packets and safety
/// releases. Callers fully prepare the INPUT slice
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

#[cfg(any(test, feature = "test-support"))]
pub(crate) fn no_syscall_boundary_with_clock(
    clock: Option<QpcClock>,
) -> (QpcTicks, Option<QpcTicks>, Option<QpcError>) {
    let clock = match clock {
        Some(clock) => clock,
        None => match QpcClock::initialize() {
            Ok(clock) => clock,
            Err(error) => return (QpcTicks::ZERO, None, Some(error)),
        },
    };
    let Ok(_) = crate::clock::qpc_frequency_checked() else {
        return (QpcTicks::ZERO, None, Some(QpcError::FrequencyUnavailable));
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
