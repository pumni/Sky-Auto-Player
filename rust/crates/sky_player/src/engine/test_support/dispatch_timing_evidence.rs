#![cfg(any(test, feature = "test-support"))]

use std::sync::atomic::Ordering;

pub(crate) struct PacketSenderAttemptEvidence {
    pub(crate) has_down: bool,
    pub(crate) has_up: bool,
    pub(crate) sender_start_qpc_ticks: Option<u64>,
    pub(crate) authored_target_qpc_ticks: u64,
    pub(crate) packet_not_before_qpc_ticks: u64,
    pub(crate) wake_qpc_ticks: Option<u64>,
    pub(crate) stale_commit_count: u64,
}

impl crate::engine::config::StartupOrderingHook {
    pub(crate) fn record_packet_sender_attempt(&self, evidence: PacketSenderAttemptEvidence) {
        if evidence.has_down {
            self.record_down_sender_attempt(
                evidence.sender_start_qpc_ticks,
                evidence.authored_target_qpc_ticks,
                evidence.packet_not_before_qpc_ticks,
                evidence.wake_qpc_ticks,
                evidence.stale_commit_count,
            );
        } else if evidence.has_up {
            self.record_up_only_sender_attempt(
                evidence.sender_start_qpc_ticks,
                evidence.authored_target_qpc_ticks,
                evidence.packet_not_before_qpc_ticks,
            );
        }
    }

    pub(crate) fn record_up_only_sender_attempt(
        &self,
        sender_start_qpc_ticks: Option<u64>,
        authored_target_qpc_ticks: u64,
        packet_not_before_qpc_ticks: u64,
    ) {
        *self
            .up_only_sender_attempt_qpc_ticks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((
            sender_start_qpc_ticks,
            authored_target_qpc_ticks,
            packet_not_before_qpc_ticks,
        ));
    }

    #[cfg(test)]
    pub(crate) fn up_only_sender_attempt_evidence(&self) -> Option<(Option<u64>, u64, u64)> {
        *self
            .up_only_sender_attempt_qpc_ticks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    pub(crate) fn metadata_only_wait_evidence(&self) -> (u64, Option<u64>, Option<u64>) {
        let target = self
            .metadata_only_wait_target_qpc_ticks
            .load(Ordering::Acquire);
        let wake = self
            .metadata_only_wait_wake_qpc_ticks
            .load(Ordering::Acquire);
        (
            self.metadata_only_wait_count.load(Ordering::Acquire),
            (target != 0).then_some(target),
            (wake != 0).then_some(wake),
        )
    }

    pub(crate) fn record_metadata_only_wait(
        &self,
        target_qpc_ticks: u64,
        wake_qpc_ticks: Option<u64>,
    ) {
        self.metadata_only_wait_count.fetch_add(1, Ordering::AcqRel);
        self.metadata_only_wait_target_qpc_ticks
            .store(target_qpc_ticks, Ordering::Release);
        self.metadata_only_wait_wake_qpc_ticks
            .store(wake_qpc_ticks.unwrap_or(0), Ordering::Release);
    }
}
