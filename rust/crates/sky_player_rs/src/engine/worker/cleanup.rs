use super::*;

pub(crate) fn cancel_coordinator_or_terminal(
    coordinator: &mut RuntimeDispatchCoordinator,
    force_full_cleanup: &mut bool,
    terminal_error: &mut Option<String>,
    secondary_errors: &mut Vec<String>,
) {
    if let Err(error) = coordinator.cancel_all() {
        *force_full_cleanup = true;
        record_termination_error(
            terminal_error,
            secondary_errors,
            format!("coordinator cancellation failure: {error}"),
        );
    }
}

pub(crate) fn release_outcome_verified(outcome: &ReleaseAllOutcome) -> bool {
    outcome.released_successfully
        && outcome.stuck_keys.is_empty()
        && !outcome.verification_inconclusive
}

pub(crate) fn release_state_verified(
    backend: &TrackedKeyState,
    outcome: &ReleaseAllOutcome,
) -> bool {
    release_outcome_verified(outcome)
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
}

pub(crate) fn clean_completion_proven(
    coordinator: &RuntimeDispatchCoordinator,
    backend: &TrackedKeyState,
) -> bool {
    let counts = coordinator.generation_status_counts();
    let all_released = counts.get("released").copied().unwrap_or_default()
        == coordinator.schedule.generation_count
        && counts.values().sum::<u64>() == coordinator.schedule.generation_count;
    all_released
        && counts.get("scheduled").copied().unwrap_or_default() == 0
        && counts.get("active").copied().unwrap_or_default() == 0
        && counts.get("release_pending").copied().unwrap_or_default() == 0
        && counts.get("dropped_backend").copied().unwrap_or_default() == 0
        && counts.get("dropped_conflict").copied().unwrap_or_default() == 0
        && counts.get("dropped_expired").copied().unwrap_or_default() == 0
        && counts.get("cancelled").copied().unwrap_or_default() == 0
        && backend.active_mask == 0
        && backend.possibly_active_mask == 0
        && backend.failed_release_mask == 0
        && backend.keys_dropped == 0
        && backend.chord_split_events == 0
        && backend.sendinput_partial_events == 0
        && backend.sendinput_zero_progress_failures == 0
        && backend.authored_keys_rejected == 0
}

pub(crate) fn describe_release_outcome(outcome: &ReleaseAllOutcome) -> String {
    format!(
        "released_successfully={}, stuck_keys={:?}, verification_inconclusive={}",
        outcome.released_successfully, outcome.stuck_keys, outcome.verification_inconclusive
    )
}

pub(crate) fn record_termination_error(
    primary: &mut Option<String>,
    secondary: &mut Vec<String>,
    error: String,
) {
    if primary.is_none() {
        *primary = Some(error);
    } else if primary.as_deref() != Some(error.as_str()) && !secondary.contains(&error) {
        secondary.push(error);
    }
}

/// Release physical input before cancelling only generations that still own it.
///
/// A suspend is resumable: authored generations that have not reached the
/// backend remain Scheduled. The backend result is checked before coordinator
/// state is changed, so an inconclusive release cannot be mistaken for a clean
/// pause.
pub(crate) fn suspend_live_input(
    backend: &mut TrackedKeyState,
    coordinator: &mut RuntimeDispatchCoordinator,
    target_hwnd: isize,
) -> Result<Vec<u64>, String> {
    let initial = backend.release_all(target_hwnd);
    let release = if release_state_verified(backend, &initial) {
        initial
    } else {
        let full = backend.release_all_full_instrument(target_hwnd);
        if !release_state_verified(backend, &full) {
            return Err(format!(
                "release verification failed (initial: {}; full: {})",
                describe_release_outcome(&initial),
                describe_release_outcome(&full),
            ));
        }
        full
    };

    debug_assert!(release_state_verified(backend, &release));
    let cancelled = coordinator
        .cancel_live_generations()
        .map_err(|error| format!("coordinator live cancellation failed: {error}"))?;
    coordinator
        .check_invariants()
        .map_err(|error| format!("coordinator invariant failure after suspension: {error}"))?;
    Ok(cancelled)
}

pub(crate) fn release_runtime_outcome(
    deferred_by_us: u64,
    sent_count: usize,
    requested_count: usize,
    _recovery_required: bool,
) -> &'static str {
    let deferred = deferred_by_us > 0;
    match (sent_count == requested_count, sent_count > 0, deferred) {
        (true, _, true) => "deferred_release",
        (true, _, false) => "sent",
        (false, true, true) => "deferred_partial_note_off",
        (false, true, false) => "partial_note_off",
        (false, false, true) => "deferred_failed_note_off",
        (false, false, false) => "failed_note_off",
    }
}
