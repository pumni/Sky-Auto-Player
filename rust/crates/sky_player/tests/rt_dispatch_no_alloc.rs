//! §8.11 — No-allocation gate for the dispatch critical path.
//!
//! Verifies that the allocation-free segment of the authored dispatch path
//! (from plan build through `dispatch_ready` — i.e. coordinator commit and
//! observer enqueue) makes zero heap allocations. Observer drain (health,
//! publish) is explicitly excluded from the counter window per §8.11.
//!
//! Uses a counting `GlobalAlloc` wrapper that increments an atomic counter on
//! every `alloc` call.  The counter is *only* read inside the assertion window;
//! allocations made by test harness setup before and after are not counted.
//!
//! Run with:
//!   cargo test --manifest-path rust/Cargo.toml -p sky_player \
//!       --features test-support --test rt_dispatch_no_alloc

use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};
use sky_player::engine::StagedDispatchOutcome;
use sky_player::engine::dispatch_primitives::{
    DispatchObservation, DispatchObservationEvidence, DispatchPath, DispatchStep,
    DownMissObservation, DownObservation, OBSERVATION_QUEUE_CAPACITY, PendingObservationQueue,
    ProductionDispatchTestHarness, UpObservation, UpTraceObservation,
    is_clean_dispatch_observation,
};

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

type ProductionPacketCapture =
    Arc<std::sync::Mutex<Vec<(PhysicalPacket, sky_dispatch_win32::input::SendEvidence)>>>;

// ---------------------------------------------------------------------------
// Counting allocator — delegates to System, but increments a global counter
// on every alloc/realloc call.
// ---------------------------------------------------------------------------

struct CountingAllocator;

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static COUNTING_ENABLED: AtomicU64 = AtomicU64::new(0);
static CURRENT_THREAD_ID: AtomicU64 = AtomicU64::new(0);

fn current_thread_id_u64() -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    std::thread::current().id().hash(&mut hasher);
    hasher.finish()
}

#[inline]
fn is_counting_active() -> bool {
    if COUNTING_ENABLED.load(Ordering::Relaxed) == 0 {
        return false;
    }
    CURRENT_THREAD_ID.load(Ordering::Relaxed) == current_thread_id_u64()
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if is_counting_active() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegates directly to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr came from System.alloc.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if is_counting_active() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegates directly to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if is_counting_active() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegates directly to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

static TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

fn enable_counting() {
    ALLOC_COUNT.store(0, Ordering::SeqCst);
    CURRENT_THREAD_ID.store(current_thread_id_u64(), Ordering::SeqCst);
    COUNTING_ENABLED.store(1, Ordering::SeqCst);
}

fn disable_counting() -> u64 {
    COUNTING_ENABLED.store(0, Ordering::SeqCst);
    CURRENT_THREAD_ID.store(0, Ordering::SeqCst);
    ALLOC_COUNT.load(Ordering::SeqCst)
}

fn production_resume_barrier_fixture() -> (
    ProductionDispatchTestHarness,
    ProductionPacketCapture,
    QpcTicks,
    QpcTicks,
) {
    use sky_dispatch_core::executor::DispatchDecision;
    use std::sync::atomic::Ordering;

    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
            .expect("production scheduler lifecycle fixture");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    let (_, cleanup_completion) =
        harness.configure_mock_cleanup_emitter_with_completion_qpc_for_test();
    let now = harness
        .qpc_now_for_test()
        .expect("resume barrier setup QPC");
    let one_second = harness
        .qpc_duration_from_us_for_test(1_000_000)
        .expect("past scheduler epoch conversion");
    let epoch = QpcTicks::from_raw(now.as_u64().saturating_sub(one_second.as_u64()));
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("resume barrier playback epoch");

    let (up_mask, down_chord) = match harness
        .evaluate_live_scheduler_for_test(now, epoch)
        .expect("initial live scheduler evaluation")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            ..
        } => (up_mask, down_chord.map(Into::into)),
        decision => panic!("expected initial production Down, got {decision:?}"),
    };
    assert_eq!(up_mask, 0);
    assert!(down_chord.is_some());
    let candidate = harness
        .stage_candidate_for_test(up_mask, down_chord, TimelineTicks::ZERO)
        .expect("stage initial production Down")
        .expect("initial production Down candidate");
    harness.set_mock_sender_start_qpc_for_test(now);
    assert!(matches!(
        harness.dispatch_staged_candidate_for_test(candidate, false),
        StagedDispatchOutcome::Dispatched
    ));

    harness
        .apply_scheduler_resumable_lifecycle_transition_with_post_cleanup_qpc_for_test(|| {
            Ok(QpcTicks::from_raw(
                cleanup_completion.load(Ordering::Acquire),
            ))
        })
        .expect("verified production scheduler cleanup");
    let completion = QpcTicks::from_raw(cleanup_completion.load(Ordering::Acquire));
    let barrier = harness
        .resume_barrier_qpc_for_test()
        .expect("production scheduler resume barrier");
    let timing = harness
        .dispatch_timing_config_for_test()
        .expect("production scheduler timing config");
    assert_eq!(timing.min_release_gap_us, 17_167);
    let gap = harness
        .qpc_duration_from_us_for_test(timing.min_release_gap_us)
        .expect("production scheduler G conversion");
    assert_eq!(barrier, completion.checked_add_duration(gap).unwrap());
    let accounting = harness.generation_accounting_for_test();
    assert_eq!(accounting.total, 2);
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.cancelled, 1);
    assert_eq!(accounting.scheduled, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    packets.lock().expect("production packet capture").clear();

    (harness, packets, epoch, barrier)
}

