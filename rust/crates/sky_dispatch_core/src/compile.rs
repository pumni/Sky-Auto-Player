//! Generation compiler for turning authored KeyAction sequences into RuntimeSchedule.

use smallvec::SmallVec;
use std::collections::{HashMap, VecDeque};
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
}

pub fn compile_runtime_intents(
    actions: &[KeyActionInput],
    allowed_scan_codes: &[u16],
) -> Result<RuntimeSchedule, CompileError> {
    if allowed_scan_codes.is_empty() || allowed_scan_codes.len() > MAX_KEYS {
        return Err(CompileError::InvalidAllowedScanCodeCount);
    }
    let mut allowed_seen = std::collections::HashSet::with_capacity(allowed_scan_codes.len());
    for &scan_code in allowed_scan_codes {
        if !allowed_seen.insert(scan_code) {
            return Err(CompileError::DuplicateAllowedScanCode(scan_code));
        }
    }
    if actions.len() > MAX_ACTIONS {
        return Err(CompileError::TooManyActions);
    }

    let key_registry = KeyRegistry::new(allowed_scan_codes);
    let mut next_generation_id: GenerationId = 0;
    let mut unmatched_downs: HashMap<u16, VecDeque<GenerationId>> = HashMap::new();
    let mut reason_table: Vec<String> = Vec::new();
    let mut reason_map: HashMap<String, ReasonId> = HashMap::new();
    let mut batches: Vec<RuntimeBatch> = Vec::with_capacity(actions.len());

    let mut get_or_insert_reason = |reason: &str| -> Result<ReasonId, CompileError> {
        if let Some(&id) = reason_map.get(reason) {
            Ok(id)
        } else {
            let id =
                ReasonId::try_from(reason_table.len()).map_err(|_| CompileError::TooManyReasons)?;
            reason_table.push(reason.to_string());
            reason_map.insert(reason.to_string(), id);
            Ok(id)
        }
    };

    let mut previous_source_index: Option<u32> = None;
    let mut previous_scheduled_us: Option<u64> = None;

    for action in actions {
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

        let mut batch_seen = std::collections::HashSet::with_capacity(action.scan_codes.len());
        for &scan_code in &action.scan_codes {
            if !allowed_seen.contains(&scan_code) {
                return Err(CompileError::ScanCodeNotAllowed {
                    action_index: action.source_action_index,
                    scan_code,
                });
            }
            if !batch_seen.insert(scan_code) {
                return Err(CompileError::DuplicateBatchScanCode {
                    action_index: action.source_action_index,
                    scan_code,
                });
            }
        }

        previous_source_index = Some(action.source_action_index);
        previous_scheduled_us = Some(action.scheduled_us);
        let reason_id = get_or_insert_reason(&action.reason)?;
        let mut intents: SmallVec<[RuntimeKeyIntent; MAX_KEYS]> = SmallVec::new();

        for &scan_code in &action.scan_codes {
            let key_slot =
                key_registry
                    .slot_for(scan_code)
                    .ok_or(CompileError::ScanCodeNotAllowed {
                        action_index: action.source_action_index,
                        scan_code,
                    })?;
            let generation_id = match action.kind {
                ActionKind::Down => {
                    let gen_id = next_generation_id;
                    next_generation_id = next_generation_id
                        .checked_add(1)
                        .ok_or(CompileError::GenerationOverflow)?;
                    unmatched_downs
                        .entry(scan_code)
                        .or_default()
                        .push_back(gen_id);
                    Some(gen_id)
                }
                ActionKind::Up => unmatched_downs
                    .get_mut(&scan_code)
                    .and_then(|queue| queue.pop_front()),
            };

            intents.push(RuntimeKeyIntent {
                source_action_index: action.source_action_index,
                generation_id,
                kind: action.kind,
                scan_code,
                key_slot,
                scheduled_us: action.scheduled_us,
                reason_id,
            });
        }

        batches.push(RuntimeBatch {
            source_action_index: action.source_action_index,
            kind: action.kind,
            scheduled_us: action.scheduled_us,
            reason_id,
            intents,
            packet_id: 0,
        });
    }

    Ok(RuntimeSchedule {
        batches,
        generation_count: next_generation_id,
        key_registry,
        reason_table,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_basic_pairing() {
        let allowed = vec![1, 2, 3];
        let actions = vec![
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Down,
                scheduled_us: 1000,
                scan_codes: vec![1, 2],
                reason: "chord".to_string(),
            },
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Up,
                scheduled_us: 2000,
                scan_codes: vec![1],
                reason: "release".to_string(),
            },
            KeyActionInput {
                source_action_index: 2,
                kind: ActionKind::Up,
                scheduled_us: 2100,
                scan_codes: vec![2],
                reason: "release".to_string(),
            },
        ];

        let sched = compile_runtime_intents(&actions, &allowed).unwrap();
        assert_eq!(sched.generation_count, 2);
        assert_eq!(sched.batches.len(), 3);

        // Down batch has gen 0 and gen 1
        assert_eq!(sched.batches[0].intents[0].generation_id, Some(0));
        assert_eq!(sched.batches[0].intents[1].generation_id, Some(1));

        // Up 1 matches gen 0
        assert_eq!(sched.batches[1].intents[0].generation_id, Some(0));
        // Up 2 matches gen 1
        assert_eq!(sched.batches[2].intents[0].generation_id, Some(1));
    }

    #[test]
    fn test_unmatched_up_suppressed() {
        let allowed = vec![1];
        let actions = vec![KeyActionInput {
            source_action_index: 0,
            kind: ActionKind::Up,
            scheduled_us: 1000,
            scan_codes: vec![1],
            reason: "stale".to_string(),
        }];

        let sched = compile_runtime_intents(&actions, &allowed).unwrap();
        assert_eq!(sched.generation_count, 0);
        assert_eq!(sched.batches[0].intents[0].generation_id, None);
    }

    #[test]
    fn test_rejects_non_monotonic_and_untrusted_actions() {
        let allowed = vec![1];
        let invalid = vec![
            KeyActionInput {
                source_action_index: 1,
                kind: ActionKind::Down,
                scheduled_us: 2,
                scan_codes: vec![1],
                reason: "first".to_string(),
            },
            KeyActionInput {
                source_action_index: 0,
                kind: ActionKind::Up,
                scheduled_us: 1,
                scan_codes: vec![1],
                reason: "second".to_string(),
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
            scan_codes: vec![2],
            reason: "invalid".to_string(),
        }];
        assert!(matches!(
            compile_runtime_intents(&outside_allowlist, &allowed),
            Err(CompileError::ScanCodeNotAllowed { scan_code: 2, .. })
        ));
    }
}
