use crate::engine::config::WorkerConfig;
use crate::engine::{TelemetryCollector, TelemetryMode};

pub(super) fn try_new_worker_telemetry(
    config: &WorkerConfig,
) -> Result<TelemetryCollector, String> {
    let (mode, capacity) = if config.observer_enabled() {
        (config.telemetry.mode, config.telemetry.capacity)
    } else {
        (TelemetryMode::Off, 0)
    };
    TelemetryCollector::try_new(mode, capacity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::config::DispatchProfile;
    use crate::engine::telemetry::RtTraceRecord;

    const REQUESTED_RING_CAPACITY: usize = 4_096;

    #[test]
    fn production_without_observer_uses_zero_capacity_while_preserving_ring_estimate() {
        let mut config = WorkerConfig::default();
        config.telemetry.mode = TelemetryMode::Ring;
        config.telemetry.capacity = REQUESTED_RING_CAPACITY;

        let collector = try_new_worker_telemetry(&config).expect("production collector");
        assert_eq!(collector.mode, TelemetryMode::Off);
        assert_eq!(collector.capacity, 0);
        assert_eq!(collector.output.records.capacity(), 0);

        let requested = TelemetryCollector::try_new(TelemetryMode::Ring, REQUESTED_RING_CAPACITY)
            .expect("requested baseline ring");
        let reserved_records = requested.output.records.capacity();
        assert!(reserved_records >= REQUESTED_RING_CAPACITY);
        let estimated_reserved_payload_bytes =
            std::mem::size_of::<RtTraceRecord>() * REQUESTED_RING_CAPACITY;
        println!(
            "estimated requested record payload: {} bytes/record × {} records = {} bytes; actual baseline ring capacity: {}; actual production-normal record capacity: {}",
            std::mem::size_of::<RtTraceRecord>(),
            REQUESTED_RING_CAPACITY,
            estimated_reserved_payload_bytes,
            reserved_records,
            collector.output.records.capacity(),
        );
    }

    #[test]
    fn enabled_observers_keep_the_requested_telemetry_mode_and_capacity() {
        let mut diagnostic = WorkerConfig::default();
        diagnostic.profile = DispatchProfile::StrictTimingDiagnostic;
        diagnostic.telemetry.mode = TelemetryMode::Ring;
        diagnostic.telemetry.capacity = REQUESTED_RING_CAPACITY;

        let diagnostic_collector =
            try_new_worker_telemetry(&diagnostic).expect("diagnostic collector");
        assert_eq!(diagnostic_collector.mode, TelemetryMode::Ring);
        assert_eq!(diagnostic_collector.capacity, REQUESTED_RING_CAPACITY);
        assert!(diagnostic_collector.output.records.capacity() >= REQUESTED_RING_CAPACITY);

        let mut qualification = WorkerConfig::default();
        qualification.timing_observer_for_qualification = true;
        qualification.telemetry.mode = TelemetryMode::Ring;
        qualification.telemetry.capacity = REQUESTED_RING_CAPACITY;

        let qualification_collector =
            try_new_worker_telemetry(&qualification).expect("qualification collector");
        assert_eq!(qualification_collector.mode, TelemetryMode::Ring);
        assert_eq!(qualification_collector.capacity, REQUESTED_RING_CAPACITY);
        assert!(qualification_collector.output.records.capacity() >= REQUESTED_RING_CAPACITY);
    }
}
