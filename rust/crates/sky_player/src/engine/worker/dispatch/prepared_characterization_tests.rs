use crate::engine::worker::{
    DispatchPreparationProbe, PreparedDispatchEntry, PreparedDispatchStream, PreparedDownHoldLimit,
};
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::input::MaterializedInstrumentKeyProfile;
use std::num::NonZeroU64;

fn action(
    source_action_index: u32,
    kind: ActionKind,
    scheduled_us: u64,
    scan_codes: &[u16],
) -> KeyActionInput {
    KeyActionInput {
        source_action_index,
        kind,
        scheduled_us,
        scan_codes: scan_codes.iter().copied().collect(),
        reason: format!("prepared-characterization-{source_action_index}").into(),
    }
}

fn build_stream(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
    min_hold_ticks: u64,
) -> PreparedDispatchStream {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(actions, allowed_scan_codes)
        .expect("characterization schedule");
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        min_hold_ticks,
        DurationTicks::from_raw(min_hold_ticks),
        |microseconds| {
            qpc_clock
                .timeline_from_us(microseconds)
                .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
        },
    )
    .expect("characterization coordinator");
    let profile = MaterializedInstrumentKeyProfile::canonical();
    let probe = DispatchPreparationProbe::default();
    PreparedDispatchStream::build(coordinator, qpc_clock, &probe, &profile)
        .expect("characterization prepared stream")
        .0
}

fn physical_policies(
    stream: &PreparedDispatchStream,
) -> Vec<(u16, u16, Option<PreparedDownHoldLimit>)> {
    stream
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            PreparedDispatchEntry::Physical(frame) => Some((
                frame.view.packet_masks.up_mask,
                frame.view.packet_masks.down_mask,
                frame.down_policy.map(|policy| policy.hold_limit),
            )),
            PreparedDispatchEntry::Metadata { .. } => None,
        })
        .collect()
}

fn stream_debug_snapshot(stream: &PreparedDispatchStream) -> Vec<String> {
    stream
        .entries()
        .iter()
        .map(|entry| format!("{entry:?}"))
        .collect()
}

#[test]
fn prepared_down_policy_materializes_exact_hold_slack_and_unpaired_state() {
    let exact_minimum = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 500, &[0x15]),
        ],
        &[0x15],
        500,
    );
    assert_eq!(
        physical_policies(&exact_minimum),
        vec![
            (
                0,
                1,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::ZERO))
            ),
            (1, 0, None)
        ]
    );

    let one_tick_over = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 501, &[0x15]),
        ],
        &[0x15],
        500,
    );
    assert_eq!(
        physical_policies(&one_tick_over),
        vec![
            (
                0,
                1,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::from_raw(1)))
            ),
            (1, 0, None)
        ]
    );

    let long_note = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
        ],
        &[0x15],
        500,
    );
    assert_eq!(
        physical_policies(&long_note),
        vec![
            (
                0,
                1,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::from_raw(
                    19_500
                )))
            ),
            (1, 0, None)
        ]
    );

    let all_unpaired = build_stream(
        &[action(0, ActionKind::Down, 0, &[0x15, 0x16])],
        &[0x15, 0x16],
        500,
    );
    assert_eq!(
        physical_policies(&all_unpaired),
        vec![(0, 0b11, Some(PreparedDownHoldLimit::NoPairedRelease))]
    );
}

#[test]
fn prepared_down_policy_uses_minimum_paired_chord_slack_and_ignores_unpaired_members() {
    let paired_chord = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15, 0x16]),
            action(1, ActionKind::Up, 501, &[0x15]),
            action(2, ActionKind::Up, 701, &[0x16]),
        ],
        &[0x15, 0x16],
        500,
    );
    assert_eq!(
        physical_policies(&paired_chord),
        vec![
            (
                0,
                0b11,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::from_raw(1)))
            ),
            (1, 0, None),
            (2, 0, None)
        ]
    );

    let paired_and_unpaired = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15, 0x16]),
            action(1, ActionKind::Up, 501, &[0x15]),
        ],
        &[0x15, 0x16],
        500,
    );
    assert_eq!(
        physical_policies(&paired_and_unpaired),
        vec![
            (
                0,
                0b11,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::from_raw(1)))
            ),
            (1, 0, None)
        ]
    );
}

#[test]
fn prepared_characterization_covers_same_key_and_mixed_boundaries() {
    let same_key = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 500, &[0x15]),
            action(2, ActionKind::Down, 2_000, &[0x15]),
            action(3, ActionKind::Up, 2_500, &[0x15]),
        ],
        &[0x15],
        500,
    );
    assert_eq!(
        physical_policies(&same_key),
        vec![
            (
                0,
                1,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::ZERO))
            ),
            (1, 0, None),
            (
                0,
                1,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::ZERO))
            ),
            (1, 0, None),
        ]
    );

    let mixed = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
            action(2, ActionKind::Down, 20_000, &[0x16]),
            action(3, ActionKind::Up, 40_000, &[0x16]),
        ],
        &[0x15, 0x16],
        500,
    );
    assert_eq!(
        physical_policies(&mixed),
        vec![
            (
                0,
                1,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::from_raw(
                    19_500
                )))
            ),
            (
                1,
                2,
                Some(PreparedDownHoldLimit::HoldSlack(DurationTicks::from_raw(
                    19_500
                )))
            ),
            (2, 0, None),
        ]
    );
}

#[test]
fn prepared_stream_suspension_reconciliation_does_not_mutate_frozen_entries() {
    let mut stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
        ],
        &[0x15],
        500,
    );
    let before = stream_debug_snapshot(&stream);
    stream
        .reconcile_resumable_suspension(&[0])
        .expect("bounded suspension reconciliation");
    assert_eq!(stream.explicitly_cancelled_generation_ids(), &[0]);
    assert_eq!(stream_debug_snapshot(&stream), before);
}

#[test]
fn independently_built_prepared_streams_match_including_policy_metadata() {
    let actions = [
        action(0, ActionKind::Down, 0, &[0x15, 0x16]),
        action(1, ActionKind::Up, 501, &[0x15]),
        action(2, ActionKind::Up, 701, &[0x16]),
        action(3, ActionKind::Down, 2_000, &[0x15]),
        action(4, ActionKind::Up, 2_501, &[0x15]),
    ];
    let first = build_stream(&actions, &[0x15, 0x16], 500);
    let second = build_stream(&actions, &[0x15, 0x16], 500);
    assert_eq!(
        stream_debug_snapshot(&first),
        stream_debug_snapshot(&second)
    );
}
