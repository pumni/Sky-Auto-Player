//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

use smallvec::SmallVec;
use std::collections::HashSet;
use std::fmt;

pub const SKY_PLAYER_SIGNATURE: usize = 0x5C1B9111;

pub const PHYSICAL_INSTRUMENT_SCAN_CODES: [u16; 15] = [
    2, 3, 4, 5, 6, // 1 2 3 4 5
    16, 17, 18, 19, 20, // Q W E R T
    30, 31, 32, 33, 34, // A S D F G
];

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAllOutcome {
    pub attempted: Vec<u16>,
    pub released_successfully: bool,
    pub stuck_keys: Vec<u16>,
    pub verification_inconclusive: bool,
}

#[cfg(windows)]
pub fn create_keyboard_input(
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

pub fn send_input_raw(scan_codes: &[u16], key_up: bool) -> PlatformSendResult {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};

        if scan_codes.is_empty() {
            return PlatformSendResult {
                requested: 0,
                inserted: 0,
                completed_us: crate::clock::qpc_now_us(),
                win32_error: 0,
            };
        }

        let packets: Vec<INPUT> = scan_codes
            .iter()
            .map(|&sc| create_keyboard_input(sc, key_up))
            .collect();
        let requested = packets.len() as u32;
        let cb_size = std::mem::size_of::<INPUT>() as i32;

        let inserted = unsafe { SendInput(requested, packets.as_ptr(), cb_size) };
        let completed_us = crate::clock::qpc_now_us();
        let win32_error = if inserted == 0 && requested > 0 {
            unsafe { windows_sys::Win32::Foundation::GetLastError() }
        } else {
            0
        };

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

pub fn emit_down_with<F>(
    scan_codes: &[u16],
    mut send_fn: F,
) -> (SmallVec<[u16; 15]>, u64, bool, u64)
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        return (SmallVec::new(), crate::clock::qpc_now_us(), true, 0);
    }
    let n = scan_codes.len();
    let res1 = send_fn(scan_codes, false);
    let landed1 = res1.inserted as usize;

    if landed1 >= n {
        let sent: SmallVec<[u16; 15]> = scan_codes.iter().copied().collect();
        return (sent, res1.completed_us, true, 0);
    }

    // Call 2: immediate remainder retry
    let remainder = &scan_codes[landed1..];
    let res2 = send_fn(remainder, false);
    let landed2 = res2.inserted as usize;

    let total_landed = landed1 + landed2;
    let success = total_landed == n;
    let keys_dropped = (n - total_landed) as u64;

    let sent: SmallVec<[u16; 15]> = scan_codes[..total_landed].iter().copied().collect();
    (sent, res2.completed_us, success, keys_dropped)
}

pub fn emit_down(scan_codes: &[u16]) -> (SmallVec<[u16; 15]>, u64, bool, u64) {
    emit_down_with(scan_codes, send_input_raw)
}

pub fn emit_up_with<F>(scan_codes: &[u16], mut send_fn: F) -> (SmallVec<[u16; 15]>, u64, bool)
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        return (SmallVec::new(), crate::clock::qpc_now_us(), true);
    }
    let n = scan_codes.len();
    let mut sent_total = 0;
    let mut zero_progress_count = 0;
    let mut last_completed_us = crate::clock::qpc_now_us();

    while sent_total < n {
        let remainder = &scan_codes[sent_total..];
        let res = send_fn(remainder, true);
        last_completed_us = res.completed_us;
        let inserted = res.inserted as usize;

        if inserted > 0 {
            sent_total += inserted;
            zero_progress_count = 0;
        } else {
            zero_progress_count += 1;
            if zero_progress_count >= 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }

    let success = sent_total == n;
    let sent: SmallVec<[u16; 15]> = scan_codes[..sent_total].iter().copied().collect();
    (sent, last_completed_us, success)
}

pub fn emit_up(scan_codes: &[u16]) -> (SmallVec<[u16; 15]>, u64, bool) {
    emit_up_with(scan_codes, send_input_raw)
}

pub type CustomEmitterFn = Box<dyn Fn(&[u16], bool) -> PlatformSendResult + Send + Sync>;

#[derive(Default)]
pub struct TrackedKeyState {
    pub active_keys: HashSet<u16>,
    pub possibly_active_keys: HashSet<u16>,
    pub failed_release_keys: HashSet<u16>,
    pub last_error: Option<String>,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub custom_emitter: Option<CustomEmitterFn>,
}

