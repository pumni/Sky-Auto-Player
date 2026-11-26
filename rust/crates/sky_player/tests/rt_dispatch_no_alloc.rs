//! Allocation gate for the production executor's staged worker path.
//!
//! The counting window covers `DispatchExecutor` decision, production worker
//! staging, sender journal, mock transport completion, and executor commit.
//! Fixture construction and deadline waits are outside that window.

use sky_dispatch_core::executor::PreparedChord;
use sky_dispatch_core::model::OnsetChordInput;
use sky_dispatch_core::time::{QpcTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::input::{PHYSICAL_INSTRUMENT_SCAN_CODES, PhysicalPacket};
use sky_player::engine::StagedDispatchOutcome;
use sky_player::engine::dispatch_primitives::ProductionDispatchTestHarness;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

fn chord(index: u32, scheduled_us: u64, scan_codes: &[u16]) -> OnsetChordInput {
    OnsetChordInput {
        source_action_index: index,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
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
    down_chord: Option<PreparedChord>,
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

fn production_verified_cleanup_resume_fixture() -> (
    ProductionDispatchTestHarness,
    sky_player::engine::dispatch_primitives::PacketCapture,
    QpcTicks,
) {
    use sky_dispatch_core::executor::DispatchDecision;

    let mut harness =
        ProductionDispatchTestHarness::new_lifecycle_sequence_for_test(10_000_000, false)
            .expect("production scheduler lifecycle fixture");
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0]);
    harness.configure_mock_cleanup_emitter_for_test();
    let now = harness
        .qpc_now_for_test()
        .expect("verified cleanup setup QPC");
    let one_second = harness
        .qpc_duration_from_us_for_test(1_000_000)
        .expect("past scheduler epoch conversion");
    let epoch = QpcTicks::from_raw(now.as_u64().saturating_sub(one_second.as_u64()));
    harness
        .reanchor_playback_clock_for_test(epoch)
        .expect("verified cleanup playback epoch");

    let (up_mask, down_chord, timing_window) = match harness
        .evaluate_live_scheduler_for_test(now, epoch)
        .expect("initial live scheduler evaluation")
    {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord, timing_window),
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

    harness.set_pause_requested_for_test(true);
    let pause_at = harness.qpc_now_for_test().expect("Pause command QPC");
    harness
        .commit_pause_for_test(sky_dispatch_core::clock::PauseReason::Manual, pause_at)
        .expect("commit Pause control");
    harness
        .apply_scheduler_resumable_lifecycle_transition_for_test()
        .expect("verified production scheduler cleanup");
    assert!(harness.is_paused_for_test());
    let accounting = harness.generation_accounting_for_test();
    assert_eq!(accounting.total, 2);
    assert_eq!(accounting.activated, 1);
    assert_eq!(accounting.cancelled, 1);
    assert_eq!(accounting.scheduled, 1);
    assert_eq!(harness.release_obligation_mask_for_test(), 0);
    packets.lock().expect("production packet capture").clear();

    let resume_at = harness.qpc_now_for_test().expect("Resume command QPC");
    harness.set_pause_requested_for_test(false);
    let resumed_epoch = harness
        .commit_resume_for_test(sky_dispatch_core::clock::PauseReason::Manual, resume_at)
        .expect("commit Resume control");
    (harness, packets, resumed_epoch)
}

#[test]
fn production_staged_worker_1_5_and_15_key_chords_no_alloc() {
    let _lock = TEST_LOCK.lock();
    let clock = clock();

    for width in [1_usize, 5, 15] {
        let keys = &PHYSICAL_INSTRUMENT_SCAN_CODES[..width];
        let key_mask = (1_u16 << width) - 1;
        let chords = [chord(0, 0, keys)];
        let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
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
        wait_until(hold_floor);

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
fn production_staged_worker_mixed_to_up_prefix_no_alloc() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let _lock = TEST_LOCK.lock();
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    let clock = clock();
    let chords = [chord(0, 0, &[0x15]), chord(1, 17_167, &[0x16])];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    harness.configure_focus_owner_for_test(1, 101);
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
    let modifier_held = Arc::new(AtomicBool::new(false));
    let held_for_query = Arc::clone(&modifier_held);
    harness.set_modifier_key_state_query_for_test(move |_| {
        if held_for_query.load(Ordering::Acquire) {
            i16::MIN
        } else {
            0
        }
    });
    let held_for_gate = Arc::clone(&modifier_held);
    harness.set_final_gate_before_syscall_hook(move || {
        held_for_gate.store(true, Ordering::Release);
    });

    enable_counting();
    let mixed_outcome = stage_and_dispatch(&mut harness, epoch, mixed_now, 1, mixed_chord);
    let allocations = disable_counting();

    assert_eq!(
        allocations, 0,
        "production Mixed-to-Up-prefix dispatch allocated"
    );
    assert!(matches!(mixed_outcome, StagedDispatchOutcome::Dispatched));
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[1].0, PhysicalPacket::new(1, 0));
    assert_eq!(packets[1].1.attempts, 1);
    assert_eq!(
        harness
            .current_executor_chord_for_test()
            .map(|chord| chord.key_mask),
        Some(2),
        "the effective Up prefix must leave the Down onset pending"
    );
    assert_eq!(harness.in_flight_mask_for_test(), 0);
}

#[test]
fn production_staged_worker_mixed_up_down_packet_no_alloc() {
    struct ForegroundOverrideReset;

    impl Drop for ForegroundOverrideReset {
        fn drop(&mut self) {
            sky_dispatch_win32::focus::set_foreground_window_for_test(None);
        }
    }

    let _lock = TEST_LOCK.lock();
    let _foreground_lock = sky_dispatch_win32::focus::lock_foreground_window_for_test();
    let _foreground_reset = ForegroundOverrideReset;
    let clock = clock();
    let chords = [chord(0, 0, &[0x15]), chord(1, 17_167, &[0x16])];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    harness.configure_focus_owner_for_test(1, 101);
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
            .expect("Mixed authored target"),
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
            .expect("Mixed hold floor"),
    );
    wait_until(authored_target.max(hold_floor));

    let mixed_now = clock.now().expect("Mixed pre-call QPC");
    let mixed_chord = harness.current_executor_chord_for_test();
    enable_counting();
    let mixed_outcome = stage_and_dispatch(&mut harness, epoch, mixed_now, 1, mixed_chord);
    let allocations = disable_counting();

    assert_eq!(allocations, 0, "healthy full Mixed packet allocated");
    assert!(matches!(mixed_outcome, StagedDispatchOutcome::Dispatched));
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 2);
    assert_eq!(packets[1].0, PhysicalPacket::new(1, 2));
    assert_eq!(packets[1].1.attempts, 1);
    drop(packets);
    assert!(
        harness.current_executor_chord_for_test().is_none(),
        "full Mixed commit must consume the Down chord"
    );
    assert_eq!(
        harness.release_obligation_mask_for_test(),
        2,
        "Up A must release while Down B creates its physical release obligation"
    );
    assert_eq!(harness.in_flight_mask_for_test(), 0);
}

