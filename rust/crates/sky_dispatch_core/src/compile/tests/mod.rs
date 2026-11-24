use super::*;

mod phase_b;

fn action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    scan_codes: impl Into<smallvec::SmallVec<[u16; 4]>>,
    reason: &str,
) -> KeyActionInput {
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.into(),
        reason: reason.into(),
    }
}

#[test]
fn compilation_cancellation_is_checked_before_and_during_work() {
    let actions: Vec<_> = (0..1_000)
        .map(|index| {
            action(
                index,
                ActionKind::Up,
                u64::from(index),
                smallvec::smallvec![1],
                "stale",
            )
        })
        .collect();
    assert_eq!(
        compile_runtime_intents_with_cancellation(&actions, &[1], || true).unwrap_err(),
        CompileError::Cancelled
    );
    let mut checks = 0;
    let result = compile_runtime_intents_with_cancellation(&actions, &[1], || {
        checks += 1;
        checks == 8
    });
    assert_eq!(result.unwrap_err(), CompileError::Cancelled);
    assert_eq!(checks, 8);
}

#[test]
fn compiler_memory_budget_denies_before_allocation() {
    let error = check_memory_budget(2_048, 2_047).expect_err("budget denial");
    assert!(error.to_string().contains("exceeds budget"));
    check_memory_budget(2_047, 2_047).expect("exact budget is admitted");
}

#[test]
fn compiler_reservation_reports_real_try_reserve_failure() {
    let mut actions = Vec::<KeyActionInput>::new();
    let error = reserve_exact(&mut actions, usize::MAX, "test actions")
        .expect_err("unrepresentable Vec capacity must be denied by try_reserve_exact");
    assert!(
        error
            .to_string()
            .contains("test actions reservation failed")
    );
    assert!(actions.is_empty());
}

#[test]
fn canonical_schedule_stores_each_generation_with_its_paired_up() {
    let schedule = compile_runtime_intents(
        &[
            action(
                0,
                ActionKind::Down,
                1_000,
                smallvec::smallvec![1, 2],
                "chord",
            ),
            action(1, ActionKind::Up, 2_000, smallvec::smallvec![1], "first up"),
            action(
                2,
                ActionKind::Up,
                2_100,
                smallvec::smallvec![2],
                "second up",
            ),
        ],
        &[1, 2],
    )
    .expect("valid paired schedule");

    assert_eq!(schedule.generation_count, 2);
    assert_eq!(schedule.authored_action_count, 3);
    assert_eq!(schedule.authored_timestamp_group_count, 3);
    assert_eq!(schedule.down_chords.len(), 1);
    assert_eq!(schedule.down_chords[0].down_mask, 0b11);
    assert_eq!(schedule.down_chords[0].intents[0].generation_id, 0);
    assert_eq!(
        schedule.down_chords[0].intents[0].paired_up_offset_us,
        2_000
    );
    assert_eq!(
        schedule.down_chords[0].intents[0].paired_up_source_action_index,
        1
    );
    assert_eq!(schedule.down_chords[0].intents[0].paired_up_reason_id, 1);
    assert_eq!(schedule.down_chords[0].intents[1].generation_id, 1);
    assert_eq!(
        schedule.down_chords[0].intents[1].paired_up_offset_us,
        2_100
    );
    assert_eq!(
        schedule.down_chords[0].intents[1].paired_up_source_action_index,
        2
    );
    assert!(schedule.unmatched_ups.is_empty());
}

#[test]
fn same_timestamp_up_closes_old_generation_before_down_opens_new_one() {
    for (same_time_order, old_up_source_index) in [
        (
            [
                action(
                    0,
                    ActionKind::Down,
                    100,
                    smallvec::smallvec![1],
                    "first down",
                ),
                action(
                    1,
                    ActionKind::Down,
                    200,
                    smallvec::smallvec![1],
                    "next down",
                ),
                action(2, ActionKind::Up, 200, smallvec::smallvec![1], "old up"),
                action(3, ActionKind::Up, 300, smallvec::smallvec![1], "new up"),
            ],
            2,
        ),
        (
            [
                action(
                    0,
                    ActionKind::Down,
                    100,
                    smallvec::smallvec![1],
                    "first down",
                ),
                action(1, ActionKind::Up, 200, smallvec::smallvec![1], "old up"),
                action(
                    2,
                    ActionKind::Down,
                    200,
                    smallvec::smallvec![1],
                    "next down",
                ),
                action(3, ActionKind::Up, 300, smallvec::smallvec![1], "new up"),
            ],
            1,
        ),
    ] {
        let schedule = compile_runtime_intents(&same_time_order, &[1])
            .expect("same timestamp retrigger is a valid Up-before-Down generation transition");
        assert_eq!(schedule.down_chords.len(), 2);
        assert_eq!(schedule.down_chords[0].intents[0].generation_id, 0);
        assert_eq!(
            schedule.down_chords[0].intents[0].paired_up_source_action_index,
            old_up_source_index
        );
        assert_eq!(schedule.down_chords[1].intents[0].generation_id, 1);
        assert_eq!(
            schedule.down_chords[1].intents[0].paired_up_source_action_index,
            3
        );
    }
}

