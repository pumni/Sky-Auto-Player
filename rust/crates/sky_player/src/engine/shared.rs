#[cfg(any(test, feature = "test-support"))]
use super::CommandTimingState;
use super::{NativeTelemetryOutput, SharedMetrics};
use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::{DurationTicks, QpcTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::event::OwnedEvent;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex as StdMutex};

/// A transition-only projection of the authoritative playback clock.
///
/// The worker publishes this anchor when `PlaybackClockState` changes. Readers
/// derive current progress from QPC without asking the worker to publish at UI
/// cadence. The sequence protects readers from observing a mixed anchor while
/// keeping the worker and supervisor free of a blocking mutex.
pub(crate) struct SharedProgressClock {
    sequence: AtomicU64,
    epoch_qpc: AtomicU64,
    pause_started_qpc: AtomicU64,
    pause_started_valid: AtomicBool,
    paused: AtomicBool,
    frozen: AtomicBool,
    valid: AtomicBool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ProgressClockSnapshot {
    pub(super) epoch_qpc: QpcTicks,
    pub(super) pause_started_qpc: Option<QpcTicks>,
    pub(super) paused: bool,
    pub(super) frozen: bool,
}

impl Default for SharedProgressClock {
    fn default() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            epoch_qpc: AtomicU64::new(0),
            pause_started_qpc: AtomicU64::new(0),
            pause_started_valid: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            frozen: AtomicBool::new(false),
            valid: AtomicBool::new(false),
        }
    }
}

impl SharedProgressClock {
    pub(super) fn publish(&self, clock: &PlaybackClockState) {
        self.publish_anchor(
            clock.epoch,
            clock.pause_interval_started,
            clock.is_paused(),
            false,
        );
    }

    /// Freeze the projection at termination while preserving the final
    /// playback epoch. A non-paused clock uses the terminal QPC as its frozen
    /// endpoint; an already-paused clock retains its pause endpoint.
    pub(super) fn publish_terminal(&self, clock: &PlaybackClockState, terminal_qpc: QpcTicks) {
        self.publish_anchor(
            clock.epoch,
            clock.pause_interval_started.or(Some(terminal_qpc)),
            true,
            true,
        );
    }

    pub(super) fn load(&self) -> Option<ProgressClockSnapshot> {
        loop {
            let sequence_before = self.sequence.load(Ordering::Acquire);
            if sequence_before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }

            let valid = self.valid.load(Ordering::Relaxed);
            let epoch_qpc = self.epoch_qpc.load(Ordering::Relaxed);
            let pause_started_qpc = self.pause_started_qpc.load(Ordering::Relaxed);
            let pause_started_valid = self.pause_started_valid.load(Ordering::Relaxed);
            let paused = self.paused.load(Ordering::Relaxed);
            let frozen = self.frozen.load(Ordering::Relaxed);
            let sequence_after = self.sequence.load(Ordering::Acquire);
            if sequence_before != sequence_after || sequence_after & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }

            return valid.then_some(ProgressClockSnapshot {
                epoch_qpc: QpcTicks::from_raw(epoch_qpc),
                pause_started_qpc: pause_started_valid
                    .then(|| QpcTicks::from_raw(pause_started_qpc)),
                paused,
                frozen,
            });
        }
    }

    fn publish_anchor(
        &self,
        epoch_qpc: QpcTicks,
        pause_started_qpc: Option<QpcTicks>,
        paused: bool,
        frozen: bool,
    ) {
        self.sequence.fetch_add(1, Ordering::AcqRel);
        self.epoch_qpc.store(epoch_qpc.as_u64(), Ordering::Relaxed);
        self.pause_started_qpc.store(
            pause_started_qpc.map_or(0, QpcTicks::as_u64),
            Ordering::Relaxed,
        );
        self.pause_started_valid
            .store(pause_started_qpc.is_some(), Ordering::Relaxed);
        self.paused.store(paused, Ordering::Relaxed);
        self.frozen.store(frozen, Ordering::Relaxed);
        self.valid.store(true, Ordering::Relaxed);
        self.sequence.fetch_add(1, Ordering::Release);
    }
}

