//! Allocation gate for the production executor's staged worker path.
//!
//! The counting window covers `DispatchExecutor` decision, production worker
//! staging, sender journal, mock transport completion, and executor commit.
//! Fixture construction and deadline waits are outside that window.

use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{QpcTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalPacket};
use sky_player::engine::StagedDispatchOutcome;
use sky_player::engine::dispatch_primitives::ProductionDispatchTestHarness;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

fn is_counting_active() -> bool {
    COUNTING_ENABLED.load(Ordering::Relaxed) != 0
        && CURRENT_THREAD_ID.load(Ordering::Relaxed) == current_thread_id_u64()
}

// SAFETY: every operation delegates to `System`; the counters do not affect
// allocation behavior or the returned pointer.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if is_counting_active() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if is_counting_active() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if is_counting_active() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

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

fn action(index: u32, kind: ActionKind, scheduled_us: u64, scan_codes: &[u16]) -> KeyActionInput {
    KeyActionInput {
        source_action_index: index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
        reason: Arc::from("production noalloc fixture"),
    }
}

fn clock() -> QpcClock {
    QpcClock::initialize().expect("QPC clock")
}

fn qpc_duration(clock: QpcClock, microseconds: u64) -> u64 {
    clock
        .duration_from_us(microseconds)
        .expect("QPC duration conversion")
        .as_u64()
}

fn stage_and_dispatch(
    harness: &mut ProductionDispatchTestHarness,
    epoch: QpcTicks,
    now: QpcTicks,
    up_mask: u16,
    down_chord: Option<sky_dispatch_core::executor::DownChordMetadata>,
) -> StagedDispatchOutcome {
    let elapsed = now.as_u64().saturating_sub(epoch.as_u64());
    let candidate = harness
        .stage_candidate_for_test(up_mask, down_chord, TimelineTicks::from_raw(elapsed))
        .expect("live executor candidate staging")
        .expect("live executor send decision");
    harness.set_mock_sender_start_qpc_for_test(now);
    harness.dispatch_staged_candidate_for_test(candidate, false)
}

fn wait_until(deadline: QpcTicks) {
    let clock = clock();
    loop {
        if clock.now().expect("QPC deadline sample") >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}

fn reanchor_in_the_past(harness: &mut ProductionDispatchTestHarness, clock: QpcClock) -> QpcTicks {
    let now = clock.now().expect("QPC epoch sample");
    let epoch = QpcTicks::from_raw(now.as_u64().saturating_sub(qpc_duration(clock, 1_000_000)));
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("production playback epoch");
    epoch
}

fn production_resume_barrier_fixture() -> (
    ProductionDispatchTestHarness,
    sky_player::engine::dispatch_primitives::PacketCapture,
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

    let (up_mask, down_chord, timing_window) = match harness
        .evaluate_live_scheduler_for_test(now, epoch)
        .expect("initial live scheduler evaluation")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord.map(Into::into), timing_window),
        decision => panic!("expected initial production Down, got {decision:?}"),
    };
    assert_eq!(up_mask, 0);
    assert!(down_chord.is_some());
    let candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
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
    let clock = clock();
    loop {
        if clock.now().expect("QPC deadline sample") >= deadline {
            return;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}

#[test]
fn production_staged_worker_1_5_and_15_key_chords_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let clock = clock();

    for width in [1_usize, 5, 15] {
        let chord = &PHYSICAL_INSTRUMENT_SCAN_CODES[..width];
        let key_mask = (1_u16 << width) - 1;
        let actions = [
            action(0, ActionKind::Down, 0, chord),
            action(1, ActionKind::Up, 20_000, chord),
        ];
        let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
        let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
        harness
            .attach_executor_for_test()
            .expect("executor invariants");
        let epoch = reanchor_in_the_past(&mut harness, clock);
        let down_now = clock.now().expect("Down pre-call QPC");
        let down_chord = harness.current_executor_chord_for_test();

        enable_counting();
        let down_outcome = stage_and_dispatch(&mut harness, epoch, down_now, 0, down_chord);
        let down_allocations = disable_counting();
        assert_eq!(down_allocations, 0, "{width}-key Down allocated");
        assert!(matches!(down_outcome, StagedDispatchOutcome::Dispatched));

        let completion = packets
            .lock()
            .expect("packet capture")
            .first()
            .and_then(|(_, evidence)| evidence.completed_ticks)
            .expect("Down completion QPC");
        let hold_floor = QpcTicks::from_raw(
            completion
                .as_u64()
                .checked_add(qpc_duration(clock, 17_167))
                .expect("H floor"),
        );
        let authored_up = QpcTicks::from_raw(
            epoch
                .as_u64()
                .checked_add(qpc_duration(clock, 20_000))
                .expect("authored Up target"),
        );
        wait_until(hold_floor.max(authored_up));

        let up_now = clock.now().expect("Up pre-call QPC");
        enable_counting();
        let up_outcome = stage_and_dispatch(&mut harness, epoch, up_now, key_mask, None);
        let up_allocations = disable_counting();
        assert_eq!(up_allocations, 0, "{width}-key Up allocated");
        assert!(matches!(up_outcome, StagedDispatchOutcome::Dispatched));
        let packets = packets.lock().expect("packet capture");
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].0, PhysicalPacket::new(0, key_mask));
        assert_eq!(packets[1].0, PhysicalPacket::new(key_mask, 0));
    }
}