fn wait_until_qpc(deadline: QpcTicks) {
    loop {
        let now = sky_dispatch_win32::clock::QpcClock::initialize()
            .expect("noalloc test QPC")
            .now()
            .expect("noalloc test QPC sample");
        if now >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
}

fn down_observation(n: u64) -> DispatchObservation {
    DispatchObservation::Down(DownObservation::test_fixture(n))
}

fn up_observation(n: u64) -> DispatchObservation {
    DispatchObservation::Up(UpObservation {
        physical_target_qpc: QpcTicks::ZERO,
        final_policy_qpc: QpcTicks::ZERO,
        pre_call_qpc: QpcTicks::ZERO,
        sendinput_completion_qpc: QpcTicks::ZERO,
        dispatch_ready_qpc: Some(QpcTicks::ZERO),
        pre_call_to_completion_ticks: DurationTicks::from_raw(n),
        wake_qpc: None,
        precision_handoff: None,
        requested_mask: 1,
        confirmed_mask: 1,
        skipped_mask: 0,
        result_status: SendTransactionStatus::Complete,
        completed_effective_ticks: TimelineTicks::from_raw(n),
        scheduled_ticks: TimelineTicks::ZERO,
        deferred_ticks: DurationTicks::ZERO,
        up_completion_error_ticks: 0,
        recovery_pause_ticks: None,
        trace: UpTraceObservation {
            event_index: 0,
            compiled_packet_index: None,
            trace_kind: 1,
            retry_reason: PacketRetryReason::None,
            send_attempts: 1,
            last_win32_error: 0,
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            sendinput_completion_ticks: Some(TimelineTicks::ZERO),
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            deferred_ticks: DurationTicks::ZERO,
            recovery_required: false,
        },
    })
}

fn down_miss_observation(n: u64) -> DispatchObservation {
    DispatchObservation::DownMiss(DownMissObservation::test_fixture(n))
}

fn clean_dispatch_evidence(requested_count: usize) -> DispatchObservationEvidence {
    DispatchObservationEvidence {
        status: SendTransactionStatus::Complete,
        attempts: 1,
        retry_reason: PacketRetryReason::None,
        requested_count,
        confirmed_count: requested_count,
        skipped_count: 0,
        timing_valid: true,
        transport_anomaly: false,
        recovery_used: false,
        chord_integrity_lost: false,
    }
}

#[test]
fn canonical_clean_dispatch_evidence_requires_every_dimension() {
    let clean = clean_dispatch_evidence(2);
    assert!(is_clean_dispatch_observation(clean));

    let cases = [
        DispatchObservationEvidence {
            status: SendTransactionStatus::PartialProgress,
            ..clean
        },
        DispatchObservationEvidence {
            attempts: 2,
            ..clean
        },
        DispatchObservationEvidence {
            retry_reason: PacketRetryReason::ZeroProgress,
            ..clean
        },
        DispatchObservationEvidence {
            confirmed_count: 1,
            ..clean
        },
        DispatchObservationEvidence {
            skipped_count: 1,
            ..clean
        },
        DispatchObservationEvidence {
            timing_valid: false,
            ..clean
        },
        DispatchObservationEvidence {
            transport_anomaly: true,
            ..clean
        },
        DispatchObservationEvidence {
            recovery_used: true,
            ..clean
        },
        DispatchObservationEvidence {
            chord_integrity_lost: true,
            ..clean
        },
    ];
    for evidence in cases {
        assert!(!is_clean_dispatch_observation(evidence));
    }
}

// ---------------------------------------------------------------------------
// §8.11 Tests
// ---------------------------------------------------------------------------

/// Single push into an empty queue does not allocate.
#[test]
fn push_single_down_observation_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let queue = PendingObservationQueue::default();
    let mut dropped = 0u64;
    let obs = down_observation(1);

    enable_counting();
    queue.push(obs, &mut dropped);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "push(DownObservation) made {allocs} heap allocation(s); expected 0"
    );
    assert_eq!(dropped, 0);
}

