use super::{
    PrecisionPacketTimingObservation, SharedMetrics, SnapshotBuffer, WorkerMetricsLocal,
    publish_lifecycle_metrics, publish_terminal_metrics, try_publish_metrics,
    try_publish_observer_metrics,
};
use sky_dispatch_win32::clock::QpcClock;
use std::num::NonZeroU64;
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn snapshot_buffer_never_publishes_torn_scalar_pair() {
    let buffer = Arc::new(SnapshotBuffer::default());
    let writer_buffer = Arc::clone(&buffer);
    let writer = thread::spawn(move || {
        for value in 1..=10_000u64 {
            let local = WorkerMetricsLocal {
                elapsed_us: value,
                total_us: value,
                ..WorkerMetricsLocal::default()
            };
            let _ = writer_buffer.try_publish_dispatch(local);
        }
    });
    for _ in 0..10_000 {
        let snapshot = buffer.load();
        assert_eq!(snapshot.elapsed_us, snapshot.total_us);
    }
    writer.join().expect("snapshot writer thread");
}

#[test]
fn concurrent_source_writers_keep_independent_snapshot_contents() {
    let buffer = Arc::new(SnapshotBuffer::default());
    let start = Arc::new(Barrier::new(3));
    let dispatch_buffer = Arc::clone(&buffer);
    let dispatch_start = Arc::clone(&start);
    let dispatch_writer = thread::spawn(move || {
        dispatch_start.wait();
        for value in 1..=10_000u64 {
            let _ = dispatch_buffer.try_publish_dispatch(WorkerMetricsLocal {
                elapsed_us: value,
                total_us: value,
                ..WorkerMetricsLocal::default()
            });
        }
    });
    let observer_buffer = Arc::clone(&buffer);
    let observer_start = Arc::clone(&start);
    let observer_writer = thread::spawn(move || {
        observer_start.wait();
        for value in 1..=10_000u64 {
            let _ = observer_buffer.try_publish_observer(WorkerMetricsLocal {
                observer_degraded_samples: value,
                wait_window_sample_count: value,
                ..WorkerMetricsLocal::default()
            });
        }
    });

    start.wait();
    for _ in 0..10_000 {
        let snapshot = buffer.load();
        assert_eq!(snapshot.elapsed_us, snapshot.total_us);
        assert_eq!(
            snapshot.observer_degraded_samples,
            snapshot.wait_window_sample_count
        );
    }
    dispatch_writer.join().expect("dispatch snapshot writer");
    observer_writer.join().expect("observer snapshot writer");

    assert!(buffer.try_publish_dispatch(WorkerMetricsLocal {
        elapsed_us: 10_001,
        total_us: 10_001,
        ..WorkerMetricsLocal::default()
    }));
    assert!(buffer.try_publish_observer(WorkerMetricsLocal {
        observer_degraded_samples: 10_001,
        wait_window_sample_count: 10_001,
        ..WorkerMetricsLocal::default()
    }));
    let snapshot = buffer.load();
    assert_eq!(snapshot.elapsed_us, 10_001);
    assert_eq!(snapshot.total_us, 10_001);
    assert_eq!(snapshot.observer_degraded_samples, 10_001);
    assert_eq!(snapshot.wait_window_sample_count, 10_001);
}

#[test]
fn dispatch_publication_is_nonblocking_when_a_reader_holds_its_slot() {
    let buffer = SnapshotBuffer::default();
    let reader = buffer.dispatch.read();
    assert!(!buffer.try_publish_dispatch(WorkerMetricsLocal {
        elapsed_us: 1,
        total_us: 1,
        ..WorkerMetricsLocal::default()
    }));
    drop(reader);
    assert!(buffer.try_publish_dispatch(WorkerMetricsLocal {
        elapsed_us: 2,
        total_us: 2,
        ..WorkerMetricsLocal::default()
    }));
}

#[test]
fn required_lifecycle_publication_survives_forced_snapshot_contention() {
    let shared = Arc::new(SharedMetrics::default());
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let active = WorkerMetricsLocal {
        active_count: 1,
        possibly_active_count: 1,
        failed_release_count: 0,
        ..WorkerMetricsLocal::default()
    };
    assert!(try_publish_metrics(&active, &shared, qpc_clock, 1, true));

    let reader = shared.snapshot.dispatch.read();
    let cleaned = WorkerMetricsLocal {
        active_count: 0,
        possibly_active_count: 0,
        failed_release_count: 0,
        ..active.clone()
    };
    assert!(
        !try_publish_metrics(&cleaned, &shared, qpc_clock, 2, true),
        "a forced packet-style publication remains best effort under reader contention"
    );

    drop(reader);

    let stale = shared.snapshot.load();
    assert_eq!(
        stale.active_count, 1,
        "a rejected packet-style publication leaves the prior snapshot stale"
    );

    let writer_shared = Arc::clone(&shared);
    let started = Arc::new(Barrier::new(2));
    let writer_started = Arc::clone(&started);
    let writer = thread::spawn(move || {
        writer_started.wait();
        publish_lifecycle_metrics(&cleaned, &writer_shared, qpc_clock, 3);
    });
    started.wait();
    writer.join().expect("required lifecycle publisher");

    let snapshot = shared.snapshot.load();
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(snapshot.failed_release_count, 0);
}

