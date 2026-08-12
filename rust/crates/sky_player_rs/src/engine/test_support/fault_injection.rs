#![cfg(any(test, feature = "test-support"))]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Outcome injected for a single mock-backend `SendInput` call (identified by
/// call index, zero-based, counting both Down and Up calls in order).
///
/// This is test-only infrastructure reachable only when `mock_backend=true`.
/// It never touches the real `SendInput` path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InjectedSendOutcome {
    /// All keys inserted; emitter waits `latency_ticks` QPC ticks (spin).
    Full { latency_ticks: u64 },
    /// Zero keys inserted (complete failure); optional spin.
    Zero {
        latency_ticks: u64,
        win32_error: u32,
    },
    /// Partial insertion: exactly `inserted` keys succeed.
    Partial {
        inserted: u8,
        latency_ticks: u64,
        win32_error: u32,
    },
    /// Emitter spin-stalls for `duration_ticks` QPC ticks without sending.
    Stall { duration_ticks: u64 },
    /// Return from the simulated send boundary, then panic before coordinator commit.
    PanicAfterSend,
    /// Return a complete send receipt but fail the post-send QPC boundary.
    QpcFailureAfterSend,
}

/// Script that maps call-index → `InjectedSendOutcome`.
///
/// Entries are matched by call index in O(n) over the script length (scripts
/// are short — a few entries at most). Calls whose index has no matching entry
/// behave as `InjectedSendOutcome::Full { latency_ticks: 0 }` (success, no latency).
#[derive(Clone, Debug, Default)]
pub struct FaultInjectionScript {
    /// `(call_index, outcome)` pairs, unsorted.
    pub entries: Vec<(usize, InjectedSendOutcome)>,
    /// Base latency applied to every call (on top of per-call outcome latency).
    pub base_latency_ticks: u64,
    /// Extra latency per key, in QPC ticks, applied to every call.
    pub per_key_latency_ticks: u64,
    pub focus_loss_after_due_before_send: bool,
    pub wait_failure: bool,
    /// When set, the mock physical probe returns Inconclusive while the flag
    /// is true. This models target-dependent proof being unavailable during a
    /// focus-loss episode without changing production probe semantics.
    pub force_inconclusive_probe: Option<Arc<AtomicBool>>,
}

impl FaultInjectionScript {
    /// No failures, no latency.
    pub fn none() -> Self {
        Self::default()
    }

    /// Fail the first 3 Up calls (transient release failure).
    pub fn transient_release() -> Self {
        Self {
            entries: vec![
                (
                    1,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    2,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    3,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
            ],
            ..Default::default()
        }
    }

    /// All Up calls fail (persistent release failure).
    ///
    /// The first Down call is index 0.  Every subsequent emitter call is an Up
    /// call or an Up retry for the fault-injection schedules used by the
    /// worker tests, so all indices from 1 onward fail.  This deliberately
    /// avoids assuming that Up calls have odd indices: an immediate retry is
    /// another Up call and must remain failed in this mode.
    pub fn persistent_release() -> Self {
        // Inject 128 failures — sufficient for any reasonable test schedule.
        let entries = (1..128)
            .map(|i| {
                (
                    i,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                )
            })
            .collect();
        Self {
            entries,
            ..Default::default()
        }
    }

    /// The first Down call gets zero progress (ZeroProgressDownOnce).
    pub fn zero_progress_down_once() -> Self {
        Self {
            entries: vec![(
                0,
                InjectedSendOutcome::Zero {
                    latency_ticks: 0,
                    win32_error: 1460,
                },
            )],
            ..Default::default()
        }
    }

    /// Both immediate Down attempts make zero progress.
    pub fn persistent_zero_down() -> Self {
        Self {
            entries: vec![
                (
                    0,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    1,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
            ],
            ..Default::default()
        }
    }

    /// The first Down attempt partially inserts a chord.
    pub fn partial_down_first_attempt() -> Self {
        Self {
            entries: vec![(
                0,
                InjectedSendOutcome::Partial {
                    inserted: 1,
                    latency_ticks: 0,
                    win32_error: 5,
                },
            )],
            ..Default::default()
        }
    }

    /// The first Down attempt is empty, then the immediate retry splits.
    pub fn partial_down_after_zero_retry() -> Self {
        Self {
            entries: vec![
                (
                    0,
                    InjectedSendOutcome::Zero {
                        latency_ticks: 0,
                        win32_error: 1460,
                    },
                ),
                (
                    1,
                    InjectedSendOutcome::Partial {
                        inserted: 1,
                        latency_ticks: 0,
                        win32_error: 5,
                    },
                ),
            ],
            ..Default::default()
        }
    }

    /// Every Up attempt makes zero progress.
    pub fn persistent_zero_up() -> Self {
        Self::persistent_release()
    }

    /// Panic after the simulated SendInput boundary and before coordinator commit.
    pub fn panic_after_send_before_commit() -> Self {
        Self {
            entries: vec![(0, InjectedSendOutcome::PanicAfterSend)],
            ..Default::default()
        }
    }

    pub fn focus_loss_after_due_before_send() -> Self {
        Self {
            focus_loss_after_due_before_send: true,
            ..Default::default()
        }
    }

    pub fn qpc_failure_after_send() -> Self {
        Self {
            entries: vec![(0, InjectedSendOutcome::QpcFailureAfterSend)],
            ..Default::default()
        }
    }

    pub fn wait_failure() -> Self {
        Self {
            wait_failure: true,
            ..Default::default()
        }
    }

    /// Resolve the outcome for `call_index`.  Returns `None` → Full success, no latency.
    pub fn resolve(&self, call_index: usize) -> Option<&InjectedSendOutcome> {
        self.entries
            .iter()
            .find(|(idx, _)| *idx == call_index)
            .map(|(_, o)| o)
    }
}