#[test]
fn production_staged_worker_mixed_up_down_packet_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let clock = clock();
    let actions = [
        action(0, ActionKind::Down, 0, &[0x15]),
        action(1, ActionKind::Up, 20_000, &[0x15]),
        action(2, ActionKind::Down, 20_000, &[0x16]),
        action(3, ActionKind::Up, 40_000, &[0x16]),
    ];
    let mut harness = ProductionDispatchTestHarness::create_harness(&actions);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0]);
    harness
        .attach_executor_for_test()
        .expect("executor invariants");
    let epoch = reanchor_in_the_past(&mut harness, clock);
    let down_now = clock.now().expect("initial Down QPC");
    let down_chord = harness.current_executor_chord_for_test();
    let down_outcome = stage_and_dispatch(&mut harness, epoch, down_now, 0, down_chord);
    assert!(matches!(down_outcome, StagedDispatchOutcome::Dispatched));

    let authored_target = QpcTicks::from_raw(
        epoch
            .as_u64()
            .checked_add(qpc_duration(clock, 20_000))
            .expect("mixed packet authored target"),
    );
    let down_completion = packets
        .lock()
        .expect("Down packet capture")
        .first()
        .and_then(|(_, evidence)| evidence.completed_ticks)
        .expect("Down completion QPC");
    let hold_floor = QpcTicks::from_raw(
        down_completion
            .as_u64()
            .checked_add(qpc_duration(clock, 17_167))
            .expect("mixed packet hold floor"),
    );
    wait_until(authored_target.max(hold_floor));
    let mixed_now = clock.now().expect("mixed packet pre-call QPC");
    let mixed_chord = harness.current_executor_chord_for_test();

    enable_counting();
    let mixed_outcome = stage_and_dispatch(&mut harness, epoch, mixed_now, 1, mixed_chord);
    let allocations = disable_counting();

    assert_eq!(allocations, 0, "production mixed Up/Down packet allocated");
    assert!(matches!(mixed_outcome, StagedDispatchOutcome::Dispatched));
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[1].0, PhysicalPacket::new(1, 2));
}

#[test]
fn lifecycle_resume_barrier_g_minus_one_waits_without_allocating_or_sending() {
    let _lock = TEST_LOCK.lock();
    use sky_dispatch_core::executor::DispatchDecision;

    let (harness, packets, epoch, barrier) = production_resume_barrier_fixture();
    let g_minus_one = QpcTicks::from_raw(barrier.as_u64() - 1);

    enable_counting();
    let decision = harness
        .evaluate_live_scheduler_for_test(g_minus_one, epoch)
        .expect("G-1 production scheduler query");
    let allocations = disable_counting();

    assert_eq!(allocations, 0, "G-1 scheduler query allocated");
    assert!(matches!(decision, DispatchDecision::Wait { deadline_qpc } if deadline_qpc == barrier));
    assert!(packets.lock().expect("packet capture").is_empty());
}

#[test]
fn lifecycle_resume_barrier_exact_g_stages_and_sends_once_without_allocating() {
    let _lock = TEST_LOCK.lock();
    use sky_dispatch_core::executor::DispatchDecision;

    let (mut harness, packets, epoch, barrier) = production_resume_barrier_fixture();
    assert!(matches!(
        harness
            .evaluate_live_scheduler_for_test(QpcTicks::from_raw(barrier.as_u64() - 1), epoch)
            .expect("G-1 scheduler evaluation"),
        DispatchDecision::Wait { deadline_qpc } if deadline_qpc == barrier
    ));

    let (up_mask, down_chord, timing_window) = {
        enable_counting();
        let decision = harness
            .evaluate_live_scheduler_for_test(barrier, epoch)
            .expect("exact-G production scheduler evaluation");
        let send = match decision {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => (up_mask, down_chord.map(Into::into), timing_window),
            other => panic!("expected exact-G Send decision, got {other:?}"),
        };
        let allocations = disable_counting();
        assert_eq!(allocations, 0, "exact-G scheduler query allocated");
        send
    };
    assert_eq!(up_mask, 0);
    assert!(down_chord.is_some());
    assert_eq!(timing_window.packet_not_before_qpc, barrier);

    enable_counting();
    let candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage exact-G production Down")
        .expect("exact-G Down candidate");
    let staging_allocations = disable_counting();
    assert_eq!(staging_allocations, 0, "exact-G staging allocated");
    assert_eq!(
        harness
            .staged_candidate_timing_window_for_test(&candidate)
            .packet_not_before_qpc,
        barrier,
        "staging must preserve cleanup completion + G"
    );

    wait_until_qpc(barrier);
    harness.set_mock_sender_start_qpc_for_test(barrier);
    enable_counting();
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    let dispatch_allocations = disable_counting();

    assert_eq!(dispatch_allocations, 0, "exact-G dispatch allocated");
    assert!(matches!(outcome, StagedDispatchOutcome::Dispatched));
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 1, "exact G performs one mock send");
    assert_eq!(packets[0].0, PhysicalPacket::new(0, 1));
}
