//! §8.11 — No-allocation gate for the dispatch critical path.
//!
//! Verifies that the allocation-free segment of the authored dispatch path
//! (from plan build through `dispatch_ready` — i.e. coordinator commit and
//! observer enqueue) makes zero heap allocations.  Observer drain (estimator,
//! health, publish) is explicitly excluded from the counter window per §8.11.
//!
//! Uses a counting `GlobalAlloc` wrapper that increments an atomic counter on
//! every `alloc` call.  The counter is *only* read inside the assertion window;
//! allocations made by test harness setup before and after are not counted.
//!
//! Run with:
//!   cargo test --manifest-path rust/Cargo.toml -p sky_player_rs \
//!       --features test-support --test rt_dispatch_no_alloc

use sky_dispatch_core::estimator::LatencyClass;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::input::{PacketRetryReason, SendTransactionStatus};
use sky_player_rs::engine::dispatch_primitives::{
    DispatchObservation, DispatchPath, DispatchStep, DownObservation, DownTraceObservation,
    EstimatorObservationEvidence, OBSERVATION_QUEUE_CAPACITY, PendingObservationQueue,
    ProductionDispatchTestHarness, UpObservation, UpTraceObservation,
    is_clean_estimator_observation,
};
use sky_player_rs::engine::observer_test_hooks::{
    observer_test_hook_guard, set_release_observer_failure_on_recovery,
    set_release_telemetry_failure_on_recovery,
};

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
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
        latency_class: LatencyClass::Hot,
        lead_down_saturated: false,
        lead_down: n,
        timeline_rebase_count: 0,
        timeline_rebase_total_ticks: DurationTicks::ZERO,
        timeline_rebase_max_ticks: DurationTicks::ZERO,
        timeline_rebase_last_reason: 0,
        sender_duration_us: n,
        delivered_count: 1,
        batch_intent_count: 1,
        completion_error_us: 0,
        estimator_evidence: clean_estimator_evidence(1),
        completed_effective: n,
        authored_batch_scheduled_us: 0,
        batch_scheduled_us: 0,
        sender_completed_qpc: QpcTicks::ZERO,
        worker_ready_qpc: QpcTicks::ZERO,
        send_warn_us: 0,
        core_post_send_warn_us: 0,
        trace: DownTraceObservation {
            event_index: 0,
            trace_kind: 0,
            result_success: true,
            requested_count: 1,
            sent_count: 1,
            skipped_count: 0,
            send_attempts: 1,
            retry_reason: PacketRetryReason::None,
            chord_integrity_lost: false,
            last_win32_error: 0,
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            sender_started_ticks: Some(TimelineTicks::ZERO),
            sender_completed_ticks: Some(TimelineTicks::ZERO),
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            applied_lead_ticks: DurationTicks::ZERO,
            recovered_retry_late: false,
            recovered_partial_up: false,
            strict_completion_late: false,
        },
    })
}

fn up_observation(n: u64) -> DispatchObservation {
    DispatchObservation::Up(UpObservation {
        latency_class: LatencyClass::Hot,
        sender_duration_us: n,
        sent_count: 1,
        scan_count: 1,
        lead_up_ticks: DurationTicks::from_raw(n),
        lead_up_saturated: false,
        completed_effective: n,
        scheduled_us: 0,
        deferred_by_us: 0,
        up_completion_error_us: 0,
        estimator_evidence: clean_estimator_evidence(1),
        sender_completed_qpc: QpcTicks::ZERO,
        worker_ready_qpc: QpcTicks::ZERO,
        send_warn_us: 0,
        core_post_send_warn_us: 0,
        recovery_pause_ticks: None,
        trace: UpTraceObservation {
            event_index: 0,
            trace_kind: 1,
            scan_count: 1,
            sent_count: 1,
            skipped_count: 0,
            send_attempts: 1,
            last_win32_error: 0,
            authored_ticks: TimelineTicks::ZERO,
            effective_deadline_ticks: TimelineTicks::ZERO,
            wake_ticks: TimelineTicks::ZERO,
            sender_started_ticks: Some(TimelineTicks::ZERO),
            sender_completed_ticks: Some(TimelineTicks::ZERO),
            completion_error_ticks: 0,
            authored_completion_error_ticks: 0,
            applied_lead_ticks: DurationTicks::ZERO,
            deferred_by_us: 0,
            recovery_required: false,
        },
    })
}