#[test]
fn unmatched_up_is_only_diagnostic_metadata_and_never_physical_work() {
    let schedule = compile_runtime_intents(
        &[action(
            7,
            ActionKind::Up,
            1_234,
            smallvec::smallvec![1, 2],
            "stale",
        )],
        &[1, 2],
    )
    .expect("unmatched Up is retained as metadata");
    assert_eq!(schedule.generation_count, 0);
    assert!(schedule.down_chords.is_empty());
    assert_eq!(schedule.unmatched_ups.len(), 2);
    assert!(schedule.unmatched_ups.iter().all(|up| {
        up.source_action_index == 7 && up.scheduled_us == 1_234 && up.reason_id == 0
    }));
}

#[test]
fn unclosed_and_overlapping_generations_are_rejected() {
    assert!(matches!(
        compile_runtime_intents(
            &[action(
                7,
                ActionKind::Down,
                1_234,
                smallvec::smallvec![1],
                "open"
            )],
            &[1]
        ),
        Err(CompileError::UnclosedGeneration {
            scan_code: 1,
            down_source_action_index: 7,
            down_scheduled_us: 1_234,
        })
    ));
    assert!(matches!(
        compile_runtime_intents(
            &[
                action(0, ActionKind::Down, 100, smallvec::smallvec![1], "first"),
                action(1, ActionKind::Down, 200, smallvec::smallvec![1], "overlap"),
            ],
            &[1]
        ),
        Err(CompileError::OverlappingSameKeyDown { .. })
    ));
}

#[test]
fn duplicate_same_timestamp_up_and_multiple_down_chords_are_rejected() {
    assert!(matches!(
        compile_runtime_intents(
            &[
                action(0, ActionKind::Up, 100, smallvec::smallvec![1], "up one"),
                action(1, ActionKind::Up, 100, smallvec::smallvec![1], "up two"),
            ],
            &[1]
        ),
        Err(CompileError::DuplicateSameTimestampUp {
            scan_code: 1,
            scheduled_us: 100
        })
    ));
    assert!(matches!(
        compile_runtime_intents(
            &[
                action(0, ActionKind::Down, 100, smallvec::smallvec![1], "left"),
                action(1, ActionKind::Down, 100, smallvec::smallvec![2], "right"),
            ],
            &[1, 2]
        ),
        Err(CompileError::SameTimestampDownBatch { scheduled_us: 100 })
    ));
}

#[test]
fn source_order_allowlist_and_batch_contents_remain_validated() {
    assert!(matches!(
        compile_runtime_intents(
            &[
                action(1, ActionKind::Up, 1, smallvec::smallvec![1], "first"),
                action(0, ActionKind::Down, 2, smallvec::smallvec![1], "second"),
            ],
            &[1]
        ),
        Err(CompileError::NonMonotonicSourceIndex { .. })
    ));
    assert!(matches!(
        compile_runtime_intents(
            &[action(
                0,
                ActionKind::Down,
                1,
                smallvec::smallvec![2],
                "invalid"
            )],
            &[1]
        ),
        Err(CompileError::ScanCodeNotAllowed { scan_code: 2, .. })
    ));
    assert!(matches!(
        compile_runtime_intents(
            &[action(
                0,
                ActionKind::Down,
                1,
                smallvec::smallvec![1, 1],
                "duplicate"
            )],
            &[1]
        ),
        Err(CompileError::DuplicateBatchScanCode { scan_code: 1, .. })
    ));
}
