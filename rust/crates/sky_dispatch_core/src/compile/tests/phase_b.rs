use super::*;

#[test]
fn test_exact_down_up_pairing_in_down_chords() {
    let allowed = vec![0x15, 0x16, 0x17];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![0x15, 0x16, 0x17],
            reason: "chord".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "up-0x15".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 2500,
            scan_codes: smallvec::smallvec![0x16],
            reason: "up-0x16".into(),
        },
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 3000,
            scan_codes: smallvec::smallvec![0x17],
            reason: "up-0x17".into(),
        },
    ];
    let sched = compile_runtime_intents(&actions, &allowed).expect("valid paired schedule");
    assert_eq!(sched.down_chords.len(), 1);
    assert_eq!(sched.schedule_end_offset_us, 3000);

    let chord = &sched.down_chords[0];
    assert_eq!(chord.chord_index, 0);
    assert_eq!(chord.scheduled_us, 1000);
    assert_eq!(chord.intents.len(), 3);

    // Verify each intent has the exact paired Up offset
    let intent0 = chord.intents.iter().find(|i| i.scan_code == 0x15).unwrap();
    assert_eq!(intent0.paired_up_offset_us, 2000);

    let intent1 = chord.intents.iter().find(|i| i.scan_code == 0x16).unwrap();
    assert_eq!(intent1.paired_up_offset_us, 2500);

    let intent2 = chord.intents.iter().find(|i| i.scan_code == 0x17).unwrap();
    assert_eq!(intent2.paired_up_offset_us, 3000);
}

#[test]
fn test_schedule_end_offset_preserves_metadata_tail() {
    let allowed = vec![0x15];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "note".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 2000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "note-off".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 5000,
            scan_codes: smallvec::smallvec![0x15],
            reason: "metadata-tail".into(),
        },
    ];
    let sched = compile_runtime_intents(&actions, &allowed).expect("valid schedule with tail");
    assert_eq!(sched.schedule_end_offset_us, 5000);
    assert_eq!(sched.down_chords.len(), 1);
    assert_eq!(sched.down_chords[0].intents[0].paired_up_offset_us, 2000);
}

#[test]
fn test_custom_profile_exact_pairing() {
    // Arbitrary non-standard scan codes
    let allowed = vec![0x33, 0x44, 0x55, 0x66];
    let actions = vec![
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 500,
            scan_codes: smallvec::smallvec![0x66, 0x33],
            reason: "chord".into(),
        },
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1200,
            scan_codes: smallvec::smallvec![0x33],
            reason: "up-33".into(),
        },
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Up,
            scheduled_us: 1500,
            scan_codes: smallvec::smallvec![0x66],
            reason: "up-66".into(),
        },
    ];
    let sched = compile_runtime_intents(&actions, &allowed).expect("custom profile valid");
    assert_eq!(sched.down_chords.len(), 1);
    let chord = &sched.down_chords[0];
    let intent_33 = chord.intents.iter().find(|i| i.scan_code == 0x33).unwrap();
    assert_eq!(intent_33.paired_up_offset_us, 1200);
    let intent_66 = chord.intents.iter().find(|i| i.scan_code == 0x66).unwrap();
    assert_eq!(intent_66.paired_up_offset_us, 1500);
}

#[test]
fn test_compile_full_15_key_chord_exact_pairing() {
    let allowed: Vec<u16> = (0..MAX_KEYS).map(|i| 0x100 + i as u16).collect();
    let mut actions = Vec::new();
    // Chord of all 15 keys at 1000 us
    actions.push(KeyActionInput {
        source_action_index: 0,
        kind: ActionKind::Down,
        scheduled_us: 1000,
        scan_codes: allowed.iter().copied().collect(),
        reason: "all-15-chord".into(),
    });
    // Individual releases at 2000 + i * 100
    for (i, &sc) in allowed.iter().enumerate() {
        actions.push(KeyActionInput {
            source_action_index: 1 + i as u32,
            kind: ActionKind::Up,
            scheduled_us: 2000 + (i as u64) * 100,
            scan_codes: smallvec::smallvec![sc],
            reason: format!("up-{sc}").into(),
        });
    }

    let sched = compile_runtime_intents(&actions, &allowed).expect("15-key chord valid");
    assert_eq!(sched.down_chords.len(), 1);
    let chord = &sched.down_chords[0];
    assert_eq!(chord.down_mask, 0x7FFF);
    assert_eq!(chord.intents.len(), 15);
    for (i, &sc) in allowed.iter().enumerate() {
        let intent = chord.intents.iter().find(|it| it.scan_code == sc).unwrap();
        assert_eq!(intent.paired_up_offset_us, 2000 + (i as u64) * 100);
    }
    assert_eq!(sched.schedule_end_offset_us, 2000 + 14 * 100);
}

#[test]
fn test_compile_multi_chord_sequence_preserves_chord_order() {
    let allowed = vec![0x15, 0x16, 0x17, 0x18];
    let actions = vec![
        // Chord 0: {0x15, 0x16} at 1000
        KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![0x15, 0x16],
            reason: "c0".into(),
        },
        // Release {0x15} at 1500
        KeyActionInput {
            source_action_index: 1,
            kind: ActionKind::Up,
            scheduled_us: 1500,
            scan_codes: smallvec::smallvec![0x15],
            reason: "u0-15".into(),
        },
        // Chord 1: {0x17, 0x18} at 1800
        KeyActionInput {
            source_action_index: 2,
            kind: ActionKind::Down,
            scheduled_us: 1800,
            scan_codes: smallvec::smallvec![0x17, 0x18],
            reason: "c1".into(),
        },
        // Release {0x16, 0x17} at 2200
        KeyActionInput {
            source_action_index: 3,
            kind: ActionKind::Up,
            scheduled_us: 2200,
            scan_codes: smallvec::smallvec![0x16, 0x17],
            reason: "u-multi".into(),
        },
        // Release {0x18} at 2500
        KeyActionInput {
            source_action_index: 4,
            kind: ActionKind::Up,
            scheduled_us: 2500,
            scan_codes: smallvec::smallvec![0x18],
            reason: "u1-18".into(),
        },
    ];

    let sched = compile_runtime_intents(&actions, &allowed).expect("multi-chord valid");
    assert_eq!(sched.down_chords.len(), 2);
    assert_eq!(sched.down_chords[0].chord_index, 0);
    assert_eq!(sched.down_chords[0].scheduled_us, 1000);
    assert_eq!(sched.down_chords[1].chord_index, 1);
    assert_eq!(sched.down_chords[1].scheduled_us, 1800);

    let c0_15 = sched.down_chords[0]
        .intents
        .iter()
        .find(|i| i.scan_code == 0x15)
        .unwrap();
    assert_eq!(c0_15.paired_up_offset_us, 1500);

    let c0_16 = sched.down_chords[0]
        .intents
        .iter()
        .find(|i| i.scan_code == 0x16)
        .unwrap();
    assert_eq!(c0_16.paired_up_offset_us, 2200);

    let c1_17 = sched.down_chords[1]
        .intents
        .iter()
        .find(|i| i.scan_code == 0x17)
        .unwrap();
    assert_eq!(c1_17.paired_up_offset_us, 2200);

    let c1_18 = sched.down_chords[1]
        .intents
        .iter()
        .find(|i| i.scan_code == 0x18)
        .unwrap();
    assert_eq!(c1_18.paired_up_offset_us, 2500);
}
