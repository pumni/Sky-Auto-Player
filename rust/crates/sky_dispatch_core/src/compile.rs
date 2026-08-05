//! Generation compiler for turning authored KeyAction sequences into RuntimeSchedule.

use std::collections::HashMap;

use thiserror::Error;

use crate::model::*;

pub const MAX_ACTIONS: usize = 1_000_000;
pub const MAX_REASON_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CompileError {
    #[error("allowed_scan_codes must contain between 1 and {MAX_KEYS} entries")]
    InvalidAllowedScanCodeCount,
    #[error("allowed_scan_codes contains duplicate scan code {0}")]
    DuplicateAllowedScanCode(u16),
    #[error("action count exceeds the configured cap of {MAX_ACTIONS}")]
    TooManyActions,
    #[error(
        "source_action_index must be strictly increasing: previous={previous}, current={current}"
    )]
    NonMonotonicSourceIndex { previous: u32, current: u32 },
    #[error("scheduled_us must be nondecreasing: previous={previous}, current={current}")]
    NonMonotonicSchedule { previous: u64, current: u64 },
    #[error(
        "multiple same-timestamp down actions are not one atomic chord; merge them before dispatch: timestamp={scheduled_us}"
    )]
    SameTimestampDownBatch { scheduled_us: u64 },
    #[error(
        "duplicate same-timestamp up action for scan code {scan_code}: timestamp={scheduled_us}"
    )]
    DuplicateSameTimestampUp { scan_code: u16, scheduled_us: u64 },
    #[error("too many authored batches at timestamp {scheduled_us} for packet metadata")]
    TooManyBatchesAtTimestamp { scheduled_us: u64 },
    #[error("action {action_index} must contain between 1 and {MAX_KEYS} scan codes")]
    InvalidBatchSize { action_index: u32 },
    #[error("action {action_index} contains duplicate scan code {scan_code}")]
    DuplicateBatchScanCode { action_index: u32, scan_code: u16 },
    #[error("action {action_index} uses scan code {scan_code} outside the prepared allowlist")]
    ScanCodeNotAllowed { action_index: u32, scan_code: u16 },
    #[error("action {action_index} reason exceeds {MAX_REASON_BYTES} UTF-8 bytes")]
    ReasonTooLong { action_index: u32 },
    #[error("too many unique reasons for the u16 reason table")]
    TooManyReasons,
    #[error("generation identifier overflow")]
    GenerationOverflow,
    #[error(
        "overlapping same-key down actions on scan code {scan_code}: first down at index {first_down_action_index} (scheduled_us={first_scheduled_us}), second down at index {second_down_action_index} (scheduled_us={second_scheduled_us})"
    )]
    OverlappingSameKeyDown {
        scan_code: u16,
        first_down_action_index: u32,
        second_down_action_index: u32,
        first_scheduled_us: u64,
        second_scheduled_us: u64,
    },
    #[error("runtime simulation failed: {0}")]
    Simulation(String),
}