impl fmt::Debug for TrackedKeyState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackedKeyState")
            .field("active_keys", &self.active_keys)
            .field("possibly_active_keys", &self.possibly_active_keys)
            .field("failed_release_keys", &self.failed_release_keys)
            .field("last_error", &self.last_error)
            .field("keys_dropped", &self.keys_dropped)
            .field("chord_split_events", &self.chord_split_events)
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

    fn do_emit_down(&mut self, scan_codes: &[u16]) -> (SmallVec<[u16; 15]>, u64, bool, u64) {
        if let Some(ref emitter) = self.custom_emitter {
            emit_down_with(scan_codes, |sc, key_up| emitter(sc, key_up))
        } else {
            emit_down(scan_codes)
        }
    }

    fn do_emit_up(&mut self, scan_codes: &[u16]) -> (SmallVec<[u16; 15]>, u64, bool) {
        if let Some(ref emitter) = self.custom_emitter {
            emit_up_with(scan_codes, |sc, key_up| emitter(sc, key_up))
        } else {
            emit_up(scan_codes)
        }
    }

    pub fn key_down(&mut self, scan_codes: &[u16]) -> InputSendResult {
        if scan_codes.is_empty() {
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                success: true,
                error: None,
                send_completed_us: crate::clock::qpc_now_us(),
            };
        }

        let mut to_send: SmallVec<[u16; 15]> = SmallVec::new();
        let mut duplicates: SmallVec<[u16; 15]> = SmallVec::new();

        for &sc in scan_codes {
            if self.active_keys.contains(&sc) {
                duplicates.push(sc);
            } else {
                to_send.push(sc);
            }
        }

        if to_send.is_empty() {
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: duplicates,
                success: true,
                error: None,
                send_completed_us: crate::clock::qpc_now_us(),
            };
        }

        for &sc in &to_send {
            self.possibly_active_keys.insert(sc);
        }

        let (sent, completed_us, success, dropped) = self.do_emit_down(&to_send);
        self.keys_dropped += dropped;

        for &sc in &sent {
            self.active_keys.insert(sc);
        }

        for &sc in &to_send {
            self.possibly_active_keys.remove(&sc);
        }

        if !success && !sent.is_empty() {
            self.chord_split_events += 1;
        }

        InputSendResult {
            sent,
            skipped_duplicates: duplicates,
            success,
            error: if success {
                None
            } else {
                Some(format!("partial note-on: {}/{}", dropped, to_send.len()))
            },
            send_completed_us: completed_us,
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
            };
        }

        let mut to_release: SmallVec<[u16; 15]> = SmallVec::new();
        let mut already_released: SmallVec<[u16; 15]> = SmallVec::new();

        for &sc in scan_codes {
            if self.active_keys.contains(&sc) || self.possibly_active_keys.contains(&sc) {
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
            };
        }

        let (sent, completed_us, success) = self.do_emit_up(&to_release);

        for &sc in &sent {
            self.active_keys.remove(&sc);
            self.possibly_active_keys.remove(&sc);
            self.failed_release_keys.remove(&sc);
        }

        if !success {
            for &sc in &to_release {
                if !sent.contains(&sc) {
                    self.failed_release_keys.insert(sc);
                }
            }
            self.last_error = Some(format!(
                "partial note-off: {}/{}",
                sent.len(),
                to_release.len()
            ));
        }

        InputSendResult {
            sent,
            skipped_duplicates: already_released,
            success,
            error: if success {
                None
            } else {
                Some("partial note-off".to_string())
            },
            send_completed_us: completed_us,
        }
    }

    pub fn release_all(&mut self) -> ReleaseAllOutcome {
        let mut to_release_set: HashSet<u16> = HashSet::new();
        to_release_set.extend(&self.active_keys);
        to_release_set.extend(&self.possibly_active_keys);
        to_release_set.extend(&self.failed_release_keys);

        if to_release_set.is_empty() {
            return ReleaseAllOutcome {
                attempted: Vec::new(),
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive: false,
            };
        }

        let mut attempted: Vec<u16> = to_release_set.into_iter().collect();
        attempted.sort_unstable();

        let mut released_successfully = false;

        for pass_idx in 0..3 {
            let (sent, _ts, success) = self.do_emit_up(&attempted);
            if success && sent.len() == attempted.len() {
                released_successfully = true;
                break;
            }
            if pass_idx < 2 {
                std::thread::sleep(std::time::Duration::from_millis(15));
            }
        }

        if released_successfully {
            self.active_keys.clear();
            self.possibly_active_keys.clear();
            self.failed_release_keys.clear();
            ReleaseAllOutcome {
                attempted,
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive: false,
            }
        } else {
            self.failed_release_keys.extend(&attempted);
            let mut stuck: Vec<u16> = self.failed_release_keys.iter().copied().collect();
            stuck.sort_unstable();
            ReleaseAllOutcome {
                attempted,
                released_successfully: false,
                stuck_keys: stuck,
                verification_inconclusive: true,
            }
        }
    }

    pub fn release_all_full_instrument(&mut self) -> ReleaseAllOutcome {
        let outcome = self.release_all();
        let _ = self.do_emit_up(&PHYSICAL_INSTRUMENT_SCAN_CODES);
        outcome
    }
}
