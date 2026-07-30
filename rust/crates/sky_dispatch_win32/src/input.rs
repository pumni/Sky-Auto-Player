//! Windows SendInput API wrappers, input packet prewarming, and tracked key backend.

use smallvec::SmallVec;
use std::collections::HashSet;
use std::fmt;

pub const SKY_PLAYER_SIGNATURE: usize = 0x5C1B9111;

pub const PHYSICAL_INSTRUMENT_SCAN_CODES: [u16; 15] = [
    0x15, 0x16, 0x17, 0x18, 0x19, // Y U I O P
    0x23, 0x24, 0x25, 0x26, 0x27, // H J K L ;
    0x31, 0x32, 0x33, 0x34, 0x35, // N M , . /
];

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

        let packets: SmallVec<[INPUT; 15]> = scan_codes
            .iter()
            .map(|&sc| create_keyboard_input(sc, key_up))
            .collect();
        let requested = packets.len() as u32;
        let cb_size = std::mem::size_of::<INPUT>() as i32;

        // SAFETY: `packets` owns `requested` contiguous, correctly aligned INPUT
        // values and remains alive and immobile for the duration of SendInput.
        // `requested` is bounded to 15 by the validated caller.
        let inserted = unsafe { SendInput(requested, packets.as_ptr(), cb_size) }.min(requested);
        let completed_us = crate::clock::qpc_now_us();
        let win32_error = if inserted != requested {
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
    let landed1 = (res1.inserted as usize).min(n);

    if landed1 >= n {
        let sent: SmallVec<[u16; 15]> = scan_codes.iter().copied().collect();
        return (sent, res1.completed_us, true, 0);
    }

    // Call 2: immediate remainder retry
    let remainder = &scan_codes[landed1..];
    let res2 = send_fn(remainder, false);
    let landed2 = (res2.inserted as usize).min(remainder.len());

    let total_landed = landed1 + landed2;
    let success = total_landed == n;
    let keys_dropped = (n - total_landed) as u64;

    let sent: SmallVec<[u16; 15]> = scan_codes[..total_landed].iter().copied().collect();
    (sent, res2.completed_us, success, keys_dropped)
}

pub fn emit_down(scan_codes: &[u16]) -> (SmallVec<[u16; 15]>, u64, bool, u64) {
    emit_down_with(scan_codes, send_input_raw)
}

fn emit_up_with_and_sleep<F, S>(
    scan_codes: &[u16],
    mut send_fn: F,
    mut sleep_fn: S,
) -> (SmallVec<[u16; 15]>, u64, bool)
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
    S: FnMut(),
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
        let inserted = (res.inserted as usize).min(remainder.len());

        if inserted > 0 {
            sent_total += inserted;
            zero_progress_count = 0;
        } else {
            zero_progress_count += 1;
            if zero_progress_count >= 3 {
                break;
            }
            sleep_fn();
        }
    }

    let success = sent_total == n;
    let sent: SmallVec<[u16; 15]> = scan_codes[..sent_total].iter().copied().collect();
    (sent, last_completed_us, success)
}

pub fn emit_up_with<F>(scan_codes: &[u16], send_fn: F) -> (SmallVec<[u16; 15]>, u64, bool)
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    emit_up_with_and_sleep(scan_codes, send_fn, || {
        std::thread::sleep(std::time::Duration::from_millis(2));
    })
}

pub fn emit_up(scan_codes: &[u16]) -> (SmallVec<[u16; 15]>, u64, bool) {
    emit_up_with(scan_codes, send_input_raw)
}

pub type CustomEmitterFn = Box<dyn Fn(&[u16], bool) -> PlatformSendResult + Send + Sync>;

pub struct TrackedKeyState {
    pub active_keys: HashSet<u16>,
    pub possibly_active_keys: HashSet<u16>,
    pub failed_release_keys: HashSet<u16>,
    pub last_error: Option<String>,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub custom_emitter: Option<CustomEmitterFn>,
}