#[test]
fn lifecycle_verified_cleanup_resume_dispatches_without_allocating() {
    let _lock = TEST_LOCK.lock();
    use sky_dispatch_core::executor::DispatchDecision;

    let (mut harness, packets, epoch) = production_verified_cleanup_resume_fixture();
    assert!(!harness.is_paused_for_test());
    let now = clock().now().expect("fresh post-Resume QPC");

    let (up_mask, down_chord, timing_window) = {
        enable_counting();
        let decision = harness
            .evaluate_live_scheduler_for_test(now, epoch)
            .expect("fresh post-Resume scheduler evaluation");
        let send = match decision {
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => (up_mask, down_chord, timing_window),
            other => panic!("expected immediate authored Send decision, got {other:?}"),
        };
        let allocations = disable_counting();
        assert_eq!(allocations, 0, "post-Resume scheduler query allocated");
        send
    };
    assert_eq!(up_mask, 0);
    assert!(down_chord.is_some());
    assert!(timing_window.packet_not_before_qpc <= now);

    enable_counting();
    let candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, down_chord, timing_window)
        .expect("stage resumed production Down")
        .expect("post-Resume Down candidate");
    let staging_allocations = disable_counting();
    assert_eq!(staging_allocations, 0, "post-Resume staging allocated");
    assert!(
        harness
            .staged_candidate_timing_window_for_test(&candidate)
            .packet_not_before_qpc
            <= now
    );

    let sender_start = clock().now().expect("resumed mock sender QPC");
    harness.set_mock_sender_start_qpc_for_test(sender_start);
    enable_counting();
    let outcome = harness.dispatch_staged_candidate_for_test(candidate, false);
    let dispatch_allocations = disable_counting();

    assert_eq!(dispatch_allocations, 0, "post-Resume dispatch allocated");
    assert!(matches!(outcome, StagedDispatchOutcome::Dispatched));
    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 1, "Resume performs one immediate mock send");
    assert_eq!(packets[0].0, PhysicalPacket::new(0, 1));
}

