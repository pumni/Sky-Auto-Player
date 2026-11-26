//! Admission and validation property tests for `sky_dispatch_core::plan`.

use sky_dispatch_core::model::{MAX_KEYS, OnsetChordInput};
use sky_dispatch_core::plan::{PlanError, build_onset_plan, build_onset_plan_with_cancellation};

const ALLOWED_15: [u16; MAX_KEYS] = [
    0x15, 0x16, 0x17, 0x18, 0x19, 0x23, 0x24, 0x25, 0x26, 0x27, 0x31, 0x32, 0x33, 0x34, 0x35,
];

#[test]
fn noncanonical_scan_code_profile_maps_to_canonical_slots() {
    let custom = [0x02u16, 0x03, 0x04];
    let chords = vec![
        OnsetChordInput {
            source_action_index: 0,
            scheduled_us: 0,
            scan_codes: vec![0x04, 0x02].into(),
        },
        OnsetChordInput {
            source_action_index: 1,
            scheduled_us: 16_667,
            scan_codes: vec![0x03, 0x02].into(),
        },
    ];
    let plan = build_onset_plan(&chords, &custom, 16_667).expect("custom profile valid");
    assert_eq!(plan.chords[0].key_mask, (1 << 0) | (1 << 2));
    assert_eq!(plan.chords[1].key_mask, (1 << 0) | (1 << 1));
    assert_eq!(plan.generation_count, 4);
}

#[test]
fn plan_cancellation_stops_large_builds_early() {
    let chords: Vec<OnsetChordInput> = (0..10_000u32)
        .map(|i| OnsetChordInput {
            source_action_index: i,
            scheduled_us: u64::from(i) * 20_000,
            scan_codes: vec![ALLOWED_15[0]].into(),
        })
        .collect();
    let mut polls = 0usize;
    let err = build_onset_plan_with_cancellation(&chords, &ALLOWED_15, 16_667, || {
        polls += 1;
        polls >= 5
    })
    .unwrap_err();
    assert_eq!(err, PlanError::Cancelled);
}

#[test]
fn rejects_duplicate_allowed_scan_codes_and_empty_chords() {
    assert_eq!(
        build_onset_plan(&[], &[0x15, 0x15], 16_667),
        Err(PlanError::DuplicateAllowedScanCode(0x15))
    );
    assert_eq!(
        build_onset_plan(&[], &[], 16_667),
        Err(PlanError::InvalidAllowedScanCodeCount)
    );
    assert_eq!(
        build_onset_plan(
            &[OnsetChordInput {
                source_action_index: 0,
                scheduled_us: 0,
                scan_codes: smallvec::SmallVec::new(),
            }],
            &ALLOWED_15,
            16_667,
        ),
        Err(PlanError::InvalidChordSize { action_index: 0 })
    );
    assert_eq!(
        build_onset_plan(
            &[OnsetChordInput {
                source_action_index: 0,
                scheduled_us: 0,
                scan_codes: vec![ALLOWED_15[0], ALLOWED_15[0]].into(),
            }],
            &ALLOWED_15,
            16_667,
        ),
        Err(PlanError::DuplicateChordScanCode {
            action_index: 0,
            scan_code: ALLOWED_15[0],
        })
    );
}
