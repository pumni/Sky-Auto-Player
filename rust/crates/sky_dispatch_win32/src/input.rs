//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

use smallvec::SmallVec;
use std::fmt;

pub const SKY_PLAYER_SIGNATURE: usize = 0x5C1B9111;

pub const PHYSICAL_INSTRUMENT_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, // Y U I O P
    0x23, 0x24, 0x25, 0x26, 0x27, // H J K L ;
    0x31, 0x32, 0x33, 0x34, 0x35, // N M , . /
];

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

fn virtual_key_for_scan_code(scan_code: u16) -> Option<i32> {
    Some(match scan_code {
        0x15 => 0x59, // Y
        0x16 => 0x55, // U
        0x17 => 0x49, // I
        0x18 => 0x4F, // O
        0x19 => 0x50, // P
        0x23 => 0x48, // H
        0x24 => 0x4A, // J
        0x25 => 0x4B, // K
        0x26 => 0x4C, // L
        0x27 => 0xBA, // ;
        0x31 => 0x4E, // N
        0x32 => 0x4D, // M
        0x33 => 0xBC, // ,
        0x34 => 0xBE, // .
        0x35 => 0xBF, // /
        _ => return None,
    })
}

fn is_scan_code_physically_down(scan_code: u16) -> Option<bool> {
    let virtual_key = virtual_key_for_scan_code(scan_code)?;
    #[cfg(windows)]
    {
        // SAFETY: GetAsyncKeyState accepts any virtual-key integer and does not
        // retain pointers or transfer ownership.
        let state = unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(virtual_key)
        };
        Some((state as u16 & 0x8000) != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = virtual_key;
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformSendResult {
    pub requested: u32,
    pub inserted: u32,
    pub completed_us: u64,
    pub win32_error: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputSendResult {
    pub sent: SmallVec<[u16; 15]>,
    pub skipped_duplicates: SmallVec<[u16; 15]>,
    pub success: bool,
    pub error: Option<String>,
    pub send_completed_us: u64,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownSendOutcome {
    Complete {
        completed_us: u64,
        sent: SmallVec<[u16; 15]>,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        retried_after_zero_progress: bool,
    },
    ZeroProgress {
        error: Option<u32>,
        completed_us: u64,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
    },
    IntegrityLost {
        inserted_prefix: u8,
        rolled_back: u8,
        rollback_residue: u8,
        first_error: Option<u32>,
        last_error: Option<u32>,
        completed_us: u64,
        sent: SmallVec<[u16; 15]>,
        skipped_duplicates: SmallVec<[u16; 15]>,
        send_attempts: u8,
        zero_progress_retries: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmitResult {
    pub sent: SmallVec<[u16; 15]>,
    pub completed_us: u64,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAllOutcome {
    pub attempted: Vec<u16>,
    pub released_successfully: bool,
    pub stuck_keys: Vec<u16>,
    pub verification_inconclusive: bool,
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
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::SetLastError;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};

        if scan_codes.is_empty() {
            return PlatformSendResult {
                requested: 0,
                inserted: 0,
                completed_us: crate::clock::qpc_now_us(),
                win32_error: 0,
            };
        }

        let mut packets: [INPUT; 15] = unsafe { std::mem::zeroed() };
        let len = scan_codes.len().min(15);
        for i in 0..len {
            let sc = scan_codes[i];
            if (sc as usize) < MAX_SCAN_CODE {
                packets[i] = if key_up {
                    UP_TEMPLATES[sc as usize]
                } else {
                    DOWN_TEMPLATES[sc as usize]
                };
            } else {
                packets[i] = create_keyboard_input(sc, key_up);
            }
        }
        let requested = len as u32;
        let cb_size = std::mem::size_of::<INPUT>() as i32;

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
        let completed_us = crate::clock::qpc_now_us();

        PlatformSendResult {
            requested,
            inserted,
            completed_us,
            win32_error,
        }
    }
    #[cfg(not(windows))]
    {
        PlatformSendResult {
            requested: scan_codes.len() as u32,
            inserted: scan_codes.len() as u32,
            completed_us: crate::clock::qpc_now_us(),
            win32_error: 0,
        }
    }
}

pub fn emit_down_with<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        return EmitResult {
            sent: SmallVec::new(),
            completed_us: crate::clock::qpc_now_us(),
            success: true,
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
        };
    }

    // A non-zero partial insertion has already destroyed chord integrity. Do
    // not send the remainder: that would make a nominally successful chord
    // arrive through two separately timestamped SendInput calls. Roll back
    // the landed prefix immediately and leave any rollback residue tracked so
    // the worker's terminal cleanup can handle it.
    if landed1 > 0 {
        let rollback = send_fn(&scan_codes[..landed1], true);
        let rollback_inserted = (rollback.inserted as usize).min(landed1);
        let rollback_error = (rollback.win32_error != 0).then_some(rollback.win32_error);
        let sent: SmallVec<[u16; 15]> = scan_codes[..landed1]
            .iter()
            .skip(rollback_inserted)
            .copied()
            .collect();
        return EmitResult {
            sent,
            completed_us: rollback.completed_us,
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
            rollback_residue_keys: landed1.saturating_sub(rollback_inserted) as u8,
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
        };
    }

    let mut sent: SmallVec<[u16; 15]> = SmallVec::new();
    let mut completed_us = retry.completed_us;
    let mut send_attempts = 2;
    let mut last_win32_error = retry_error.or(first_win32_error);
    if retry_inserted > 0 {
        let rollback = send_fn(&scan_codes[..retry_inserted], true);
        let rollback_inserted = (rollback.inserted as usize).min(retry_inserted);
        sent.extend(
            scan_codes[..retry_inserted]
                .iter()
                .skip(rollback_inserted)
                .copied(),
        );
        completed_us = rollback.completed_us;
        send_attempts = 3;
        last_win32_error = (rollback.win32_error != 0)
            .then_some(rollback.win32_error)
            .or(last_win32_error);
    }
    let rollback_residue_keys = sent.len();
    EmitResult {
        sent,
        completed_us,
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
        keys_rolled_back: if retry_inserted > 0 {
            (retry_inserted.saturating_sub(rollback_residue_keys)) as u8
        } else {
            0
        },
        rollback_residue_keys: rollback_residue_keys as u8,
    }
}

pub fn emit_down(scan_codes: &[u16]) -> EmitResult {
    emit_down_with(scan_codes, send_input_raw)
}

/// Emit a note-off without delaying the real-time worker.
///
/// A partial `SendInput` result gets one immediate remainder retry.  Any
/// delayed retry belongs to the coordinator, which can then enter an
/// interruptible recovery pause instead of blocking command handling inside
/// the platform seam.
fn emit_up_with_immediate<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        return EmitResult {
            sent: SmallVec::new(),
            completed_us: crate::clock::qpc_now_us(),
            success: true,
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
        };
    }

    let remainder = &scan_codes[first_inserted..];
    let second = send_fn(remainder, true);
    let second_inserted = (second.inserted as usize).min(remainder.len());
    let sent_total = first_inserted + second_inserted;
    let second_win32_error = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32_error = second_win32_error.or(first_win32_error);

    let success = sent_total == n;
    let sent: SmallVec<[u16; 15]> = scan_codes[..sent_total].iter().copied().collect();
    EmitResult {
        sent,
        completed_us: second.completed_us,
        success,
        keys_dropped: (n - sent_total) as u64,
        first_win32_error: first_win32_error.or(second_win32_error),
        last_win32_error,
        send_attempts: 2,
        zero_progress_retries: u8::from(first_inserted == 0),
        first_inserted: first_inserted as u8,
        partial_progress: sent_total > 0 && !success,
        retried_after_zero_progress: first_inserted == 0,
        chord_integrity_lost: false,
        keys_inserted_before_failure: if success { 0 } else { sent_total as u8 },
        keys_rolled_back: 0,
        rollback_residue_keys: 0,
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
    pub custom_emitter: Option<CustomEmitterFn>,
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
            .field("custom_emitter", &self.custom_emitter.is_some())
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

    fn do_emit_down(&mut self, scan_codes: &[u16]) -> EmitResult {
        if let Some(ref emitter) = self.custom_emitter {
            emit_down_with(scan_codes, |sc, key_up| emitter(sc, key_up))
        } else {
            emit_down(scan_codes)
        }
    }

    fn do_emit_up(&mut self, scan_codes: &[u16]) -> EmitResult {
        if let Some(ref emitter) = self.custom_emitter {
            emit_up_with(scan_codes, |sc, key_up| emitter(sc, key_up))
        } else {
            emit_up(scan_codes)
        }
    }

    pub fn key_down(&mut self, scan_codes: &[u16]) -> DownSendOutcome {
        if scan_codes.is_empty() {
            return DownSendOutcome::Complete {
                completed_us: crate::clock::qpc_now_us(),
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                send_attempts: 0,
                zero_progress_retries: 0,
                retried_after_zero_progress: false,
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
            return DownSendOutcome::Complete {
                completed_us: crate::clock::qpc_now_us(),
                sent: SmallVec::new(),
                skipped_duplicates: duplicates,
                send_attempts: 0,
                zero_progress_retries: 0,
                retried_after_zero_progress: false,
            };
        }

        for &sc in &to_send {
            self.possibly_active_mask |= key_mask(sc).unwrap_or(0);
        }

        let emitted = self.do_emit_down(&to_send);
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

        for &sc in &emitted.sent {
            self.active_mask |= key_mask(sc).unwrap_or(0);
        }

        for &sc in &to_send {
            self.possibly_active_mask &= !key_mask(sc).unwrap_or(0);
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
                inserted_prefix: emitted.keys_inserted_before_failure,
                rolled_back: emitted.keys_rolled_back,
                rollback_residue: emitted.rollback_residue_keys,
                first_error: emitted.first_win32_error,
                last_error: emitted.last_win32_error,
                completed_us: emitted.completed_us,
                sent: emitted.sent,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
            }
        } else if !emitted.success {
            DownSendOutcome::ZeroProgress {
                error: emitted.last_win32_error.or(emitted.first_win32_error),
                completed_us: emitted.completed_us,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                first_error: emitted.first_win32_error,
                last_error: emitted.last_win32_error,
            }
        } else {
            DownSendOutcome::Complete {
                completed_us: emitted.completed_us,
                sent: emitted.sent,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                retried_after_zero_progress: emitted.retried_after_zero_progress,
            }
        }
    }

    pub fn key_up(&mut self, scan_codes: &[u16]) -> InputSendResult {
        if scan_codes.is_empty() {
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                success: true,
                error: None,
                send_completed_us: crate::clock::qpc_now_us(),
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
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: already_released,
                success: true,
                error: None,
                send_completed_us: crate::clock::qpc_now_us(),
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
            };
        }

        let emitted = self.do_emit_up(&to_release);

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
        }
    }

    pub fn release_all(&mut self) -> ReleaseAllOutcome {
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
        if self.custom_emitter.is_none() {
            for &scan_code in &attempted {
                match is_scan_code_physically_down(scan_code) {
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
                    is_scan_code_physically_down(scan_code).unwrap_or_else(|| {
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

        if released_successfully && stuck.is_empty() {
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

    pub fn release_all_full_instrument(&mut self) -> ReleaseAllOutcome {
        let _tracked_outcome = self.release_all();
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

        if self.custom_emitter.is_none() {
            for &scan_code in &attempted {
                match is_scan_code_physically_down(scan_code) {
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
                    is_scan_code_physically_down(*scan_code).unwrap_or_else(|| {
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

        if release_successful && stuck.is_empty() {
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
            completed_us,
            win32_error: 0,
        }
    }

    #[test]
    fn down_retry_matrix_is_exact_and_clamped() {
        for (script, expected_sent, expected_dropped, expected_calls, expected_success) in [
            (vec![3], 3, 0, 1, true),
            // A partial first insertion is rolled back; the remainder is
            // never emitted as a second note-on chord.
            (vec![2, 1], 1, 1, 2, false),
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
    fn partial_note_on_marks_integrity_loss_and_rolls_back_prefix() {
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
    fn up_retry_matrix_is_immediate_and_bounded() {
        for (script, expected_sent, expected_calls, expected_success) in [
            (vec![3], 3, 1, true),
            (vec![1, 2], 3, 2, true),
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
    fn structured_win32_errors_survive_down_retry() {
        let mut calls = 0;
        let emitted = emit_down_with(&[2, 3], |codes, _| {
            calls += 1;
            PlatformSendResult {
                requested: codes.len() as u32,
                inserted: 1,
                completed_us: calls,
                win32_error: if calls == 1 { 5 } else { 0 },
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
                completed_us: calls,
                win32_error: if calls == 1 { 5 } else { 1460 },
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
                completed_us: 1,
                win32_error: 5,
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
    fn instrument_scan_codes_and_virtual_key_mapping_match_python_layout() {
        assert_eq!(
            PHYSICAL_INSTRUMENT_SCAN_CODES,
            [
                0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34,
                0x35,
            ]
        );
        let virtual_keys: Vec<i32> = PHYSICAL_INSTRUMENT_SCAN_CODES
            .iter()
            .map(|&scan_code| virtual_key_for_scan_code(scan_code).unwrap())
            .collect();
        assert_eq!(
            virtual_keys,
            [
                0x59, 0x55, 0x49, 0x4F, 0x50, 0x48, 0x4A, 0x4B, 0x4C, 0xBA, 0x4E, 0x4D, 0xBC, 0xBE,
                0xBF,
            ]
        );
    }

    #[test]
    fn full_instrument_release_reports_unreleased_keys() {
        let mut state = TrackedKeyState::with_emitter(|codes, key_up| PlatformSendResult {
            requested: codes.len() as u32,
            inserted: if key_up { 0 } else { codes.len() as u32 },
            completed_us: 10,
            win32_error: 5,
        });
        let outcome = state.release_all_full_instrument();
        assert!(!outcome.released_successfully);
        assert_eq!(outcome.attempted, PHYSICAL_INSTRUMENT_SCAN_CODES);
        assert_eq!(outcome.stuck_keys, PHYSICAL_INSTRUMENT_SCAN_CODES);
        assert_eq!(state.failed_release_mask.count_ones(), 15);
    }
}
