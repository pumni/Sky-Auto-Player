use proptest::prelude::*;
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::testing::simulate_schedule;

fn action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    scan_codes: Vec<u16>,
) -> KeyActionInput {
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.into(),
        reason: "property".into(),
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Production executor simulation properties for generation accounting
    /// and the authored hold floor; pure compiler properties remain in `properties`.
    #[test]
    fn authored_lifecycle_preserves_generation_and_min_hold_invariants(
        mut scan_codes in prop::collection::vec(2_u16..=16, 1..=15),
        source_start in 1_u32..=100_000,
        source_gap in 1_u32..=100_000,
        authored_hold_us in 1_u64..=50_000,
        min_hold_us in 1_u64..=50_000,
        send_latency_us in 0_u64..=5_000,
    ) {
        prop_assume!(authored_hold_us >= min_hold_us);
        scan_codes.sort_unstable();
        scan_codes.dedup();
        let down_scheduled_us = 1_000;
        let actions = vec![
            action(
                source_start,
                ActionKind::Down,
                down_scheduled_us,
                scan_codes.clone(),
            ),
            action(
                source_start + source_gap,
                ActionKind::Up,
                down_scheduled_us + authored_hold_us,
                scan_codes.clone(),
            ),
        ];

        let result = simulate_schedule(
            &actions,
            &scan_codes,
            min_hold_us,
            send_latency_us,
        )
        .expect("generated non-overlapping chord must compile and simulate");

        prop_assert!(result.is_finished);
        prop_assert_eq!(
            result.status_counts.values().sum::<u64>(),
            result.total_generations,
        );
        prop_assert_eq!(
            result.status_counts.get("released").copied(),
            Some(result.total_generations),
        );

        for generation_id in 0..result.total_generations {
            let down = result
                .events
                .iter()
                .find(|event| {
                    event.kind == "down"
                        && event.generation_ids.contains(&Some(generation_id))
                })
                .expect("each generation must have a Down event");
            let up = result
                .events
                .iter()
                .find(|event| {
                    event.kind == "up"
                        && event.generation_ids.contains(&Some(generation_id))
                })
                .expect("each generation must have an Up event");
            let authored_floor = down
                .scheduled_us
                .checked_add(min_hold_us)
                .expect("bounded generated timestamps cannot overflow");
            prop_assert!(
                up.actual_us >= authored_floor,
                "generation {generation_id} released at {} before authored floor {authored_floor}",
                up.actual_us,
            );
        }
    }
}
