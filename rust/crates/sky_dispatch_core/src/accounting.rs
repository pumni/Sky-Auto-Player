//! Allocation-free generation accounting shared by production dispatch and
//! the test-support legacy coordinator.

use serde::{Deserialize, Serialize};

/// Counter-only terminal-generation summary used by the live executor.
///
/// Active counts are derived from the executor's active slots at query time;
/// this type stores only terminal generations.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GenerationCounters {
    pub released: u64,
    pub dropped_conflict: u64,
    pub dropped_backend: u64,
    pub dropped_expired: u64,
    pub cancelled: u64,
}

impl GenerationCounters {
    pub(crate) fn terminal_total_checked(&self) -> Option<u64> {
        self.released
            .checked_add(self.dropped_conflict)?
            .checked_add(self.dropped_backend)?
            .checked_add(self.dropped_expired)?
            .checked_add(self.cancelled)
    }

    pub fn terminal_total(&self) -> u64 {
        self.terminal_total_checked()
            .expect("terminal generation counters must remain checked")
    }
}

/// Stable serialized lifecycle summary for a compiled generation ledger.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationAccounting {
    pub total: u64,
    pub activated: u64,
    pub scheduled: u64,
    pub active: u64,
    pub released: u64,
    pub dropped_conflict: u64,
    pub dropped_backend: u64,
    pub dropped_expired: u64,
    pub cancelled: u64,
}