#[test]
fn production_blocked_same_key_repeat_wait_and_replan_no_alloc() {
    use sky_dispatch_core::executor::DispatchDecision;

    let _lock = TEST_LOCK.lock();
    let clock = clock();
    let chords = [chord(0, 0, &[0x15]), chord(1, 17_167, &[0x15])];
    let mut harness = ProductionDispatchTestHarness::create_harness(&chords);
    let packets = harness.configure_mock_packet_sender_for_test(&[0, 0, 0, 0]);
    harness
        .attach_executor_for_test()
        .expect("executor invariants");
    let epoch = reanchor_in_the_past(&mut harness, clock);

    // 1. Dispatch first Down on slot 0.
    let now = clock.now().expect("first Down QPC");
    let first_chord = harness.current_executor_chord_for_test();
    let outcome = stage_and_dispatch(&mut harness, epoch, now, 0, first_chord);
    assert!(matches!(outcome, StagedDispatchOutcome::Dispatched));

    // 2. While slot 0 is still held (before completion + H), the same-key
    //    repeat is blocked and evaluate_at must return Wait without allocating.
    let blocked_now = clock.now().expect("blocked evaluation QPC");
    enable_counting();
    let blocked_decision = harness
        .evaluate_live_scheduler_for_test(blocked_now, epoch)
        .expect("blocked scheduler evaluation");
    let wait_allocations = disable_counting();
    assert_eq!(
        wait_allocations, 0,
        "blocked same-key Wait evaluation allocated"
    );
    let DispatchDecision::Wait { deadline_qpc } = blocked_decision else {
        panic!("expected blocked Wait before H elapses, got {blocked_decision:?}");
    };

    // 3. Wait until the completion-anchored release deadline, then evaluate,
    //    stage, and dispatch Up-only (repeat Down stays blocked until Up commits).
    wait_until(deadline_qpc);
    let up_now = clock.now().expect("Up release QPC");
    enable_counting();
    let up_decision = harness
        .evaluate_live_scheduler_for_test(up_now, epoch)
        .expect("Up scheduler evaluation");
    let (up_mask, up_down, up_window) = match up_decision {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord, timing_window),
        other => panic!("expected Up-only Send while repeat is blocked, got {other:?}"),
    };
    let up_candidate = harness
        .stage_candidate_with_scheduler_window_for_test(up_mask, up_down, up_window)
        .expect("stage Up-only candidate")
        .expect("Up-only candidate");
    harness.set_mock_sender_start_qpc_for_test(up_now);
    let up_outcome = harness.dispatch_staged_candidate_for_test(up_candidate, false);
    let up_allocations = disable_counting();
    assert_eq!(
        up_allocations, 0,
        "blocked-repeat Up-only dispatch allocated"
    );
    assert_eq!(up_mask, 1);
    assert!(up_down.is_none());
    assert!(matches!(up_outcome, StagedDispatchOutcome::Dispatched));

    // 4. Immediately after Up commits, fresh scheduler evaluation unblocks the
    //    repeat Down with zero allocations.
    let repeat_now = clock.now().expect("repeat Down QPC");
    enable_counting();
    let repeat_decision = harness
        .evaluate_live_scheduler_for_test(repeat_now, epoch)
        .expect("unblocked repeat evaluation");
    let (rep_up, rep_down, rep_window) = match repeat_decision {
        DispatchDecision::Send {
            up_mask,
            down_chord,
            timing_window,
        } => (up_mask, down_chord, timing_window),
        other => panic!("expected unblocked repeat Down Send, got {other:?}"),
    };
    let rep_candidate = harness
        .stage_candidate_with_scheduler_window_for_test(rep_up, rep_down, rep_window)
        .expect("stage repeat Down candidate")
        .expect("repeat Down candidate");
    harness.set_mock_sender_start_qpc_for_test(repeat_now);
    let rep_outcome = harness.dispatch_staged_candidate_for_test(rep_candidate, false);
    let rep_allocations = disable_counting();
    assert_eq!(
        rep_allocations, 0,
        "unblocked repeat Down dispatch allocated"
    );
    assert_eq!(rep_up, 0);
    assert_eq!(rep_down.map(|c| c.key_mask), Some(1));
    assert!(matches!(rep_outcome, StagedDispatchOutcome::Dispatched));

    let packets = packets.lock().expect("packet capture");
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[0].0, PhysicalPacket::new(0, 1));
    assert_eq!(packets[1].0, PhysicalPacket::new(1, 0));
    assert_eq!(packets[2].0, PhysicalPacket::new(0, 1));
}