/// push + pop_front cycle does not allocate.
#[test]
fn push_pop_cycle_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let queue = PendingObservationQueue::default();
    let mut dropped = 0u64;

    // Warm up outside the window.
    queue.push(down_observation(1), &mut dropped);
    let _ = queue.pop_front();

    enable_counting();
    queue.push(down_observation(2), &mut dropped);
    let popped = queue.pop_front();
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "push+pop_front made {allocs} heap allocation(s); expected 0"
    );
    assert!(popped.is_some());
}

/// Up-observation enqueue does not allocate.
#[test]
fn push_up_observation_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let queue = PendingObservationQueue::default();
    let mut dropped = 0u64;
    let obs = up_observation(42);

    enable_counting();
    queue.push(obs, &mut dropped);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "push(UpObservation) made {allocs} heap allocation(s); expected 0"
    );
}

/// Filling the queue to capacity and dropping a new observation does not
/// allocate or block the producer.
#[test]
fn push_at_capacity_drop_new_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let queue = PendingObservationQueue::default();
    let mut dropped = 0u64;

    // Fill to capacity outside the measurement window.
    for i in 0..OBSERVATION_QUEUE_CAPACITY {
        queue.push(down_observation(i as u64), &mut dropped);
    }
    assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
    assert_eq!(dropped, 0);

    // One more push is dropped without allocating or waiting for the consumer.
    enable_counting();
    queue.push(down_observation(999), &mut dropped);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "push at capacity (drop-new path) made {allocs} heap allocation(s); expected 0"
    );
    assert_eq!(dropped, 1, "overflow must increment dropped counter");
    assert_eq!(
        queue.len(),
        OBSERVATION_QUEUE_CAPACITY,
        "queue must remain at capacity after drop-new"
    );
}

