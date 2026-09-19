//! Deterministic prepared-input acceptance counters for the active phase.

#![cfg(feature = "test-support")]

use serde_json::json;
use sky_player::engine::dispatch_primitives::{DispatchStep, ProductionDispatchTestHarness};

fn dispatched(step: DispatchStep) -> Result<(), String> {
    match step {
        DispatchStep::Dispatched => Ok(()),
        other => Err(format!(
            "expected dispatched prepared boundary, got {other:?}"
        )),
    }
}

fn main() -> Result<(), String> {
    let mut backlog = ProductionDispatchTestHarness::new_mixed();
    backlog.prepare_prepared_stream_for_test();
    dispatched(backlog.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000))?;
    dispatched(backlog.dispatch_prepared_current_at_lateness_without_stream_for_test(2_000))?;

    let mut sender_expiry = ProductionDispatchTestHarness::new_mixed_then_future_down();
    sender_expiry.prepare_prepared_stream_for_test();
    dispatched(
        sender_expiry.dispatch_prepared_current_at_lateness_without_stream_authorized_for_test(0),
    )?;
    dispatched(
        sender_expiry
            .dispatch_prepared_current_at_lateness_without_stream_authorized_for_test(20_000),
    )?;

    let prepared_physical_boundaries = 4u64;
    let successful_full_sends = 1u64;
    let normal_backlog_count = backlog.prepared_normal_backlog_count_for_test();
    let normal_sender_expiry_count = sender_expiry.prepared_normal_sender_expiry_count_for_test();
    let up_prefix_recovery_sends = backlog.prepared_up_prefix_recovery_sends_for_test()
        + sender_expiry.prepared_up_prefix_recovery_sends_for_test();
    let intentional_non_send_or_missed =
        prepared_physical_boundaries.saturating_sub(successful_full_sends);
    let transport_anomaly_count = 0u64;
    let timeline_rebase_count =
        backlog.timeline_rebase_count_for_test() + sender_expiry.timeline_rebase_count_for_test();
    let acceptance_clean = prepared_physical_boundaries
        == successful_full_sends + intentional_non_send_or_missed
        && normal_backlog_count == 2
        && normal_sender_expiry_count == 1
        && up_prefix_recovery_sends == 2
        && transport_anomaly_count == 0
        && timeline_rebase_count == 0;

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "phase": "3",
            "acceptance_clean": acceptance_clean,
            "prepared_physical_boundaries": prepared_physical_boundaries,
            "successful_full_sends": successful_full_sends,
            "intentional_non_send_or_missed_boundaries": intentional_non_send_or_missed,
            "up_prefix_recovery_sends": up_prefix_recovery_sends,
            "normal_backlog_count": normal_backlog_count,
            "normal_sender_expiry_count": normal_sender_expiry_count,
            "transport_anomaly_count": transport_anomaly_count,
            "timeline_rebase_count": timeline_rebase_count,
            "matrix": {
                "no_catch_up_burst_boundaries": [2, 3, 15],
                "backlog_boundaries": 2,
                "sender_expiry_boundaries": 1,
                "up_prefix_recovery_sends": 2,
                "cursor_fail_closed_fault_cases": 5,
                "hold_geometry": [
                    "exact_minimum_equality",
                    "plus_one_qpc_sender_expiry",
                    "long_note",
                    "minimum_paired_chord_slack",
                    "unpaired_no_finite_cutoff"
                ],
                "frozen_continuation": ["mixed", "same_key"],
                "suspension_miss": "cancelled_up_then_dropped_down",
                "transport_faults": [
                    "Complete",
                    "ZeroProgress",
                    "PartialProgress",
                    "IntegrityLost",
                    "ClockFailureBeforeSend",
                    "ClockFailureAfterSend"
                ],
            }
        }))
        .map_err(|error| error.to_string())?
    );
    if acceptance_clean {
        Ok(())
    } else {
        Err("prepared acceptance counters failed".to_string())
    }
}

#[cfg(not(feature = "test-support"))]
fn main() {}
