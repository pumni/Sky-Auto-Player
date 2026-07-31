use proptest::prelude::*;
use sky_dispatch_core::compile::compile_runtime_intents;
use sky_dispatch_core::coordinator::RuntimeDispatchCoordinator;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};

proptest! {
    #[test]
    fn completion_anchor_and_generation_counts_hold_for_valid_chords(
        mut scan_codes in prop::collection::vec(2_u16..=6, 1..=5),
        authored_hold_us in 1_u64..20_000,
        min_hold_us in 1_u64..20_000,
        send_latency_us in 0_u64..2_000,
        release_lead_us in 0_u64..2_000,
    ) {
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let down_scheduled_us = 1_000;
        let up_scheduled_us = down_scheduled_us + authored_hold_us;
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: down_scheduled_us,
                scan_codes: scan_codes.clone(),
                reason: "property-down".to_string(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: up_scheduled_us,
                scan_codes: scan_codes.clone(),
                reason: "property-up".to_string(),
            },
        ];
        let schedule = compile_runtime_intents(&actions, &scan_codes).unwrap();
        let generation_count = schedule.generation_count;
        let mut coordinator = RuntimeDispatchCoordinator::new(schedule, min_hold_us, |us| sky_dispatch_core::time::TimelineTicks(us));

        let (down, _) = coordinator
            .pop_next_due_authored(down_scheduled_us, 0)
            .expect("down must be due");
        let (playable, conflicts) = coordinator.split_down_intents(&down.intents);
        prop_assert!(conflicts.is_empty());
        let completed_us = down_scheduled_us + send_latency_us;
        coordinator.activate_sent_downs(
            &playable,
            &scan_codes,
            down_scheduled_us,
            sky_dispatch_core::time::TimelineTicks(down_scheduled_us),
            completed_us,
            sky_dispatch_core::time::TimelineTicks(completed_us),
        );

        let (up, _) = coordinator
            .pop_next_due_authored(up_scheduled_us, 0)
            .expect("up must be due");
        let (requested, suppressed) = coordinator.request_releases(&up.intents);
        prop_assert!(suppressed.is_empty());
        let expected_due = up_scheduled_us
            .saturating_sub(release_lead_us)
            .max(completed_us + min_hold_us);
        prop_assert_eq!(
            coordinator.next_pending_release_us(release_lead_us),
            Some(expected_due)
        );
        if expected_due > 0 {
            prop_assert!(
                coordinator
                    .pop_due_pending(expected_due - 1, release_lead_us)
                    .is_empty()
            );
        }
        let due = coordinator.pop_due_pending(expected_due, release_lead_us);
        prop_assert_eq!(due.len(), requested.len());
        coordinator.complete_releases(&due, &scan_codes, &[]);

        let counts = coordinator.generation_status_counts();
        let count_total: u64 = counts.values().sum();
        prop_assert_eq!(count_total, generation_count);
        prop_assert_eq!(counts.get("released"), Some(&generation_count));
        prop_assert!(coordinator.is_finished());
    }
}