impl ProgressClockSnapshot {
    pub(super) fn elapsed_us(self, now_qpc: QpcTicks, qpc_clock: QpcClock) -> u64 {
        let endpoint = if self.paused || self.frozen {
            self.pause_started_qpc.unwrap_or(self.epoch_qpc)
        } else {
            now_qpc
        };
        let elapsed_ticks = endpoint
            .checked_duration_since(self.epoch_qpc)
            .unwrap_or(DurationTicks::ZERO);
        qpc_clock.duration_to_us(elapsed_ticks).unwrap_or_default()
    }
}

/// Cross-thread session resources with one explicit owner.
///
/// The worker receives a borrow of this aggregate for its lifetime instead of
/// receiving a separate list of atomics and synchronization primitives. The
/// individual resources retain their existing types and ordering semantics.
pub(super) struct SessionCommands {
    pub(super) interrupt: Arc<OwnedEvent>,
    pub(super) system_power: Arc<SystemPowerState>,
    pub(super) desired_pause: AtomicBool,
    pub(super) quit_requested: AtomicBool,
    pub(super) skip_requested: AtomicBool,
    pub(super) panic_requested: AtomicBool,
    pub(super) supervisor_expired: SupervisorLeaseState,
    pub(super) focus_active: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: CommandTimingState,
}

/// One atomic decision shared by supervisor progress, the watchdog, and the
/// final musical-Down control gate. The high bit permanently latches expiry;
/// QPC's nonnegative signed-counter range leaves that bit available, and the
/// remaining bits retain the last accepted progress stamp.
pub(crate) struct SupervisorLeaseState {
    state: AtomicU64,
}

impl SupervisorLeaseState {
    const EXPIRED_BIT: u64 = 1 << 63;
    const TICKS_MASK: u64 = !Self::EXPIRED_BIT;

    pub(crate) fn new(initial_progress: QpcTicks) -> Self {
        let ticks = initial_progress.as_u64();
        Self {
            state: AtomicU64::new(if ticks & Self::EXPIRED_BIT == 0 {
                ticks
            } else {
                ticks | Self::EXPIRED_BIT
            }),
        }
    }

    pub(crate) fn reset(&self, progress: QpcTicks) {
        let ticks = progress.as_u64();
        self.state.store(
            if ticks & Self::EXPIRED_BIT == 0 {
                ticks
            } else {
                ticks | Self::EXPIRED_BIT
            },
            Ordering::Release,
        );
    }

    pub(crate) fn is_expired(&self) -> bool {
        self.state.load(Ordering::Acquire) & Self::EXPIRED_BIT != 0
    }

    pub(crate) fn last_progress_ticks(&self) -> u64 {
        self.state.load(Ordering::Acquire) & Self::TICKS_MASK
    }

    pub(crate) fn latch_expired(&self) {
        self.state.fetch_or(Self::EXPIRED_BIT, Ordering::AcqRel);
    }