pub fn compile_runtime_intents(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
) -> Result<RuntimeSchedule, CompileError> {
    if allowed_scan_codes.is_empty() || allowed_scan_codes.len() > MAX_KEYS {
        return Err(CompileError::InvalidAllowedScanCodeCount);
    }
    for (index, &scan_code) in allowed_scan_codes.iter().enumerate() {
        if allowed_scan_codes[..index].contains(&scan_code) {
            return Err(CompileError::DuplicateAllowedScanCode(scan_code));
        }
    }
    if actions.len() > MAX_ACTIONS {
        return Err(CompileError::TooManyActions);
    }

    let key_registry = KeyRegistry::new(allowed_scan_codes);
    let mut next_generation_id: GenerationId = 0;

    #[derive(Clone, Copy, Debug)]
    struct OpenGeneration {
        generation_id: GenerationId,
        down_action_index: u32,
        down_scheduled_us: u64,
    }
    let mut open_generation_by_slot: [Option<OpenGeneration>; MAX_KEYS] = [None; MAX_KEYS];

    let mut reason_table: Vec<String> = Vec::new();
    let mut reason_map: HashMap<String, ReasonId> = HashMap::new();
    let intent_capacity = actions
        .iter()
        .map(|action| action.scan_codes.len())
        .sum::<usize>();
    let mut batches = Vec::with_capacity(actions.len());
    let mut intents = Vec::with_capacity(intent_capacity);
    let mut packets = Vec::with_capacity(actions.len());

    let mut get_or_insert_reason = |reason: &str| -> Result<ReasonId, CompileError> {
        if let Some(&id) = reason_map.get(reason) {
            Ok(id)
        } else {
            let id =
                ReasonId::try_from(reason_table.len()).map_err(|_| CompileError::TooManyReasons)?;
            reason_table.push(reason.into());
            reason_map.insert(reason.into(), id);
            Ok(id)
        }
    };

    let mut previous_source_index: Option<u32> = None;
    let mut previous_scheduled_us: Option<u64> = None;
    let mut group_start = 0usize;

    while group_start < actions.len() {
        let scheduled_us = actions[group_start].scheduled_us;
        let mut group_end = group_start + 1;
        while group_end < actions.len() && actions[group_end].scheduled_us == scheduled_us {
            group_end += 1;
        }

        let packet_id =
            PacketId::try_from(packets.len()).map_err(|_| CompileError::TooManyActions)?;
        let first_batch_index =
            u32::try_from(batches.len()).map_err(|_| CompileError::TooManyActions)?;
        let mut down_action: Option<usize> = None;
        for (offset, action) in actions[group_start..group_end].iter().enumerate() {
            if let Some(previous) = previous_source_index
                && action.source_action_index <= previous
            {
                return Err(CompileError::NonMonotonicSourceIndex {
                    previous,
                    current: action.source_action_index,
                });
            }
            if let Some(previous) = previous_scheduled_us
                && action.scheduled_us < previous
            {
                return Err(CompileError::NonMonotonicSchedule {
                    previous,
                    current: action.scheduled_us,
                });
            }
            if action.scan_codes.is_empty() || action.scan_codes.len() > MAX_KEYS {
                return Err(CompileError::InvalidBatchSize {
                    action_index: action.source_action_index,
                });
            }
            if action.reason.len() > MAX_REASON_BYTES {
                return Err(CompileError::ReasonTooLong {
                    action_index: action.source_action_index,
                });
            }
            let mut batch_seen_mask: u16 = 0;
            for &scan_code in &action.scan_codes {
                let Some(key_slot) = key_registry.slot_for(scan_code) else {
                    return Err(CompileError::ScanCodeNotAllowed {
                        action_index: action.source_action_index,
                        scan_code,
                    });
                };
                let bit = 1u16 << key_slot;
                if batch_seen_mask & bit != 0 {
                    return Err(CompileError::DuplicateBatchScanCode {
                        action_index: action.source_action_index,
                        scan_code,
                    });
                }
                batch_seen_mask |= bit;
            }
            get_or_insert_reason(&action.reason)?;
            if action.kind == ActionKind::Down {
                if down_action.is_some() {
                    return Err(CompileError::SameTimestampDownBatch { scheduled_us });
                }
                down_action = Some(group_start + offset);
            }
            previous_source_index = Some(action.source_action_index);
            previous_scheduled_us = Some(action.scheduled_us);
        }

        // Build all release intents first. This is the compiler's canonical
        // physical order even when authored actions arrived Down-before-Up.
        let up_intent_start =
            u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
        let mut up_mask = 0u16;
        let mut seen_up_mask = 0u16;
        let mut group_batches: Vec<CompiledBatch> = Vec::with_capacity(group_end - group_start);
        for action in &actions[group_start..group_end] {
            if action.kind != ActionKind::Up {
                continue;
            }
            let reason_id = get_or_insert_reason(&action.reason)?;
            let intent_start =
                u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
            let mut sorted_scan_codes = action.scan_codes.clone();
            sorted_scan_codes.sort_by_key(|&sc| key_registry.slot_for(sc).unwrap_or(0xFF));
            for &scan_code in &sorted_scan_codes {
                let key_slot = key_registry
                    .slot_for(scan_code)
                    .expect("allowlist validation must precede packet compilation");
                let bit = 1u16 << key_slot;
                if seen_up_mask & bit != 0 {
                    return Err(CompileError::DuplicateSameTimestampUp {
                        scan_code,
                        scheduled_us,
                    });
                }
                seen_up_mask |= bit;
                let generation_id =
                    open_generation_by_slot[key_slot as usize].map(|g| g.generation_id);
                open_generation_by_slot[key_slot as usize] = None;
                if generation_id.is_some() {
                    up_mask |= bit;
                }
                intents.push(CompactIntent::new(
                    generation_id.unwrap_or(NO_GENERATION_ID),
                    key_slot,
                ));
            }
            group_batches.push(CompiledBatch {
                source_action_index: action.source_action_index,
                kind: ActionKind::Up,
                scheduled_us,
                reason_id,
                intent_start,
                intent_len: u8::try_from(action.scan_codes.len())
                    .expect("validated batch length is at most MAX_KEYS"),
                packet_id,
            });
        }
        let up_intent_len = u8::try_from(
            intents.len() - usize::try_from(up_intent_start).expect("u32 range is usize-safe"),
        )
        .expect("one timestamp has at most fifteen release intents");

        let down_intent_start =
            u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
        let mut down_mask = 0u16;
        let mut down_source_action_index = None;
        if let Some(down_index) = down_action {
            let action = &actions[down_index];
            let reason_id = get_or_insert_reason(&action.reason)?;
            let intent_start =
                u32::try_from(intents.len()).map_err(|_| CompileError::TooManyActions)?;
            let mut sorted_scan_codes = action.scan_codes.clone();
            sorted_scan_codes.sort_by_key(|&sc| key_registry.slot_for(sc).unwrap_or(0xFF));
            for &scan_code in &sorted_scan_codes {
                let key_slot = key_registry
                    .slot_for(scan_code)
                    .expect("allowlist validation must precede packet compilation");
                let bit = 1u16 << key_slot;
                if let Some(open) = open_generation_by_slot[key_slot as usize] {
                    return Err(CompileError::OverlappingSameKeyDown {
                        scan_code,
                        first_down_action_index: open.down_action_index,
                        second_down_action_index: action.source_action_index,
                        first_scheduled_us: open.down_scheduled_us,
                        second_scheduled_us: scheduled_us,
                    });
                }
                if next_generation_id > MAX_COMPACT_GENERATION_ID {
                    return Err(CompileError::GenerationOverflow);
                }
                let generation_id = next_generation_id;
                next_generation_id = next_generation_id
                    .checked_add(1)
                    .ok_or(CompileError::GenerationOverflow)?;
                open_generation_by_slot[key_slot as usize] = Some(OpenGeneration {
                    generation_id,
                    down_action_index: action.source_action_index,
                    down_scheduled_us: scheduled_us,
                });
                down_mask |= bit;
                intents.push(CompactIntent::new(generation_id, key_slot));
            }
            group_batches.push(CompiledBatch {
                source_action_index: action.source_action_index,
                kind: ActionKind::Down,
                scheduled_us,
                reason_id,
                intent_start,
                intent_len: u8::try_from(action.scan_codes.len())
                    .expect("validated batch length is at most MAX_KEYS"),
                packet_id,
            });
            down_source_action_index = Some(action.source_action_index);
        }
        group_batches.sort_unstable_by_key(|batch| batch.source_action_index);
        batches.extend(group_batches);
        let batch_count = u16::try_from(group_end - group_start)
            .map_err(|_| CompileError::TooManyBatchesAtTimestamp { scheduled_us })?;
        packets.push(CompiledPacket {
            packet_id,
            scheduled_us,
            first_batch_index,
            batch_count,
            up_mask,
            down_mask,
            up_intent_start,
            up_intent_len,
            down_intent_start,
            down_intent_len: u8::try_from(
                intents.len()
                    - usize::try_from(down_intent_start).expect("u32 range is usize-safe"),
            )
            .expect("one timestamp has at most fifteen activation intents"),
            down_source_action_index,
        });
        group_start = group_end;
    }

    Ok(RuntimeSchedule {
        packets,
        batches,
        intents,
        generation_count: next_generation_id,
        key_registry,
        reason_table,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::time::TimelineTicks;

    #[test]
    fn test_compile_basic_pairing() {
        let allowed = vec![1, 2, 3];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: smallvec::smallvec![1, 2],
                reason: "chord".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 2000,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 2100,
                scan_codes: smallvec::smallvec![2],
                reason: "release".into(),
            },
        ];

        let sched = compile_runtime_intents(&actions, &allowed).unwrap();
        assert_eq!(sched.generation_count, 2);
        assert_eq!(sched.batches.len(), 3);

        // Down batch has gen 0 and gen 1
        let down = sched.materialize_batch(0, 0);
        assert_eq!(down.intents[0].generation_id, Some(0));
        assert_eq!(down.intents[1].generation_id, Some(1));

        // Up 1 matches gen 0
        let up_one = sched.materialize_batch(1, 0);
        assert_eq!(up_one.intents[0].generation_id, Some(0));
        // Up 2 matches gen 1
        let up_two = sched.materialize_batch(2, 0);
        assert_eq!(up_two.intents[0].generation_id, Some(1));
    }

    #[test]
    fn test_unmatched_up_suppressed() {
        let allowed = vec![1];
        let actions = vec![KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Up,
            scheduled_us: 1000,
            scan_codes: smallvec::smallvec![1],
            reason: "stale".into(),
        }];

        let sched = compile_runtime_intents(&actions, &allowed).unwrap();
        assert_eq!(sched.generation_count, 0);
        assert_eq!(sched.materialize_batch(0, 0).intents[0].generation_id, None);
    }

    #[test]
    fn multiple_same_timestamp_down_batches_are_rejected_as_non_atomic() {
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "left".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![2],
                reason: "right".into(),
            },
        ];
        assert!(matches!(
            compile_runtime_intents(&actions, &[1, 2]),
            Err(CompileError::SameTimestampDownBatch { scheduled_us: 100 })
        ));
    }

    #[test]
    fn mixed_timestamp_packet_canonicalizes_up_before_down() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 100,
                    scan_codes: smallvec::smallvec![1],
                    reason: "first".into(),
                },
                // Authored order is deliberately Down-before-Up at this
                // timestamp. The packet must still release generation 0
                // before activating generation 1.
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 200,
                    scan_codes: smallvec::smallvec![1, 2],
                    reason: "retrigger".into(),
                },
                KeyActionInput {
                    source_action_index: 2,
                    kind: ActionKind::Up,
                    scheduled_us: 200,
                    scan_codes: smallvec::smallvec![1],
                    reason: "release".into(),
                },
            ],
            &[1, 2],
        )
        .unwrap();

        assert_eq!(schedule.packets.len(), 2);
        assert_eq!(schedule.packets[0].packet_id, 0);
        assert_eq!(schedule.packets[1].packet_id, 1);
        let packet = schedule.view_packet_ticks(1, TimelineTicks::ZERO).unwrap();
        assert_eq!(packet.up_mask(), 0b01);
        assert_eq!(packet.down_mask(), 0b11);
        assert_eq!(packet.header.down_source_action_index, Some(1));
        assert_eq!(packet.up_intents[0].generation_id(), 0);
        assert_eq!(packet.down_intents[0].generation_id(), 1);
        assert_eq!(packet.down_intents[1].generation_id(), 2);
        assert_eq!(schedule.batches[1].kind, ActionKind::Down);
        assert_eq!(schedule.batches[2].kind, ActionKind::Up);
    }

    #[test]
    fn duplicate_same_timestamp_up_is_rejected() {
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "stale one".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "stale two".into(),
            },
        ];
        assert!(matches!(
            compile_runtime_intents(&actions, &[1]),
            Err(CompileError::DuplicateSameTimestampUp {
                scan_code: 1,
                scheduled_us: 100
            })
        ));
    }

    #[test]
    fn stale_up_is_not_included_in_packet_physical_mask() {
        let schedule = compile_runtime_intents(
            &[KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 100,
                scan_codes: smallvec::smallvec![1],
                reason: "stale".into(),
            }],
            &[1],
        )
        .unwrap();

        let packet = schedule.view_packet_ticks(0, TimelineTicks::ZERO).unwrap();
        assert_eq!(packet.up_mask(), 0);
        assert_eq!(packet.up_intents.len(), 1);
        assert_eq!(packet.up_intents[0].generation_id(), NO_GENERATION_ID);
    }

    #[test]
    fn test_rejects_non_monotonic_and_untrusted_actions() {
        let allowed = vec![1];
        let invalid = vec![
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 2,
                scan_codes: smallvec::smallvec![1],
                reason: "first".into(),
            },
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 1,
                scan_codes: smallvec::smallvec![1],
                reason: "second".into(),
            },
        ];
        assert!(matches!(
            compile_runtime_intents(&invalid, &allowed),
            Err(CompileError::NonMonotonicSourceIndex { .. })
        ));

        let outside_allowlist = vec![KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Down,
            scheduled_us: 0,
            scan_codes: smallvec::smallvec![2],
            reason: "invalid".into(),
        }];
        assert!(matches!(
            compile_runtime_intents(&outside_allowlist, &allowed),
            Err(CompileError::ScanCodeNotAllowed { scan_code: 2, .. })
        ));
    }

    #[test]
    fn schedule_uses_a_flat_intent_arena() {
        let schedule = compile_runtime_intents(
            &[
                KeyActionInput {
                    source_action_index: 0,
                    kind: ActionKind::Down,
                    scheduled_us: 10,
                    scan_codes: smallvec::smallvec![1],
                    reason: "single".into(),
                },
                KeyActionInput {
                    source_action_index: 1,
                    kind: ActionKind::Down,
                    scheduled_us: 20,
                    scan_codes: smallvec::smallvec![2, 3],
                    reason: "chord".into(),
                },
            ],
            &[1, 2, 3],
        )
        .unwrap();
        assert_eq!(schedule.intents.len(), 3);
        assert_eq!(schedule.batches[0].intent_len, 1);
        assert_eq!(schedule.batches[1].intent_len, 2);
        assert!(std::mem::size_of::<CompiledBatch>() <= 32);
        assert_eq!(std::mem::size_of::<CompactIntent>(), 8);
    }

    #[test]
    fn test_reject_overlapping_same_key_down() {
        let allowed = vec![1];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: smallvec::smallvec![1],
                reason: "first down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 2000,
                scan_codes: smallvec::smallvec![1],
                reason: "overlapping down".into(),
            },
        ];
        let err = compile_runtime_intents(&actions, &allowed).unwrap_err();
        assert!(matches!(
            err,
            CompileError::OverlappingSameKeyDown {
                scan_code: 1,
                first_down_action_index: 0,
                second_down_action_index: 1,
                ..
            }
        ));
    }

    #[test]
    fn test_allow_down_down_different_keys() {
        let allowed = vec![1, 2];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: smallvec::smallvec![1],
                reason: "down 1".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 2000,
                scan_codes: smallvec::smallvec![2],
                reason: "down 2".into(),
            },
        ];
        let sched = compile_runtime_intents(&actions, &allowed).unwrap();
        assert_eq!(sched.generation_count, 2);
    }

    #[test]
    fn test_reject_chord_overlapping_active_key() {
        let allowed = vec![1, 2, 3];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: smallvec::smallvec![1, 2],
                reason: "chord 1".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1500,
                scan_codes: smallvec::smallvec![1],
                reason: "release 1".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 2000,
                scan_codes: smallvec::smallvec![2, 3],
                reason: "chord 2".into(),
            },
        ];
        let err = compile_runtime_intents(&actions, &allowed).unwrap_err();
        assert!(matches!(
            err,
            CompileError::OverlappingSameKeyDown { scan_code: 2, .. }
        ));
    }

    #[test]
    fn test_reused_key_after_up_allowed() {
        let allowed = vec![1];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: smallvec::smallvec![1],
                reason: "first down".into(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 1500,
                scan_codes: smallvec::smallvec![1],
                reason: "release".into(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Down,
                scheduled_us: 2000,
                scan_codes: smallvec::smallvec![1],
                reason: "second down".into(),
            },
        ];
        let sched = compile_runtime_intents(&actions, &allowed).unwrap();
        assert_eq!(sched.generation_count, 2);
    }
}
