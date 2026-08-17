use proptest::prelude::*;
use sky_dispatch_core::compile::{CompileError, compile_runtime_intents};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::testing::simulate_schedule;
use sky_dispatch_core::time::TimelineTicks;

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

    /// Source IDs are metadata, every valid generation reaches one terminal
    /// state, and a physical Up preserves the pre-admitted authored hold.
    #[test]
    fn authored_lifecycle_preserves_generation_and_min_hold_invariants(
        mut scan_codes in prop::collection::vec(2_u16..=16, 1..=15),
        source_start in 1_u32..=100_000,
        source_gap in 1_u32..=100_000,
        authored_hold_us in 1_u64..=50_000,
        min_hold_us in 1_u64..=50_000,
        send_latency_us in 0_u64..=5_000,
    ) {
        // The simulation models a Down transport completion at
        // `down_scheduled_us + send_latency_us`.  A valid generated case must
        // therefore leave the full configured physical hold before its
        // authored Up; cases that do not are covered by the explicit
        // fail-closed coordinator tests.
        prop_assume!(authored_hold_us >= min_hold_us.saturating_add(send_latency_us));
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

    #[test]
    fn compiler_accepts_non_overlapping_retrigger_and_rejects_overlap(
        scan_code in 2_u16..=16,
        gap_us in 1_u64..=50_000,
    ) {
        let valid = vec![
            action(10, ActionKind::Down, 0, vec![scan_code]),
            action(20, ActionKind::Up, gap_us, vec![scan_code]),
            action(30, ActionKind::Down, gap_us + 1, vec![scan_code]),
            action(40, ActionKind::Up, gap_us * 2 + 1, vec![scan_code]),
        ];
        prop_assert!(compile_runtime_intents(&valid, &[scan_code]).is_ok());

        let overlapping = vec![
            action(10, ActionKind::Down, 0, vec![scan_code]),
            action(20, ActionKind::Down, gap_us, vec![scan_code]),
        ];
        let rejected_overlap = matches!(
            compile_runtime_intents(&overlapping, &[scan_code]),
            Err(CompileError::OverlappingSameKeyDown { .. })
        );
        prop_assert!(rejected_overlap);
    }

    /// Packet intent ordering follows the immutable key registry regardless
    /// of the order in which a chord's scan codes were authored.
    #[test]
    fn canonical_chord_order_is_stable(
        mut scan_codes in prop::collection::vec(2_u16..=16, 2..=15),
        rotation in any::<usize>(),
    ) {
        scan_codes.sort_unstable();
        scan_codes.dedup();
        prop_assume!(scan_codes.len() >= 2);

        let mut allowed = scan_codes.clone();
        let allowed_len = allowed.len();
        allowed.rotate_left(rotation % allowed_len);
        let mut authored = allowed.clone();
        authored.reverse();
        let actions = vec![
            action(1, ActionKind::Down, 0, authored.clone()),
            action(2, ActionKind::Up, 1_000, authored),
        ];
        let schedule = compile_runtime_intents(&actions, &allowed)
            .expect("unique allowed chord must compile");
        let packet = schedule
            .view_packet_ticks(0, TimelineTicks::ZERO)
            .expect("compiled first packet must be viewable");
        let actual: Vec<u16> = packet
            .down_intents
            .iter()
            .map(|intent| {
                packet
                    .registry
                    .scan_code_for(intent.key_slot())
                    .expect("compiled key slot must remain registered")
            })
            .collect();
        prop_assert_eq!(actual, allowed);
    }
}
