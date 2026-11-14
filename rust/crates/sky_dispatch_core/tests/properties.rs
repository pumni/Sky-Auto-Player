use proptest::prelude::*;
use sky_dispatch_core::compile::{CompileError, compile_runtime_intents};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
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
