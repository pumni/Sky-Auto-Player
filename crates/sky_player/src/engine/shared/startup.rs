use std::sync::atomic::{AtomicU32, Ordering};

/// Worker-owned startup publication state. The low bits contain the phase;
/// readiness and arm publication are monotonic evidence bits, and an abort
/// reason is committed in the same CAS that cancels a pre-arm transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum StartupPhase {
    Idle,
    Preparing,
    Ready,
    Arming,
    Armed,
    Cancelled,
    Failed,
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(crate) enum StartupAbortReason {
    None,
    Quit,
    Skip,
    PanicRelease,
    SupervisorLeaseExpired,
    SystemSuspend,
    FocusInvalidated,
    TargetInvalidated,
    OwnerInvalidated,
    WorkerFailure,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StartupSnapshot {
    pub(crate) phase: StartupPhase,
    pub(crate) abort_reason: StartupAbortReason,
    pub(crate) ready_published: bool,
    pub(crate) armed_published: bool,
    pub(crate) system_suspend_terminal: bool,
}

const STARTUP_PHASE_MASK: u32 = 0x0f;
const STARTUP_READY_PUBLISHED: u32 = 1 << 8;
const STARTUP_ARMED_PUBLISHED: u32 = 1 << 9;
// Monotonic for one physical session. It shares the publication word with
// Armed so the power callback and worker arm CAS have one linearization point.
const STARTUP_SYSTEM_SUSPEND_TERMINAL: u32 = 1 << 10;
const STARTUP_ABORT_SHIFT: u32 = 16;
const STARTUP_ABORT_MASK: u32 = 0xff << STARTUP_ABORT_SHIFT;

const fn startup_word(
    phase: StartupPhase,
    abort_reason: StartupAbortReason,
    ready_published: bool,
    armed_published: bool,
) -> u32 {
    (phase as u32)
        | if ready_published {
            STARTUP_READY_PUBLISHED
        } else {
            0
        }
        | if armed_published {
            STARTUP_ARMED_PUBLISHED
        } else {
            0
        }
        | ((abort_reason as u32) << STARTUP_ABORT_SHIFT)
}

fn decode_startup_word(raw: u32) -> StartupSnapshot {
    let phase = match raw & STARTUP_PHASE_MASK {
        0 => StartupPhase::Idle,
        1 => StartupPhase::Preparing,
        2 => StartupPhase::Ready,
        3 => StartupPhase::Arming,
        4 => StartupPhase::Armed,
        5 => StartupPhase::Cancelled,
        6 => StartupPhase::Failed,
        _ => StartupPhase::Invalid,
    };
    let encoded_abort_reason = match (raw & STARTUP_ABORT_MASK) >> STARTUP_ABORT_SHIFT {
        0 => StartupAbortReason::None,
        1 => StartupAbortReason::Quit,
        2 => StartupAbortReason::Skip,
        3 => StartupAbortReason::PanicRelease,
        4 => StartupAbortReason::SupervisorLeaseExpired,
        5 => StartupAbortReason::SystemSuspend,
        6 => StartupAbortReason::FocusInvalidated,
        7 => StartupAbortReason::TargetInvalidated,
        8 => StartupAbortReason::OwnerInvalidated,
        9 => StartupAbortReason::WorkerFailure,
        _ => StartupAbortReason::WorkerFailure,
    };
    let system_suspend_terminal = raw & STARTUP_SYSTEM_SUSPEND_TERMINAL != 0;
    let abort_reason = if system_suspend_terminal
        && !matches!(
            encoded_abort_reason,
            StartupAbortReason::PanicRelease
                | StartupAbortReason::SupervisorLeaseExpired
                | StartupAbortReason::FocusInvalidated
                | StartupAbortReason::TargetInvalidated
                | StartupAbortReason::OwnerInvalidated
                | StartupAbortReason::WorkerFailure
        ) {
        StartupAbortReason::SystemSuspend
    } else {
        encoded_abort_reason
    };
    StartupSnapshot {
        phase,
        abort_reason,
        ready_published: raw & STARTUP_READY_PUBLISHED != 0,
        armed_published: raw & STARTUP_ARMED_PUBLISHED != 0,
        system_suspend_terminal,
    }
}

#[inline]
fn startup_phase(raw: u32) -> StartupPhase {
    decode_startup_word(raw).phase
}

fn cancel_startup_before_arm(state: &AtomicU32, reason: StartupAbortReason) -> bool {
    if reason == StartupAbortReason::None {
        return false;
    }
    loop {
        let current = state.load(Ordering::Acquire);
        let snapshot = decode_startup_word(current);
        if snapshot.system_suspend_terminal
            && reason != StartupAbortReason::SystemSuspend
            && !matches!(
                reason,
                StartupAbortReason::PanicRelease
                    | StartupAbortReason::SupervisorLeaseExpired
                    | StartupAbortReason::FocusInvalidated
                    | StartupAbortReason::TargetInvalidated
                    | StartupAbortReason::OwnerInvalidated
                    | StartupAbortReason::WorkerFailure
            )
        {
            return false;
        }
        if !matches!(
            snapshot.phase,
            StartupPhase::Idle
                | StartupPhase::Preparing
                | StartupPhase::Ready
                | StartupPhase::Arming
        ) {
            return false;
        }
        let cancelled = startup_word(
            StartupPhase::Cancelled,
            reason,
            snapshot.ready_published,
            snapshot.armed_published,
        ) | (current & STARTUP_SYSTEM_SUSPEND_TERMINAL);
        if state
            .compare_exchange_weak(current, cancelled, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return true;
        }
    }
}

fn move_startup_phase(
    state: &AtomicU32,
    from: StartupPhase,
    to: StartupPhase,
    ready_published: bool,
    armed_published: bool,
) -> bool {
    let current = state.load(Ordering::Acquire);
    if startup_phase(current) != from || current & STARTUP_SYSTEM_SUSPEND_TERMINAL != 0 {
        return false;
    }
    let next = startup_word(
        to,
        StartupAbortReason::None,
        ready_published,
        armed_published,
    ) | (current & STARTUP_SYSTEM_SUSPEND_TERMINAL);
    state
        .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
}

pub(super) struct StartupPublicationState(AtomicU32);

impl Default for StartupPublicationState {
    fn default() -> Self {
        Self(AtomicU32::new(startup_word(
            StartupPhase::Idle,
            StartupAbortReason::None,
            false,
            false,
        )))
    }
}

impl StartupPublicationState {
    pub(super) fn snapshot(&self) -> StartupSnapshot {
        decode_startup_word(self.0.load(Ordering::Acquire))
    }

    pub(super) fn system_suspend_terminal(&self) -> bool {
        self.0.load(Ordering::Acquire) & STARTUP_SYSTEM_SUSPEND_TERMINAL != 0
    }

    pub(super) fn begin_preparing(&self) -> bool {
        move_startup_phase(
            &self.0,
            StartupPhase::Idle,
            StartupPhase::Preparing,
            false,
            false,
        )
    }

    pub(super) fn publish_ready(&self) -> bool {
        move_startup_phase(
            &self.0,
            StartupPhase::Preparing,
            StartupPhase::Ready,
            true,
            false,
        )
    }

    pub(super) fn begin_arming(&self) -> bool {
        move_startup_phase(
            &self.0,
            StartupPhase::Ready,
            StartupPhase::Arming,
            true,
            false,
        )
    }

    pub(super) fn publish_armed(&self) -> bool {
        move_startup_phase(
            &self.0,
            StartupPhase::Arming,
            StartupPhase::Armed,
            true,
            true,
        )
    }

    pub(super) fn cancel_prearm(&self, reason: StartupAbortReason) -> bool {
        cancel_startup_before_arm(&self.0, reason)
    }

    /// Latch system suspension in one bounded atomic RMW. The bit is never
    /// cleared by Resume; only the next session reset after callback rundown
    /// can clear it.
    pub(super) fn latch_system_suspend_terminal(&self) {
        self.0
            .fetch_or(STARTUP_SYSTEM_SUSPEND_TERMINAL, Ordering::AcqRel);
    }

    pub(super) fn fail(&self, reason: StartupAbortReason) -> bool {
        loop {
            let current = self.0.load(Ordering::Acquire);
            let snapshot = decode_startup_word(current);
            if !matches!(
                snapshot.phase,
                StartupPhase::Idle
                    | StartupPhase::Preparing
                    | StartupPhase::Ready
                    | StartupPhase::Arming
            ) {
                return false;
            }
            let failed = startup_word(
                StartupPhase::Failed,
                reason,
                snapshot.ready_published,
                snapshot.armed_published,
            ) | (current & STARTUP_SYSTEM_SUSPEND_TERMINAL);
            if self
                .0
                .compare_exchange_weak(current, failed, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return true;
            }
        }
    }

    pub(super) fn reset(&self) {
        self.0.store(
            startup_word(StartupPhase::Idle, StartupAbortReason::None, false, false),
            Ordering::Release,
        );
    }
}
