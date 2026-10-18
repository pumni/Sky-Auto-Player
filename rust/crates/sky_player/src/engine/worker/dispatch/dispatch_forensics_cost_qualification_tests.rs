use super::super::super::hold_forensics::{ProductionForensicsAnomaly, ProductionHoldForensics};
use super::super::super::observation::DownObservation;
use super::{
    DurationTicks, HOLD_TICKS, PacketSample, PendingObservationQueue, QualificationWorkload,
    RELEASE_TICKS, SendTransactionStatus, TimelineTicks, WaitOutcome, WaitResult,
    WorkerMetricsLocal, WorkerRuntime, make_guard, make_samples, publish_pending_wait_observation,
    record_wait_attempt,
};
use crate::engine::telemetry::{SharedMetrics, TelemetryCollector, TelemetryMode};
use crate::engine::worker::dispatch::observation::record_down_send_telemetry;
use crate::engine::worker::dispatch_loop::publish_live_metrics_after_dispatch;
use crate::engine::worker::health::publish_backend_counters;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::input::TrackedKeyState;
use std::num::NonZeroU64;
use std::time::Instant;

#[cfg(windows)]
#[test]
#[ignore = "manual Windows component-cost qualification; see docs/dispatch-forensics-cost-qualification.md"]
fn dispatch_cost_qualification() {
    const BATCH_SAMPLES: usize = 512;
    const TARGET_PACKET_OPERATIONS: usize = 512;
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let anomaly_ring_storage_bytes = std::mem::size_of::<[ProductionForensicsAnomaly; 32]>()
        + std::mem::size_of::<[bool; 32]>()
        + std::mem::size_of::<usize>()
        + std::mem::size_of::<u64>();
    println!(
        "#476 forensics-storage anomaly_record_bytes={} ring_capacity=32 production_ring_storage_bytes={anomaly_ring_storage_bytes}",
        std::mem::size_of::<ProductionForensicsAnomaly>(),
    );

    for workload in QualificationWorkload::ALL {
        let raw = workload.raw_cycle();
        let cycles_per_batch = TARGET_PACKET_OPERATIONS.div_ceil(raw.len());
        let mut builder_guard = make_guard();
        let mut measured_guard = make_guard();
        let mut forensics = ProductionHoldForensics::default();
        forensics.set_frame_policies(
            DurationTicks::from_raw(HOLD_TICKS),
            DurationTicks::from_raw(RELEASE_TICKS),
        );
        let mut forensics_metrics = WorkerMetricsLocal::default();
        let backend = TrackedKeyState::default();
        let mut backend_metrics = WorkerMetricsLocal::default();
        let shared = SharedMetrics::default();
        let mut wait_runtime = WorkerRuntime::default();
        let wait_queue = PendingObservationQueue::default();
        let mut wait_metrics = WorkerMetricsLocal::default();
        let mut costs = QualificationCosts::default();
        let mut actual_operations_per_batch = 0;

        for batch_index in 0..BATCH_SAMPLES {
            let samples = make_samples(workload, &mut builder_guard, batch_index, cycles_per_batch);
            let operations = samples.len() as f64;
            actual_operations_per_batch = samples.len();

            let started = Instant::now();
            for (index, sample) in samples.iter().enumerate() {
                forensics.observe_packet_result(
                    sample.packet,
                    index as u32,
                    QpcTicks::from_raw(sample.target_ticks),
                    QpcTicks::from_raw(sample.pre_call_ticks),
                    QpcTicks::from_raw(sample.completion_ticks),
                    SendTransactionStatus::Complete,
                    &mut forensics_metrics,
                );
            }
            costs
                .forensics
                .push(started.elapsed().as_nanos() as f64 / operations);

            let started = Instant::now();
            for sample in &samples {
                let window = measured_guard
                    .query(
                        QpcTicks::from_raw(sample.target_ticks),
                        sample.packet.up_mask,
                        sample.packet.down_mask,
                    )
                    .expect("guard qualification query");
                measured_guard
                    .observe_successful_packet(
                        QpcTicks::from_raw(sample.completion_ticks),
                        sample.packet.up_mask,
                        sample.packet.down_mask,
                    )
                    .expect("guard qualification completion");
                std::hint::black_box(window);
            }
            costs
                .guard_query_and_update
                .push(started.elapsed().as_nanos() as f64 / operations);

            let started = Instant::now();
            for _ in &samples {
                publish_backend_counters(
                    std::hint::black_box(&backend),
                    std::hint::black_box(&mut backend_metrics),
                );
            }
            costs
                .backend_counter_refresh
                .push(started.elapsed().as_nanos() as f64 / operations);
            std::hint::black_box(&backend_metrics);

            let started = Instant::now();
            for sample in &samples {
                publish_live_metrics_after_dispatch(
                    &backend_metrics,
                    &shared,
                    qpc_clock,
                    QpcTicks::from_raw(sample.completion_ticks),
                );
            }
            costs
                .best_effort_snapshot_publication
                .push(started.elapsed().as_nanos() as f64 / operations);

            let started = Instant::now();
            for sample in &samples {
                let wait = record_wait_attempt(
                    Some(&wait_queue),
                    Some(WaitResult {
                        outcome: WaitOutcome::Deadline,
                        wake_qpc: Some(QpcTicks::from_raw(sample.pre_call_ticks)),
                        spin_ticks: DurationTicks::ZERO,
                    }),
                    QpcTicks::from_raw(sample.target_ticks),
                    DurationTicks::from_raw(
                        sample.pre_call_ticks.saturating_sub(sample.target_ticks),
                    ),
                    TimelineTicks::from_raw(sample.target_ticks),
                    QpcTicks::ZERO,
                );
                wait_runtime.pending_wait_observation = wait;
                publish_pending_wait_observation(
                    &mut wait_runtime,
                    Some(&wait_queue),
                    &mut wait_metrics,
                );
                std::hint::black_box(wait_queue.pop_front());
            }
            costs
                .pending_wait_capture_and_queue
                .push(started.elapsed().as_nanos() as f64 / operations);
            assert_eq!(wait_metrics.observer_queue_high_watermark, 1);

            let mut telemetry = TelemetryCollector::new(TelemetryMode::Ring, samples.len());
            let observations = samples
                .iter()
                .enumerate()
                .map(|(index, sample)| make_down_observation(*sample, index as u32))
                .collect::<Vec<_>>();
            let started = Instant::now();
            for observation in &observations {
                record_down_send_telemetry(
                    observation,
                    &mut telemetry,
                    2,
                    5,
                    true,
                    0,
                    0,
                    Some(TimelineTicks::from_raw(observation.pre_call_qpc.as_u64())),
                    Some(TimelineTicks::from_raw(
                        observation.sendinput_completion_qpc.as_u64(),
                    )),
                    Some(TimelineTicks::from_raw(observation.pre_call_qpc.as_u64())),
                    QpcTicks::from_raw(observation.sendinput_completion_qpc.as_u64() + 2),
                    false,
                    false,
                )
                .expect("telemetry qualification record");
            }
            costs
                .telemetry_record_and_ring
                .push(started.elapsed().as_nanos() as f64 / operations);
            assert_eq!(telemetry.output.accepted, samples.len() as u64);
            assert_eq!(telemetry.output.dropped, 0);
        }

        costs.report(workload.name(), BATCH_SAMPLES, actual_operations_per_batch);
        assert_eq!(forensics_metrics.production_hold_floor_violation_count, 0);
        assert_eq!(
            forensics_metrics.production_release_floor_violation_count,
            0
        );
        assert_eq!(wait_metrics.observer_dropped_samples, 0);
    }
}

