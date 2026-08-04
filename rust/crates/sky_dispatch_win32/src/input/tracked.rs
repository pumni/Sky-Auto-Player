use super::*;
use crate::clock::QpcClock;
use smallvec::SmallVec;
use std::fmt;

/// Emit a note-off without delaying the real-time worker.
///
/// A partial `SendInput` result gets one immediate retry of the whole
/// requested set. Any delayed retry belongs to the coordinator, which can then enter an
/// interruptible recovery pause instead of blocking command handling inside
/// the platform seam.
pub(crate) fn emit_up_with_immediate<F>(scan_codes: &[u16], mut send_fn: F) -> EmitResult
where
    F: FnMut(&[u16], bool) -> PlatformSendResult,
{
    if scan_codes.is_empty() {
        let (started_ticks, completed_ticks, completed_us, timing_error) =
            no_syscall_boundary_with_clock(None);
        return EmitResult {
            sent: SmallVec::new(),
            completed_us,
            started_ticks,
            completed_ticks,
            success: timing_error.is_none(),
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
            timing_error,
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
            started_ticks: Some(first.started_ticks),
            completed_ticks: first.completed_ticks,
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
            timing_error: first.timing_error,
        };
    }

    // A partial Up is also uncertain: retry the entire requested set instead
    // of assuming SendInput's inserted count identifies a prefix.
    let second = send_fn(scan_codes, true);
    let second_inserted = (second.inserted as usize).min(n);
    let success = second_inserted >= n;
    let second_win32_error = (second.win32_error != 0).then_some(second.win32_error);
    let last_win32_error = second_win32_error.or(first_win32_error);
    EmitResult {
        sent: if success {
            scan_codes.iter().copied().collect()
        } else {
            SmallVec::new()
        },
        completed_us: second.completed_us,
        started_ticks: Some(first.started_ticks),
        completed_ticks: second.completed_ticks,
        success,
        keys_dropped: u64::from(!success),
        first_win32_error: first_win32_error.or(second_win32_error),
        last_win32_error,
        send_attempts: 2,
        zero_progress_retries: u8::from(first_inserted == 0),
        first_inserted: first_inserted as u8,
        partial_progress: (first_inserted > 0 || second_inserted > 0) && !success,
        retried_after_zero_progress: first_inserted == 0,
        chord_integrity_lost: false,
        keys_inserted_before_failure: if success {
            0
        } else {
            first_inserted.max(second_inserted) as u8
        },
        keys_rolled_back: 0,
        rollback_residue_keys: 0,
        timing_error: first.timing_error.or(second.timing_error),
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

#[cfg(any(test, feature = "test-support"))]
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
    pub timing_error: Option<crate::clock::QpcError>,
    #[cfg(any(test, feature = "test-support"))]
    pub custom_emitter: Option<CustomEmitterFn>,
    qpc_clock: Option<QpcClock>,
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
            .field("timing_error", &self.timing_error)
            .field("qpc_clock_configured", &self.qpc_clock.is_some())
            .finish()
    }
}

impl TrackedKeyState {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_emitter<F>(emitter: F) -> Self
    where
        F: Fn(&[u16], bool) -> PlatformSendResult + Send + Sync + 'static,
    {
        Self {
            custom_emitter: Some(Box::new(emitter)),
            ..Default::default()
        }
    }

