use crate::engine::TelemetryCollector;
use crate::engine::config::EffectiveRuntimePolicy;

pub(super) fn try_new_worker_telemetry(
    effective_policy: &EffectiveRuntimePolicy,
) -> Result<TelemetryCollector, String> {
    TelemetryCollector::try_new(
        effective_policy.telemetry_mode,
        effective_policy.telemetry_capacity,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::TelemetryMode;
    use crate::engine::config::{DispatchProfile, WorkerConfig};
    use crate::engine::telemetry::RtTraceRecord;

    const REQUESTED_RING_CAPACITY: usize = 4_096;

    #[test]
    fn production_without_observer_uses_zero_capacity_while_preserving_ring_estimate() {
        let mut config = WorkerConfig::default();
        config.telemetry.mode = TelemetryMode::Ring;
        config.telemetry.capacity = REQUESTED_RING_CAPACITY;

        let policy = EffectiveRuntimePolicy::for_worker(&config);
        let collector = try_new_worker_telemetry(&policy).expect("production collector");
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
        let mut diagnostic = WorkerConfig {
            profile: DispatchProfile::StrictTimingDiagnostic,
            ..Default::default()
        };
        diagnostic.telemetry.mode = TelemetryMode::Ring;
        diagnostic.telemetry.capacity = REQUESTED_RING_CAPACITY;

        let diagnostic_policy = EffectiveRuntimePolicy::for_worker(&diagnostic);
        let diagnostic_collector =
            try_new_worker_telemetry(&diagnostic_policy).expect("diagnostic collector");
        assert_eq!(diagnostic_collector.mode, TelemetryMode::Ring);
        assert_eq!(diagnostic_collector.capacity, REQUESTED_RING_CAPACITY);
        assert!(diagnostic_collector.output.records.capacity() >= REQUESTED_RING_CAPACITY);

        let mut qualification = WorkerConfig {
            timing_observer_for_qualification: true,
            ..Default::default()
        };
        qualification.telemetry.mode = TelemetryMode::Ring;
        qualification.telemetry.capacity = REQUESTED_RING_CAPACITY;

        let qualification_policy = EffectiveRuntimePolicy::for_worker(&qualification);
        let qualification_collector =
            try_new_worker_telemetry(&qualification_policy).expect("qualification collector");
        assert_eq!(qualification_collector.mode, TelemetryMode::Ring);
        assert_eq!(qualification_collector.capacity, REQUESTED_RING_CAPACITY);
        assert!(qualification_collector.output.records.capacity() >= REQUESTED_RING_CAPACITY);

        let qualification_off = WorkerConfig {
            timing_observer_for_qualification: true,
            telemetry: crate::engine::TelemetryOptions {
                mode: TelemetryMode::Off,
                capacity: REQUESTED_RING_CAPACITY,
            },
            ..Default::default()
        };
        let qualification_off_policy = EffectiveRuntimePolicy::for_worker(&qualification_off);
        assert!(qualification_off_policy.observer_enabled);
        assert_eq!(qualification_off_policy.telemetry_mode, TelemetryMode::Off);
        assert_eq!(
            qualification_off_policy.telemetry_capacity,
            REQUESTED_RING_CAPACITY
        );
        let qualification_off_collector =
            try_new_worker_telemetry(&qualification_off_policy).expect("off collector");
        assert_eq!(qualification_off_collector.mode, TelemetryMode::Off);
        assert_eq!(
            qualification_off_collector.capacity,
            REQUESTED_RING_CAPACITY
        );
        assert_eq!(qualification_off_collector.output.records.capacity(), 0);
    }
}
