use super::super::{
    AuthoredUpIdentity, GenerationAccounting, GenerationStatus, RuntimeDispatchCoordinator,
};
use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput, MAX_KEYS, NO_GENERATION_ID, RuntimeSchedule};
use crate::time::TimelineTicks;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ReferenceUpIdentity {
    generation_id: u64,
    key_slot: u8,
    packet_index: usize,
    intent_index: usize,
}

impl From<AuthoredUpIdentity> for ReferenceUpIdentity {
    fn from(value: AuthoredUpIdentity) -> Self {
        Self {
            generation_id: value.generation_id(),
            key_slot: value.key_slot(),
            packet_index: value.packet_index(),
            intent_index: value.intent_index(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReferenceStatus {
    Scheduled,
    Active,
    Released,
    DroppedConflict,
    DroppedBackend,
    DroppedExpired,
    Cancelled,
}

impl From<GenerationStatus> for ReferenceStatus {
    fn from(value: GenerationStatus) -> Self {
        match value {
            GenerationStatus::Scheduled => Self::Scheduled,
            GenerationStatus::Active => Self::Active,
            GenerationStatus::Released => Self::Released,
            GenerationStatus::DroppedConflict => Self::DroppedConflict,
            GenerationStatus::DroppedBackend => Self::DroppedBackend,
            GenerationStatus::DroppedExpired => Self::DroppedExpired,
            GenerationStatus::Cancelled => Self::Cancelled,
        }
    }
}

struct HistoricalReferenceLedger {
    statuses: Vec<ReferenceStatus>,
    active_by_slot: [Option<u64>; MAX_KEYS],
    cancelled_up_by_slot: [Option<ReferenceUpIdentity>; MAX_KEYS],
    up_by_generation: Vec<Option<ReferenceUpIdentity>>,
    activated: u64,
}

impl HistoricalReferenceLedger {
    fn new(schedule: &RuntimeSchedule) -> Self {
        let mut up_by_generation = vec![None; schedule.generation_count as usize];
        for (packet_index, packet) in schedule.packets.iter().enumerate() {
            let start = packet.up_intent_start as usize;
            let end = start + usize::from(packet.up_intent_len);
            for (offset, intent) in schedule.intents[start..end].iter().copied().enumerate() {
                let generation_id = intent.generation_id();
                if generation_id == NO_GENERATION_ID {
                    continue;
                }
                assert!(up_by_generation[generation_id as usize].is_none());
                up_by_generation[generation_id as usize] = Some(ReferenceUpIdentity {
                    generation_id,
                    key_slot: intent.key_slot(),
                    packet_index,
                    intent_index: start + offset,
                });
            }
        }
        Self {
            statuses: vec![ReferenceStatus::Scheduled; schedule.generation_count as usize],
            active_by_slot: [None; MAX_KEYS],
            cancelled_up_by_slot: [None; MAX_KEYS],
            up_by_generation,
            activated: 0,
        }
    }

    fn cancel_live(&mut self) -> usize {
        let mut cancelled = 0;
        for slot in 0..MAX_KEYS {
            let Some(generation_id) = self.active_by_slot[slot] else {
                continue;
            };
            assert_eq!(
                self.statuses[generation_id as usize],
                ReferenceStatus::Active
            );
            assert!(self.cancelled_up_by_slot[slot].is_none());
            let identity = self.up_by_generation[generation_id as usize]
                .expect("paired generation has one authored Up identity");
            assert_eq!(usize::from(identity.key_slot), slot);
            self.statuses[generation_id as usize] = ReferenceStatus::Cancelled;
            self.active_by_slot[slot] = None;
            self.cancelled_up_by_slot[slot] = Some(identity);
            cancelled += 1;
        }
        cancelled
    }

    fn commit_packet(&mut self, schedule: &RuntimeSchedule, packet_index: usize) {
        let packet = &schedule.packets[packet_index];
        let up_start = packet.up_intent_start as usize;
        let up_end = up_start + usize::from(packet.up_intent_len);
        for (offset, intent) in schedule.intents[up_start..up_end]
            .iter()
            .copied()
            .enumerate()
        {
            let generation_id = intent.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = usize::from(intent.key_slot());
            match self.statuses[generation_id as usize] {
                ReferenceStatus::Active => {
                    assert_eq!(self.active_by_slot[slot], Some(generation_id));
                    self.statuses[generation_id as usize] = ReferenceStatus::Released;
                    self.active_by_slot[slot] = None;
                }
                ReferenceStatus::Cancelled => {
                    let identity = ReferenceUpIdentity {
                        generation_id,
                        key_slot: intent.key_slot(),
                        packet_index,
                        intent_index: up_start + offset,
                    };
                    assert_eq!(self.cancelled_up_by_slot[slot], Some(identity));
                    self.statuses[generation_id as usize] = ReferenceStatus::Released;
                    self.cancelled_up_by_slot[slot] = None;
                }
                ReferenceStatus::DroppedExpired => {}
                other => panic!("Up encountered unexpected historical state {other:?}"),
            }
        }

        let down_start = packet.down_intent_start as usize;
        let down_end = down_start + usize::from(packet.down_intent_len);
        for intent in schedule.intents[down_start..down_end].iter().copied() {
            let generation_id = intent.generation_id();
            if generation_id == NO_GENERATION_ID {
                continue;
            }
            let slot = usize::from(intent.key_slot());
            assert!(self.active_by_slot[slot].is_none());
            assert!(self.cancelled_up_by_slot[slot].is_none());
            assert_eq!(
                self.statuses[generation_id as usize],
                ReferenceStatus::Scheduled
            );
            self.statuses[generation_id as usize] = ReferenceStatus::Active;
            self.active_by_slot[slot] = Some(generation_id);
            self.activated += 1;
        }
    }

    fn accounting(&self) -> GenerationAccounting {
        let mut result = GenerationAccounting {
            total: self.statuses.len() as u64,
            activated: self.activated,
            ..GenerationAccounting::default()
        };
        for status in &self.statuses {
            match status {
                ReferenceStatus::Scheduled => result.scheduled += 1,
                ReferenceStatus::Active => result.active += 1,
                ReferenceStatus::Released => result.released += 1,
                ReferenceStatus::DroppedConflict => result.dropped_conflict += 1,
                ReferenceStatus::DroppedBackend => result.dropped_backend += 1,
                ReferenceStatus::DroppedExpired => result.dropped_expired += 1,
                ReferenceStatus::Cancelled => result.cancelled += 1,
            }
        }
        result
    }
}

#[derive(Clone, Copy)]
struct DeterministicRandom(u64);

impl DeterministicRandom {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }
}

fn randomized_legal_schedule(seed: u64, notes_per_key: usize) -> RuntimeSchedule {
    let mut random = DeterministicRandom(seed.max(1));
    let scan_codes = (0..MAX_KEYS)
        .map(|slot| 0x15 + slot as u16)
        .collect::<Vec<_>>();
    let mut events = Vec::with_capacity(MAX_KEYS * notes_per_key * 2);
    for slot in 0..MAX_KEYS {
        let mut down_ticks = random.next() % 9;
        for _ in 0..notes_per_key {
            let hold = 1 + random.next() % 15;
            let up_ticks = down_ticks + hold;
            events.push((down_ticks, ActionKind::Down, slot));
            events.push((up_ticks, ActionKind::Up, slot));
            down_ticks = up_ticks + 1 + random.next() % 11;
        }
    }
    events.sort_by_key(|(ticks, kind, slot)| {
        (
            *ticks,
            if *kind == ActionKind::Up { 0u8 } else { 1u8 },
            *slot,
        )
    });

    let mut actions = Vec::new();
    let mut cursor = 0;
    let mut source_action_index = 0u32;
    while cursor < events.len() {
        let ticks = events[cursor].0;
        let mut group_end = cursor + 1;
        while group_end < events.len() && events[group_end].0 == ticks {
            group_end += 1;
        }
        for kind in [ActionKind::Up, ActionKind::Down] {
            let slots = events[cursor..group_end]
                .iter()
                .filter_map(|(_, event_kind, slot)| (*event_kind == kind).then_some(*slot))
                .collect::<Vec<_>>();
            if slots.is_empty() {
                continue;
            }
            actions.push(KeyActionInput {
                source_action_index,
                kind,
                scheduled_us: ticks,
                scan_codes: slots.iter().map(|slot| scan_codes[*slot]).collect(),
                reason: "randomized resumable ledger oracle".into(),
            });
            source_action_index += 1;
        }
        cursor = group_end;
    }
    compile_runtime_intents(&actions, &scan_codes).expect("random schedule is legal and paired")
}

fn assert_matches_reference(
    coordinator: &RuntimeDispatchCoordinator,
    reference: &HistoricalReferenceLedger,
) {
    let expected_statuses = reference
        .statuses
        .iter()
        .copied()
        .map(|status| match status {
            ReferenceStatus::Scheduled => GenerationStatus::Scheduled,
            ReferenceStatus::Active => GenerationStatus::Active,
            ReferenceStatus::Released => GenerationStatus::Released,
            ReferenceStatus::DroppedConflict => GenerationStatus::DroppedConflict,
            ReferenceStatus::DroppedBackend => GenerationStatus::DroppedBackend,
            ReferenceStatus::DroppedExpired => GenerationStatus::DroppedExpired,
            ReferenceStatus::Cancelled => GenerationStatus::Cancelled,
        })
        .collect::<Vec<_>>();
    assert_eq!(coordinator.generation_states.as_ref(), expected_statuses);
    assert_eq!(coordinator.generation_accounting(), reference.accounting());
    for slot in 0..MAX_KEYS {
        assert_eq!(
            coordinator.active_by_slot[slot]
                .as_ref()
                .map(|active| active.generation_id),
            reference.active_by_slot[slot]
        );
        assert_eq!(
            coordinator.cancelled_up_by_slot[slot].map(ReferenceUpIdentity::from),
            reference.cancelled_up_by_slot[slot]
        );
    }
    coordinator
        .check_invariants()
        .expect("full historical ledger audit agrees with local state");
}

#[test]
fn randomized_resumable_sequences_match_independent_historical_ledger() {
    // Reproduction rule: seed[i] = 0x4560_0001 + i * 0x9e37_79b9.
    // Thirty-two seeds each create 15 * 16 = 240 paired generations: 7,680
    // independently modeled legal generation sequences in total.
    for seed_index in 0..32u64 {
        let seed = 0x4560_0001u64.wrapping_add(seed_index.wrapping_mul(0x9e37_79b9));
        let schedule = randomized_legal_schedule(seed, 16);
        let reference_schedule = schedule.clone();
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
        let mut reference = HistoricalReferenceLedger::new(&reference_schedule);
        let mut random = DeterministicRandom(seed ^ 0xd1b5_4a32_d192_ed03);
        let mut operations = 0usize;

        while !coordinator.is_finished() {
            if random.next().is_multiple_of(3) {
                let actual_cancelled = coordinator
                    .cancel_live_generations_after_verified_resumable_cleanup()
                    .expect("bounded verified-cleanup cancellation");
                let expected_cancelled = reference.cancel_live();
                assert_eq!(actual_cancelled, expected_cancelled, "seed {seed:#x}");
                assert_matches_reference(&coordinator, &reference);
            }

            let prepared = coordinator
                .prepare_current_authored_packet()
                .expect("prepare randomized authored packet")
                .expect("remaining generation has an authored packet");
            let packet_index = prepared.frame.packet_index;
            coordinator
                .validate_frozen_authored_commit(&prepared.commit)
                .expect("frozen commit has current cancellation proof");
            coordinator
                .commit_prepared_authored_frame_success_frozen(
                    &prepared.commit,
                    prepared.frame.authored_ticks,
                    prepared.frame.authored_ticks,
                )
                .expect("commit randomized authored packet");
            reference.commit_packet(&reference_schedule, packet_index);
            assert_matches_reference(&coordinator, &reference);
            operations += 1;
        }

        assert!(operations > 0, "seed {seed:#x} exercised no packets");
        assert!(reference.cancelled_up_by_slot.iter().all(Option::is_none));
        assert_eq!(reference.accounting().cancelled, 0);
        assert_eq!(
            reference.accounting().released,
            reference.accounting().total
        );
    }
}

fn full_chord_schedule(generation_count: usize) -> RuntimeSchedule {
    let scan_codes = (0..MAX_KEYS)
        .map(|slot| 0x15 + slot as u16)
        .collect::<Vec<_>>();
    let mut actions = Vec::new();
    let mut remaining = generation_count;
    let mut cycle = 0u64;
    let mut source_action_index = 0u32;
    while remaining > 0 {
        let chord = remaining.min(MAX_KEYS);
        let chord_keys = &scan_codes[..chord];
        let down_ticks = cycle * 3;
        actions.push(KeyActionInput {
            source_action_index,
            kind: ActionKind::Down,
            scheduled_us: down_ticks,
            scan_codes: chord_keys.iter().copied().collect(),
            reason: "bounded operation-count fixture".into(),
        });
        source_action_index += 1;
        actions.push(KeyActionInput {
            source_action_index,
            kind: ActionKind::Up,
            scheduled_us: down_ticks + 1,
            scan_codes: chord_keys.iter().copied().collect(),
            reason: "bounded operation-count fixture".into(),
        });
        source_action_index += 1;
        remaining -= chord;
        cycle += 1;
    }
    compile_runtime_intents(&actions, &scan_codes)
        .expect("operation-count schedule stays within action cap")
}

#[test]
fn resumable_reconciliation_visits_fixed_slots_at_million_generation_scale() {
    for generation_count in [1usize, 16_384, 1_000_000] {
        let schedule = full_chord_schedule(generation_count);
        assert_eq!(schedule.generation_count, generation_count as u64);
        let mut coordinator =
            RuntimeDispatchCoordinator::new(schedule, 0, 0, TimelineTicks::from_raw);
        let down = coordinator
            .prepare_current_authored_packet()
            .expect("prepare first active chord")
            .expect("first Down exists");
        coordinator
            .commit_prepared_authored_frame_success_frozen(
                &down.commit,
                TimelineTicks::ZERO,
                TimelineTicks::ZERO,
            )
            .expect("commit first Down chord");

        coordinator.reset_operation_evidence_for_test_support();
        let cancelled = coordinator
            .cancel_live_generations_after_verified_resumable_cleanup()
            .expect("cancel at most fifteen live owners");
        assert_eq!(cancelled, generation_count.min(MAX_KEYS));
        let evidence = coordinator.operation_evidence_for_test_support();
        assert_eq!(evidence.resumable_cancel_slot_visits, MAX_KEYS as u64);
        assert_eq!(evidence.local_invariant_slot_visits, (MAX_KEYS * 2) as u64);
        assert_eq!(evidence.deep_audit_generation_visits, 0);
        assert_eq!(
            coordinator.cancelled_up_token_count_for_test_support(),
            generation_count.min(MAX_KEYS)
        );

        coordinator.reset_operation_evidence_for_test_support();
        coordinator
            .check_invariants()
            .expect("deliberate full audit remains available");
        let audit = coordinator.operation_evidence_for_test_support();
        assert_eq!(audit.deep_audit_generation_visits, generation_count as u64);
        assert_eq!(audit.local_invariant_slot_visits, MAX_KEYS as u64);
    }
}
