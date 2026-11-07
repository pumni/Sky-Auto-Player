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
fn prepared_input_occupancy_histogram_excludes_metadata_and_counts_chords() {
    let stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15, 0x16, 0x17]),
            action(1, ActionKind::Up, 20_000, &[0x15, 0x16, 0x17]),
            action(2, ActionKind::Up, 40_000, &[0x15]),
        ],
        &[0x15, 0x16, 0x17],
    );
    let histogram = stream.physical_input_event_histogram_for_test();
    assert_eq!(histogram.iter().sum::<usize>(), stream.physical_count());
    assert_eq!(histogram[3], 2);
    assert_eq!(histogram[0], 0);
    assert_eq!(histogram[1], 0);
    assert_eq!(stream.entries().len(), 3); // two physical chords and one stale-Up metadata
}

#[test]
fn prepared_input_histogram_covers_all_canonical_chord_widths() {
    let canonical = sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
    for width in 1..=canonical.len() {
        let chord = &canonical[..width];
        let stream = build_stream(
            &[
                action(0, ActionKind::Down, 0, chord),
                action(1, ActionKind::Up, 20_000, chord),
                // A stale Up is metadata, not another physical INPUT.
                action(2, ActionKind::Up, 40_000, &canonical[..1]),
            ],
            &canonical,
        );
        let bins = stream.physical_input_event_histogram_for_test();
        let packets = physical_packets(&stream);
        assert_eq!(stream.entries().len(), 3, "width={width}");
        assert_eq!(packets.len(), 2, "width={width}");
        assert_eq!(bins[0], 0, "width={width}");
        assert_eq!(bins.iter().sum::<usize>(), 2, "width={width}");
        assert_eq!(bins[width], 2, "width={width}");
        assert_eq!(
            bins.iter()
                .enumerate()
                .map(|(size, count)| size * count)
                .sum::<usize>(),
            2 * width,
            "width={width}"
        );
        println!(
            "CHORD_OCCUPANCY width={width} physical_packets=2 input_events={}",
            2 * width
        );
    }
}

#[test]
fn prepared_input_histogram_counts_maximum_mixed_up_down_packet_once() {
    let canonical = sky_dispatch_win32::input::PHYSICAL_INSTRUMENT_SCAN_CODES;
    let left = &canonical[..7];
    let right = &canonical[7..];
    let stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, left),
            // Disjoint directions at the same authored boundary compile to one
            // 15-event mixed packet: seven Up followed by eight Down.
            action(1, ActionKind::Up, 20_000, left),
            action(2, ActionKind::Down, 20_000, right),
            action(3, ActionKind::Up, 40_000, right),
            action(4, ActionKind::Up, 60_000, &canonical[..1]),
        ],
        &canonical,
    );
    let packets = physical_packets(&stream);
    let bins = stream.physical_input_event_histogram_for_test();
    assert_eq!(stream.entries().len(), 4);
    assert_eq!(packets.len(), 3);
    assert_eq!(packets[0], (0, 0x007f));
    assert_eq!(packets[1], (0x007f, 0x7f80));
    assert_eq!(packets[2], (0x7f80, 0));
    assert_eq!(bins[0], 0);
    assert_eq!(bins[7], 1);
    assert_eq!(bins[8], 1);
    assert_eq!(bins[15], 1);
    assert_eq!(bins.iter().sum::<usize>(), 3);
    println!("MIXED_OCCUPANCY up=7 down=8 events=15 physical_packets=3 metadata=1");
}

#[test]
fn prepared_stream_storage_reports_allocated_capacity_separately_from_used_entries() {
    let stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
            action(2, ActionKind::Up, 40_000, &[0x15]),
        ],
        &[0x15],
    );
    let used_entries = stream.entries().len();
    let physical_entries = stream.physical_count();
    let metadata_entries = used_entries - physical_entries;
    let entry_bytes = std::mem::size_of::<PreparedDispatchEntry>();
    let (capacity, reserved_bytes) = stream.prepared_entry_allocation_for_test();

    assert!(physical_entries > 0);
    assert!(metadata_entries > 0);
    assert!(capacity >= used_entries);
    assert_eq!(reserved_bytes, capacity * entry_bytes);
    assert!(reserved_bytes >= std::mem::size_of_val(stream.entries()));
}

#[test]
fn prepared_physical_frames_reuse_the_materialized_target_without_extra_storage() {
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
    let mut targets = Vec::new();
    let mut metadata = Vec::new();
    for entry in stream.entries() {
        match entry {
            PreparedDispatchEntry::Physical(frame) => {
                assert_eq!(
                    std::mem::size_of_val(frame),
                    std::mem::size_of_val(&frame.view),
                );
                targets.push(frame.offset_ticks().as_u64());
            }
            PreparedDispatchEntry::Metadata { offset_ticks, .. } => {
                metadata.push(offset_ticks.as_u64());
            }
        }
    }
    assert_eq!(targets, [0, 20_000, 40_000, 60_000]);
    assert_eq!(metadata, [80_000]);
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
fn prepared_stream_has_no_song_sized_cancellation_history() {
    let stream = build_stream(
        &[
            action(0, ActionKind::Down, 0, &[0x15]),
            action(1, ActionKind::Up, 20_000, &[0x15]),
        ],
        &[0x15],
    );
    assert_eq!(stream.len(), 2);
    assert!(
        std::mem::size_of::<PreparedDispatchStream>() <= 32,
        "prepared stream must remain a fixed-size cursor plus its entry vector"
    );
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
