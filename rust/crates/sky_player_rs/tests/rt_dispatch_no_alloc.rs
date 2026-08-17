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
//!   cargo test --manifest-path rust/Cargo.toml -p sky_player_rs \
//!       --features test-support --test rt_dispatch_no_alloc

use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{PacketRetryReason, PhysicalPacket, SendTransactionStatus};
use sky_player_rs::engine::dispatch_primitives::{
    DispatchObservation, DispatchObservationEvidence, DispatchPath, DispatchStep, DownObservation,
    DownTraceObservation, OBSERVATION_QUEUE_CAPACITY, PendingObservationQueue,
    ProductionDispatchTestHarness, UpObservation, UpTraceObservation,
    is_clean_dispatch_observation,
};

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

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

fn down_observation(n: u64) -> DispatchObservation {
    DispatchObservation::Down(DownObservation {
        path: DispatchPath::DownOnly { down_count: 1 },
        physical_target_qpc: QpcTicks::ZERO,
        final_admission_qpc: QpcTicks::ZERO,
        sendinput_completed_qpc: QpcTicks::ZERO,
        dispatch_ready_qpc: Some(QpcTicks::ZERO),
        admission_to_completion_ticks: DurationTicks::from_raw(n),
        wake_qpc: None,
        requested_packet: PhysicalPacket::new(0, 1),
        confirmed_mask: 1,
        skipped_mask: 0,
        completed_effective_ticks: TimelineTicks::from_raw(n),
        trace: DownTraceObservation {
            event_index: n as u32,
            trace_kind: 0,
            result_status: SendTransactionStatus::Complete,
            send_attempts: 1,
            retry_reason: PacketRetryReason::None,
            chord_integrity_lost: false,
            last_win32_error: 0,
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            final_admission_ticks: Some(TimelineTicks::ZERO),
            sendinput_completed_ticks: Some(TimelineTicks::ZERO),
            recovered_retry_late: false,
            recovered_partial_up: false,
            strict_completion_late: false,
        },
    })
}

