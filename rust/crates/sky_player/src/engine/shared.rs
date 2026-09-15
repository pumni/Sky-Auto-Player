#[cfg(any(test, feature = "test-support"))]
use super::CommandTimingState;
use super::{NativeTelemetryOutput, SharedMetrics};
use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::{DurationTicks, QpcTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::event::OwnedEvent;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};
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
    pub(super) focus_active: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: CommandTimingState,
}

pub(super) const SYSTEM_POWER_OS_SUSPENDED: u8 = 1 << 0;
pub(super) const SYSTEM_POWER_DOWN_BLOCKED: u8 = 1 << 1;
pub(super) const SYSTEM_POWER_SUSPEND_PENDING: u8 = 1 << 2;
pub(super) const SYSTEM_POWER_RESUME_PENDING: u8 = 1 << 3;
const SYSTEM_POWER_PENDING_MASK: u8 = SYSTEM_POWER_SUSPEND_PENDING | SYSTEM_POWER_RESUME_PENDING;

/// Lock-free notification state shared by the OS callback and playback worker.
/// The callback only updates atomics and signals the already-owned interrupt.
pub(crate) struct SystemPowerState {
    state: AtomicU8,
    active: AtomicBool,
    suspend_boundary_qpc: AtomicU64,
    suspend_count: AtomicU64,
    resume_count: AtomicU64,
    duplicate_count: AtomicU64,
}

impl Default for SystemPowerState {
    fn default() -> Self {
        Self {
            state: AtomicU8::new(0),
            active: AtomicBool::new(true),
            suspend_boundary_qpc: AtomicU64::new(0),
            suspend_count: AtomicU64::new(0),
            resume_count: AtomicU64::new(0),
            duplicate_count: AtomicU64::new(0),
        }
    }
}

impl SystemPowerState {
    pub(super) fn notify_at(
        &self,
        suspended: bool,
        suspend_boundary_qpc: Option<QpcTicks>,
        interrupt: &OwnedEvent,
    ) -> bool {
        if !self.active.load(Ordering::Acquire) {
            return false;
        }
        loop {
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
    }

    pub(super) fn notify(&self, suspended: bool, interrupt: &OwnedEvent) -> bool {
        let suspend_boundary_qpc = suspended
            .then(|| sky_dispatch_win32::clock::qpc_now_ticks_checked().ok())
            .flatten();
        self.notify_at(suspended, suspend_boundary_qpc, interrupt)
    }

    pub(super) fn suspend_boundary_qpc(&self) -> Option<QpcTicks> {
        let raw = self.suspend_boundary_qpc.load(Ordering::Acquire);
        (raw != 0).then(|| QpcTicks::from_raw(raw))
    }

    pub(super) fn clear_suspend_boundary(&self) {
        self.suspend_boundary_qpc.store(0, Ordering::Release);
    }

    pub(super) fn reset_for_new_session(&self) {
        self.state.store(0, Ordering::Release);
        self.suspend_boundary_qpc.store(0, Ordering::Release);
        self.suspend_count.store(0, Ordering::Relaxed);
        self.resume_count.store(0, Ordering::Relaxed);
        self.duplicate_count.store(0, Ordering::Relaxed);
        self.active.store(true, Ordering::Release);
    }

    pub(super) fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
        self.state.store(0, Ordering::Release);
        self.suspend_boundary_qpc.store(0, Ordering::Release);
    }

    pub(super) fn take_pending(&self) -> u8 {
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
                active: AtomicBool::new(false),
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

    pub(crate) fn reset_for_new_session(&self) {
        self.state.reset_for_new_session();
    }
}

pub(super) struct SessionTarget {
    pub(super) target_hwnd: AtomicIsize,
    pub(super) target_generation: AtomicU64,
}

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
    pub(super) supervisor_heartbeat_ticks: AtomicU64,
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
mod tests {
    use super::*;
    use sky_dispatch_core::clock::PauseReason;
    use std::num::NonZeroU64;

    fn test_qpc_clock() -> QpcClock {
        QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap())
    }

    #[test]
    fn progress_projection_advances_without_worker_publication() {
        let clock =
            PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
        let shared = SharedProgressClock::default();
        shared.publish(&clock);
        let anchor = shared.load().expect("published playback anchor");
        let qpc_clock = test_qpc_clock();

        assert_eq!(
            anchor.elapsed_us(QpcTicks::from_raw(2_000), qpc_clock),
            1_000
        );
        assert_eq!(
            anchor.elapsed_us(QpcTicks::from_raw(3_000), qpc_clock),
            2_000
        );
    }

