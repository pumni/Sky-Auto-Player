use super::down_transaction::emit_down_once_with;
#[cfg(any(test, feature = "test-support"))]
use super::outcome::PlatformSendResult;
use super::outcome::{
    PacketRetryReason, PhysicalKeyPreflightError, PhysicalPacket, ReleaseAllOutcome, SendEvidence,
    SendTransactionOutcome, SendTransactionStatus,
};
use super::packet::send_physical_packet_once_with_clock;
use super::physical::{
    CleanupVerification, InstrumentPhysicalState, ReconciledRelease,
    instrument_physical_state_for_mask, mask_for_scan_codes, reconcile_release_observation,
};
use super::raw::{no_syscall_boundary_with_clock, send_input_raw, send_input_raw_with_clock};
use super::scan_code::{FULL_INSTRUMENT_MASK, key_mask, scan_codes_from_mask};
use super::up_transaction::emit_up_once_with;
use crate::clock::QpcClock;
use smallvec::SmallVec;
use std::fmt;

/// Emit a note-off without delaying the real-time worker.
#[cfg(any(test, feature = "test-support"))]
pub type CustomEmitterFn = Box<dyn Fn(&[u16], bool) -> PlatformSendResult + Send + Sync>;

#[cfg(any(test, feature = "test-support"))]
pub type CustomPacketEmitterFn =
    Box<dyn Fn(PhysicalPacket) -> SendTransactionOutcome + Send + Sync>;

/// Test-only deterministic physical probe used by the cleanup FSM.
///
/// The signature receives the still-unresolved mask and the transport-confirmed
/// mask so a test can model transport progress across retry attempts. When no
/// probe is installed, a custom emitter must never synthesize a physical
/// verdict; the probe resolves to Inconclusive (fail-closed).
#[cfg(any(test, feature = "test-support"))]
pub type CustomProbeFn = Box<dyn Fn(u16, u16) -> InstrumentPhysicalState + Send + Sync>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseScope {
    Tracked,
    FullInstrument,
}