fn up_observation(n: u64) -> DispatchObservation {
    DispatchObservation::Up(UpObservation {
        physical_target_qpc: QpcTicks::ZERO,
        final_admission_qpc: QpcTicks::ZERO,
        sendinput_completed_qpc: QpcTicks::ZERO,
        dispatch_ready_qpc: Some(QpcTicks::ZERO),
        admission_to_completion_ticks: DurationTicks::from_raw(n),
        wake_qpc: None,
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
            trace_kind: 1,
            retry_reason: PacketRetryReason::None,
            send_attempts: 1,
            last_win32_error: 0,
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            final_admission_ticks: Some(TimelineTicks::ZERO),
            sendinput_completed_ticks: Some(TimelineTicks::ZERO),
            dispatch_start_error_ticks: n as i64,
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            deferred_ticks: DurationTicks::ZERO,
            recovery_required: false,
        },
    })
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
    let mut high = 0u64;
    let obs = down_observation(1);

    enable_counting();
    queue.push(obs, &mut dropped, &mut high);
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
    let mut high = 0u64;

    // Warm up outside the window.
    queue.push(down_observation(1), &mut dropped, &mut high);
    let _ = queue.pop_front();

    enable_counting();
    queue.push(down_observation(2), &mut dropped, &mut high);
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
    let mut high = 0u64;
    let obs = up_observation(42);

    enable_counting();
    queue.push(obs, &mut dropped, &mut high);
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
    let mut high = 0u64;

    // Fill to capacity outside the measurement window.
    for i in 0..OBSERVATION_QUEUE_CAPACITY {
        queue.push(down_observation(i as u64), &mut dropped, &mut high);
    }
    assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
    assert_eq!(dropped, 0);

    // One more push is dropped without allocating or waiting for the consumer.
    enable_counting();
    queue.push(down_observation(999), &mut dropped, &mut high);
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

    for newest in [down_observation(999), up_observation(999)] {
        let queue = PendingObservationQueue::default();
        let mut dropped = 0_u64;
        let mut high = 0_u64;
        for index in 0..OBSERVATION_QUEUE_CAPACITY {
            queue.push(down_observation(index as u64), &mut dropped, &mut high);
        }

        queue.push(newest, &mut dropped, &mut high);

        assert_eq!(dropped, 1, "one newest observation must be dropped");
        assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
        let first = queue.pop_front().expect("queue remains non-empty");
        match first {
            DispatchObservation::Down(observation) => {
                assert_eq!(observation.trace.event_index, 0);
            }
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
    let mut high = 0u64;

    enable_counting();
    for i in 0..BURST {
        queue.push(down_observation(i as u64), &mut dropped, &mut high);
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
    // Producer paths no longer call queue.len(); high-watermark is an
    // observer-side diagnostic and remains zero on this hard path.
    assert_eq!(high, 0);
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

    enable_counting();
    let plan = harness.plan_current_dispatch();
    assert_eq!(
        plan.authored_path().unwrap(),
        DispatchPath::DownOnly { down_count: 1 }
    );
    let step = harness.dispatch_authored_with_plan(&plan);
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
}

/// Mixed production dispatch hard-path makes ZERO heap allocations.
#[test]
fn production_mixed_hard_path_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_mixed();

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

#[test]
fn production_deadline_handoff_down_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let plan = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_test(QpcTicks::from_raw(1));

    enable_counting();
    let step = harness.dispatch_due_from_plan_for_test(&plan);
    let allocs = disable_counting();

    assert_eq!(allocs, 0);
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_deadline_handoff_mixed_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_mixed();
    let initial_plan = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_authored_with_plan(&initial_plan),
        DispatchStep::Dispatched
    ));
    harness.advance_playback_time_us(10_000);
    let plan = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_test(QpcTicks::from_raw(1));

    enable_counting();
    let step = harness.dispatch_due_from_plan_for_test(&plan);
    let allocs = disable_counting();

    assert_eq!(allocs, 0);
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_deadline_handoff_up_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_uponly_release();
    harness.advance_playback_time_us(10_000);
    let plan = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_test(QpcTicks::from_raw(1));

    enable_counting();
    let step = harness.dispatch_due_from_plan_for_test(&plan);
    let allocs = disable_counting();

    assert_eq!(allocs, 0);
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_fifteen_key_down_chord_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_chord_with_gap(15, 0);

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
        harness.dispatch_due_from_plan_for_test(&first),
        DispatchStep::Dispatched
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
        Some(DispatchPath::UpOnly { up_count: 1 })
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
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first),
        DispatchStep::Dispatched
    ));
    let deferred = harness.plan_current_dispatch();
    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&deferred),
        Ok(DispatchStep::Dispatched)
    ));

    harness.advance_playback_time_us(1_000);
    let coalesced = harness.plan_current_dispatch();
    assert_eq!(
        coalesced.authored_path(),
        Some(DispatchPath::Mixed {
            up_count: 1,
            down_count: 1,
        })
    );
    harness.set_deadline_wake_for_plan_for_test(&coalesced);
    enable_counting();
    let step = harness.dispatch_due_from_plan_for_test(&coalesced);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "coalesced Mixed path allocated {allocs} time(s)");
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_metadata_only_deferred_commit_at_equal_boundary_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_pending_release_with_metadata_boundary();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first),
        DispatchStep::Dispatched
    ));
    let second = harness.plan_current_dispatch();
    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&second),
        Ok(DispatchStep::Dispatched)
    ));
    let deferred = harness.plan_current_dispatch();
    assert!(matches!(
        harness.wait_and_dispatch_current_plan(&deferred),
        Ok(DispatchStep::Dispatched)
    ));

    harness.advance_playback_time_us(1_000);
    let equal_boundary = harness.plan_current_dispatch();
    harness.set_deadline_wake_for_plan_for_test(&equal_boundary);
    enable_counting();
    let step = harness.dispatch_due_from_plan_for_test(&equal_boundary);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "equal-boundary metadata commit allocated {allocs} time(s)"
    );
    assert!(matches!(step, DispatchStep::Dispatched));
}

#[test]
fn production_backlog_abort_decision_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_down_only();
    let first = harness.plan_current_dispatch();
    assert!(matches!(
        harness.dispatch_due_from_plan_for_test(&first),
        DispatchStep::Dispatched
    ));
    harness.advance_playback_time_us(100_000);

    enable_counting();
    let overdue = harness.plan_current_dispatch();
    let step = harness.dispatch_due_from_plan_for_test(&overdue);
    let allocs = disable_counting();

    assert_eq!(allocs, 0, "backlog decision allocated {allocs} time(s)");
    assert!(match step {
        DispatchStep::Terminate(error) => error.contains("catch-up"),
        DispatchStep::TerminateStatic(error) => error.contains("catch-up"),
        _ => false,
    });
}