    pub fn with_qpc_clock(clock: QpcClock) -> Self {
        Self {
            qpc_clock: Some(clock),
            ..Default::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn uses_custom_emitter(&self) -> bool {
        self.custom_emitter.is_some()
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn uses_custom_emitter(&self) -> bool {
        false
    }

    /// Admit a real playback start/resume only when the user is not holding an
    /// instrument key. Mock emitters do not represent physical keyboard state,
    /// so they are explicitly exempt from this host preflight.
    pub fn ensure_instrument_keys_physically_up(
        &self,
        target_hwnd: isize,
    ) -> Result<(), PhysicalKeyPreflightError> {
        if self.uses_custom_emitter() {
            return Ok(());
        }
        if target_hwnd == 0 {
            return Err(PhysicalKeyPreflightError::VerificationInconclusive);
        }
        match instrument_physical_state_for_mask(target_hwnd, FULL_INSTRUMENT_MASK) {
            InstrumentPhysicalState::AllUp => Ok(()),
            InstrumentPhysicalState::Held(held) => {
                Err(PhysicalKeyPreflightError::UserHeld(held.into_vec()))
            }
            InstrumentPhysicalState::Inconclusive => {
                Err(PhysicalKeyPreflightError::VerificationInconclusive)
            }
        }
    }

    fn do_emit_down(&mut self, scan_codes: &[u16]) -> EmitResult {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_down_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_down_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_down(scan_codes)
        }
    }

    fn do_emit_up(&mut self, scan_codes: &[u16]) -> EmitResult {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_up_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_up_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_up(scan_codes)
        }
    }

    pub fn key_down(&mut self, scan_codes: &[u16]) -> DownSendOutcome {
        if scan_codes.is_empty() {
            let (started_ticks, completed_ticks, completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return DownSendOutcome::Complete {
                completed_us,
                started_ticks,
                completed_ticks,
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                send_attempts: 0,
                zero_progress_retries: 0,
                retried_after_zero_progress: false,
                timing_error,
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
            let (started_ticks, completed_ticks, completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return DownSendOutcome::Complete {
                completed_us,
                started_ticks,
                completed_ticks,
                sent: SmallVec::new(),
                skipped_duplicates: duplicates,
                send_attempts: 0,
                zero_progress_retries: 0,
                retried_after_zero_progress: false,
                timing_error,
            };
        }

        for &sc in &to_send {
            self.possibly_active_mask |= key_mask(sc).unwrap_or(0);
        }

        let emitted = self.do_emit_down(&to_send);
        self.timing_error = emitted.timing_error;
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

        if emitted.chord_integrity_lost {
            // SendInput's inserted count is not a trustworthy prefix receipt.
            // Keep the complete chord possibly active until full cleanup has
            // verified that every requested key is up.
            for &sc in &to_send {
                let bit = key_mask(sc).unwrap_or(0);
                self.active_mask &= !bit;
                self.possibly_active_mask |= bit;
            }
        } else {
            for &sc in &emitted.sent {
                self.active_mask |= key_mask(sc).unwrap_or(0);
            }

            for &sc in &to_send {
                self.possibly_active_mask &= !key_mask(sc).unwrap_or(0);
            }
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
                inserted_before_failure: emitted.keys_inserted_before_failure,
                rolled_back: emitted.keys_rolled_back,
                rollback_residue: emitted.rollback_residue_keys,
                first_error: emitted.first_win32_error,
                last_error: emitted.last_win32_error,
                completed_us: emitted.completed_us,
                started_ticks: emitted.started_ticks,
                completed_ticks: emitted.completed_ticks,
                sent: emitted.sent,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                timing_error: emitted.timing_error,
            }
        } else if !emitted.success {
            DownSendOutcome::ZeroProgress {
                error: emitted.last_win32_error.or(emitted.first_win32_error),
                completed_us: emitted.completed_us,
                started_ticks: emitted.started_ticks,
                completed_ticks: emitted.completed_ticks,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                first_error: emitted.first_win32_error,
                last_error: emitted.last_win32_error,
                timing_error: emitted.timing_error,
            }
        } else {
            DownSendOutcome::Complete {
                completed_us: emitted.completed_us,
                started_ticks: emitted.started_ticks,
                completed_ticks: emitted.completed_ticks,
                sent: emitted.sent,
                skipped_duplicates: duplicates,
                send_attempts: emitted.send_attempts,
                zero_progress_retries: emitted.zero_progress_retries,
                retried_after_zero_progress: emitted.retried_after_zero_progress,
                timing_error: emitted.timing_error,
            }
        }
    }

    pub fn key_up(&mut self, scan_codes: &[u16]) -> InputSendResult {
        if scan_codes.is_empty() {
            let (send_started_ticks, send_completed_ticks, send_completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: SmallVec::new(),
                success: timing_error.is_none(),
                error: None,
                send_completed_us,
                send_started_ticks,
                send_completed_ticks,
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
                timing_error,
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
            let (send_started_ticks, send_completed_ticks, send_completed_us, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return InputSendResult {
                sent: SmallVec::new(),
                skipped_duplicates: already_released,
                success: timing_error.is_none(),
                error: None,
                send_completed_us,
                send_started_ticks,
                send_completed_ticks,
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
                timing_error,
            };
        }

        let emitted = self.do_emit_up(&to_release);
        self.timing_error = emitted.timing_error;

        if emitted.partial_progress {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
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
            send_started_ticks: emitted.started_ticks,
            send_completed_ticks: emitted.completed_ticks,
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
            timing_error: emitted.timing_error,
        }
    }

    pub fn release_all(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
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
        if !self.uses_custom_emitter() {
            match instrument_physical_state_for_mask(target_hwnd, tracked_mask) {
                InstrumentPhysicalState::AllUp => {}
                InstrumentPhysicalState::Held(held) => {
                    for scan_code in held {
                        if attempted.contains(&scan_code) {
                            stuck.push(scan_code);
                        }
                    }
                }
                InstrumentPhysicalState::Inconclusive => verification_inconclusive = true,
            }
        }

        if !stuck.is_empty() {
            for delay_ms in [50, 100] {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let _ = self.do_emit_up(&stuck);
                let retry_mask = match mask_for_scan_codes(&stuck) {
                    Some(mask) => mask,
                    None => {
                        verification_inconclusive = true;
                        break;
                    }
                };
                match instrument_physical_state_for_mask(target_hwnd, retry_mask) {
                    InstrumentPhysicalState::AllUp => stuck.clear(),
                    InstrumentPhysicalState::Held(held) => {
                        stuck.retain(|scan_code| held.contains(scan_code));
                    }
                    InstrumentPhysicalState::Inconclusive => {
                        verification_inconclusive = true;
                    }
                }
                if stuck.is_empty() {
                    released_successfully = true;
                    break;
                }
            }
        }

        if released_successfully && stuck.is_empty() && !verification_inconclusive {
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

    pub fn release_all_full_instrument(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        let _tracked_outcome = self.release_all(target_hwnd);
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

        if !self.uses_custom_emitter() {
            match instrument_physical_state_for_mask(target_hwnd, FULL_INSTRUMENT_MASK) {
                InstrumentPhysicalState::AllUp => {}
                InstrumentPhysicalState::Held(held) => {
                    for scan_code in held {
                        if !stuck.contains(&scan_code) {
                            stuck.push(scan_code);
                        }
                    }
                }
                InstrumentPhysicalState::Inconclusive => verification_inconclusive = true,
            }
        }

        if !stuck.is_empty() {
            for delay_ms in [50, 100] {
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                let retry_sent = self.do_emit_up(&stuck).sent;
                if self.uses_custom_emitter() {
                    stuck.retain(|scan_code| !retry_sent.contains(scan_code));
                } else {
                    let retry_mask = match mask_for_scan_codes(&stuck) {
                        Some(mask) => mask,
                        None => {
                            verification_inconclusive = true;
                            break;
                        }
                    };
                    match instrument_physical_state_for_mask(target_hwnd, retry_mask) {
                        InstrumentPhysicalState::AllUp => {
                            stuck.retain(|scan_code| !retry_sent.contains(scan_code))
                        }
                        InstrumentPhysicalState::Held(held) => stuck.retain(|scan_code| {
                            !retry_sent.contains(scan_code) || held.contains(scan_code)
                        }),
                        InstrumentPhysicalState::Inconclusive => {
                            verification_inconclusive = true;
                        }
                    }
                }
                if stuck.is_empty() {
                    release_successful = true;
                    break;
                }
            }
        }

        if release_successful && stuck.is_empty() && !verification_inconclusive {
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