#[test]
fn terminal_snapshot_merges_and_clears_the_observer_slot_once() {
    let shared = SharedMetrics::default();
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let dispatch = WorkerMetricsLocal {
        elapsed_us: 7,
        total_us: 11,
        ..WorkerMetricsLocal::default()
    };
    let observer = WorkerMetricsLocal {
        observer_degraded_samples: 3,
        wait_window_sample_count: 5,
        ..WorkerMetricsLocal::default()
    };
    assert!(shared.snapshot.try_publish_dispatch(dispatch.clone()));
    assert!(shared.snapshot.try_publish_observer(observer.clone()));
    let live = shared.snapshot.load();
    assert_eq!(live.observer_degraded_samples, 3);
    assert_eq!(live.wait_window_sample_count, 5);

    let mut terminal = dispatch;
    terminal.merge_observer(&observer);
    publish_terminal_metrics(&terminal, &shared, qpc_clock, 100);

    let published = shared.snapshot.load();
    assert_eq!(published.elapsed_us, 7);
    assert_eq!(published.total_us, 11);
    assert_eq!(published.observer_degraded_samples, 3);
    assert_eq!(published.wait_window_sample_count, 5);
}

#[test]
fn terminal_cleanup_snapshot_is_visible_after_forced_publication_contention() {
    let shared = Arc::new(SharedMetrics::default());
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let live = WorkerMetricsLocal {
        active_count: 1,
        possibly_active_count: 1,
        ..WorkerMetricsLocal::default()
    };
    let observer = WorkerMetricsLocal {
        observer_degraded_samples: 7,
        ..WorkerMetricsLocal::default()
    };
    assert!(shared.snapshot.try_publish_dispatch(live));
    assert!(shared.snapshot.try_publish_observer(observer.clone()));

    let mut terminal = WorkerMetricsLocal {
        active_count: 0,
        possibly_active_count: 0,
        failed_release_count: 0,
        ..WorkerMetricsLocal::default()
    };
    terminal.merge_observer(&observer);

    let dispatch_reader = shared.snapshot.dispatch.read();
    assert!(
        !shared.snapshot.try_publish_terminal(&mut terminal),
        "terminal aggregation must report dispatch-slot contention"
    );
    assert_eq!(dispatch_reader.active_count, 1);
    drop(dispatch_reader);

    let observer_reader = shared.snapshot.observer.read();
    assert!(
        !shared.snapshot.try_publish_terminal(&mut terminal),
        "terminal aggregation must report observer-slot contention"
    );
    drop(observer_reader);

    assert_eq!(terminal.active_count, 0);
    assert_eq!(terminal.observer_degraded_samples, 7);
    let still_live = shared.snapshot.load();
    assert_eq!(still_live.active_count, 1);
    assert_eq!(still_live.observer_degraded_samples, 7);
    publish_terminal_metrics(&terminal, &shared, qpc_clock, 11);

    let snapshot = shared.snapshot.load();
    assert_eq!(snapshot.active_count, 0);
    assert_eq!(snapshot.possibly_active_count, 0);
    assert_eq!(snapshot.failed_release_count, 0);
    assert_eq!(snapshot.observer_degraded_samples, 7);
}

#[test]
fn observer_and_dispatch_publications_do_not_overwrite_each_other() {
    let shared = SharedMetrics::default();
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let dispatch = WorkerMetricsLocal {
        elapsed_us: 4,
        total_us: 9,
        ..WorkerMetricsLocal::default()
    };
    let observer = WorkerMetricsLocal {
        observer_degraded_samples: 2,
        wait_window_sample_count: 6,
        ..WorkerMetricsLocal::default()
    };
    assert!(try_publish_metrics(&dispatch, &shared, qpc_clock, 0, true));
    assert!(try_publish_observer_metrics(
        &observer, &shared, qpc_clock, 0, true
    ));

    let published = shared.snapshot.load();
    assert_eq!(published.elapsed_us, 4);
    assert_eq!(published.total_us, 9);
    assert_eq!(published.observer_degraded_samples, 2);
    assert_eq!(published.wait_window_sample_count, 6);
}

