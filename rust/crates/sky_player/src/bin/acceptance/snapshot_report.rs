use serde_json::{Value, json};
use sky_player::engine::EngineSnapshot;
use sky_dispatch_win32::input::FULL_INSTRUMENT_MASK;
use super::{EventRecord, PhysicalExpectation, Verdict};

pub(super) fn cleanup_evidence_clean(full_mask_required: bool, attempted_mask: u16, attempts: u8, released: bool, stuck_mask: u16, verification_inconclusive: bool, transport_anomaly: bool) -> bool {
    released && stuck_mask == 0 && !verification_inconclusive && !transport_anomaly && (!full_mask_required || (attempted_mask == FULL_INSTRUMENT_MASK && attempts >= 1))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct NativeCleanupEvidence { pub(super) terminal_error: bool, pub(super) partial_events: u64, pub(super) zero_progress_events: u64, pub(super) active_count: usize, pub(super) possibly_active_count: usize, pub(super) failed_release_count: usize, pub(super) release_obligation_mask: u16, pub(super) release_failed: bool, pub(super) stuck_mask: u16, pub(super) verification_inconclusive: bool, pub(super) transport_anomaly: bool }
impl NativeCleanupEvidence { pub(super) fn is_anomalous(self) -> bool { self.terminal_error || self.partial_events != 0 || self.zero_progress_events != 0 || self.active_count != 0 || self.possibly_active_count != 0 || self.failed_release_count != 0 || self.release_obligation_mask != 0 || self.release_failed || self.stuck_mask != 0 || self.verification_inconclusive || self.transport_anomaly } }
pub(super) fn preterminal_verdict(observer: Verdict, cleanup_anomaly: bool) -> Verdict { if cleanup_anomaly { Verdict::Fail } else { observer } }

pub(super) fn focus_evidence_clean(paused: bool, final_gate_focus_losses: u64, target_changes: u64, sink_events_clean: bool, probe_events_empty: bool) -> bool {
    let focus_transition_observed = paused || final_gate_focus_losses >= 1;
    focus_transition_observed && target_changes == 0 && sink_events_clean && probe_events_empty
}

pub(super) fn target_change_preflight_error(snapshot: &EngineSnapshot) -> bool { snapshot.outcome.as_deref() == Some("error") && snapshot.terminal_error.as_deref().is_some_and(|error| error.contains("instrument key preflight failed")) }
fn empty_successful_cleanup(snapshot: &EngineSnapshot) -> bool {
    let outcome = snapshot.release_outcome.as_ref();
    snapshot.final_release_obligation_mask == 0 && snapshot.failed_release_count == 0 && outcome.is_some_and(|outcome| outcome.attempted_mask == 0 && outcome.attempts == 0 && outcome.released_successfully && outcome.stuck_mask == 0 && !outcome.verification_inconclusive && !outcome.transport_anomaly)
}
pub(super) fn target_change_cleanup_exception(snapshot: &EngineSnapshot) -> bool { target_change_preflight_error(snapshot) && snapshot.keys_inserted_before_failure == 0 && snapshot.active_count == 0 && snapshot.possibly_active_count == 0 && snapshot.sendinput_partial_events == 0 && snapshot.sendinput_zero_progress_failures == 0 && empty_successful_cleanup(snapshot) }
pub(super) fn preflight_user_held_cleanup_exception(snapshot: &EngineSnapshot) -> bool { snapshot.outcome.as_deref() == Some("finished") && snapshot.last_error.as_deref().is_some_and(|error| error.contains("instrument key preflight failed during preroll") && error.contains("physically held before playback")) && snapshot.keys_inserted_before_failure == 0 && snapshot.active_count == 0 && snapshot.possibly_active_count == 0 && snapshot.failed_release_count == 0 && snapshot.final_release_obligation_mask == 0 && snapshot.sendinput_partial_events == 0 && snapshot.sendinput_zero_progress_failures == 0 && snapshot.release_outcome.as_ref().is_none_or(|outcome| outcome.attempted_mask == 0 && outcome.attempts == 0 && outcome.released_successfully && outcome.stuck_mask == 0 && !outcome.verification_inconclusive && !outcome.transport_anomaly) }
pub(super) fn supervisor_expiry_cleanup_exception(snapshot: &EngineSnapshot) -> bool { snapshot.terminal_error.as_deref() == Some("supervisor_lease_expired") && snapshot.active_count == 0 && snapshot.possibly_active_count == 0 && snapshot.sendinput_partial_events == 0 && snapshot.sendinput_zero_progress_failures == 0 && snapshot.timeline_rebase_count == 0 && empty_successful_cleanup(snapshot) }

pub(super) fn ambiguous_packet_event_sequence_matches(events: &[EventRecord], first: PhysicalExpectation, second: PhysicalExpectation) -> bool {
    let expected = [("key_press", first), ("key_release", first), ("key_press", second), ("key_release", first), ("key_release", second)];
    super::reconcile_event_sequence(events, &expected).is_ok()
}

pub(super) fn preflight_user_held_result(snapshot: &EngineSnapshot, events: &[EventRecord]) -> (Verdict, &'static str) {
    if preflight_user_held_cleanup_exception(snapshot) && events.is_empty() {
        (Verdict::Pass, "pre-arm UserHeld preflight rejected the session before any key event or cleanup transport")
    } else {
        (Verdict::Fail, "pre-arm UserHeld preflight did not fail closed with zero keyboard transport events")
    }
}

pub(super) fn ambiguous_packet_cleanup_qualified(snapshot: &EngineSnapshot, mask: u16) -> bool {
    snapshot.outcome.as_deref() == Some("error")
        && snapshot.terminal_error.is_some()
        && snapshot.active_count == 0
        && snapshot.possibly_active_count == 0
        && snapshot.failed_release_count == 0
        && snapshot.final_release_obligation_mask == 0
        && snapshot.sendinput_partial_events == 1
        && snapshot.sendinput_zero_progress_failures == 0
        && snapshot.release_outcome.as_ref().is_some_and(|outcome| {
            outcome.attempted_mask == mask
                && outcome.attempts >= 1
                && outcome.released_successfully
                && outcome.stuck_mask == 0
                && !outcome.verification_inconclusive
                && !outcome.transport_anomaly
        })
}

pub(super) fn snapshot_json(snapshot: &EngineSnapshot) -> Value {
    let accounting = snapshot.generation_accounting;
    let stuck_keys = snapshot
        .release_outcome
        .as_ref()
        .map_or(0, |outcome| u64::from(outcome.stuck_mask.count_ones()));
    let release = snapshot.release_outcome.as_ref().map(|outcome| {
        json!({
            "attempted_mask": outcome.attempted_mask,
            "transport_anomaly": outcome.transport_anomaly,
            "released_successfully": outcome.released_successfully,
            "stuck_mask": outcome.stuck_mask,
            "verification_inconclusive": outcome.verification_inconclusive,
            "attempts": outcome.attempts,
        })
    });

    json!({
        "status": snapshot.status,
        "outcome": snapshot.outcome,
        "last_error": snapshot.last_error,
        "active_count": snapshot.active_count,
        "possibly_active_count": snapshot.possibly_active_count,
        "failed_release_count": snapshot.failed_release_count,
        "keys_inserted_before_failure": snapshot.keys_inserted_before_failure,
        "stuck_keys": stuck_keys,
        "terminal_error": snapshot.terminal_error,
        "keys_dropped": snapshot.keys_dropped,
        "chord_split_events": snapshot.chord_split_events,
        "sendinput_partial_events": snapshot.sendinput_partial_events,
        "sendinput_zero_progress_failures": snapshot.sendinput_zero_progress_failures,
        "chord_integrity_lost": snapshot.chord_integrity_lost,
        "max_sendinput_pre_call_lateness_us": snapshot.max_sendinput_pre_call_lateness_us,
        "wait_planned_gap_max_us": snapshot.wait_planned_gap_max_us,
        "wait_planned_gap_hot_count": snapshot.wait_planned_gap_hot_count,
        "wait_planned_gap_cold_count": snapshot.wait_planned_gap_cold_count,
        "wait_planned_gap_hot_lateness_max_us": snapshot.wait_planned_gap_hot_lateness_max_us,
        "wait_planned_gap_cold_lateness_max_us": snapshot.wait_planned_gap_cold_lateness_max_us,
        "wait_interrupted_count": snapshot.wait_interrupted_count,
        "timeline_rebase_count": snapshot.timeline_rebase_count,
        "timeline_rebase_total_us": snapshot.timeline_rebase_total_us,
        "timeline_rebase_max_us": snapshot.timeline_rebase_max_us,
        "physical_target_to_wake_max_us": snapshot.physical_target_to_wake_max_us,
        "wake_to_final_policy_max_us": snapshot.wake_to_final_policy_max_us,
        "final_policy_to_pre_call_max_us": snapshot.final_policy_to_pre_call_max_us,
        "wake_to_send_max_us": snapshot.wake_to_send_max_us,
        "sendinput_duration_max_us": snapshot.sendinput_duration_max_us,
        "generation_total": accounting.total,
        "generation_activated": accounting.activated,
        "generation_released": accounting.released,
        "generation_scheduled": accounting.scheduled,
        "generation_active": accounting.active,
        "generation_dropped_conflict": accounting.dropped_conflict,
        "generation_dropped_backend": accounting.dropped_backend,
        "generation_dropped_expired": accounting.dropped_expired,
        "generation_cancelled": accounting.cancelled,
        "final_release_obligation_mask": snapshot.final_release_obligation_mask,
        "pre_call_lt_250us": snapshot.pre_call_lt_250us,
        "pre_call_250_500us": snapshot.pre_call_250_500us,
        "pre_call_500_750us": snapshot.pre_call_500_750us,
        "pre_call_750_1000us": snapshot.pre_call_750_1000us,
        "pre_call_1000_1500us": snapshot.pre_call_1000_1500us,
        "pre_call_1500_2000us": snapshot.pre_call_1500_2000us,
        "pre_call_ge_2000us": snapshot.pre_call_ge_2000us,
        "missed_down_boundaries": snapshot.missed_down_boundaries,
        "missed_down_keys": snapshot.missed_down_keys,
        "missed_unobserved_backlog_boundaries": snapshot.missed_unobserved_backlog_boundaries,
        "missed_physical_window_boundaries": snapshot.missed_physical_window_boundaries,
        "final_sender_window_expirations": snapshot.final_sender_window_expirations,
        "release_floor_infeasible_boundaries": snapshot.release_floor_infeasible_boundaries,
        "hold_floor_delay_boundaries": snapshot.hold_floor_delay_boundaries,
        "max_hold_floor_delay_us": snapshot.max_hold_floor_delay_us,
        "last_hold_floor_delay_mask": snapshot.last_hold_floor_delay_mask,
        "last_hold_floor_authored_target_qpc_ticks": snapshot.last_hold_floor_authored_target_qpc_ticks,
        "last_hold_floor_not_before_qpc_ticks": snapshot.last_hold_floor_not_before_qpc_ticks,
        "release_floor_delay_boundaries": snapshot.release_floor_delay_boundaries,
        "max_release_floor_delay_us": snapshot.max_release_floor_delay_us,
        "last_release_floor_delay_mask": snapshot.last_release_floor_delay_mask,
        "last_release_floor_authored_target_qpc_ticks": snapshot.last_release_floor_authored_target_qpc_ticks,
        "last_release_floor_not_before_qpc_ticks": snapshot.last_release_floor_not_before_qpc_ticks,
        "final_gate_control_rejections": snapshot.final_gate_control_rejections,
        "final_gate_focus_losses": snapshot.final_gate_focus_losses,
        "final_gate_target_changes": snapshot.final_gate_target_changes,
        "final_gate_lease_expirations": snapshot.final_gate_lease_expirations,
        "production_forensics_available": snapshot.production_forensics_available,
        "production_forensics_version": snapshot.production_forensics_version,
        "production_hold_pair_samples": snapshot.production_hold_pair_samples,
        "production_min_hold_start_after_down_completion_ticks": snapshot.production_min_hold_start_after_down_completion_ticks,
        "production_hold_floor_violation_count": snapshot.production_hold_floor_violation_count,
        "production_release_floor_samples": snapshot.production_release_floor_samples,
        "production_min_down_start_after_up_completion_ticks": snapshot.production_min_down_start_after_up_completion_ticks,
        "production_release_floor_violation_count": snapshot.production_release_floor_violation_count,
        "production_hold_floor_ticks": snapshot.production_hold_floor_ticks,
        "production_release_floor_ticks": snapshot.production_release_floor_ticks,
        "production_same_key_overlap_forensics_count": snapshot.production_same_key_overlap_forensics_count,
        "production_anchor_overwrite_count": snapshot.production_anchor_overwrite_count,
        "production_unmatched_up_count": snapshot.production_unmatched_up_count,
        "production_anomaly_ring_overwrite_count": snapshot.production_anomaly_ring_overwrite_count,
        "production_forensics_anomaly_count": snapshot.production_forensics_anomaly_count,
        "production_structural_anomaly_count": snapshot.production_structural_anomaly_count,
        "production_timing_diagnostic_count": snapshot.production_timing_diagnostic_count,
        "release_outcome": release,
    })
}