/// Queue overflow is a producer-only drop-new operation. The existing raw
/// records remain queued for deferred processing; no observer materialization
/// or telemetry builder is available on this path.
#[test]
fn overflow_drops_newest_for_down_and_up() {
    let _lock = TEST_LOCK.lock();

    for newest in [
        down_observation(999),
        up_observation(999),
        down_miss_observation(999),
    ] {
        let queue = PendingObservationQueue::default();
        let mut dropped = 0_u64;
        for index in 0..OBSERVATION_QUEUE_CAPACITY {
            queue.push(down_observation(index as u64), &mut dropped);
        }

        queue.push(newest, &mut dropped);

        assert_eq!(dropped, 1, "one newest observation must be dropped");
        assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
        let first = queue.pop_front().expect("queue remains non-empty");
        match first {
            DispatchObservation::Down(observation) => {
                assert_eq!(observation.trace.event_index, 0);
            }
            DispatchObservation::DownMiss(_) => panic!("miss observation not expected"),
            DispatchObservation::Up(_) => panic!("unexpected Up observation in seeded queue"),
            DispatchObservation::Wait(_) => panic!("wait observation not expected"),
            DispatchObservation::StaleMetadata(_) => panic!("stale observation not expected"),
            DispatchObservation::BlockedUnfocused(_) => {
                panic!("blocked observation not expected")
            }
        }

        let mut last = None;
        while let Some(observation) = queue.pop_front() {
            last = Some(observation);
        }
        match last.expect("newest observation must be retained") {
            DispatchObservation::Down(observation) => {
                assert_eq!(
                    observation.trace.event_index,
                    (OBSERVATION_QUEUE_CAPACITY - 1) as u32
                )
            }
            DispatchObservation::DownMiss(_) => panic!("newest miss observation must be dropped"),
            DispatchObservation::Up(_) => panic!("newest Up observation must be dropped"),
            DispatchObservation::Wait(_) => panic!("wait observation not expected"),
            DispatchObservation::StaleMetadata(_) => panic!("stale observation not expected"),
            DispatchObservation::BlockedUnfocused(_) => {
                panic!("blocked observation not expected")
            }
        }
    }
}

/// Burst of N pushes followed by a full drain — no allocations throughout.
#[test]
fn burst_push_and_drain_no_alloc() {
    let _lock = TEST_LOCK.lock();
    const BURST: usize = 16;
    let queue = PendingObservationQueue::default();
    let mut dropped = 0u64;

    enable_counting();
    for i in 0..BURST {
        let observation = if i % 2 == 0 {
            down_miss_observation(i as u64)
        } else {
            down_observation(i as u64)
        };
        queue.push(observation, &mut dropped);
    }
    let mut drained = 0usize;
    while queue.pop_front().is_some() {
        drained += 1;
    }
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "burst push+drain made {allocs} heap allocation(s); expected 0"
    );
    assert_eq!(drained, BURST);
    assert_eq!(dropped, 0);
    // The producer never samples queue occupancy; the observer consumer owns
    // the high-watermark diagnostic.
}

/// pop_front on an empty queue does not allocate.
#[test]
fn pop_empty_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let queue = PendingObservationQueue::default();

    enable_counting();
    let result = queue.pop_front();
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "pop_front on empty queue made {allocs} heap allocation(s); expected 0"
    );
    assert!(result.is_none());
}

/// DownOnly production dispatch hard-path makes ZERO heap allocations.
#[test]
fn production_down_only_hard_path_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.set_live_diagnostics_enabled_for_test(true);
    harness.align_next_plan_to_future_for_test(100_000);

    enable_counting();
    let plan = harness.plan_current_dispatch();
    assert_eq!(
        plan.authored_path().unwrap(),
        DispatchPath::DownOnly { down_count: 1 }
    );
    let step = harness.dispatch_authored_with_plan(&plan);
    harness.publish_live_diagnostics_for_test();
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "production DownOnly hard-path made {allocs} heap allocation(s); expected 0"
    );
    assert!(
        matches!(step, DispatchStep::Dispatched),
        "down-only dispatch returned {step:?}"
    );
    assert!(harness.has_active_generation(0x15));
    assert_eq!(harness.chord_integrity_lost_count(), 0);
    assert_eq!(
        harness
            .fine_pre_call_bucket_counts_for_test()
            .iter()
            .sum::<u64>(),
        1,
        "successful production SendInput dispatch must record one fine pre-call bucket"
    );
    assert_eq!(harness.published_sender_sample_count_for_test(), 1);
}

