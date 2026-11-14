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
