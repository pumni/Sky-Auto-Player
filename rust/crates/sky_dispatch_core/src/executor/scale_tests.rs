use super::*;
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput, RuntimeSchedule};
use crate::time::{DurationTicks, TimelineTicks};

fn test_ticks(us: u64) -> Result<TimelineTicks, std::convert::Infallible> {
    Ok(TimelineTicks::from_raw(us))
}

// ---------------------------------------------------------------------------
// Checkpoint B4: Scale fixtures (100, 1,000, 10,000 generations; widths 1, 5, 15)
// ---------------------------------------------------------------------------

fn generate_scale_schedule(
    generations: usize,
    chord_width: usize,
    allowed: &[u16],
) -> RuntimeSchedule {
    assert!(chord_width >= 1 && chord_width <= allowed.len());
    let mut actions = Vec::with_capacity((generations / chord_width) * 2);
    let chords_count = generations / chord_width;
    let mut action_idx = 0u32;
    let time_step_us = 50_000u64; // 50 ms between chords
    let hold_duration_us = 30_000u64; // 30 ms hold

    for i in 0..chords_count {
        let t_down = i as u64 * time_step_us;
        let t_up = t_down + hold_duration_us;
        let sc_slice = (0..chord_width)
            .map(|k| allowed[(i * chord_width + k) % allowed.len()])
            .collect::<smallvec::SmallVec<[u16; 4]>>();

        actions.push(KeyActionInput {
            source_action_index: action_idx,
            kind: ActionKind::Down,
            scheduled_us: t_down,
            scan_codes: sc_slice.clone(),
            reason: "scale-down".into(),
        });
        action_idx += 1;

        actions.push(KeyActionInput {
            source_action_index: action_idx,
            kind: ActionKind::Up,
            scheduled_us: t_up,
            scan_codes: sc_slice,
            reason: "scale-up".into(),
        });
        action_idx += 1;
    }

    compile_runtime_intents(&actions, allowed).expect("valid scale schedule")
}

fn execute_scale_scenario(schedule: &RuntimeSchedule) {
    let min_hold = DurationTicks::from_raw(20_000); // 20 ms
    let frame_time = DurationTicks::from_raw(16_000); // 16 ms
    let mut executor = DispatchExecutor::new(schedule, min_hold, frame_time, test_ticks).unwrap();

    let mut sim_time = TimelineTicks::ZERO;
    let mut steps = 0usize;
    let max_allowed_steps = schedule.down_chords.len() * 4 + 100;
    while !executor.is_finished(sim_time) {
        steps += 1;
        assert!(steps <= max_allowed_steps, "infinite loop protection");
        match executor.evaluate(sim_time) {
            DispatchAction::Send {
                up_mask,
                down_chord,
            } => {
                let down_mask = down_chord.map_or(0, |c| c.down_mask);
                executor
                    .on_send_completed(up_mask, down_mask, sim_time)
                    .unwrap();
            }
            DispatchAction::Wait { deadline_ticks } => {
                assert!(deadline_ticks >= sim_time);
                sim_time = deadline_ticks;
            }
            DispatchAction::Finished => {
                break;
            }
        }
    }
    assert_eq!(executor.active_mask(), 0);
    assert_eq!(executor.accounting().active, 0);
    assert_eq!(executor.accounting().released, schedule.generation_count);
}