/// Mixed production dispatch hard-path makes ZERO heap allocations.
#[test]
fn production_mixed_hard_path_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    harness.align_next_plan_to_future_for_test(100_000);

    // Step 1: Dispatch initial Down key A COMPLETELY OUTSIDE allocation measurement.
    let plan0 = harness.plan_current_dispatch();
    assert_eq!(
        plan0.authored_path().unwrap(),
        DispatchPath::DownOnly { down_count: 1 }
    );
    let step0 = harness.dispatch_authored_with_plan(&plan0);
    assert!(matches!(step0, DispatchStep::Dispatched));
    assert!(harness.has_active_generation(0x15));

    // Step 2: Advance deterministic effective time to mixed packet deadline
    harness.advance_playback_time_us(10_000);

    // Step 3: Measurement window covering real production plan + prepare + admission + send + commit
    enable_counting();
    let plan = harness.plan_current_dispatch();
    assert_eq!(
        plan.authored_path().unwrap(),
        DispatchPath::Mixed {
            up_count: 1,
            down_count: 1,
        }
    );
    let step = harness.dispatch_authored_with_plan(&plan);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "production Mixed hard-path made {allocs} heap allocation(s); expected 0"
    );
    assert!(
        matches!(step, DispatchStep::Dispatched),
        "mixed dispatch returned {step:?}"
    );

    // Assert coordinator state: old generation A terminal/released, new generation B active
    assert!(!harness.has_active_generation(0x15));
    assert!(harness.has_active_generation(0x16));
    assert_eq!(harness.chord_integrity_lost_count(), 0);
}

/// A normal mixed packet remains allocation-free when completion evidence is
/// present: floor timing sends it atomically without Up recovery.
#[test]
fn production_normal_mixed_completion_floor_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let packets = harness.configure_packet_capture();

    let first = harness.plan_current_dispatch();
    let first_target = harness
        .physical_target_qpc_for_test(&first)
        .expect("first Down target");
    let before_first_target = QpcTicks::from_raw(
        first_target
            .as_u64()
            .checked_sub(1)
            .expect("first Down future tick"),
    );
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&first, before_first_target),
        DispatchStep::NoWork
    ));
    let first_step = harness.dispatch_at_qpc_for_test(&first, first_target);
    assert!(matches!(first_step, DispatchStep::Dispatched));

    enable_counting();
    let mixed = harness.plan_current_dispatch();
    let mixed_target = harness
        .physical_target_qpc_for_test(&mixed)
        .expect("mixed authored target");
    let before_mixed_target = QpcTicks::from_raw(
        mixed_target
            .as_u64()
            .checked_sub(1)
            .expect("mixed future tick"),
    );
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&mixed, before_mixed_target),
        DispatchStep::NoWork
    ));
    let physical_wait_target = harness
        .physical_wait_target_for_test(&mixed)
        .expect("mixed physical wait target")
        .expect("mixed physical target");
    let classify_step = harness.dispatch_at_qpc_for_test(&mixed, physical_wait_target);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "normal mixed completion-floor path made {allocs} allocation(s)"
    );
    assert!(matches!(classify_step, DispatchStep::Dispatched));
    assert_eq!(
        packets.lock().expect("packet capture").as_slice(),
        &[PhysicalPacket::new(0, 1), PhysicalPacket::new(1, 2)]
    );
}

#[test]
fn production_deadline_handoff_down_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(1, 100_000);
    harness.align_next_plan_to_future_for_test(100_000);
    let plan = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_test(QpcTicks::from_raw(1));

    enable_counting();
    let step = harness.dispatch_at_plan_target_for_test(&plan);
    let allocs = disable_counting();

    assert_eq!(allocs, 0);
    assert!(matches!(step, DispatchStep::Dispatched), "step={step:?}");
}

#[test]
fn production_deadline_handoff_mixed_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_mixed_events_with_gap(2, 100_000);
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_test(QpcTicks::from_raw(1));

    enable_counting();
    let step = harness.dispatch_at_plan_target_for_test(&plan);
    let allocs = disable_counting();

    assert_eq!(allocs, 0);
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_deadline_handoff_up_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_uponly_release_with_gap(100_000);
    harness.advance_playback_time_us(100_000);
    let plan = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_test(QpcTicks::from_raw(1));

    enable_counting();
    let step = harness.wait_and_dispatch_current_plan(&plan);
    let allocs = disable_counting();

    assert!(
        matches!(step, Ok(DispatchStep::Dispatched)),
        "step={step:?}"
    );
    assert_eq!(allocs, 0);
}