impl Default for TrackedKeyState {
    fn default() -> Self {
        Self {
            active_keys: HashSet::with_capacity(15),
            possibly_active_keys: HashSet::with_capacity(15),
            failed_release_keys: HashSet::with_capacity(15),
            last_error: None,
            keys_dropped: 0,
            chord_split_events: 0,
            custom_emitter: None,
        }
    }
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
        if success {
            if self.failed_release_keys.is_empty() {
                self.last_error = None;
            }
        } else {
            self.last_error = Some(format!(
                "partial note-on: {} of {} keys dropped",
                dropped,
                to_send.len()
            ));
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
            if self.active_keys.contains(&sc)
                || self.possibly_active_keys.contains(&sc)
                || self.failed_release_keys.contains(&sc)
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
        } else if self.failed_release_keys.is_empty() {
            self.last_error = None;
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
            self.active_keys.clear();
            self.possibly_active_keys.clear();
            self.failed_release_keys.clear();
            self.last_error = None;
            return ReleaseAllOutcome {
                attempted,
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive,
            };
        }

        if stuck.is_empty() {
            self.failed_release_keys.extend(&attempted);
        } else {
            self.failed_release_keys.extend(&stuck);
        }
        self.last_error = Some("tracked release incomplete".to_string());
        let mut reported_stuck: Vec<u16> = self.failed_release_keys.iter().copied().collect();
        reported_stuck.sort_unstable();
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
        let (sent, _completed_us, send_success) = self.do_emit_up(&attempted);
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
                let (retry_sent, _, _) = self.do_emit_up(&stuck);
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
            self.active_keys.clear();
            self.possibly_active_keys.clear();
            self.failed_release_keys.clear();
            self.last_error = None;
            return ReleaseAllOutcome {
                attempted,
                released_successfully: true,
                stuck_keys: Vec::new(),
                verification_inconclusive,
            };
        }

        self.failed_release_keys.extend(&stuck);
        self.last_error = Some(format!(
            "full-instrument release incomplete: {}/{} keys unresolved",
            stuck.len(),
            attempted.len()
        ));
        let mut reported_stuck: Vec<u16> = self.failed_release_keys.iter().copied().collect();
        reported_stuck.sort_unstable();
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
        for (script, expected_sent, expected_calls, expected_success) in [
            (vec![3], 3, 1, true),
            (vec![2, 1], 3, 2, true),
            (vec![0, 0], 0, 2, false),
            (vec![1, 1], 2, 2, false),
            (vec![99], 3, 1, true),
        ] {
            let mut returns = VecDeque::from(script);
            let mut calls = 0;
            let (sent, _, success, dropped) = emit_down_with(&[2, 3, 4], |codes, _| {
                calls += 1;
                scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
            });
            assert_eq!(sent.len(), expected_sent);
            assert_eq!(calls, expected_calls);
            assert_eq!(success, expected_success);
            assert_eq!(dropped, (3 - expected_sent) as u64);
        }
    }

    #[test]
    fn up_retry_matrix_resets_progress_and_bounds_zero_progress() {
        for (script, expected_sent, expected_calls, expected_sleeps, expected_success) in [
            (vec![1, 1, 1], 3, 3, 0, true),
            (vec![0, 1, 2], 3, 3, 1, true),
            (vec![0, 0, 0], 0, 3, 2, false),
            (vec![99], 3, 1, 0, true),
        ] {
            let mut returns = VecDeque::from(script);
            let mut calls = 0;
            let mut sleeps = 0;
            let (sent, _, success) = emit_up_with_and_sleep(
                &[2, 3, 4],
                |codes, _| {
                    calls += 1;
                    scripted_result(codes.len(), returns.pop_front().unwrap_or(0), calls)
                },
                || sleeps += 1,
            );
            assert_eq!(sent.len(), expected_sent);
            assert_eq!(calls, expected_calls);
            assert_eq!(sleeps, expected_sleeps);
            assert_eq!(success, expected_success);
        }
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
        assert_eq!(state.failed_release_keys.len(), 15);
    }
}
