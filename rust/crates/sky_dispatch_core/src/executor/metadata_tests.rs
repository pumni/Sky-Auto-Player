use crate::compile::compile_runtime_intents;
use crate::model::{ActionKind, KeyActionInput};
use crate::time::TimelineTicks;

#[test]
fn unmatched_up_metadata_remains_reachable_across_scan_boundaries() {
    for count in [32usize, 33, 10_000] {
        let actions: Vec<_> = (0..count)
            .map(|index| KeyActionInput {
                source_action_index: index as u32,
                kind: ActionKind::Up,
                scheduled_us: index as u64 + 1,
                scan_codes: smallvec::smallvec![0x15],
                reason: "unmatched up".into(),
            })
            .collect();
        let schedule = compile_runtime_intents(&actions, &[0x15]).expect("valid schedule");
        let mut coordinator = crate::coordinator::RuntimeDispatchCoordinator::new(
            schedule,
            0,
            0,
            TimelineTicks::from_raw,
        );
        for index in 0..count {
            let before = coordinator.cursor;
            let prepared = coordinator
                .prepare_current_stale_packet_at(TimelineTicks::from_raw(count as u64 + 1))
                .expect("stale preparation must not swallow errors")
                .expect("unmatched Up must remain reachable");
            assert_eq!(
                coordinator.cursor, before,
                "preparation must not commit metadata {index}"
            );
            coordinator
                .commit_stale_packet(prepared)
                .expect("stale commit");
            assert_eq!(coordinator.cursor, before + 1);
        }
        assert!(
            coordinator
                .prepare_current_stale_packet_at(TimelineTicks::from_raw(count as u64 + 1))
                .expect("finished scan")
                .is_none()
        );
    }
}