#[test]
fn production_fifteen_key_down_chord_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(15, 1);
    harness.align_next_plan_to_future_for_test(100_000);

    enable_counting();
    let plan = harness.plan_current_dispatch();
    let step = harness.dispatch_authored_with_plan(&plan);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "15-key Down chord allocated {allocs} time(s)");
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_pending_release_up_only_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_deferred_release_with_unrelated_down();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&first),
        Ok(DispatchStep::Dispatched)
    ));
    let authored = harness.plan_current_dispatch();
    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&authored),
        Ok(DispatchStep::Dispatched)
    ));

    enable_counting();
    let pending = harness.plan_current_dispatch();
    assert_eq!(
        pending.authored_path(),
        Some(DispatchPath::UpOnly { up_count: 2 })
    );
    let step = harness.wait_and_dispatch_current_plan(&pending);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "pending Up-only path allocated {allocs} time(s)");
    assert!(matches!(step, Ok(DispatchStep::Dispatched)));
}

#[test]
fn production_coalesced_pending_up_and_authored_down_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness =
        ProductionDispatchTestHarness::new_coalesced_pending_release_with_unrelated_down();
    harness.align_next_plan_to_future_for_test(100_000);
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&first),
        Ok(DispatchStep::Dispatched)
    ));
    harness.seed_pending_release_for_test(0x15, 21_000);
    harness.advance_playback_time_us(21_000);
    harness.align_next_plan_to_future_for_test(100_000);
    let coalesced = harness.plan_current_dispatch();
    assert_eq!(
        coalesced.authored_path(),
        Some(DispatchPath::Mixed {
            up_count: 1,
            down_count: 1,
        })
    );
    enable_counting();
    // The allocation assertion is about the frozen-plan dispatch segment;
    // use the test-controlled exact boundary so host timer overshoot cannot
    // turn this structural probe into a deadline failure.
    let step = harness.dispatch_at_plan_target_for_test(&coalesced);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "coalesced Mixed path allocated {allocs} time(s)");
    assert!(matches!(step, DispatchStep::Dispatched), "step={step:?}");
}

#[test]
fn production_metadata_only_stale_commit_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_stale_metadata_boundary();
    harness.align_next_plan_to_future_for_test(100_000);
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        &first,
        sky_player::engine::dispatch_primitives::NextDispatchPlan::Metadata(_)
    ));
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        DispatchStep::Dispatched
    ));
    let second = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&second),
        DispatchStep::Dispatched
    ));
    let third = harness.plan_current_dispatch();
    let third_step = harness.dispatch_at_plan_target_for_test(&third);
    assert!(
        matches!(third_step, DispatchStep::Dispatched),
        "third step={third_step:?}"
    );

    let metadata = harness.plan_current_dispatch();
    assert!(matches!(
        &metadata,
        sky_player::engine::dispatch_primitives::NextDispatchPlan::Metadata(_)
    ));
    enable_counting();
    let step = harness.dispatch_at_plan_target_for_test(&metadata);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "stale metadata commit allocated {allocs} time(s)"
    );
    assert!(matches!(step, DispatchStep::Dispatched), "step={step:?}");
}

#[test]
fn production_missed_down_recovery_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.align_next_plan_to_future_for_test(100_000);
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        DispatchStep::Dispatched
    ));
    harness.advance_playback_time_us(100_000);

    enable_counting();
    let overdue = harness.plan_current_dispatch();
    let step = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&overdue);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "backlog decision allocated {allocs} time(s)");
    assert!(matches!(step, DispatchStep::Dispatched), "step={step:?}");
}

#[test]
fn production_mixed_missed_down_recovery_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        DispatchStep::Dispatched
    ));
    harness.advance_playback_time_us(100_000);
    let overdue = harness.plan_current_dispatch();
    harness.configure_packet_capture();

    enable_counting();
    let classify_step = harness.dispatch_same_frozen_plan_after_due_without_wait_for_test(&overdue);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "overdue mixed normal dispatch allocated {allocs} time(s)"
    );
    assert!(
        matches!(classify_step, DispatchStep::Dispatched),
        "step={classify_step:?}"
    );
}

