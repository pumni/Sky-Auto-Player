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
    let canonical = vec![0x15];
    let sched = generate_scale_schedule(40, 1, &canonical);
    let coord =
        crate::coordinator::RuntimeDispatchCoordinator::new(sched, 0, 0, TimelineTicks::from_raw);
    assert_eq!(coord.next_stale_packet_target_ticks(), None);
}
