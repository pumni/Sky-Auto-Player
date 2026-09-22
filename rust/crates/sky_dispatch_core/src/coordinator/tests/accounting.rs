use super::super::{GenerationStatus, RuntimeDispatchCoordinator};
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};
use crate::testing::assert_clean_generation_completion;
use crate::time::TimelineTicks;

fn action(kind: ActionKind, scheduled_us: u64, scan_codes: &[u16]) -> KeyActionInput {
    KeyActionInput {
        source_action_index: (scheduled_us * 256 + u64::from(scan_codes[0])) as u32,
        kind,
        scheduled_us,
        scan_codes: scan_codes.to_vec().into(),
        reason: "generation accounting test".into(),
    }
}

fn coordinator(actions: &[KeyActionInput]) -> RuntimeDispatchCoordinator {
    let mut scan_codes = actions
        .iter()
        .flat_map(|action| action.scan_codes.iter().copied())
        .collect::<Vec<_>>();
    scan_codes.sort_unstable();
    scan_codes.dedup();
    let schedule = compile_runtime_intents(actions, &scan_codes).expect("valid test schedule");
    RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw)
}

fn commit_next_authored(coordinator: &mut RuntimeDispatchCoordinator, now: u64) {
    let prepared = coordinator
        .prepare_current_authored_packet()
        .expect("prepare authored packet")
        .expect("authored packet exists");
    let ticks = TimelineTicks::from_raw(now);
    coordinator
        .commit_prepared_authored_frame_success_frozen(&prepared.commit, ticks, ticks)
        .expect("commit authored packet");
}

#[test]
fn single_key_accounting_tracks_activation_and_release() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15]),
        action(ActionKind::Up, 100, &[0x15]),
    ]);

    commit_next_authored(&mut coordinator, 0);
    assert_eq!(coordinator.generation_accounting().activated, 1);
    assert_eq!(coordinator.generation_accounting().released, 0);
    assert_eq!(coordinator.generation_accounting().active, 1);

    commit_next_authored(&mut coordinator, 100);
    let accounting = coordinator.generation_accounting();
    assert_clean_generation_completion(accounting);
    coordinator
        .check_invariants()
        .expect("valid clean accounting");
}

#[test]
fn chord_accounting_counts_each_generation_in_one_down_and_up_packet() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15, 0x16]),
        action(ActionKind::Up, 100, &[0x15, 0x16]),
    ]);

    commit_next_authored(&mut coordinator, 0);
    let active = coordinator.generation_accounting();
    assert_eq!(active.total, 2);
    assert_eq!(active.activated, 2);
    assert_eq!(active.released, 0);
    assert_eq!(active.active, 2);

    commit_next_authored(&mut coordinator, 100);
    assert_clean_generation_completion(coordinator.generation_accounting());
}

#[test]
fn same_key_retrigger_has_two_paired_generations() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15]),
        action(ActionKind::Up, 100, &[0x15]),
        action(ActionKind::Down, 200, &[0x15]),
        action(ActionKind::Up, 300, &[0x15]),
    ]);

    for now in [0, 100, 200, 300] {
        commit_next_authored(&mut coordinator, now);
    }

    let accounting = coordinator.generation_accounting();
    assert_eq!(accounting.total, 2);
    assert_eq!(accounting.activated, 2);
    assert_eq!(accounting.released, 2);
    assert_eq!(accounting.active, 0);
}

#[test]
fn mixed_packet_releases_old_and_activates_unrelated_generation_atomically() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15]),
        action(ActionKind::Up, 100, &[0x15]),
        action(ActionKind::Down, 100, &[0x16]),
        action(ActionKind::Up, 200, &[0x16]),
    ]);

    commit_next_authored(&mut coordinator, 0);
    commit_next_authored(&mut coordinator, 100);
    let mixed = coordinator.generation_accounting();
    assert_eq!(mixed.total, 2);
    assert_eq!(mixed.activated, 2);
    assert_eq!(mixed.released, 1);
    assert_eq!(mixed.active, 1);
    assert_eq!(mixed.released + mixed.active, mixed.activated);

    commit_next_authored(&mut coordinator, 200);
    assert_clean_generation_completion(coordinator.generation_accounting());
}

#[test]
fn live_hold_satisfies_activated_equals_released_plus_active() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15]),
        action(ActionKind::Up, 100, &[0x15]),
    ]);
    commit_next_authored(&mut coordinator, 0);

    let accounting = coordinator.generation_accounting();
    assert_eq!(
        accounting.activated,
        accounting.released + accounting.active
    );
    assert_eq!(accounting.scheduled, 0);
}

#[test]
fn stale_unmatched_up_has_no_musical_generation_accounting() {
    let mut coordinator = coordinator(&[action(ActionKind::Up, 0, &[0x15])]);
    let stale = coordinator
        .prepare_current_stale_packet()
        .expect("prepare stale packet")
        .expect("stale packet exists");
    coordinator
        .commit_stale_packet(stale)
        .expect("commit stale packet");

    assert_eq!(coordinator.generation_accounting().total, 0);
    assert_eq!(coordinator.generation_accounting().activated, 0);
    assert_eq!(coordinator.generation_accounting().released, 0);
}

#[test]
fn pre_activation_terminalization_does_not_increment_activation() {
    for status in [
        GenerationStatus::DroppedExpired,
        GenerationStatus::DroppedConflict,
        GenerationStatus::DroppedBackend,
        GenerationStatus::Cancelled,
    ] {
        let mut coordinator = coordinator(&[
            action(ActionKind::Down, 0, &[0x15]),
            action(ActionKind::Up, 100, &[0x15]),
        ]);
        coordinator
            .transition_generation(0, GenerationStatus::Scheduled, status)
            .expect("pre-activation terminalization");
        let accounting = coordinator.generation_accounting();
        assert_eq!(accounting.activated, 0);
        assert_eq!(accounting.active, 0);
        assert_eq!(accounting.scheduled, 0);
        coordinator
            .check_invariants()
            .expect("valid terminal accounting");
    }
}

#[test]
fn clean_natural_completion_has_only_released_generations() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15, 0x16]),
        action(ActionKind::Up, 100, &[0x15, 0x16]),
        action(ActionKind::Down, 200, &[0x17]),
        action(ActionKind::Up, 300, &[0x17]),
    ]);
    for now in [0, 100, 200, 300] {
        commit_next_authored(&mut coordinator, now);
    }

    assert_clean_generation_completion(coordinator.generation_accounting());
}

#[test]
fn malformed_transitions_and_activation_overflow_fail_closed() {
    let mut coordinator = coordinator(&[
        action(ActionKind::Down, 0, &[0x15]),
        action(ActionKind::Up, 100, &[0x15]),
    ]);
    let before = coordinator.generation_accounting();

    assert!(
        coordinator
            .transition_generation(0, GenerationStatus::Active, GenerationStatus::Released)
            .is_err()
    );
    assert_eq!(coordinator.generation_accounting(), before);
    assert!(
        coordinator
            .transition_generation(0, GenerationStatus::Scheduled, GenerationStatus::Released)
            .is_err()
    );
    assert_eq!(coordinator.generation_accounting(), before);

    coordinator.activated_generation_count = u64::MAX;
    let error = coordinator
        .transition_generation(0, GenerationStatus::Scheduled, GenerationStatus::Active)
        .expect_err("activation overflow must fail closed");
    assert!(
        error
            .to_string()
            .contains("activated generation counter overflow")
    );
    assert_eq!(
        coordinator.generation_states[0],
        GenerationStatus::Scheduled
    );
}