#[test]
fn production_strict_late_authorized_down_send_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_two_down_boundaries();
    harness.set_strict_timing_for_test(true);
    harness.set_strict_down_completion_late_us_for_test(1_000_000);
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_at_plan_target_for_test(&first),
        DispatchStep::Dispatched
    ));
    let future = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&future),
        DispatchStep::NoWork
    ));
    harness.configure_packet_capture();

    let target = harness
        .physical_target_qpc_for_test(&future)
        .expect("strict future Down target");
    let late_now = target
        .checked_add_duration(DurationTicks::from_raw(1))
        .expect("strict late Down boundary");

    enable_counting();
    let step = harness.dispatch_at_qpc_for_test(&future, late_now);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "late strict Down dispatch allocated {allocs} time(s)"
    );
    assert!(matches!(step, DispatchStep::Dispatched), "step={step:?}");
}

#[test]
fn production_normal_late_down_dispatch_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness =
        ProductionDispatchTestHarness::new_dense_future_boundary_with_gap_for_test(5_000);
    let timing_margin_us = harness
        .timing_margin_us_for_benchmark()
        .expect("timing margin");
    let packets = harness.configure_packet_capture();
    let plan = harness.plan_current_dispatch();
    let target = harness
        .physical_target_qpc_for_test(&plan)
        .expect("physical target");
    // Target pre-call time is beyond the authored physical target. Normal sender
    // admission must still dispatch without a lateness-only cutoff.
    let late_offset_us = timing_margin_us + 1_000;
    let late_now = target
        .checked_add_duration(
            harness
                .qpc_duration_from_us_for_test(late_offset_us)
                .expect("duration conversion"),
        )
        .expect("late QPC");

    // Observe future boundary before target outside the allocation measurement window.
    let before_target =
        QpcTicks::from_raw(target.as_u64().checked_sub(1).expect("target is nonzero"));
    assert!(matches!(
        harness.dispatch_at_qpc_for_test(&plan, before_target),
        DispatchStep::NoWork
    ));

    enable_counting();
    let step = harness.dispatch_at_qpc_for_test(&plan, late_now);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "normal late Down dispatch allocated {allocs} time(s)"
    );
    assert!(
        matches!(step, DispatchStep::Dispatched),
        "normal late Down must dispatch cleanly: step={step:?}"
    );
    assert_eq!(
        packets.lock().unwrap().len(),
        1,
        "normal late Down must emit exactly 1 packet"
    );
}

#[test]
fn lifecycle_resume_barrier_guard_query_no_alloc() {
    let _lock = TEST_LOCK.lock();
    use sky_dispatch_core::executor::DispatchDecision;

    let (harness, packets, epoch, barrier) = production_resume_barrier_fixture();
    let one_tick_early = QpcTicks::from_raw(barrier.as_u64() - 1);

    enable_counting();
    let queried = harness
        .evaluate_live_scheduler_for_test(one_tick_early, epoch)
        .expect("G-1 production scheduler query");
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "G-1 production scheduler query allocated");
    assert!(matches!(
        queried,
        DispatchDecision::Wait { deadline_qpc } if deadline_qpc == barrier
    ));
    assert!(
        packets
            .lock()
            .expect("production packet capture")
            .is_empty(),
        "G-1 denial performs no production send"
    );
}

