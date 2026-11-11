use super::{StartupCheckpoint, StartupOrderingHook};
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InterruptedWaitEvidence {
    pub(crate) sequence: u64,
    pub(crate) outcome: sky_dispatch_win32::wait::WaitOutcome,
    pub(crate) spin_ticks: u64,
    pub(crate) physical_target_qpc_ticks: u64,
    pub(crate) planned_wait_ticks: u64,
    pub(crate) wake_qpc_ticks: Option<u64>,
    pub(crate) deadline_crossing_qpc_ticks: Option<u64>,
    pub(crate) waiter_return_qpc_ticks: Option<u64>,
}

impl StartupOrderingHook {
    pub fn last_down_window(&self) -> (u64, u64) {
        let (_, authored, not_before) = self.last_down_window_snapshot();
        (authored, not_before)
    }

    pub fn last_down_window_snapshot(&self) -> (u64, u64, u64) {
        loop {
            let before = self.down_window_sequence.load(Ordering::Acquire);
            if before & 1 != 0 {
                std::hint::spin_loop();
                continue;
            }
            let authored = self.down_authored_target_qpc_ticks.load(Ordering::Relaxed);
            let not_before = self.down_not_before_qpc_ticks.load(Ordering::Relaxed);
            let after = self.down_window_sequence.load(Ordering::Acquire);
            if before == after {
                return (after / 2, authored, not_before);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn interrupted_wait_count(&self) -> u64 {
        self.interrupted_wait_count.load(Ordering::Acquire)
    }

    #[cfg(test)]
    pub(crate) fn last_interrupted_wait_evidence(&self) -> Option<InterruptedWaitEvidence> {
        self.interrupted_wait_evidence
            .lock()
            .ok()
            .and_then(|evidence| *evidence)
    }

    pub(crate) fn record_down_window(&self, authored_target: u64, not_before: u64) {
        self.down_window_sequence.fetch_add(1, Ordering::AcqRel);
        self.down_authored_target_qpc_ticks
            .store(authored_target, Ordering::Relaxed);
        self.down_not_before_qpc_ticks
            .store(not_before, Ordering::Relaxed);
        self.down_window_sequence.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn record_down_window_before_physical_wait(
        &self,
        authored_target: u64,
        not_before: u64,
    ) {
        self.record_down_window(authored_target, not_before);
        self.visit_startup_checkpoint(StartupCheckpoint::PhysicalWait);
    }

    pub(crate) fn record_interrupted_wait(&self, evidence: InterruptedWaitEvidence) {
        self.interrupted_wait_count.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut last) = self.interrupted_wait_evidence.lock() {
            *last = Some(evidence);
        }
    }
}
