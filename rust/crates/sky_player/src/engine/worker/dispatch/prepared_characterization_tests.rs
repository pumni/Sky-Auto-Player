use crate::engine::worker::{
    DispatchPreparationProbe, PreparedDispatchEntry, PreparedDispatchStream,
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

fn build_stream(actions: &[KeyActionInput], allowed_scan_codes: &[u16]) -> PreparedDispatchStream {
    let schedule = sky_dispatch_core::compile::compile_runtime_intents(actions, allowed_scan_codes)
        .expect("prepared stream schedule");
    let qpc_clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let coordinator = RuntimeDispatchCoordinator::try_new_ticks(
        schedule,
        500,
        DurationTicks::from_raw(500),
        |microseconds| {
            qpc_clock
                .timeline_from_us(microseconds)
                .map_err(|error| CoordinatorError::TimeConversion(format!("{error:?}")))
        },
    )
    .expect("prepared stream coordinator");
    PreparedDispatchStream::build(
        coordinator,
        qpc_clock,
        &DispatchPreparationProbe::default(),
        &MaterializedInstrumentKeyProfile::canonical(),
    )
    .expect("prepared stream")
    .0
}

fn physical_packets(stream: &PreparedDispatchStream) -> Vec<(u16, u16)> {
    stream
        .entries()
        .iter()
        .filter_map(|entry| match entry {
            PreparedDispatchEntry::Physical(frame) => Some((
                frame.view.packet_masks.up_mask,
                frame.view.packet_masks.down_mask,
            )),
            PreparedDispatchEntry::Metadata { .. } => None,
        })
        .collect()
}

fn complete_transport_outcome(
    packet: sky_dispatch_win32::input::PhysicalPacket,
    clock: QpcClock,
) -> SendTransactionOutcome {
    let now = clock.now().expect("prepared sender QPC");
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
fn prepared_stream_contains_only_materialized_physical_boundaries() {
    let stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
            action(2, ActionKind::Down, 40_000, &[0x15]),
            action(3, ActionKind::Up, 60_000, &[0x15]),
            action(4, ActionKind::Up, 80_000, &[0x15]),
        ],
        &[0x15],
    );

    assert_eq!(
        physical_packets(&stream),
        vec![(0, 1), (1, 0), (0, 1), (1, 0)]
    );
}

#[test]
fn late_prepared_down_still_has_one_transport_attempt() {
    let stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
        ],
        &[0x15],
    );
    let frame = stream
        .entries()
        .iter()
        .find_map(|entry| match entry {
            PreparedDispatchEntry::Physical(frame) if frame.view.packet_masks.down_mask != 0 => {
                Some(frame)
            }
            _ => None,
        })
        .expect("prepared Down frame");
    let calls = Arc::new(AtomicU64::new(0));
    let captured_calls = Arc::clone(&calls);
    let clock = QpcClock::from_frequency_hz(NonZeroU64::new(1_000_000).unwrap());
    let mut backend = TrackedKeyState::with_qpc_clock(clock);
    backend.set_packet_emitter(move |packet| {
        captured_calls.fetch_add(1, Ordering::SeqCst);
        complete_transport_outcome(packet, clock)
    });

    let outcome = backend.send_prepared_physical_packet_with_start(
        &frame.view.prepared_packet,
        QpcTicks::from_raw(100_000),
    );
    assert_eq!(outcome.status, SendTransactionStatus::Complete);
    assert_eq!(outcome.evidence.attempts, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn prepared_stream_suspension_reconciliation_does_not_mutate_frozen_entries() {
    let mut stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
        ],
        &[0x15],
    );
    let before = format!("{:?}", stream.entries());
    stream
        .reconcile_resumable_suspension(&[0])
        .expect("bounded suspension reconciliation");
    assert_eq!(stream.explicitly_cancelled_generation_ids(), &[0]);
    assert_eq!(format!("{:?}", stream.entries()), before);
}

#[test]
fn independently_built_prepared_streams_match() {
    let actions = [
        action(0, ActionKind::Down, 0, &[0x15, 0x16]),
        action(1, ActionKind::Up, 501, &[0x15]),
        action(2, ActionKind::Up, 701, &[0x16]),
        action(3, ActionKind::Down, 2_000, &[0x15]),
        action(4, ActionKind::Up, 2_501, &[0x15]),
    ];
    let first = build_stream(&actions, &[0x15, 0x16]);
    let second = build_stream(&actions, &[0x15, 0x16]);
    assert_eq!(
        format!("{:?}", first.entries()),
        format!("{:?}", second.entries())
    );
}