#[cfg(windows)]
fn make_down_observation(sample: PacketSample, index: u32) -> DownObservation {
    let mut observation = DownObservation::test_fixture(u64::from(index));
    observation.allow_pre_epoch_startup_dispatch = true;
    observation.physical_target_qpc = QpcTicks::from_raw(sample.target_ticks);
    observation.physical_timing_window = sample.timing_window;
    observation.final_policy_qpc = Some(QpcTicks::from_raw(sample.pre_call_ticks));
    observation.pre_call_qpc = QpcTicks::from_raw(sample.pre_call_ticks);
    observation.sendinput_completion_qpc = QpcTicks::from_raw(sample.completion_ticks);
    observation.dispatch_ready_qpc = Some(QpcTicks::from_raw(sample.completion_ticks + 2));
    observation.wake_qpc = Some(QpcTicks::from_raw(sample.pre_call_ticks));
    observation.requested_packet = sample.packet;
    observation.confirmed_mask = sample.packet.up_mask | sample.packet.down_mask;
    observation.skipped_mask = 0;
    observation.trace.event_index = index;
    observation.trace.compiled_packet_index = Some(u64::from(index));
    observation.trace.authored_ticks = TimelineTicks::from_raw(sample.target_ticks);
    observation.trace.effective_deadline_ticks = TimelineTicks::from_raw(sample.target_ticks);
    observation
}

#[cfg(windows)]
#[derive(Default)]
struct QualificationCosts {
    forensics: Vec<f64>,
    guard_query_and_update: Vec<f64>,
    backend_counter_refresh: Vec<f64>,
    best_effort_snapshot_publication: Vec<f64>,
    pending_wait_capture_and_queue: Vec<f64>,
    telemetry_record_and_ring: Vec<f64>,
}

#[cfg(windows)]
impl QualificationCosts {
    fn report(&mut self, workload: &str, batch_samples: usize, target_operations: usize) {
        Self::report_one(
            workload,
            "production-forensics",
            batch_samples,
            target_operations,
            &mut self.forensics,
        );
        Self::report_one(
            workload,
            "guard-query-update",
            batch_samples,
            target_operations,
            &mut self.guard_query_and_update,
        );
        Self::report_one(
            workload,
            "backend-counter-refresh",
            batch_samples,
            target_operations,
            &mut self.backend_counter_refresh,
        );
        Self::report_one(
            workload,
            "best-effort-snapshot-publication",
            batch_samples,
            target_operations,
            &mut self.best_effort_snapshot_publication,
        );
        Self::report_one(
            workload,
            "pending-wait-capture-queue",
            batch_samples,
            target_operations,
            &mut self.pending_wait_capture_and_queue,
        );
        Self::report_one(
            workload,
            "telemetry-record-ring",
            batch_samples,
            target_operations,
            &mut self.telemetry_record_and_ring,
        );
    }

    fn report_one(
        workload: &str,
        component: &str,
        batch_samples: usize,
        target_operations: usize,
        values: &mut [f64],
    ) {
        values.sort_by(f64::total_cmp);
        let percentile = |rank: usize| -> f64 {
            let index = (values.len() * rank).div_ceil(100).saturating_sub(1);
            values[index.min(values.len() - 1)]
        };
        println!(
            "#476 component-cost workload={workload} component={component} batch_samples={batch_samples} target_packet_ops_per_batch={target_operations} batch_mean_ns_per_packet_p50={:.2} p95={:.2} p99={:.2} max={:.2}",
            percentile(50),
            percentile(95),
            percentile(99),
            values[values.len() - 1],
        );
    }
}
