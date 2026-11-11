use super::{
    NATIVE_TELEMETRY_SCHEMA_VERSION, NATIVE_UP_SLOT_TRACE_CAPACITY, NativeTelemetrySummary,
    RtTraceRecord, TelemetryMode, TimingSemantics, UpSlotTrace,
};
use std::collections::VecDeque;

#[derive(Debug, Default, serde::Serialize)]
pub struct NativeTelemetryOutput {
    pub schema_version: u32,
    pub qpc_frequency_hz: u64,
    pub records: VecDeque<RtTraceRecord>,
    pub up_slot_records: Vec<UpSlotTrace>,
    /// Number of per-slot Up observations offered to the observer publication
    /// path, including records later lost or disabled by telemetry mode.
    pub up_slot_records_expected: u64,
    /// Up-slot observations not present in `up_slot_records`, excluding those
    /// intentionally omitted because capture mode was off.
    pub up_slot_records_dropped: u64,
    pub up_slot_records_dropped_by_observer_queue: u64,
    /// Up-slot records stranded by observer failure or queue shutdown.
    pub up_slot_records_dropped_by_observer_abort: u64,
    pub up_slot_records_dropped_by_pool_storage: u64,
    pub up_slot_records_dropped_by_telemetry_ring: u64,
    pub up_slot_records_dropped_by_sidecar_capacity: u64,
    /// Up observations intentionally not retained because telemetry was off.
    pub up_slot_records_not_captured_by_mode: u64,
    /// Final conservation check: expected = captured + dropped + not captured.
    pub up_slot_accounting_status: UpSlotAccountingStatus,
    pub up_slot_accounting_invalid_reason: Option<&'static str>,
    pub up_slot_capture_mode: &'static str,
    pub summary: NativeTelemetrySummary,
    pub attempted: u64,
    pub accepted: u64,
    pub dropped: u64,
    /// Observations dropped by the bounded producer queue before telemetry
    /// could inspect them. Kept separate from ring-capacity drops.
    pub observer_queue_dropped: u64,
    pub truncated: bool,
    pub timing_semantics: TimingSemantics,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpSlotAccountingStatus {
    #[default]
    NotFinalized,
    Valid,
    Invalid,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum UpSlotLossReason {
    ObserverQueue,
    ObserverAbort,
    PoolStorage,
    TelemetryRing,
    SidecarCapacity,
    CaptureModeOff,
}

impl NativeTelemetryOutput {
    pub(crate) fn account_up_slot_loss(&mut self, reason: UpSlotLossReason, count: u64) {
        if count == 0 {
            return;
        }
        match reason {
            UpSlotLossReason::ObserverQueue => {
                self.up_slot_records_dropped_by_observer_queue = self
                    .up_slot_records_dropped_by_observer_queue
                    .saturating_add(count);
            }
            UpSlotLossReason::ObserverAbort => {
                self.up_slot_records_dropped_by_observer_abort = self
                    .up_slot_records_dropped_by_observer_abort
                    .saturating_add(count);
            }
            UpSlotLossReason::PoolStorage => {
                self.up_slot_records_dropped_by_pool_storage = self
                    .up_slot_records_dropped_by_pool_storage
                    .saturating_add(count);
            }
            UpSlotLossReason::TelemetryRing => {
                self.up_slot_records_dropped_by_telemetry_ring = self
                    .up_slot_records_dropped_by_telemetry_ring
                    .saturating_add(count);
            }
            UpSlotLossReason::SidecarCapacity => {
                self.up_slot_records_dropped_by_sidecar_capacity = self
                    .up_slot_records_dropped_by_sidecar_capacity
                    .saturating_add(count);
            }
            UpSlotLossReason::CaptureModeOff => {
                self.up_slot_records_not_captured_by_mode = self
                    .up_slot_records_not_captured_by_mode
                    .saturating_add(count);
                return;
            }
        }
        self.up_slot_records_dropped = self
            .up_slot_records_dropped_by_observer_queue
            .saturating_add(self.up_slot_records_dropped_by_observer_abort)
            .saturating_add(self.up_slot_records_dropped_by_pool_storage)
            .saturating_add(self.up_slot_records_dropped_by_telemetry_ring)
            .saturating_add(self.up_slot_records_dropped_by_sidecar_capacity);
        self.truncated = true;
    }

    pub(crate) fn accounted_up_slot_records(&self) -> Option<u64> {
        u64::try_from(self.up_slot_records.len())
            .ok()?
            .checked_add(self.up_slot_records_dropped)?
            .checked_add(self.up_slot_records_not_captured_by_mode)
    }

    pub(crate) fn finalize_up_slot_accounting(&mut self, observer_accounting_complete: bool) {
        self.up_slot_accounting_invalid_reason = None;
        self.up_slot_accounting_status = if !observer_accounting_complete {
            self.up_slot_accounting_invalid_reason = Some("observer_accounting_incomplete");
            UpSlotAccountingStatus::Invalid
        } else {
            match self.accounted_up_slot_records() {
                Some(accounted) if accounted == self.up_slot_records_expected => {
                    UpSlotAccountingStatus::Valid
                }
                Some(_) => {
                    self.up_slot_accounting_invalid_reason = Some("conservation_mismatch");
                    UpSlotAccountingStatus::Invalid
                }
                None => {
                    self.up_slot_accounting_invalid_reason = Some("conservation_overflow");
                    UpSlotAccountingStatus::Invalid
                }
            }
        };
    }

    pub(crate) fn try_new(mode: TelemetryMode, capacity: usize) -> Result<Self, String> {
        let mut records = VecDeque::new();
        if matches!(mode, TelemetryMode::Ring) {
            records
                .try_reserve_exact(capacity)
                .map_err(|error| format!("telemetry ring allocation failed: {error}"))?;
        }
        let mut up_slot_records = Vec::new();
        if matches!(mode, TelemetryMode::Ring) {
            up_slot_records
                .try_reserve_exact(NATIVE_UP_SLOT_TRACE_CAPACITY)
                .map_err(|error| format!("Up-slot sidecar allocation failed: {error}"))?;
        }
        Ok(Self {
            schema_version: NATIVE_TELEMETRY_SCHEMA_VERSION,
            records,
            up_slot_records,
            up_slot_capture_mode: if matches!(mode, TelemetryMode::Ring) {
                "ring"
            } else {
                "off"
            },
            ..Self::default()
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new(mode: TelemetryMode, capacity: usize) -> Self {
        Self {
            schema_version: NATIVE_TELEMETRY_SCHEMA_VERSION,
            records: if matches!(mode, TelemetryMode::Ring) {
                // Reserve the complete bounded buffer at construction time so no
                // heap allocation occurs on the first push_back during dispatch.
                let mut records = VecDeque::with_capacity(capacity);
                records.reserve_exact(capacity);
                records
            } else {
                VecDeque::new()
            },
            up_slot_records: if matches!(mode, TelemetryMode::Ring) {
                Vec::with_capacity(NATIVE_UP_SLOT_TRACE_CAPACITY)
            } else {
                Vec::new()
            },
            up_slot_capture_mode: if matches!(mode, TelemetryMode::Ring) {
                "ring"
            } else {
                "off"
            },
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod up_slot_accounting_tests {
    use super::{NativeTelemetryOutput, UpSlotAccountingStatus};

    #[test]
    fn export_marks_up_slot_conservation_mismatch_invalid() {
        let mut output = NativeTelemetryOutput {
            up_slot_records_expected: 1,
            ..NativeTelemetryOutput::default()
        };
        output.finalize_up_slot_accounting(true);

        assert_eq!(
            output.up_slot_accounting_status,
            UpSlotAccountingStatus::Invalid
        );
        assert_eq!(
            output.up_slot_accounting_invalid_reason,
            Some("conservation_mismatch")
        );
    }

    #[test]
    fn export_marks_incomplete_observer_accounting_invalid() {
        let mut output = NativeTelemetryOutput::default();
        output.finalize_up_slot_accounting(false);

        assert_eq!(
            output.up_slot_accounting_status,
            UpSlotAccountingStatus::Invalid
        );
        assert_eq!(
            output.up_slot_accounting_invalid_reason,
            Some("observer_accounting_incomplete")
        );
    }
}