#[cfg(test)]
pub(crate) static TEST_RELEASE_SLEEP_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(not(test))]
fn release_retry_sleep(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

#[cfg(test)]
fn release_retry_sleep(_ms: u64) {
    TEST_RELEASE_SLEEP_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

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
    #[cfg(any(test, feature = "test-support"))]
    pub custom_packet_emitter: Option<CustomPacketEmitterFn>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) custom_probe: Option<CustomProbeFn>,
    #[cfg(any(test, feature = "test-support"))]
    pub full_instrument_release_calls: u64,
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

    #[cfg(any(test, feature = "test-support"))]
    pub fn with_packet_emitter<F>(emitter: F) -> Self
    where
        F: Fn(PhysicalPacket) -> SendTransactionOutcome + Send + Sync + 'static,
    {
        Self {
            custom_packet_emitter: Some(Box::new(emitter)),
            ..Default::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_emitter<F>(&mut self, emitter: F)
    where
        F: Fn(&[u16], bool) -> PlatformSendResult + Send + Sync + 'static,
    {
        self.custom_emitter = Some(Box::new(emitter));
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_packet_emitter<F>(&mut self, emitter: F)
    where
        F: Fn(PhysicalPacket) -> SendTransactionOutcome + Send + Sync + 'static,
    {
        self.custom_packet_emitter = Some(Box::new(emitter));
    }

    /// Install deterministic success test emitters for both single scan code and packet paths.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_test_emitters(&mut self) {
        let clock = self.qpc_clock;
        self.custom_emitter = Some(Box::new(move |scan_codes, _key_up| PlatformSendResult {
            requested: scan_codes.len() as u8,
            inserted: scan_codes.len() as u8,
            started_ticks: clock
                .and_then(|c| c.now().ok())
                .unwrap_or(crate::clock::QpcTicks::ZERO),
            completed_ticks: clock.and_then(|c| c.now().ok()),
            win32_error: 0,
            timing_error: None,
        }));
        self.custom_packet_emitter = Some(Box::new(move |packet| SendTransactionOutcome {
            status: SendTransactionStatus::Complete,
            evidence: SendEvidence {
                requested_mask: packet.up_mask | packet.down_mask,
                confirmed_mask: packet.up_mask | packet.down_mask,
                skipped_mask: 0,
                first_inserted: packet.event_count(),
                attempts: 1,
                zero_progress_retries: 0,
                retry_reason: PacketRetryReason::None,
                first_win32_error: None,
                last_win32_error: None,
                started_ticks: clock.and_then(|c| c.now().ok()),
                completed_ticks: clock.and_then(|c| c.now().ok()),
                timing_error: None,
            },
        }));
    }

    /// Install a deterministic physical probe for the cleanup FSM.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_probe<F>(&mut self, probe: F)
    where
        F: Fn(u16, u16) -> InstrumentPhysicalState + Send + Sync + 'static,
    {
        self.custom_probe = Some(Box::new(probe));
    }

    pub fn with_qpc_clock(clock: QpcClock) -> Self {
        Self {
            qpc_clock: Some(clock),
            ..Default::default()
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    fn uses_custom_emitter(&self) -> bool {
        self.custom_emitter.is_some() || self.custom_packet_emitter.is_some()
    }

    #[cfg(not(any(test, feature = "test-support")))]
    fn uses_custom_emitter(&self) -> bool {
        false
    }

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

    fn do_emit_down(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_down_once_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_down_once_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_down_once_with(scan_codes, send_input_raw)
        }
    }

    fn do_emit_up(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_up_once_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_up_once_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_up_once_with(scan_codes, send_input_raw)
        }
    }

    /// Single-send note-off for operator-owned cleanup retries. The cleanup FSM
    /// bounds the raw `SendInput` count itself; this must never perform an
    /// internal retry.
    fn do_emit_up_once(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(ref emitter) = self.custom_emitter {
            return emit_up_once_with(scan_codes, |sc, key_up| emitter(sc, key_up));
        }
        if let Some(clock) = self.qpc_clock {
            emit_up_once_with(scan_codes, |sc, key_up| {
                send_input_raw_with_clock(sc, key_up, clock)
            })
        } else {
            emit_up_once_with(scan_codes, send_input_raw)
        }
    }

    /// Resolve the physical probe for the cleanup FSM.
    ///
    /// A test-only `custom_probe` closure provides deterministic evidence keyed
    /// on the unresolved and transport-confirmed masks. When no probe is
    /// installed, a simulated transport emitter must NOT be allowed to
    /// synthesize an AllUp/Held verdict from transport confirmation alone — the
    /// physical probe and transport evidence are independent dimensions.
    /// Without a probe the result is deliberately Inconclusive so a custom
    /// emitter can never invent all-up evidence.
    #[cfg(any(test, feature = "test-support"))]
    #[inline]
    fn resolve_release_probe(
        &self,
        target_hwnd: isize,
        unresolved_mask: u16,
        transport_confirmed_mask: u16,
    ) -> InstrumentPhysicalState {
        if let Some(probe) = &self.custom_probe {
            probe(unresolved_mask, transport_confirmed_mask)
        } else if self.uses_custom_emitter() {
            InstrumentPhysicalState::Inconclusive
        } else {
            #[cfg(windows)]
            {
                let _ = target_hwnd;
                instrument_physical_state_for_mask(target_hwnd, unresolved_mask)
            }
            #[cfg(not(windows))]
            {
                let _ = (target_hwnd, unresolved_mask);
                InstrumentPhysicalState::Inconclusive
            }
        }
    }

    #[cfg(not(any(test, feature = "test-support")))]
    #[inline]
    fn resolve_release_probe(
        &self,
        target_hwnd: isize,
        unresolved_mask: u16,
        _transport_confirmed_mask: u16,
    ) -> InstrumentPhysicalState {
        instrument_physical_state_for_mask(target_hwnd, unresolved_mask)
    }

    pub fn key_down(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        let requested_mask = mask_for_scan_codes(scan_codes).unwrap_or(0);
        if scan_codes.is_empty() || requested_mask == 0 {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask: 0,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks,
                    timing_error,
                },
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

        let skipped_mask = mask_for_scan_codes(&duplicates).unwrap_or(0);

        if to_send.is_empty() {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: requested_mask,
                    skipped_mask,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks,
                    timing_error,
                },
            };
        }

        for &sc in &to_send {
            self.possibly_active_mask |= key_mask(sc).unwrap_or(0);
        }

        let emitted = self.do_emit_down(&to_send);
        self.timing_error = emitted.evidence.timing_error;

        if matches!(
            emitted.status,
            SendTransactionStatus::PartialProgress | SendTransactionStatus::IntegrityLost
        ) {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
        }
        if matches!(emitted.status, SendTransactionStatus::ZeroProgress) {
            self.sendinput_zero_progress_failures =
                self.sendinput_zero_progress_failures.saturating_add(1);
        }

        let to_send_mask = mask_for_scan_codes(&to_send).unwrap_or(0);

        match emitted.status {
            SendTransactionStatus::Complete => {
                self.active_mask |= to_send_mask;
                self.possibly_active_mask &= !to_send_mask;
            }
            SendTransactionStatus::IntegrityLost => {
                self.active_mask &= !to_send_mask;
                self.possibly_active_mask |= to_send_mask;
                self.chord_split_events = self.chord_split_events.saturating_add(1);
            }
            SendTransactionStatus::ClockFailureAfterSend => {
                // SendInput may have inserted one or more keys, but the
                // completion boundary is unknown.  Keep ownership uncertain
                // so cleanup/recovery can include every requested key.
                self.active_mask &= !to_send_mask;
                self.possibly_active_mask |= to_send_mask;
            }
            SendTransactionStatus::ZeroProgress | SendTransactionStatus::ClockFailureBeforeSend => {
                self.possibly_active_mask &= !to_send_mask;
            }
            SendTransactionStatus::PartialProgress => {
                // The current Down primitive classifies an inserted partial
                // packet as IntegrityLost.  Keep this arm explicit so a
                // future transport classification cannot silently confirm it.
                self.active_mask &= !to_send_mask;
                self.possibly_active_mask |= to_send_mask;
                self.chord_split_events = self.chord_split_events.saturating_add(1);
            }
        }

        if !emitted.is_success() {
            self.chords_rejected = self.chords_rejected.saturating_add(1);
            self.authored_keys_rejected = self
                .authored_keys_rejected
                .saturating_add(to_send.len() as u64);
        }

        if emitted.is_success() {
            if self.failed_release_mask == 0 {
                self.last_error = None;
            }
        } else {
            self.last_error = Some(format!("note-on rejected; status={:?}", emitted.status,));
        }

        SendTransactionOutcome {
            status: emitted.status,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask: if emitted.is_success() {
                    requested_mask
                } else {
                    0
                },
                skipped_mask,
                ..emitted.evidence
            },
        }
    }

    /// Send one validated physical packet. The packet builder is the sole
    /// authored/release transport: it emits all Up events before all Down
    /// events in one `SendInput` call.
    pub fn send_physical_packet(&mut self, packet: PhysicalPacket) -> SendTransactionOutcome {
        let outcome = {
            #[cfg(any(test, feature = "test-support"))]
            if let Some(emitter) = self.custom_packet_emitter.as_ref() {
                emitter(packet)
            } else {
                let Some(clock) = self.qpc_clock else {
                    self.last_error = Some("packet sender has no QPC clock".to_string());
                    return SendTransactionOutcome {
                        status: SendTransactionStatus::ZeroProgress,
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
                    };
                };
                send_physical_packet_once_with_clock(packet, clock)
            }
            #[cfg(not(any(test, feature = "test-support")))]
            {
                let Some(clock) = self.qpc_clock else {
                    self.last_error = Some("packet sender has no QPC clock".to_string());
                    return SendTransactionOutcome {
                        status: SendTransactionStatus::ZeroProgress,
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
                    };
                };
                send_physical_packet_once_with_clock(packet, clock)
            }
        };

        let confirmed_mask = outcome.evidence.confirmed_mask;
        match outcome.status {
            SendTransactionStatus::Complete => {
                let union = packet.up_mask | packet.down_mask;
                self.active_mask = (self.active_mask & !packet.up_mask) | packet.down_mask;
                self.possibly_active_mask &= !union;
                self.failed_release_mask &= !packet.up_mask;
                if self.failed_release_mask == 0 {
                    self.last_error = None;
                }
            }
            SendTransactionStatus::ZeroProgress => {
                self.sendinput_zero_progress_failures =
                    self.sendinput_zero_progress_failures.saturating_add(1);
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet made zero progress: {} events requested",
                    packet.event_count()
                ));
            }
            SendTransactionStatus::PartialProgress | SendTransactionStatus::IntegrityLost => {
                let uncertain_mask = packet.up_mask | packet.down_mask;
                self.active_mask &= !uncertain_mask;
                self.possibly_active_mask |= uncertain_mask;
                self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
                self.chord_split_events = self.chord_split_events.saturating_add(1);
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet partially inserted: {} of {} events",
                    outcome.evidence.first_inserted,
                    packet.event_count()
                ));
            }
            SendTransactionStatus::ClockFailureBeforeSend
            | SendTransactionStatus::ClockFailureAfterSend => {
                self.timing_error = outcome.evidence.timing_error;
                if matches!(outcome.status, SendTransactionStatus::ClockFailureAfterSend) {
                    let uncertain_mask = packet.up_mask | packet.down_mask;
                    self.active_mask &= !uncertain_mask;
                    self.possibly_active_mask |= uncertain_mask;
                }
                self.chords_rejected = self.chords_rejected.saturating_add(1);
                self.authored_keys_rejected = self
                    .authored_keys_rejected
                    .saturating_add(u64::from(packet.down_mask.count_ones()));
                self.last_error = Some(format!(
                    "physical packet QPC failure ({:?})",
                    outcome.status
                ));
            }
        }
        if packet.up_mask != 0 && !outcome.is_success() {
            self.active_mask &= !confirmed_mask;
            self.possibly_active_mask &= !confirmed_mask;
            self.failed_release_mask &= !confirmed_mask;
            self.failed_release_mask |= packet.up_mask & !confirmed_mask;
        }
        outcome
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn key_down_physical_packet(&mut self, packet: PhysicalPacket) -> SendTransactionOutcome {
        self.send_physical_packet(packet)
    }

    pub fn key_up(&mut self, scan_codes: &[u16]) -> SendTransactionOutcome {
        let requested_mask = mask_for_scan_codes(scan_codes).unwrap_or(0);
        if scan_codes.is_empty() || requested_mask == 0 {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask: 0,
                    confirmed_mask: 0,
                    skipped_mask: 0,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks,
                    timing_error,
                },
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

        let skipped_mask = mask_for_scan_codes(&already_released).unwrap_or(0);

        if to_release.is_empty() {
            let (started_ticks, completed_ticks, timing_error) =
                no_syscall_boundary_with_clock(self.qpc_clock);
            return SendTransactionOutcome {
                status: SendTransactionStatus::Complete,
                evidence: SendEvidence {
                    requested_mask,
                    confirmed_mask: 0,
                    skipped_mask,
                    first_inserted: 0,
                    attempts: 0,
                    zero_progress_retries: 0,
                    retry_reason: PacketRetryReason::None,
                    first_win32_error: None,
                    last_win32_error: None,
                    started_ticks: Some(started_ticks),
                    completed_ticks,
                    timing_error,
                },
            };
        }

        let emitted = self.do_emit_up(&to_release);
        self.timing_error = emitted.evidence.timing_error;

        if matches!(emitted.status, SendTransactionStatus::PartialProgress) {
            self.sendinput_partial_events = self.sendinput_partial_events.saturating_add(1);
        }

        let confirmed_mask = emitted.evidence.confirmed_mask;
        self.active_mask &= !confirmed_mask;
        self.possibly_active_mask &= !confirmed_mask;
        self.failed_release_mask &= !confirmed_mask;

        if !emitted.is_success() {
            let unconfirmed_released =
                mask_for_scan_codes(&to_release).unwrap_or(0) & !confirmed_mask;
            self.failed_release_mask |= unconfirmed_released;
            self.last_error = Some("partial note-off".to_string());
        } else if self.failed_release_mask == 0 {
            self.last_error = None;
        }

        SendTransactionOutcome {
            status: emitted.status,
            evidence: SendEvidence {
                requested_mask,
                confirmed_mask,
                skipped_mask,
                ..emitted.evidence
            },
        }
    }

    pub fn release_scope(&mut self, scope: ReleaseScope, target_hwnd: isize) -> ReleaseAllOutcome {
        let requested_mask = match scope {
            ReleaseScope::Tracked => {
                self.active_mask | self.possibly_active_mask | self.failed_release_mask
            }
            ReleaseScope::FullInstrument => FULL_INSTRUMENT_MASK,
        };

        if requested_mask == 0 {
            return ReleaseAllOutcome {
                attempted_mask: 0,
                transport_anomaly: false,
                released_successfully: true,
                stuck_mask: 0,
                verification_inconclusive: false,
                attempts: 0,
            };
        }

        let mut unresolved_mask = requested_mask;
        const RELEASE_RETRY_DELAYS_MS: [u64; 3] = [15, 50, 100];

        // Independent evidence dimensions: `transport_anomaly` records that the
        // release path saw any non-clean transport outcome, while the typed
        // `verification` verdict records what the physical probe could decide.
        // `released_successfully` is only true when the probe verified AllUp.
        let mut transport_anomaly = false;
        let mut final_verification: Option<CleanupVerification> = None;

        for attempt_idx in 0..4 {
            if attempt_idx > 0 {
                release_retry_sleep(RELEASE_RETRY_DELAYS_MS[attempt_idx - 1]);
            }
            if unresolved_mask == 0 {
                break;
            }

            let previous_unresolved = unresolved_mask;
            let send_codes = scan_codes_from_mask(unresolved_mask);
            let emitted = self.do_emit_up_once(&send_codes);
            let transport_confirmed_mask = emitted.evidence.confirmed_mask;

            // Aggregate transport-anomaly evidence from this single-send note.
            transport_anomaly |= !matches!(emitted.status, SendTransactionStatus::Complete)
                || emitted.evidence.attempts > 1
                || emitted.evidence.retry_reason != PacketRetryReason::None
                || emitted.evidence.first_win32_error.is_some()
                || emitted.evidence.last_win32_error.is_some();

            let physical_state =
                self.resolve_release_probe(target_hwnd, unresolved_mask, transport_confirmed_mask);

            let reconciled = reconcile_release_observation(
                unresolved_mask,
                transport_confirmed_mask,
                physical_state,
            );

            // The typed verdict for this transition. The final outcome derives
            // exclusively from the last observation so the two evidence
            // dimensions never bleed into each other.
            final_verification = Some(match reconciled {
                ReconciledRelease::VerifiedAllUp => CleanupVerification::AllUp,
                ReconciledRelease::Held(held_mask) => CleanupVerification::Held(held_mask),
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    CleanupVerification::Inconclusive(unconfirmed_mask)
                }
            });

            // Transport-confirmed keys are released at the physical layer
            // regardless of what physical probing later determines. Clear them
            // from tracking state immediately rather than waiting for a final
            // VerifiedAllUp, so a narrowed unresolved set never orphans a key.
            let resolved_this_transition = match reconciled {
                ReconciledRelease::VerifiedAllUp => previous_unresolved,
                ReconciledRelease::Held(held_mask) => previous_unresolved & !held_mask,
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    previous_unresolved & !unconfirmed_mask
                }
            };
            self.active_mask &= !resolved_this_transition;
            self.possibly_active_mask &= !resolved_this_transition;
            self.failed_release_mask &= !resolved_this_transition;

            match reconciled {
                ReconciledRelease::VerifiedAllUp => {
                    if self.failed_release_mask == 0 {
                        self.last_error = None;
                    }
                    return ReleaseAllOutcome {
                        attempted_mask: requested_mask,
                        transport_anomaly,
                        released_successfully: true,
                        stuck_mask: 0,
                        verification_inconclusive: false,
                        attempts: (attempt_idx + 1) as u8,
                    };
                }
                ReconciledRelease::Held(held_mask) => {
                    unresolved_mask = held_mask;
                }
                ReconciledRelease::Inconclusive(unconfirmed_mask) => {
                    unresolved_mask = unconfirmed_mask;
                    if unconfirmed_mask == 0 {
                        self.last_error = Some(match scope {
                            ReleaseScope::Tracked => "tracked release unverified".to_string(),
                            ReleaseScope::FullInstrument => {
                                "full-instrument release unverified".to_string()
                            }
                        });
                        return ReleaseAllOutcome {
                            attempted_mask: requested_mask,
                            transport_anomaly,
                            released_successfully: false,
                            stuck_mask: 0,
                            verification_inconclusive: true,
                            attempts: (attempt_idx + 1) as u8,
                        };
                    }
                }
            }
        }

        // Final verdict: exactly one of the independent dimensions decides the
        // outcome. `verification_inconclusive` is probe-derived only and is
        // never OR-ed with `transport_anomaly`.
        let verification = final_verification.unwrap_or(CleanupVerification::Held(unresolved_mask));
        let released_successfully = verification.is_success();
        let verification_inconclusive = verification.is_inconclusive();
        let stuck_mask = if released_successfully {
            0
        } else {
            unresolved_mask
        };

        let error_msg = match scope {
            ReleaseScope::Tracked => "tracked release incomplete".to_string(),
            ReleaseScope::FullInstrument => format!(
                "full-instrument release incomplete: {}/{} keys unresolved",
                scan_codes_from_mask(unresolved_mask).len(),
                scan_codes_from_mask(requested_mask).len()
            ),
        };
        if !released_successfully {
            self.last_error = Some(error_msg);
            self.failed_release_mask |= stuck_mask;
        }

        ReleaseAllOutcome {
            attempted_mask: requested_mask,
            transport_anomaly,
            released_successfully,
            stuck_mask,
            verification_inconclusive,
            attempts: 4,
        }
    }

    pub fn release_all(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        self.release_scope(ReleaseScope::Tracked, target_hwnd)
    }

    pub fn release_all_full_instrument(&mut self, target_hwnd: isize) -> ReleaseAllOutcome {
        #[cfg(any(test, feature = "test-support"))]
        {
            self.full_instrument_release_calls =
                self.full_instrument_release_calls.saturating_add(1);
        }
        self.release_scope(ReleaseScope::FullInstrument, target_hwnd)
    }
}