#[test]
fn publication_derives_public_lateness_microseconds_from_ticks() {
    let shared = SharedMetrics::default();
    let local = WorkerMetricsLocal {
        max_sendinput_pre_call_lateness_ticks: 1_234,
        max_sendinput_pre_call_lateness_us: 999_999,
        wait_planned_gap_max_ticks: 2_345,
        physical_target_to_wake_max_ticks: 3_456,
        ..WorkerMetricsLocal::default()
    };
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());

    try_publish_metrics(&local, &shared, qpc_clock, 0, true);

    let published = shared.snapshot.load();
    assert_eq!(published.max_sendinput_pre_call_lateness_ticks, 1_234);
    assert_eq!(published.max_sendinput_pre_call_lateness_us, 1_234);
    assert_eq!(published.wait_planned_gap_max_us, 2_345);
    assert_eq!(published.physical_target_to_wake_max_us, 3_456);
}

#[test]
fn publication_derives_floor_delay_microseconds_from_qpc_ticks() {
    let shared = SharedMetrics::default();
    let local = WorkerMetricsLocal {
        max_hold_floor_delay_ticks: 1_234,
        max_release_floor_delay_ticks: 2_345,
        ..WorkerMetricsLocal::default()
    };
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());

    try_publish_metrics(&local, &shared, qpc_clock, 0, true);

    let published = shared.snapshot.load();
    assert_eq!(published.max_hold_floor_delay_ticks, 1_234);
    assert_eq!(published.max_hold_floor_delay_us, 1_234);
    assert_eq!(published.max_release_floor_delay_ticks, 2_345);
    assert_eq!(published.max_release_floor_delay_us, 2_345);
}

#[test]
fn precision_timing_summary_keeps_exact_boundaries_and_lower_bounds() {
    let mut metrics = WorkerMetricsLocal::default();
    metrics.observe_precision_packet_timing(PrecisionPacketTimingObservation {
        authored_target_qpc: 100,
        packet_not_before_qpc: 110,
        pre_call_qpc: Some(125),
        sendinput_completion_qpc: Some(132),
        previous_sendinput_completion_qpc: Some(115),
        previous_sender_completion_known: true,
        transport_anomaly: true,
    });
    assert_eq!(metrics.precision_packet_attempts, 1);
    assert_eq!(metrics.authored_lateness_samples, 1);
    assert_eq!(metrics.max_authored_lateness_ticks, 25);
    assert_eq!(metrics.max_authored_lateness_abs_ticks, 25);
    assert_eq!(metrics.physical_floor_displacement_samples, 1);
    assert_eq!(metrics.max_physical_floor_displacement_ticks, 10);
    assert_eq!(metrics.residual_lateness_samples, 1);
    assert_eq!(metrics.max_residual_lateness_ticks, 15);
    assert_eq!(metrics.hol_delay_lower_bound_samples, 1);
    assert_eq!(metrics.max_hol_delay_lower_bound_ticks, 5);
    assert_eq!(metrics.unattributed_lateness_samples, 1);
    assert_eq!(metrics.max_unattributed_lateness_ticks, 10);
    assert_eq!(metrics.sendinput_envelope_samples, 1);
    assert_eq!(metrics.max_sendinput_envelope_ticks, 7);
    assert_eq!(metrics.overdue_packet_observations, 1);
    assert_eq!(metrics.transport_anomalies, 1);
    assert_eq!(metrics.timing_arithmetic_anomalies, 0);
}

#[test]
fn precision_timing_summary_does_not_invent_missing_samples_and_counts_bad_ordering() {
    let mut metrics = WorkerMetricsLocal::default();
    metrics.observe_precision_packet_timing(PrecisionPacketTimingObservation {
        authored_target_qpc: 200,
        packet_not_before_qpc: 190,
        ..PrecisionPacketTimingObservation::default()
    });
    assert_eq!(metrics.precision_packet_attempts, 1);
    assert_eq!(metrics.pre_call_qpc_unavailable_attempts, 1);
    assert_eq!(metrics.sendinput_completion_qpc_unavailable_attempts, 1);
    assert_eq!(metrics.authored_lateness_samples, 0);
    assert_eq!(metrics.residual_lateness_samples, 0);
    assert_eq!(metrics.sendinput_envelope_samples, 0);
    assert_eq!(metrics.physical_floor_displacement_samples, 0);
    assert_eq!(metrics.timing_arithmetic_anomalies, 1);
    assert_eq!(metrics.overdue_packet_observations, 0);
}
