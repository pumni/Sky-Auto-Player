use super::prepared::prepared_down_sender_cutoff;
use crate::engine::worker::{
    DispatchPreparationProbe, PreparedDispatchEntry, PreparedDispatchStream, PreparedDownHoldLimit,
};
use sky_dispatch_core::coordinator::{CoordinatorError, RuntimeDispatchCoordinator};
use sky_dispatch_core::model::{ActionKind, KeyActionInput};
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::{QpcClock, QpcTicks};
use sky_dispatch_win32::input::{
    MaterializedInstrumentKeyProfile, PacketRetryReason, SendEvidence, SendTransactionOutcome,
    SendTransactionStatus, TrackedKeyState,
};
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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

fn first_physical_frame(
    stream: &PreparedDispatchStream,
) -> &crate::engine::worker::PreparedDispatchFrame {
    match stream.entries().first().expect("prepared physical entry") {
        PreparedDispatchEntry::Physical(frame) => frame,
        PreparedDispatchEntry::Metadata { .. } => panic!("expected prepared physical entry"),
    }
}

fn complete_transport_outcome(
    packet: sky_dispatch_win32::input::PhysicalPacket,
    clock: QpcClock,
) -> SendTransactionOutcome {
    let now = clock.now().expect("prepared cutoff test QPC");
    let requested_mask = packet.up_mask | packet.down_mask;
    SendTransactionOutcome {
        status: SendTransactionStatus::Complete,
        evidence: SendEvidence {
            requested_mask,
            confirmed_mask: requested_mask,
            skipped_mask: 0,
            first_inserted: packet.event_count(),
            attempts: 1,
            zero_progress_retries: 0,
            retry_reason: PacketRetryReason::None,
            first_win32_error: None,
            last_win32_error: None,
            started_ticks: Some(now),
            completed_ticks: Some(now),
            timing_error: None,
        },
    }
}

#[test]
fn prepared_cutoff_composition_is_exact_and_sender_authoritative() {
    let target = QpcTicks::from_raw(10_000);
    let exact_minimum = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 500, &[0x15]),
        ],
        &[0x15],
        500,
    );
    let exact_frame = first_physical_frame(&exact_minimum);
    let exact_cutoff = prepared_down_sender_cutoff(exact_frame, target)
        .expect("exact cutoff arithmetic")
        .expect("exact paired cutoff");
    assert_eq!(exact_cutoff, target);

    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let calls = Arc::new(AtomicU64::new(0));
    let captured_calls = Arc::clone(&calls);
    let mut backend = TrackedKeyState::with_qpc_clock(clock);
    backend.set_packet_emitter(move |packet| {
        captured_calls.fetch_add(1, Ordering::SeqCst);
        complete_transport_outcome(packet, clock)
    });
    let equality = backend.send_prepared_physical_packet_at_target_with_cutoff(
        &exact_frame.view.prepared_packet,
        clock,
        target,
        Some(exact_cutoff),
        Some(exact_cutoff),
    );
    assert_eq!(equality.status, SendTransactionStatus::Complete);
    assert_eq!(equality.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let plus_one = exact_cutoff
        .checked_add_duration(DurationTicks::from_raw(1))
        .expect("cutoff plus one tick");
    let expired = backend.send_prepared_physical_packet_at_target_with_cutoff(
        &exact_frame.view.prepared_packet,
        clock,
        target,
        Some(exact_cutoff),
        Some(plus_one),
    );
    assert_eq!(expired.status, SendTransactionStatus::DownExpiredBeforeSend);
    assert_eq!(expired.evidence.attempts, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let long_note = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
        ],
        &[0x15],
        500,
    );
    assert_eq!(
        prepared_down_sender_cutoff(first_physical_frame(&long_note), target)
            .expect("long-note cutoff arithmetic"),
        Some(QpcTicks::from_raw(29_500))
    );

    let min_chord = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15, 0x16]),
            action(1, ActionKind::Up, 501, &[0x15]),
            action(2, ActionKind::Up, 701, &[0x16]),
        ],
        &[0x15, 0x16],
        500,
    );
    assert_eq!(
        prepared_down_sender_cutoff(first_physical_frame(&min_chord), target)
            .expect("chord cutoff arithmetic"),
        Some(QpcTicks::from_raw(10_001))
    );

    let unpaired = build_stream(
        &[action(0, ActionKind::Down, 0, &[0x15, 0x16])],
        &[0x15, 0x16],
        500,
    );
    assert_eq!(
        prepared_down_sender_cutoff(first_physical_frame(&unpaired), target)
            .expect("unpaired cutoff classification"),
        None
    );
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