    #[test]
    fn progress_projection_freezes_pause_and_excludes_pause_on_resume() {
        let mut clock =
            PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
        let shared = SharedProgressClock::default();
        let qpc_clock = test_qpc_clock();

        clock
            .enter_pause(PauseReason::Manual, QpcTicks::from_raw(2_000))
            .unwrap();
        shared.publish(&clock);
        let paused_anchor = shared.load().expect("published pause anchor");
        assert!(paused_anchor.paused);
        assert_eq!(
            paused_anchor.elapsed_us(QpcTicks::from_raw(9_000), qpc_clock),
            1_000
        );

        clock
            .exit_pause(PauseReason::Manual, QpcTicks::from_raw(5_000))
            .unwrap();
        shared.publish(&clock);
        let resumed_anchor = shared.load().expect("published resume anchor");
        assert!(!resumed_anchor.paused);
        assert_eq!(
            resumed_anchor.elapsed_us(QpcTicks::from_raw(7_000), qpc_clock),
            3_000
        );
    }

    #[test]
    fn progress_projection_clamps_before_future_epoch() {
        let clock =
            PlaybackClockState::new(QpcTicks::from_raw(2_000), DurationTicks::ZERO).unwrap();
        let shared = SharedProgressClock::default();
        shared.publish(&clock);
        let anchor = shared.load().expect("published future anchor");

        assert_eq!(
            anchor.elapsed_us(QpcTicks::from_raw(1_000), test_qpc_clock()),
            0
        );
    }

    #[test]
    fn terminal_progress_projection_stays_frozen() {
        let clock =
            PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
        let shared = SharedProgressClock::default();
        shared.publish_terminal(&clock, QpcTicks::from_raw(3_000));
        let anchor = shared.load().expect("published terminal anchor");

        assert!(anchor.frozen);
        assert_eq!(
            anchor.elapsed_us(QpcTicks::from_raw(9_000), test_qpc_clock()),
            2_000
        );
    }

    #[test]
    fn focus_pause_before_future_epoch_remains_clamped() {
        let mut clock =
            PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
        clock
            .enter_pause(PauseReason::Focus, QpcTicks::from_raw(900))
            .unwrap();
        let shared = SharedProgressClock::default();
        shared.publish(&clock);
        let anchor = shared.load().expect("published focus anchor");

        assert_eq!(
            anchor.elapsed_us(QpcTicks::from_raw(2_000), test_qpc_clock()),
            0
        );
    }

    #[test]
    fn power_notifications_are_idempotent_and_down_stays_blocked_until_worker_resume() {
        let interrupt = OwnedEvent::new_auto_reset().expect("interrupt event");
        let power = SystemPowerState::default();

        assert!(power.notify(true, &interrupt));
        assert!(!power.notify(true, &interrupt));
        assert!(power.os_suspended());
        assert!(power.down_blocked());
        assert_eq!(power.take_pending(), SYSTEM_POWER_SUSPEND_PENDING);

        assert!(power.notify(false, &interrupt));
        assert!(!power.notify(false, &interrupt));
        assert!(!power.os_suspended());
        assert!(power.down_blocked());
        assert_eq!(power.take_pending(), SYSTEM_POWER_RESUME_PENDING);

        assert!(power.complete_resume());
        assert!(!power.down_blocked());
        let (_, _, suspends, resumes, duplicates) = power.snapshot();
        assert_eq!((suspends, resumes, duplicates), (1, 1, 2));

        assert!(power.notify(true, &interrupt));
        assert!(!power.complete_resume());
        assert!(power.down_blocked());
    }

    #[test]
    fn suspend_boundary_is_captured_before_worker_wakes_and_survives_resume_notification() {
        let endpoint = SystemPowerEndpoint::new().expect("power endpoint");
        endpoint.reset_for_new_session();
        let suspend_qpc = QpcTicks::from_raw(10_000);
        assert!(endpoint.notify_system_power(true, Some(suspend_qpc)));
        // The worker has not run yet. A resume callback may arrive while it is
        // asleep; the original suspend boundary must remain available.
        assert!(endpoint.notify_system_power(false, None));
        assert_eq!(endpoint.state().suspend_boundary_qpc(), Some(suspend_qpc));
    }

    #[test]
    fn callback_suspend_boundary_excludes_sleep_sized_qpc_interval() {
        let mut clock =
            PlaybackClockState::new(QpcTicks::from_raw(1_000), DurationTicks::ZERO).unwrap();
        let endpoint = SystemPowerEndpoint::new().expect("power endpoint");
        endpoint.reset_for_new_session();
        let suspend_qpc = QpcTicks::from_raw(2_000);
        let resume_qpc = QpcTicks::from_raw(2_000_000_000);
        assert!(endpoint.notify_system_power(true, Some(suspend_qpc)));
        assert!(endpoint.notify_system_power(false, None));
        clock
            .enter_pause(
                PauseReason::SystemSuspend,
                endpoint.state().suspend_boundary_qpc().unwrap(),
            )
            .unwrap();
        clock
            .exit_pause(PauseReason::SystemSuspend, resume_qpc)
            .unwrap();
        assert_eq!(
            test_qpc_clock()
                .duration_to_us(DurationTicks::from_raw(
                    clock.get_elapsed(resume_qpc).unwrap().as_u64(),
                ))
                .unwrap(),
            1_000
        );
    }
}
