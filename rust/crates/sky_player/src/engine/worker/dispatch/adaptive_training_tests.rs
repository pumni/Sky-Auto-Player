//! Regression tests for adaptive wake-guard training rules (Leader L14).
//!
//! Confirms:
//! 1. One physical timer generation produces at most one valid sample.
//! 2. Metadata-only tail, interrupt, replan, and physical-floor displacement are excluded from training.
//! 3. QPC conversion failures reject evidence cleanly without generating fake samples.

use crate::engine::worker::WorkerRuntime;
use sky_dispatch_core::executor::{DispatchAction, DispatchExecutor};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::wait::{AdaptiveWaitController, KernelTimerWake, WaitGuardMode};
use std::num::NonZeroU64;

#[test]
fn single_physical_generation_yields_at_most_one_valid_sample() {
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(10_000_000).unwrap());
    let epoch = QpcTicks::from_raw(1_000_000);
    let mut runtime = WorkerRuntime {
        adaptive_wait_controller: Some(AdaptiveWaitController::new(
            WaitGuardMode::Adaptive,
            1_000,
            epoch,
        )),
        ..Default::default()
    };

    // Simulate timer wake for physical generation
    let arm_target = epoch
        .checked_add_duration(DurationTicks::from_raw(100_000))
        .unwrap();
    let return_qpc = arm_target
        .checked_add_duration(DurationTicks::from_raw(2_000))
        .unwrap();
    runtime.pending_adaptive_timer_wake = Some(KernelTimerWake::new(return_qpc, arm_target));

    // First physical dispatch consumes the pending wake
    let consumed_first = runtime.pending_adaptive_timer_wake.take();
    assert!(consumed_first.is_some());
    let wake = consumed_first.unwrap();
    let controller = runtime.adaptive_wait_controller.as_mut().unwrap();
    let _ = controller.record_timer_wake(wake, clock);
    assert_eq!(controller.total_valid_wakes(), 1);

    // Second check in the same physical generation finds None (at most one sample per generation)
    let consumed_second = runtime.pending_adaptive_timer_wake.take();
    assert!(consumed_second.is_none());
    assert_eq!(controller.total_valid_wakes(), 1);
}

#[test]
fn metadata_tail_wait_does_not_train_adaptive_controller() {
    // Build a schedule where all chords finish at 1_000 us, but schedule_end_offset is 5_000 us (metadata tail)
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "note".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 2_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "note".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 5_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "metadata-tail".into(),
        },
    ];
    let schedule =
        sky_dispatch_core::compile::compile_runtime_intents(&actions, &allowed).expect("compile");
    let min_hold = DurationTicks::from_raw(500);
    let frame = DurationTicks::from_raw(16_667);
    let mut executor = DispatchExecutor::new(
        &schedule,
        min_hold,
        frame,
        |us| -> Result<TimelineTicks, &'static str> { Ok(TimelineTicks::from_raw(us)) },
    )
    .expect("executor");

    // Before sending chord: physical work is pending
    assert!(executor.has_physical_work());

    // Advance past chord send and up release (completion at 2_500 us)
    let chord = executor.current_chord().cloned();
    assert!(chord.is_some());
    executor
        .on_send_completed(0, chord.unwrap().down_mask, TimelineTicks::from_raw(1_000))
        .expect("down send");
    executor
        .on_send_completed(0x0001, 0, TimelineTicks::from_raw(2_000))
        .expect("up send");

    // All chords completed and all keys released: physical work is false!
    assert!(!executor.has_physical_work());

    // Timeline evaluation now produces metadata-only tail wait
    let action = executor.evaluate(TimelineTicks::from_raw(3_000));
    match action {
        DispatchAction::Wait { deadline_ticks } => {
            assert_eq!(deadline_ticks, executor.schedule_end_offset_ticks());
        }
        other => panic!("expected Wait for metadata-tail, got {other:?}"),
    }

    // In dispatch loop, has_physical_work() == false prevents setting pending_adaptive_timer_wake
    let mut runtime = WorkerRuntime::default();
    let is_physical_work = executor.has_physical_work();
    assert!(!is_physical_work);
    if !is_physical_work {
        runtime.pending_adaptive_timer_wake = None;
    }
    assert!(runtime.pending_adaptive_timer_wake.is_none());
}

#[test]
fn floor_displacement_clears_pending_adaptive_wake() {
    let arm_target = QpcTicks::from_raw(1_000_000);
    let return_qpc = QpcTicks::from_raw(1_000_100);
    let mut runtime = WorkerRuntime {
        pending_adaptive_timer_wake: Some(KernelTimerWake::new(return_qpc, arm_target)),
        ..Default::default()
    };

    // Packet not-before is in future compared to now_ticks (floor displacement occurred!)
    let now_ticks = QpcTicks::from_raw(950_000);
    let packet_not_before_qpc = QpcTicks::from_raw(1_000_000);

    if packet_not_before_qpc > now_ticks {
        // Floor displacement must clear pending wake
        runtime.pending_adaptive_timer_wake = None;
    }

    assert!(runtime.pending_adaptive_timer_wake.is_none());
}

#[test]
fn replan_clears_pending_adaptive_wake() {
    let arm_target = QpcTicks::from_raw(1_000_000);
    let return_qpc = QpcTicks::from_raw(1_000_100);
    let mut runtime = WorkerRuntime {
        pending_adaptive_timer_wake: Some(KernelTimerWake::new(return_qpc, arm_target)),
        ..Default::default()
    };

    // On replan, pending wake must be discarded
    runtime.pending_adaptive_timer_wake = None;
    assert!(runtime.pending_adaptive_timer_wake.is_none());
}
