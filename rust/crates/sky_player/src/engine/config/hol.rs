use serde::{Deserialize, Serialize};
use sky_dispatch_win32::clock::QpcClock;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HolStallCycleRecord {
    pub cycle_index: u32,
    pub source_action_index: u32,
    pub down_mask: u16,
    pub pre_stall_qpc: u64,
    pub post_stall_qpc: u64,
    pub stall_duration_ticks: u64,
}

#[derive(Debug, Default)]
pub struct HolStallController {
    pub(crate) enabled: AtomicBool,
    pub(crate) remaining_cycles: AtomicU32,
    pub(crate) duration_us: AtomicU64,
    pub(crate) target_mask: AtomicU16,
    pub(crate) count: AtomicU64,
    pub(crate) last_pre_qpc: AtomicU64,
    pub(crate) last_post_qpc: AtomicU64,
    pub(crate) clock_error: AtomicBool,
    pub(crate) records: Mutex<Vec<HolStallCycleRecord>>,
}

impl HolStallController {
    pub fn enable_cycles(&self, duration_us: u64, target_mask: u16, cycles: u32) {
        self.duration_us.store(duration_us, Ordering::Release);
        self.target_mask.store(target_mask, Ordering::Release);
        self.remaining_cycles.store(cycles, Ordering::Release);
        self.clock_error.store(false, Ordering::Release);
        self.enabled
            .store(cycles > 0 && duration_us > 0, Ordering::Release);
    }

    pub fn evidence(&self) -> (u64, u64, u64, bool) {
        (
            self.count.load(Ordering::Acquire),
            self.last_pre_qpc.load(Ordering::Acquire),
            self.last_post_qpc.load(Ordering::Acquire),
            self.clock_error.load(Ordering::Acquire),
        )
    }

    pub fn cycle_records(&self) -> Vec<HolStallCycleRecord> {
        self.records
            .lock()
            .map(|records| records.clone())
            .unwrap_or_default()
    }

    pub fn clock_error(&self) -> bool {
        self.clock_error.load(Ordering::Acquire)
    }

    pub(crate) fn maybe_inject_hol_stall(
        &self,
        down_mask: u16,
        source_action_index: u32,
        qpc_clock: QpcClock,
    ) {
        if !self.enabled.load(Ordering::Acquire) {
            return;
        }
        let target_mask = self.target_mask.load(Ordering::Acquire);
        if target_mask != 0 && (down_mask & target_mask) == 0 {
            return;
        }
        let duration_us = self.duration_us.load(Ordering::Acquire);
        if duration_us == 0 {
            return;
        }

        let prev = self.remaining_cycles.load(Ordering::Acquire);
        if prev == 0 {
            self.enabled.store(false, Ordering::Release);
            return;
        }
        let prev = self.remaining_cycles.fetch_sub(1, Ordering::AcqRel);
        if prev <= 1 {
            self.enabled.store(false, Ordering::Release);
        }
        if prev == 0 {
            return;
        }

        let Ok(pre_qpc) = qpc_clock.now() else {
            self.clock_error.store(true, Ordering::Release);
            return;
        };
        let Ok(duration_ticks) = qpc_clock.duration_from_us(duration_us) else {
            self.clock_error.store(true, Ordering::Release);
            return;
        };
        let Ok(target_qpc) = pre_qpc.checked_add_duration(duration_ticks) else {
            self.clock_error.store(true, Ordering::Release);
            return;
        };

        loop {
            match qpc_clock.now() {
                Ok(now) => {
                    if now >= target_qpc {
                        self.last_pre_qpc.store(pre_qpc.as_u64(), Ordering::Release);
                        self.last_post_qpc.store(now.as_u64(), Ordering::Release);
                        let cycle_index = self.count.fetch_add(1, Ordering::AcqRel) as u32;
                        if let Ok(mut records) = self.records.lock() {
                            records.push(HolStallCycleRecord {
                                cycle_index,
                                source_action_index,
                                down_mask,
                                pre_stall_qpc: pre_qpc.as_u64(),
                                post_stall_qpc: now.as_u64(),
                                stall_duration_ticks: now.as_u64().saturating_sub(pre_qpc.as_u64()),
                            });
                        }
                        break;
                    }
                    std::hint::spin_loop();
                }
                Err(_) => {
                    self.clock_error.store(true, Ordering::Release);
                    break;
                }
            }
        }
    }
}
