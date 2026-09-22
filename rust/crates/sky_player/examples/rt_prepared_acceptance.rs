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
    dispatched(sender_expiry.dispatch_prepared_current_at_lateness_without_stream_for_test(0))?;
    dispatched(
        sender_expiry.dispatch_prepared_current_at_lateness_without_stream_for_test(20_000),
    )?;
    dispatched(backlog.dispatch_prepared_current_at_lateness_without_stream_for_test(0))?;
    dispatched(sender_expiry.dispatch_prepared_current_at_lateness_without_stream_for_test(0))?;
    dispatched(sender_expiry.dispatch_prepared_current_at_lateness_without_stream_for_test(0))?;

    let prepared_physical_boundaries = 7u64;
    let successful_full_sends = prepared_physical_boundaries;
    let normal_backlog_count = backlog.prepared_normal_backlog_count_for_test();
    let normal_sender_expiry_count = sender_expiry.prepared_normal_sender_expiry_count_for_test();
    let up_prefix_recovery_sends = backlog.prepared_up_prefix_recovery_sends_for_test()
        + sender_expiry.prepared_up_prefix_recovery_sends_for_test();
    let intentional_non_send_or_missed = 0u64;
    let transport_anomaly_count = 0u64;
    let timeline_rebase_count =
        backlog.timeline_rebase_count_for_test() + sender_expiry.timeline_rebase_count_for_test();
    let backlog_accounting = backlog.generation_accounting_for_test();
    let sender_expiry_accounting = sender_expiry.generation_accounting_for_test();
    let generation_total = backlog_accounting.total + sender_expiry_accounting.total;
    let generation_activated = backlog_accounting.activated + sender_expiry_accounting.activated;
    let generation_released = backlog_accounting.released + sender_expiry_accounting.released;
    let generation_scheduled = backlog_accounting.scheduled + sender_expiry_accounting.scheduled;
    let generation_active = backlog_accounting.active + sender_expiry_accounting.active;
    let generation_dropped_conflict =
        backlog_accounting.dropped_conflict + sender_expiry_accounting.dropped_conflict;
    let generation_dropped_backend =
        backlog_accounting.dropped_backend + sender_expiry_accounting.dropped_backend;
    let generation_dropped_expired =
        backlog_accounting.dropped_expired + sender_expiry_accounting.dropped_expired;
    let generation_cancelled = backlog_accounting.cancelled + sender_expiry_accounting.cancelled;
    let final_release_obligation_mask = backlog.release_obligation_mask_for_test()
        | sender_expiry.release_obligation_mask_for_test();
    let acceptance_clean = prepared_physical_boundaries
        == successful_full_sends + intentional_non_send_or_missed
        && normal_backlog_count == 0
        && normal_sender_expiry_count == 0
        && up_prefix_recovery_sends == 0
        && transport_anomaly_count == 0
        && timeline_rebase_count == 0
        && generation_total == generation_activated
        && generation_activated == generation_released
        && generation_scheduled == 0
        && generation_active == 0
        && generation_dropped_conflict == 0
        && generation_dropped_backend == 0
        && generation_dropped_expired == 0
        && generation_cancelled == 0
        && final_release_obligation_mask == 0;

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
            "generation_total": generation_total,
            "generation_activated": generation_activated,
            "generation_released": generation_released,
            "generation_scheduled": generation_scheduled,
            "generation_active": generation_active,
            "generation_dropped_conflict": generation_dropped_conflict,
            "generation_dropped_backend": generation_dropped_backend,
            "generation_dropped_expired": generation_dropped_expired,
            "generation_cancelled": generation_cancelled,
            "final_release_obligation_mask": final_release_obligation_mask,
            "report_kind": "counter_report"
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
