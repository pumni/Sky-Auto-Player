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

    /// Multiple sequential chords with varying sizes preserve full generation accounting,
    /// complete release, and zero leaked active state.
    #[test]
    fn multi_chord_sequence_preserves_generation_accounting_and_release_obligation(
        mut scan_codes in prop::collection::vec(2_u16..=16, 2..=6),
        min_hold_us in 10_000_u64..=20_000,
        gap_us in 25_000_u64..=40_000,
    ) {
        scan_codes.sort_unstable();
        scan_codes.dedup();
        prop_assume!(scan_codes.len() >= 2);

        let split = scan_codes.len() / 2;
        let chord_a = scan_codes[..split].to_vec();
        let chord_b = scan_codes[split..].to_vec();

        let t0 = 1_000;
        let t1 = t0 + min_hold_us;
        let t2 = t1 + gap_us;
        let t3 = t2 + min_hold_us;

        let actions = vec![
            action(1, ActionKind::Down, t0, chord_a.clone()),
            action(2, ActionKind::Up, t1, chord_a.clone()),
            action(3, ActionKind::Down, t2, chord_b.clone()),
            action(4, ActionKind::Up, t3, chord_b.clone()),
        ];

        let result = simulate_schedule(&actions, &scan_codes, min_hold_us, 500)
            .expect("two sequential non-overlapping chords must compile and simulate");

        prop_assert!(result.is_finished);
        let expected_generations = (chord_a.len() + chord_b.len()) as u64;
        prop_assert_eq!(result.total_generations, expected_generations);
        prop_assert_eq!(
            result.status_counts.get("released").copied(),
            Some(expected_generations)
        );
        prop_assert_eq!(result.status_counts.get("active").copied().unwrap_or(0), 0);
    }

    /// Generation identifiers are assigned in strictly contiguous ascending order [0..N-1].
    #[test]
    fn generation_identifiers_are_monotonic_and_contiguous(
        mut scan_codes in prop::collection::vec(2_u16..=16, 1..=5),
        min_hold_us in 5_000_u64..=15_000,
    ) {
        scan_codes.sort_unstable();
        scan_codes.dedup();

        let actions = vec![
            action(1, ActionKind::Down, 1_000, scan_codes.clone()),
            action(2, ActionKind::Up, 1_000 + min_hold_us, scan_codes.clone()),
        ];

        let result = simulate_schedule(&actions, &scan_codes, min_hold_us, 100)
            .expect("chord must compile and simulate");

        let mut observed_generations = Vec::new();
        for event in &result.events {
            if event.kind == "down" {
                for gen_id in &event.generation_ids {
                    if let Some(id) = gen_id {
                        observed_generations.push(*id);
                    }
                }
            }
        }

        let expected: Vec<u64> = (0..result.total_generations).collect();
        prop_assert_eq!(observed_generations, expected);
    }
}