#[test]
fn lifecycle_resume_barrier_healthy_down_dispatch_no_alloc() {
    let _lock = TEST_LOCK.lock();
    use sky_dispatch_core::executor::DispatchDecision;

    let (mut harness, packets, epoch, barrier) = production_resume_barrier_fixture();
    let before_barrier = QpcTicks::from_raw(barrier.as_u64() - 1);
    assert!(matches!(
        harness
            .evaluate_live_scheduler_for_test(before_barrier, epoch)
            .expect("G-1 production scheduler evaluation"),
        DispatchDecision::Wait { deadline_qpc } if deadline_qpc == barrier
    ));
    let (up_mask, down_chord) = {
        enable_counting();
        let evaluated = harness
            .evaluate_live_scheduler_for_test(barrier, epoch)
            .expect("exact-G production scheduler evaluation");
        let details = match evaluated {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                ..
            } => (up_mask, down_chord.map(Into::into)),
            decision => panic!("expected exact-G production Send, got {decision:?}"),
        };
        let allocs = disable_counting();
        assert_eq!(allocs, 0, "exact-G production scheduler query allocated");
        details
    };
    assert_eq!(up_mask, 0);
    assert!(down_chord.is_some());
    enable_counting();
    let candidate = harness
        .stage_candidate_for_test(up_mask, down_chord, TimelineTicks::ZERO)
        .expect("stage exact-G production Down")
        .expect("exact-G production Down candidate");
    let stage_allocs = disable_counting();
    assert_eq!(stage_allocs, 0, "exact-G production staging allocated");

    wait_until_qpc(barrier);
    harness.set_mock_sender_start_qpc_for_test(barrier);

    enable_counting();
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "healthy Down with resume barrier allocated");
    assert!(matches!(outcome, StagedDispatchOutcome::Dispatched));
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 1);
    assert_eq!(packets[0].0, PhysicalPacket::new(0, 1));
}

#[test]
fn shipping_executor_b_sendinput_suffix_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let physical_packet = PhysicalPacket::new(0, 0b1);
    let profile = sky_dispatch_win32::input::MaterializedInstrumentKeyProfile::canonical();
    let prepared_packet = sky_dispatch_win32::input::PreparedPhysicalPacket::try_new_with_profile(
        physical_packet,
        &profile,
    )
    .expect("prepared packet");

    let chord = harness.current_executor_chord_for_test();

    let target_qpc = harness.qpc_now_for_test().expect("target QPC");

    enable_counting();
    let res = harness.execute_shipping_b_sendinput_suffix_for_test(
        &prepared_packet,
        0,
        chord.as_ref(),
        target_qpc,
    );
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "actual shipping executor B SendInput suffix allocated"
    );
    assert!(res.is_ok(), "suffix failed: {res:?}");
}

#[test]
fn shipping_b_dispatch_staged_candidate_down_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");

    enable_counting();
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "actual production dispatch_staged_candidate Down allocated memory"
    );
    assert!(
        matches!(
            outcome,
            sky_player::engine::StagedDispatchOutcome::Dispatched
        ),
        "outcome={outcome:?}"
    );
}

#[test]
fn shipping_b_dispatch_staged_candidate_up_only_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    // First complete down
    let chord = harness.current_executor_chord_for_test();
    let candidate_down = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");
    let outcome_down = harness.dispatch_staged_candidate_for_test(candidate_down, false);
    assert!(matches!(
        outcome_down,
        sky_player::engine::StagedDispatchOutcome::Dispatched
    ));

    // Now stage Up
    let candidate_up = harness
        .stage_candidate_for_test(1, None, TimelineTicks::from_raw(20_000))
        .expect("stage candidate")
        .expect("some candidate");

    enable_counting();
    let outcome_up = harness.dispatch_staged_candidate_for_test(candidate_up, true);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "actual production dispatch_staged_candidate Up allocated memory"
    );
    assert!(
        matches!(
            outcome_up,
            sky_player::engine::StagedDispatchOutcome::Dispatched
        ),
        "outcome={outcome_up:?}"
    );
}

#[test]
fn shipping_b_dispatch_staged_candidate_15_slot_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_chord(15);
    harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.attach_executor_for_test().expect("attach executor");

    let chord = harness.current_executor_chord_for_test();
    let candidate_15 = harness
        .stage_candidate_for_test(0, chord, TimelineTicks::ZERO)
        .expect("stage candidate")
        .expect("some candidate");

    enable_counting();
    let outcome = harness.dispatch_staged_candidate_for_test(candidate_15, true);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "actual production dispatch_staged_candidate 15-slot allocated memory"
    );
    assert!(
        matches!(
            outcome,
            sky_player::engine::StagedDispatchOutcome::Dispatched
        ),
        "outcome={outcome:?}"
    );
}