fn clean_estimator_evidence(requested_count: usize) -> EstimatorObservationEvidence {
    EstimatorObservationEvidence {
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
fn canonical_clean_estimator_evidence_requires_every_dimension() {
    let clean = clean_estimator_evidence(2);
    assert!(is_clean_estimator_observation(clean));

    let cases = [
        EstimatorObservationEvidence {
            status: SendTransactionStatus::PartialProgress,
            ..clean
        },
        EstimatorObservationEvidence {
            attempts: 2,
            ..clean
        },
        EstimatorObservationEvidence {
            retry_reason: PacketRetryReason::ZeroProgress,
            ..clean
        },
        EstimatorObservationEvidence {
            confirmed_count: 1,
            ..clean
        },
        EstimatorObservationEvidence {
            skipped_count: 1,
            ..clean
        },
        EstimatorObservationEvidence {
            timing_valid: false,
            ..clean
        },
        EstimatorObservationEvidence {
            transport_anomaly: true,
            ..clean
        },
        EstimatorObservationEvidence {
            recovery_used: true,
            ..clean
        },
        EstimatorObservationEvidence {
            chord_integrity_lost: true,
            ..clean
        },
    ];
    for evidence in cases {
        assert!(!is_clean_estimator_observation(evidence));
    }
}

// ---------------------------------------------------------------------------
// §8.11 Tests
// ---------------------------------------------------------------------------

/// Single push into an empty queue does not allocate.
#[test]
fn push_single_down_observation_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut queue = PendingObservationQueue::default();
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
    let mut queue = PendingObservationQueue::default();
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
    let mut queue = PendingObservationQueue::default();
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

/// Filling the queue to capacity and triggering the drop-oldest eviction path
/// does not allocate.
#[test]
fn push_at_capacity_eviction_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut queue = PendingObservationQueue::default();
    let mut dropped = 0u64;
    let mut high = 0u64;

    // Fill to capacity outside the measurement window.
    for i in 0..OBSERVATION_QUEUE_CAPACITY {
        queue.push(down_observation(i as u64), &mut dropped, &mut high);
    }
    assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
    assert_eq!(dropped, 0);

    // One more push should evict the oldest without allocating.
    enable_counting();
    queue.push(down_observation(999), &mut dropped, &mut high);
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "push at capacity (eviction path) made {allocs} heap allocation(s); expected 0"
    );
    assert_eq!(dropped, 1, "eviction must increment dropped counter");
    assert_eq!(
        queue.len(),
        OBSERVATION_QUEUE_CAPACITY,
        "queue must remain at capacity after eviction"
    );
}

/// Queue overflow is a producer-only drop-oldest operation.  The newest raw
/// record remains queued for deferred processing; no observer materialization
/// or telemetry builder is available on this path.
#[test]
fn overflow_drops_oldest_and_keeps_newest_for_down_and_up() {
    let _lock = TEST_LOCK.lock();

    for (newest, expected_marker) in [
        (down_observation(999), 999_u64),
        (up_observation(999), 999_u64),
    ] {
        let mut queue = PendingObservationQueue::default();
        let mut dropped = 0_u64;
        let mut high = 0_u64;
        for index in 0..OBSERVATION_QUEUE_CAPACITY {
            queue.push(down_observation(index as u64), &mut dropped, &mut high);
        }

        queue.push(newest, &mut dropped, &mut high);

        assert_eq!(dropped, 1, "one oldest observation must be evicted");
        assert_eq!(queue.len(), OBSERVATION_QUEUE_CAPACITY);
        let first = queue.pop_front().expect("queue remains non-empty");
        match first {
            DispatchObservation::Down(observation) => {
                assert_eq!(observation.lead_down, 1);
            }
            DispatchObservation::Up(_) => panic!("oldest Down observation was not retained order"),
            DispatchObservation::Wait(_) => panic!("wait observation not expected"),
        }

        let mut last = None;
        while let Some(observation) = queue.pop_front() {
            last = Some(observation);
        }
        match last.expect("newest observation must be retained") {
            DispatchObservation::Down(observation) => {
                assert_eq!(observation.lead_down, expected_marker)
            }
            DispatchObservation::Up(observation) => {
                assert_eq!(observation.lead_up_ticks.as_u64(), expected_marker)
            }
            DispatchObservation::Wait(_) => panic!("wait observation not expected"),
        }
    }
}

/// Burst of N pushes followed by a full drain — no allocations throughout.
#[test]
fn burst_push_and_drain_no_alloc() {
    let _lock = TEST_LOCK.lock();
    const BURST: usize = 16;
    let mut queue = PendingObservationQueue::default();
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
    assert_eq!(high, BURST as u64);
}

/// pop_front on an empty queue does not allocate.
#[test]
fn pop_empty_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut queue = PendingObservationQueue::default();

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
    harness.advance_playback_time_us(1000);

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