#[test]
fn scale_fixture_100_generations_width_1_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(100, 1, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_100_generations_width_5_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(100, 5, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_100_generations_width_15_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(105, 15, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_1000_generations_width_1_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(1_000, 1, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_1000_generations_width_5_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(1_000, 5, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_1000_generations_width_15_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(1_005, 15, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_10000_generations_width_1_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(10_000, 1, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_10000_generations_width_5_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(10_000, 5, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_10000_generations_width_15_canonical() {
    let canonical = (0x15..0x15 + 15).collect::<Vec<u16>>();
    let sched = generate_scale_schedule(10_005, 15, &canonical);
    execute_scale_scenario(&sched);
}

#[test]
fn scale_fixture_1000_generations_width_5_noncanonical() {
    // Non-canonical scan codes (arbitrary keyboard layout)
    let noncanonical = vec![
        0x02, 0x03, 0x04, 0x05, 0x06, 0x10, 0x11, 0x12, 0x1E, 0x1F, 0x20, 0x2C, 0x2D, 0x2E, 0x39,
    ];
    let sched = generate_scale_schedule(1_000, 5, &noncanonical);
    execute_scale_scenario(&sched);
}

#[test]
fn cleanup_overflow_preflights_without_mutation() {
    let canonical = vec![0x15, 0x16];
    let sched = generate_scale_schedule(2, 1, &canonical);
    let mut exec = DispatchExecutor::new(
        &sched,
        DurationTicks::from_raw(500),
        DurationTicks::from_raw(16_667),
        test_ticks,
    )
    .unwrap();

    exec.on_send_completed(0, 0x01, TimelineTicks::from_raw(10_000))
        .unwrap();
    assert_eq!(exec.active_mask(), 0x01);

    exec.counters.cancelled = u64::MAX;
    assert!(exec.check_cleanup_overflow().is_err());

    let res = exec.on_verified_cleanup(TimelineTicks::from_raw(20_000));
    assert!(res.is_err());
    assert_eq!(
        exec.active_mask(),
        0x01,
        "slots must not be cleared on overflow"
    );
}

#[test]
fn divergent_coordinator_executor_active_mask_fails_closed() {
    let canonical = vec![0x15, 0x16];
    let sched = generate_scale_schedule(2, 1, &canonical);
    let mut coord = crate::coordinator::RuntimeDispatchCoordinator::new(
        sched.clone(),
        0,
        0,
        TimelineTicks::from_raw,
    );
    let mut exec = DispatchExecutor::new(
        &sched,
        DurationTicks::from_raw(500),
        DurationTicks::from_raw(16_667),
        test_ticks,
    )
    .unwrap();

    exec.on_send_completed(0, 0x01, TimelineTicks::from_raw(10_000))
        .unwrap();
    coord.executor = Some(exec);

    let res = coord.cancel_live_generations_after_verified_resumable_cleanup_with_time(
        TimelineTicks::from_raw(20_000),
    );
    assert!(res.is_err(), "divergent active mask must fail closed");
}

#[test]
fn next_stale_packet_target_ticks_is_bounded_by_32() {
    let mut actions = Vec::with_capacity(41);
    for chord in 0..20u32 {
        let down_us = u64::from(chord) * 100_000;
        actions.push(KeyActionInput {
            source_action_index: chord * 2,
            kind: ActionKind::Down,
            scheduled_us: down_us,
            scan_codes: smallvec::smallvec![0x15],
            reason: "bounded-lookahead-down".into(),
        });
        actions.push(KeyActionInput {
            source_action_index: chord * 2 + 1,
            kind: ActionKind::Up,
            scheduled_us: down_us + 50_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "bounded-lookahead-up".into(),
        });
    }
    actions.push(KeyActionInput {
        source_action_index: 40,
        kind: ActionKind::Up,
        scheduled_us: 2_000_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "bounded-lookahead-stale-up".into(),
    });
    let sched = compile_runtime_intents(&actions, &[0x15]).expect("bounded lookahead schedule");
    let coord =
        crate::coordinator::RuntimeDispatchCoordinator::new(sched, 0, 0, TimelineTicks::from_raw);
    assert_eq!(coord.next_stale_packet_target_ticks(), Ok(None));
}

#[test]
fn next_stale_packet_target_ticks_reports_target_and_corruption() {
    let actions = [KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Up,
        scheduled_us: 1_000,
        scan_codes: smallvec::smallvec![0x15],
        reason: "lookahead-stale-up".into(),
    }];
    let schedule = compile_runtime_intents(&actions, &[0x15]).expect("stale lookahead schedule");
    let coordinator = crate::coordinator::RuntimeDispatchCoordinator::new(
        schedule.clone(),
        0,
        0,
        TimelineTicks::from_raw,
    );
    assert_eq!(
        coordinator.next_stale_packet_target_ticks(),
        Ok(Some(TimelineTicks::from_raw(1_000)))
    );

    let mut corrupt_schedule = schedule;
    corrupt_schedule.batches[0].packet_id = u32::MAX;
    let corrupt = crate::coordinator::RuntimeDispatchCoordinator::new(
        corrupt_schedule,
        0,
        0,
        TimelineTicks::from_raw,
    );
    assert!(matches!(
        corrupt.next_stale_packet_target_ticks(),
        Err(crate::coordinator::CoordinatorError::Schedule(
            crate::model::RuntimeScheduleError::InvalidPacketIndex { .. }
        ))
    ));
}

#[test]
fn leader_l9_prescribed_interleaved_hol_model_verifies_b_down_not_blocked() {
    // Parameters from Leader L9:
    // GameFps = 60, TimingMarginUs = 500, T = 16_667 us, H = 2*T + 500 = 33_834 us.
    // Repeating cycle every 100_000 us, starting at 50_000 us:
    // Key A Down @ 50_000 us (slot 0)
    // Key A Up   @ 84_000 us (slot 0, hold = 34_000 us >= H)
    // Key B Down @ 85_000 us (slot 1)
    // Key B Up   @ 119_000 us (slot 1, hold = 34_000 us >= H)
    //
    // A 5,000 us worker stall delays Key A Down completion to 55_000 us.
    // Key A release due is pushed to: max(84_000, 55_000 + 33_834) = 88_834 us.
    // In Phase A (ordered pipeline), Key B Down (85_000 us) is HOL-blocked behind Key A Up until 88_834 us (lateness: 3,834 us).
    // In Phase B (independent 15-slot executor), Key B Down is dispatched on time at 85_000 us (lateness: 0 us)!
    let allowed = vec![0x15, 0x16];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 50_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A Down".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 84_000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "A Up".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 85_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B Down".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 119_000,
            scan_codes: smallvec::smallvec![0x16],
            reason: "B Up".into(),
        },
    ];

    let sched = compile_runtime_intents(&actions, &allowed).expect("valid schedule");
    let min_hold = DurationTicks::from_raw(33_834);
    let frame_time = DurationTicks::from_raw(16_667);
    let mut exec = DispatchExecutor::new(&sched, min_hold, frame_time, test_ticks).unwrap();

    // 1. At 50_000 us: Key A Down is evaluated and returned for dispatch.
    let step_a_down = exec.evaluate(TimelineTicks::from_raw(50_000));
    assert!(matches!(
        step_a_down,
        DispatchAction::Send {
            up_mask: 0,
            down_chord: Some(chord),
        } if chord.down_mask == 0x01
    ));

    // A 5,000 us stall occurs; Key A Down completes at 55_000 us.
    exec.on_send_completed(0, 0x01, TimelineTicks::from_raw(55_000))
        .unwrap();

    // Key A's release floor is now 55_000 + 33_834 = 88_834 us.
    // 2. At 84_000 us: authored Key A Up is not yet physically eligible (84_000 < 88_834).
    let step_84 = exec.evaluate(TimelineTicks::from_raw(84_000));
    assert_eq!(
        step_84,
        DispatchAction::Wait {
            deadline_ticks: TimelineTicks::from_raw(85_000),
        },
        "next event due is Key B Down at 85_000 us, not waiting for Key A release floor"
    );

    // 3. At 85_000 us: Key B Down is evaluated.
    // In Phase B, slot 1 is free and Key B Down is dispatched IMMEDIATELY at 85_000 us!
    let step_b_down = exec.evaluate(TimelineTicks::from_raw(85_000));
    assert!(
        matches!(
            step_b_down,
            DispatchAction::Send {
                up_mask: 0,
                down_chord: Some(chord),
            } if chord.down_mask == 0x02
        ),
        "Key B Down MUST be dispatched on time at 85_000 us without HOL blocking"
    );

    // Key B Down completes on time at 85_000 us.
    exec.on_send_completed(0, 0x02, TimelineTicks::from_raw(85_000))
        .unwrap();

    // 4. At 88_834 us: Key A Up physical release floor expires and is dispatched.
    let step_a_up = exec.evaluate(TimelineTicks::from_raw(88_834));
    assert_eq!(
        step_a_up,
        DispatchAction::Send {
            up_mask: 0x01,
            down_chord: None,
        }
    );
    exec.on_send_completed(0x01, 0, TimelineTicks::from_raw(88_834))
        .unwrap();

    // 5. At 119_000 us: Key B Up is dispatched.
    let step_b_up = exec.evaluate(TimelineTicks::from_raw(119_000));
    assert_eq!(
        step_b_up,
        DispatchAction::Send {
            up_mask: 0x02,
            down_chord: None,
        }
    );
    exec.on_send_completed(0x02, 0, TimelineTicks::from_raw(119_000))
        .unwrap();

    // 6. Schedule cleanly finished.
    let step_fin = exec.evaluate(TimelineTicks::from_raw(119_001));
    assert_eq!(step_fin, DispatchAction::Finished);
}