    pub(crate) fn check_expired(&self, now: QpcTicks, timeout: DurationTicks) -> bool {
        if timeout == DurationTicks::ZERO {
            return false;
        }
        let now = now.as_u64();
        if now & Self::EXPIRED_BIT != 0 {
            self.latch_expired();
            return true;
        }
        loop {
            let state = self.state.load(Ordering::Acquire);
            if state & Self::EXPIRED_BIT != 0 {
                return true;
            }
            let previous = state & Self::TICKS_MASK;
            if previous == 0 || now < previous || now - previous <= timeout.as_u64() {
                return false;
            }
            if self
                .state
                .compare_exchange(
                    state,
                    state | Self::EXPIRED_BIT,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    pub(crate) fn publish_progress(&self, now: QpcTicks, timeout: DurationTicks) -> bool {
        if timeout == DurationTicks::ZERO {
            return !self.is_expired();
        }
        let now = now.as_u64();
        if now & Self::EXPIRED_BIT != 0 {
            self.latch_expired();
            return false;
        }
        loop {
            let state = self.state.load(Ordering::Acquire);
            if state & Self::EXPIRED_BIT != 0 {
                return false;
            }
            let previous = state & Self::TICKS_MASK;
            if previous != 0 && now >= previous && now - previous > timeout.as_u64() {
                if self
                    .state
                    .compare_exchange(
                        state,
                        state | Self::EXPIRED_BIT,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_ok()
                {
                    return false;
                }
                continue;
            }
            if now < previous {
                return true;
            }
            if self
                .state
                .compare_exchange(state, now, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }
}

pub(super) const SYSTEM_POWER_OS_SUSPENDED: u8 = 1 << 0;
pub(super) const SYSTEM_POWER_DOWN_BLOCKED: u8 = 1 << 1;
pub(super) const SYSTEM_POWER_SUSPEND_PENDING: u8 = 1 << 2;
pub(super) const SYSTEM_POWER_RESUME_PENDING: u8 = 1 << 3;
const SYSTEM_POWER_PENDING_MASK: u8 = SYSTEM_POWER_SUSPEND_PENDING | SYSTEM_POWER_RESUME_PENDING;
const SYSTEM_POWER_NOTIFY_CAS_ATTEMPTS: usize = 8;

/// Notification state shared by the OS callback and playback worker.
///
/// Callback mutation uses an even/odd epoch plus an in-flight rundown count.
/// Lifecycle operations serialize on `lifecycle`, close the old epoch, wait
/// for callbacks that already acquired authority, clear old state and event
/// signals, then open the next even epoch. Callbacks never take that mutex.
pub struct SystemPowerState {
    state: AtomicU8,
    callback_epoch: AtomicU64,
    callbacks_in_flight: AtomicU64,
    lifecycle: StdMutex<()>,
    suspend_boundary_qpc: AtomicU64,
    suspend_count: AtomicU64,
    resume_count: AtomicU64,
    duplicate_count: AtomicU64,
    #[cfg(any(test, feature = "test-support"))]
    after_epoch_capture: TestPause,
    #[cfg(any(test, feature = "test-support"))]
    after_lease_acquired: TestPause,
    #[cfg(any(test, feature = "test-support"))]
    force_notify_cas_failures: AtomicBool,
}

impl Default for SystemPowerState {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(0),
            // Epoch zero is the initial active epoch for standalone worker
            // tests. The stable process endpoint starts closed at epoch one.
            callback_epoch: AtomicU64::new(0),
            callbacks_in_flight: AtomicU64::new(0),
            lifecycle: StdMutex::new(()),
            suspend_boundary_qpc: AtomicU64::new(0),
            suspend_count: AtomicU64::new(0),
            resume_count: AtomicU64::new(0),
            duplicate_count: AtomicU64::new(0),
            #[cfg(any(test, feature = "test-support"))]
            after_epoch_capture: TestPause::default(),
            #[cfg(any(test, feature = "test-support"))]
            after_lease_acquired: TestPause::default(),
            #[cfg(any(test, feature = "test-support"))]
            force_notify_cas_failures: AtomicBool::new(false),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct TestPause {
    armed: AtomicBool,
    reached: AtomicBool,
    released: AtomicBool,
}

#[cfg(any(test, feature = "test-support"))]
impl TestPause {
    fn arm(&self) {
        self.reached.store(false, Ordering::Relaxed);
        self.released.store(false, Ordering::Relaxed);
        self.armed.store(true, Ordering::Release);
    }

    fn pause_if_armed(&self) {
        if self.armed.swap(false, Ordering::AcqRel) {
            self.reached.store(true, Ordering::Release);
            while !self.released.load(Ordering::Acquire) {
                std::hint::spin_loop();
            }
        }
    }
}

struct SystemPowerCallbackLease<'a> {
    state: &'a SystemPowerState,
}

impl Drop for SystemPowerCallbackLease<'_> {
    fn drop(&mut self) {
        self.state
            .callbacks_in_flight
            .fetch_sub(1, Ordering::SeqCst);
    }
}

impl SystemPowerState {
    /// Acquires mutation authority for one callback epoch. The SeqCst order
    /// makes validation and epoch close comparable: either validation wins,
    /// in which case rundown observes the reference, or close wins and this
    /// callback returns without touching state, counters, boundary, or event.
    fn enter_callback(&self) -> Option<SystemPowerCallbackLease<'_>> {
        let captured_epoch = self.callback_epoch.load(Ordering::SeqCst);
        if captured_epoch & 1 != 0 {
            return None;
        }
        #[cfg(any(test, feature = "test-support"))]
        self.after_epoch_capture.pause_if_armed();

        // A process cannot have u64::MAX callbacks executing concurrently;
        // this monotonic count therefore cannot wrap during a live process.
        self.callbacks_in_flight.fetch_add(1, Ordering::SeqCst);
        if self.callback_epoch.load(Ordering::SeqCst) != captured_epoch {
            self.callbacks_in_flight.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        #[cfg(any(test, feature = "test-support"))]
        self.after_lease_acquired.pause_if_armed();
        Some(SystemPowerCallbackLease { state: self })
    }

    fn close_epoch_and_rundown(&self) -> u64 {
        let observed_epoch = self.callback_epoch.load(Ordering::SeqCst);
        let closed_epoch = if observed_epoch & 1 == 0 {
            // Active epochs are even and can only reach u64::MAX - 1, so
            // closing by one is representable and permanently rejects entry.
            observed_epoch + 1
        } else {
            observed_epoch
        };
        self.callback_epoch.store(closed_epoch, Ordering::SeqCst);
        while self.callbacks_in_flight.load(Ordering::SeqCst) != 0 {
            std::thread::yield_now();
        }
        closed_epoch
    }

    pub(super) fn notify_at(
        &self,
        suspended: bool,
        suspend_boundary_qpc: Option<QpcTicks>,
        interrupt: &OwnedEvent,
    ) -> bool {
        let Some(_lease) = self.enter_callback() else {
            return false;
        };
        self.notify_authorized(suspended, suspend_boundary_qpc, interrupt)
    }

    fn notify_with_qpc(
        &self,
        suspended: bool,
        interrupt: &OwnedEvent,
        read_qpc: impl FnOnce() -> Option<QpcTicks>,
    ) -> bool {
        let Some(_lease) = self.enter_callback() else {
            return false;
        };
        let suspend_boundary_qpc = suspended.then(read_qpc).flatten();
        self.notify_authorized(suspended, suspend_boundary_qpc, interrupt)
    }

    fn notify_authorized(
        &self,
        suspended: bool,
        suspend_boundary_qpc: Option<QpcTicks>,
        interrupt: &OwnedEvent,
    ) -> bool {
        for _ in 0..SYSTEM_POWER_NOTIFY_CAS_ATTEMPTS {
            let current = self.state.load(Ordering::Acquire);
            let os_suspended = current & SYSTEM_POWER_OS_SUSPENDED != 0;
            if os_suspended == suspended {
                self.duplicate_count.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            if suspended {
                // Publish the boundary before exposing SUSPEND_PENDING. A
                // worker that observes the state immediately after the CAS
                // must never be able to see a missing boundary.
                self.suspend_boundary_qpc.store(
                    suspend_boundary_qpc.map_or(0, QpcTicks::as_u64),
                    Ordering::Release,
                );
            }
            #[cfg(any(test, feature = "test-support"))]
            if self.force_notify_cas_failures.load(Ordering::Relaxed) {
                continue;
            }
            let next = if suspended {
                current
                    | SYSTEM_POWER_OS_SUSPENDED
                    | SYSTEM_POWER_DOWN_BLOCKED
                    | SYSTEM_POWER_SUSPEND_PENDING
            } else {
                (current & !SYSTEM_POWER_OS_SUSPENDED)
                    | SYSTEM_POWER_DOWN_BLOCKED
                    | SYSTEM_POWER_RESUME_PENDING
            };
            if self
                .state
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                if suspended {
                    self.suspend_count.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.resume_count.fetch_add(1, Ordering::Relaxed);
                }
                let _ = interrupt.signal();
                return true;
            }
        }

        // Contention beyond the fixed retry budget must never reopen Down
        // admission on an ambiguous notification history. Treat it as a
        // conservative suspend; only a current-epoch resume can reopen the
        // gate after worker-side suspend/resume processing.
        self.state.fetch_or(
            SYSTEM_POWER_OS_SUSPENDED | SYSTEM_POWER_DOWN_BLOCKED | SYSTEM_POWER_SUSPEND_PENDING,
            Ordering::AcqRel,
        );
        let _ = interrupt.signal();
        false
    }

    pub(super) fn notify(&self, suspended: bool, interrupt: &OwnedEvent) -> bool {
        self.notify_with_qpc(suspended, interrupt, || {
            sky_dispatch_win32::clock::qpc_now_ticks_checked().ok()
        })
    }

    pub(super) fn suspend_boundary_qpc(&self) -> Option<QpcTicks> {
        let raw = self.suspend_boundary_qpc.load(Ordering::Acquire);
        (raw != 0).then(|| QpcTicks::from_raw(raw))
    }

    pub(super) fn clear_suspend_boundary(&self) {
        self.suspend_boundary_qpc.store(0, Ordering::Release);
    }

    pub(super) fn reset_for_new_session(&self, interrupt: &OwnedEvent) -> Result<(), String> {
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let closed_epoch = self.close_epoch_and_rundown();
        self.state.store(0, Ordering::Release);
        self.suspend_boundary_qpc.store(0, Ordering::Release);
        self.suspend_count.store(0, Ordering::Relaxed);
        self.resume_count.store(0, Ordering::Relaxed);
        self.duplicate_count.store(0, Ordering::Relaxed);
        // The auto-reset event is shared across sessions. Drain its old
        // signal only after every callback that could set it has rundown.
        let _ = interrupt.try_take();
        let next_epoch = closed_epoch.checked_add(1).ok_or_else(|| {
            "system power callback epoch exhausted; endpoint remains closed".to_string()
        })?;
        self.callback_epoch.store(next_epoch, Ordering::SeqCst);
        Ok(())
    }

    #[cfg(test)]
    fn pause_after_epoch_capture_for_test(&self) {
        self.after_epoch_capture.arm();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn arm_epoch_capture_pause_for_test(&self) {
        self.after_epoch_capture.arm();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn epoch_capture_pause_reached_for_test(&self) -> bool {
        self.after_epoch_capture.reached.load(Ordering::Acquire)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn release_epoch_capture_pause_for_test(&self) {
        self.after_epoch_capture
            .released
            .store(true, Ordering::Release);
    }

    #[cfg(test)]
    pub(crate) fn force_notify_cas_failures_for_test(&self, force: bool) {
        self.force_notify_cas_failures
            .store(force, Ordering::Relaxed);
    }

    #[cfg(test)]
    fn pause_after_lease_for_test(&self) {
        self.after_lease_acquired.arm();
    }

    #[cfg(test)]
    fn test_pause(&self, after_epoch_capture: bool) -> &TestPause {
        if after_epoch_capture {
            &self.after_epoch_capture
        } else {
            &self.after_lease_acquired
        }
    }

    pub(super) fn deactivate(&self, interrupt: &OwnedEvent) {
        let _lifecycle = self
            .lifecycle
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.close_epoch_and_rundown();
        self.state.store(0, Ordering::Release);
        self.suspend_boundary_qpc.store(0, Ordering::Release);
        let _ = interrupt.try_take();
    }

    pub(super) fn take_pending(&self) -> u8 {
        let state = self.state.load(Ordering::Acquire);
        if state & SYSTEM_POWER_PENDING_MASK == 0 {
            return 0;
        }
        self.state
            .fetch_and(!SYSTEM_POWER_PENDING_MASK, Ordering::AcqRel)
            & SYSTEM_POWER_PENDING_MASK
    }

    pub(super) fn down_blocked(&self) -> bool {
        self.state.load(Ordering::Acquire) & SYSTEM_POWER_DOWN_BLOCKED != 0
    }

    pub(super) fn os_suspended(&self) -> bool {
        self.state.load(Ordering::Acquire) & SYSTEM_POWER_OS_SUSPENDED != 0
    }

    /// Complete worker-side resume only if no newer suspend has arrived.
    pub(super) fn complete_resume(&self) -> bool {
        loop {
            let current = self.state.load(Ordering::Acquire);
            if current & (SYSTEM_POWER_OS_SUSPENDED | SYSTEM_POWER_SUSPEND_PENDING) != 0 {
                return false;
            }
            if self
                .state
                .compare_exchange(
                    current,
                    current & !SYSTEM_POWER_DOWN_BLOCKED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                return true;
            }
        }
    }

    pub(super) fn snapshot(&self) -> (bool, bool, u64, u64, u64) {
        let state = self.state.load(Ordering::Acquire);
        (
            state & SYSTEM_POWER_OS_SUSPENDED != 0,
            state & SYSTEM_POWER_DOWN_BLOCKED != 0,
            self.suspend_count.load(Ordering::Relaxed),
            self.resume_count.load(Ordering::Relaxed),
            self.duplicate_count.load(Ordering::Relaxed),
        )
    }
}

/// Stable process/runtime endpoint used by the Windows power callback. It
/// owns the interrupt and notification state so callback registration can
/// outlive individual playback sessions without retaining per-session raw
/// pointers or weak references.
pub struct SystemPowerEndpoint {
    state: Arc<SystemPowerState>,
    interrupt: Arc<OwnedEvent>,
}

impl SystemPowerEndpoint {
    pub fn new() -> Result<Arc<Self>, String> {
        let interrupt = OwnedEvent::new_auto_reset()
            .ok_or_else(|| "failed to create system power command event".to_string())?;
        Ok(Arc::new(Self {
            state: Arc::new(SystemPowerState {
                callback_epoch: AtomicU64::new(1),
                ..SystemPowerState::default()
            }),
            interrupt: Arc::new(interrupt),
        }))
    }

    pub(crate) fn state(&self) -> Arc<SystemPowerState> {
        self.state.clone()
    }

    pub(crate) fn interrupt(&self) -> Arc<OwnedEvent> {
        self.interrupt.clone()
    }

    pub fn notify_system_power(
        &self,
        suspended: bool,
        suspend_boundary_qpc: Option<QpcTicks>,
    ) -> bool {
        self.state
            .notify_at(suspended, suspend_boundary_qpc, &self.interrupt)
    }

    /// Route an OS callback while holding its current epoch authority, including
    /// the QPC read used for a suspend boundary.
    pub fn notify_system_power_with_clock(&self, suspended: bool, qpc_clock: QpcClock) -> bool {
        self.state
            .notify_with_qpc(suspended, &self.interrupt, || qpc_clock.now().ok())
    }

    pub(crate) fn reset_for_new_session(&self) -> Result<(), String> {
        self.state.reset_for_new_session(&self.interrupt)
    }

    #[cfg(feature = "test-support")]
    pub fn arm_callback_epoch_capture_pause_for_test(&self) {
        self.state.arm_epoch_capture_pause_for_test();
    }

    #[cfg(feature = "test-support")]
    pub fn callback_epoch_capture_pause_reached_for_test(&self) -> bool {
        self.state.epoch_capture_pause_reached_for_test()
    }

    #[cfg(feature = "test-support")]
    pub fn release_callback_epoch_capture_pause_for_test(&self) {
        self.state.release_epoch_capture_pause_for_test();
    }
}

pub(super) use super::target::SessionTarget;

pub(super) struct SessionLifecycle {
    pub(super) lifecycle: AtomicU8,
    pub(super) terminal_outcome: AtomicU8,
    pub(super) completed: (StdMutex<bool>, Condvar),
}

pub(super) struct SessionPublication {
    pub(super) metrics: std::sync::Arc<SharedMetrics>,
    pub(super) progress_clock: SharedProgressClock,
    pub(super) telemetry_output: Mutex<Option<NativeTelemetryOutput>>,
    pub(super) priority_acquired: Mutex<String>,
    pub(super) startup_requested_ticks: AtomicU64,
    pub(super) epoch_qpc: AtomicU64,
    pub(super) pre_roll_us: AtomicU64,
    pub(super) armed: AtomicBool,
    pub(super) startup_ready_ticks: AtomicU64,
    pub(super) startup_latency_us: AtomicU64,
    pub(super) startup_ready: AtomicBool,
}

pub(super) struct SessionShared {
    pub(super) commands: SessionCommands,
    pub(super) target: SessionTarget,
    pub(super) lifecycle: SessionLifecycle,
    pub(super) publication: SessionPublication,
}

#[cfg(test)]
#[path = "shared_tests.rs"]
mod tests;