/// Pending-release production dispatch hard-path makes ZERO heap allocations.
#[test]
fn production_pending_release_hard_path_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let mut harness = ProductionDispatchTestHarness::new_uponly_release();

    // Down A was physically dispatched outside the window. Advance to the
    // authored Up request, then let the production coordinator create the
    // pending release outside the measurement window.
    harness.advance_playback_time_us(1000);
    harness.seed_pending_release_for_test();

    enable_counting();
    let plan = harness.plan_current_dispatch();
    let pending_plan = plan.pending().expect("pending release plan");
    let effective_now = harness.current_effective_time();
    let due_pending = harness.pop_due_pending_for_plan(effective_now, &plan);
    assert_eq!(
        due_pending.len(),
        1,
        "plan must own one due pending release"
    );
    let step = harness.dispatch_pending_release_with_plan(
        due_pending,
        Some(pending_plan),
        LatencyClass::Hot,
    );
    let allocs = disable_counting();

    assert_eq!(
        allocs, 0,
        "production pending-release hard-path made {allocs} heap allocation(s); expected 0"
    );
    assert!(matches!(step, DispatchStep::Dispatched));
    assert!(!harness.has_active_generation(0x15));
    assert_eq!(harness.backend_active_mask(), 0);
    assert_eq!(harness.backend_possibly_active_mask(), 0);
    assert_eq!(harness.chord_integrity_lost_count(), 0);
}

fn exhaust_pending_release_recovery(harness: &mut ProductionDispatchTestHarness) -> DispatchStep {
    let drain_observer = |harness: &mut ProductionDispatchTestHarness| {
        loop {
            match harness.drain_observer() {
                Ok(0) => break Ok(()),
                Ok(_) => continue,
                Err(step) => break Err(step),
            }
        }
    };
    for _ in 0..=usize::from(sky_dispatch_core::coordinator::MAX_RELEASE_RETRIES) {
        let plan = harness.plan_current_dispatch();
        let pending_plan = plan.pending().expect("recovery must retain pending plan");
        let due_pending =
            harness.pop_due_pending_for_plan(TimelineTicks::from_raw(u64::MAX), &plan);
        assert_eq!(
            due_pending.len(),
            1,
            "recovery retry must remain coordinator-owned"
        );
        let step = harness.dispatch_pending_release_with_plan(
            due_pending,
            Some(pending_plan),
            LatencyClass::Hot,
        );
        if matches!(step, DispatchStep::Terminate(_)) {
            if let Err(observer_step) = drain_observer(harness) {
                return observer_step;
            }
            return step;
        }
        if let Err(step) = drain_observer(harness) {
            return step;
        }
    }
    panic!("persistent release failure did not exhaust recovery");
}

#[test]
fn exhausted_release_recovery_precedes_telemetry_failure() {
    let _lock = TEST_LOCK.lock();
    let _hooks = observer_test_hook_guard();
    set_release_telemetry_failure_on_recovery(true);
    let calls = Arc::new(AtomicU64::new(0));
    let mut harness = ProductionDispatchTestHarness::new_uponly_release();
    harness.configure_persistent_release_failure(Arc::clone(&calls));
    harness.advance_playback_time_us(1000);
    harness.seed_pending_release_for_test();

    let step = exhaust_pending_release_recovery(&mut harness);

    assert!(matches!(step, DispatchStep::Terminate(message) if message.contains("telemetry")));
    assert!(harness.full_instrument_release_calls() > 0);
    assert!(calls.load(Ordering::SeqCst) > 0);
    assert!(sky_player_rs::engine::observer_test_hooks::release_recovery_completed_before_ready());
}

#[test]
fn exhausted_release_recovery_precedes_observer_failure() {
    let _lock = TEST_LOCK.lock();
    let _hooks = observer_test_hook_guard();
    set_release_observer_failure_on_recovery(true);
    let calls = Arc::new(AtomicU64::new(0));
    let mut harness = ProductionDispatchTestHarness::new_uponly_release();
    harness.configure_persistent_release_failure(Arc::clone(&calls));
    harness.advance_playback_time_us(1000);
    harness.seed_pending_release_for_test();

    let step = exhaust_pending_release_recovery(&mut harness);

    assert!(matches!(step, DispatchStep::Terminate(message) if message.contains("observer")));
    assert!(harness.full_instrument_release_calls() > 0);
    assert!(calls.load(Ordering::SeqCst) > 0);
    assert!(sky_player_rs::engine::observer_test_hooks::release_recovery_completed_before_ready());
}
