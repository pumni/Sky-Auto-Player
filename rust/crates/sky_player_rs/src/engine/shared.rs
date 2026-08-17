#[cfg(any(test, feature = "test-support"))]
use super::CommandTimingState;
use super::{NativeTelemetryOutput, SharedMetrics};
use parking_lot::Mutex;
use sky_dispatch_core::clock::PlaybackClockState;
use sky_dispatch_core::time::{DurationTicks, QpcTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::event::OwnedEvent;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, AtomicU64, Ordering};
use std::sync::{Condvar, Mutex as StdMutex};

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
    pub(super) interrupt: OwnedEvent,
    pub(super) desired_pause: AtomicBool,
    pub(super) quit_requested: AtomicBool,
    pub(super) skip_requested: AtomicBool,
    pub(super) panic_requested: AtomicBool,
    pub(super) focus_active: AtomicBool,
    #[cfg(any(test, feature = "test-support"))]
    pub(super) command_timing: CommandTimingState,
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
            .enter_pause("manual", QpcTicks::from_raw(2_000))
            .unwrap();
        shared.publish(&clock);
        let paused_anchor = shared.load().expect("published pause anchor");
        assert!(paused_anchor.paused);
        assert_eq!(
            paused_anchor.elapsed_us(QpcTicks::from_raw(9_000), qpc_clock),
            1_000
        );

        clock
            .exit_pause("manual", QpcTicks::from_raw(5_000))
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
        clock.enter_pause("focus", QpcTicks::from_raw(900)).unwrap();
        let shared = SharedProgressClock::default();
        shared.publish(&clock);
        let anchor = shared.load().expect("published focus anchor");

        assert_eq!(
            anchor.elapsed_us(QpcTicks::from_raw(2_000), test_qpc_clock()),
            0
        );
    }
}
