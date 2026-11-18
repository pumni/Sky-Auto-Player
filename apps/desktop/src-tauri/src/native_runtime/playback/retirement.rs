use super::*;
use sky_player::engine::WorkerTerminalResult;

#[cfg(test)]
thread_local! {
    static RECOVERY_TEST_RAW_SENDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(in crate::native_runtime) fn reset_recovery_test_raw_sends() {
    RECOVERY_TEST_RAW_SENDS.with(|count| count.set(0));
}

#[cfg(test)]
pub(in crate::native_runtime) fn recovery_test_raw_sends() -> u64 {
    RECOVERY_TEST_RAW_SENDS.with(std::cell::Cell::get)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_runtime) enum TerminalReleaseEvidence {
    NotRequired,
    Verified {
        obligation_mask: u16,
    },
    Unverified {
        obligation_mask: u16,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::native_runtime) enum WorkerQuiescenceProof {
    NoWorkerStarted,
    WorkerJoined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_runtime) struct TerminalRetirement {
    pub(in crate::native_runtime) quiescence: WorkerQuiescenceProof,
    pub(in crate::native_runtime) release: TerminalReleaseEvidence,
    pub(in crate::native_runtime) publication: PlaybackTerminalPublication,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::native_runtime) enum PlaybackHandoffDecision {
    AwaitQuiescence,
    Retire(TerminalRetirement),
}

#[derive(Debug)]
pub(in crate::native_runtime) enum PlaybackActivation {
    Started,
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(in crate::native_runtime) enum DryRunTerminalRequest {
    Quit = 1,
    Skipped = 2,
}

pub(in crate::native_runtime) fn terminal_release_evidence(
    physical: bool,
    release_outcome: Option<&sky_dispatch_win32::input::ReleaseAllOutcome>,
    obligation_mask: u16,
) -> TerminalReleaseEvidence {
    if !physical {
        return TerminalReleaseEvidence::NotRequired;
    }
    let Some(outcome) = release_outcome else {
        return TerminalReleaseEvidence::Unverified {
            obligation_mask,
            detail: format!(
                "terminal release outcome was unavailable; residual obligation mask=0x{obligation_mask:04x}"
            ),
        };
    };
    if outcome.released_successfully
        && outcome.stuck_mask == 0
        && !outcome.verification_inconclusive
        && obligation_mask == 0
    {
        TerminalReleaseEvidence::Verified { obligation_mask }
    } else {
        TerminalReleaseEvidence::Unverified {
            obligation_mask,
            detail: format!(
                "terminal release was not verified (released_successfully={}, stuck_mask=0x{:04x}, verification_inconclusive={}, residual obligation mask=0x{obligation_mask:04x})",
                outcome.released_successfully,
                outcome.stuck_mask,
                outcome.verification_inconclusive,
            ),
        }
    }
}

pub(in crate::native_runtime) fn terminal_release_evidence_after_startup(
    physical: bool,
    snapshot: &EngineSnapshot,
) -> TerminalReleaseEvidence {
    if physical
        && !snapshot.startup_armed
        && snapshot.release_outcome.is_none()
        && snapshot.final_release_obligation_mask == 0
        && matches!(
            snapshot.terminal_trigger_reason.as_deref(),
            Some("quit" | "skipped" | "system_suspend" | "panic_release_requested")
        )
    {
        // The startup gate rejects every input before Armed. With no Armed
        // publication, no terminal release result, and no obligation, a
        // pre-arm terminal has no physical key state to reconcile.
        return TerminalReleaseEvidence::NotRequired;
    }
    terminal_release_evidence(
        physical,
        snapshot.release_outcome.as_ref(),
        snapshot.final_release_obligation_mask,
    )
}

pub(in crate::native_runtime) fn terminal_publication_after_worker(
    status: EnginePollStatus,
    release: &TerminalReleaseEvidence,
    terminal_error: Option<&str>,
    last_error: Option<&str>,
    terminal_trigger: Option<&str>,
    failed_release_count: u64,
) -> PlaybackTerminalPublication {
    let worker_error = terminal_error.or(last_error);
    let release_error = match release {
        TerminalReleaseEvidence::Unverified { detail, .. } => Some(detail.as_str()),
        TerminalReleaseEvidence::NotRequired | TerminalReleaseEvidence::Verified { .. } => None,
    };
    let failed = is_failure_status(status)
        || terminal_error.is_some()
        || worker_error.is_some()
        || failed_release_count > 0
        || release_error.is_some();
    if failed {
        let code =
            if is_failure_status(status) || terminal_error.is_some() || worker_error.is_some() {
                match status {
                    EnginePollStatus::Panicked => "native_player_panicked",
                    EnginePollStatus::Poisoned => "native_player_poisoned",
                    EnginePollStatus::Error | EnginePollStatus::Invalid => "native_player_failed",
                    _ => "native_player_failed",
                }
            } else {
                "terminal_release_unverified"
            };
        let mut details = Vec::new();
        if let Some(reason) = worker_error {
            details.push(format!("worker diagnostic: {reason}"));
        }
        if let Some(trigger) = terminal_trigger {
            details.push(format!("terminal trigger: {trigger}"));
        }
        if let Some(release_error) = release_error {
            details.push(release_error.to_string());
        }
        if failed_release_count > 0 {
            details.push(format!(
                "{} terminal release attempt(s) failed",
                failed_release_count
            ));
        }
        if details.is_empty() {
            details.push("the native worker did not report a successful terminal result".into());
        }
        PlaybackTerminalPublication::Failed {
            code: code.into(),
            message: format!(
                "Native playback ended with worker outcome `{}`; {}",
                status.as_str(),
                details.join("; ")
            ),
        }
    } else {
        match status {
            EnginePollStatus::SystemSuspended => PlaybackTerminalPublication::Failed {
                code: "system_suspend".into(),
                message: "Windows suspended or hibernated the system. This playback session has ended; start a new session after resume.".into(),
            },
            EnginePollStatus::Finished | EnginePollStatus::Quit | EnginePollStatus::Skipped => {
                PlaybackTerminalPublication::Finished {
                    outcome: terminal_success_outcome(status).into(),
                    message: match status {
                        EnginePollStatus::Finished => "Playback finished".into(),
                        EnginePollStatus::Quit => "Playback stopped".into(),
                        EnginePollStatus::Skipped => "Playback skipped".into(),
                        _ => unreachable!(),
                    },
                }
            }
            _ => PlaybackTerminalPublication::Failed {
                code: "native_player_terminal_unknown".into(),
                message: format!(
                    "Native playback worker ended without an authoritative terminal result (`{}`).",
                    status.as_str()
                ),
            },
        }
    }
}

pub(in crate::native_runtime) fn terminal_retirement_after_worker_with_result(
    status: EnginePollStatus,
    snapshot: &EngineSnapshot,
    physical: bool,
    worker_result: Option<&WorkerTerminalResult>,
) -> TerminalRetirement {
    let release = worker_result.map_or_else(
        || terminal_release_evidence_after_startup(physical, snapshot),
        |result| {
            if !physical {
                TerminalReleaseEvidence::NotRequired
            } else if result.residual.outstanding_mask == 0 {
                TerminalReleaseEvidence::Verified { obligation_mask: 0 }
            } else {
                TerminalReleaseEvidence::Unverified {
                    obligation_mask: result.residual.outstanding_mask,
                    detail: format!(
                        "worker terminal result retained release residual (needs_up=0x{:04x}, needs_probe=0x{:04x})",
                        result.residual.needs_up_mask,
                        result.residual.needs_probe_mask,
                    ),
                }
            }
        },
    );
    let publication = terminal_publication_after_worker(
        status,
        &release,
        worker_result
            .and_then(|result| result.terminal_error.as_deref())
            .or(snapshot.terminal_error.as_deref()),
        snapshot.last_error.as_deref(),
        snapshot.terminal_trigger_reason.as_deref(),
        snapshot.failed_release_count as u64,
    );
    TerminalRetirement {
        quiescence: WorkerQuiescenceProof::WorkerJoined,
        release,
        publication,
    }
}

#[cfg(test)]
pub(in crate::native_runtime) fn decide_playback_handoff(
    joined: &Result<bool, String>,
    status: Option<EnginePollStatus>,
    snapshot: Option<&EngineSnapshot>,
    physical: bool,
) -> PlaybackHandoffDecision {
    decide_playback_handoff_with_result(joined, status, snapshot, physical, None)
}

pub(in crate::native_runtime) fn decide_playback_handoff_with_result(
    joined: &Result<bool, String>,
    status: Option<EnginePollStatus>,
    snapshot: Option<&EngineSnapshot>,
    physical: bool,
    worker_result: Option<&WorkerTerminalResult>,
) -> PlaybackHandoffDecision {
    if !matches!(joined, Ok(true)) {
        return PlaybackHandoffDecision::AwaitQuiescence;
    }
    match (status, snapshot) {
        (Some(status), Some(snapshot)) if status.is_terminal() => PlaybackHandoffDecision::Retire(
            terminal_retirement_after_worker_with_result(status, snapshot, physical, worker_result),
        ),
        (Some(status), None) if !physical && status.is_terminal() => {
            let publication = match status {
                EnginePollStatus::Finished | EnginePollStatus::Quit | EnginePollStatus::Skipped => {
                    PlaybackTerminalPublication::Finished {
                        outcome: terminal_success_outcome(status).into(),
                        message: match status {
                            EnginePollStatus::Finished => "Playback finished".into(),
                            EnginePollStatus::Quit => "Playback stopped".into(),
                            EnginePollStatus::Skipped => "Playback skipped".into(),
                            _ => unreachable!(),
                        },
                    }
                }
                _ => PlaybackTerminalPublication::Failed {
                    code: "native_player_failed".into(),
                    message: format!(
                        "Non-physical playback ended with worker outcome `{}`.",
                        status.as_str()
                    ),
                },
            };
            PlaybackHandoffDecision::Retire(TerminalRetirement {
                quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                release: TerminalReleaseEvidence::NotRequired,
                publication,
            })
        }
        _ => {
            let release = snapshot.map_or_else(
                || terminal_release_evidence(physical, None, 0),
                |snapshot| {
                    terminal_release_evidence(
                        physical,
                        snapshot.release_outcome.as_ref(),
                        snapshot.final_release_obligation_mask,
                    )
                },
            );
            PlaybackHandoffDecision::Retire(TerminalRetirement {
                quiescence: WorkerQuiescenceProof::WorkerJoined,
                release,
                publication: PlaybackTerminalPublication::Failed {
                    code: "native_player_terminal_unknown".into(),
                    message: "The native worker stopped without an authoritative terminal result."
                        .into(),
                },
            })
        }
    }
}

#[cfg(test)]
pub(in crate::native_runtime) fn publish_terminal_poll_state(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    status: EnginePollStatus,
) -> Result<PlaybackTerminalPublication, String> {
    let terminal_state = playback_event_state(status);
    let is_failure = is_failure_status(status);
    let publication = if is_failure {
        PlaybackTerminalPublication::Failed {
            code: match status {
                EnginePollStatus::Panicked => "native_player_panicked",
                EnginePollStatus::Poisoned => "native_player_poisoned",
                _ => "native_player_failed",
            }
            .into(),
            message: format!("Native playback worker ended with `{}`.", status.as_str()),
        }
    } else if status == EnginePollStatus::SystemSuspended {
        PlaybackTerminalPublication::Failed {
            code: "system_suspend".into(),
            message: "Windows suspended or hibernated the system. This playback session has ended; start a new session after resume.".into(),
        }
    } else {
        PlaybackTerminalPublication::Finished {
            outcome: terminal_success_outcome(status).into(),
            message: match status {
                EnginePollStatus::Quit => "Playback stopped".into(),
                EnginePollStatus::Skipped => "Playback skipped".into(),
                _ => "Playback finished".into(),
            },
        }
    };
    if *last_event_state != terminal_state {
        set_playback_state(
            active,
            if is_failure || status == EnginePollStatus::SystemSuspended {
                PlaybackSessionState::Failed
            } else {
                PlaybackSessionState::Finished
            },
        )?;
        let (message, outcome) = match &publication {
            PlaybackTerminalPublication::Finished { outcome, message } => {
                (Some(message.clone()), Some(outcome.clone()))
            }
            PlaybackTerminalPublication::Failed { message, .. } => (Some(message.clone()), None),
        };
        publish_playback_state(events, active, terminal_state, message, outcome)?;
        *last_event_state = terminal_state;
    }
    Ok(publication)
}

pub(in crate::native_runtime) fn physical_startup_failure(
    error: &str,
) -> (&'static str, &'static str) {
    if error.starts_with("suspend_resume_registration_failed") {
        (
            "system_power_notifications_unavailable",
            "Windows suspend and resume notifications could not be registered, so physical playback was stopped safely.",
        )
    } else if error == "no admissible visible Sky window was found" {
        (
            "target_not_found",
            "Sky window was not found. Open Sky and make sure its window is visible, then try again.",
        )
    } else if error.starts_with("target Sky process has higher integrity") {
        (
            "target_integrity_mismatch",
            "Sky is running with higher Windows integrity than Sky Auto Player. Run both apps at the same permission level, then try again.",
        )
    } else if error == "validated Sky window could not be focused" {
        (
            "target_focus_failed",
            "Sky's window could not be focused. Bring the Sky window to the foreground, then try again.",
        )
    } else {
        (
            "native_worker_failed",
            "The native playback engine could not start. Try again; if it continues, restart Sky Auto Player.",
        )
    }
}

#[cfg(test)]
pub(in crate::native_runtime) fn publish_terminal_poll_result(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    last_event_state: &mut PlaybackEventState,
    status: EnginePollStatus,
) -> Result<(), String> {
    let publication = publish_terminal_poll_state(events, active, last_event_state, status)?;
    publish_terminal_event(events, active, &publication)
}

pub(in crate::native_runtime) fn publish_terminal_event(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    publication: &PlaybackTerminalPublication,
) -> Result<(), String> {
    match publication {
        PlaybackTerminalPublication::Finished { outcome, message } => {
            publish_playback_finished(events, active, outcome, message)
        }
        PlaybackTerminalPublication::Failed { code, message } => {
            publish_playback_failed(events, active, code, message)
        }
    }
}

pub(in crate::native_runtime) fn release_terminal_ownership(
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
    retirement: TerminalRetirement,
) -> Result<(), String> {
    *active
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = match &retirement.publication {
        PlaybackTerminalPublication::Finished { .. } => PlaybackSessionState::Finished,
        PlaybackTerminalPublication::Failed { .. } => PlaybackSessionState::Failed,
    };
    commit_terminal_status(active_slot, last_terminal, active, &retirement)?;
    release_terminal_resources(active, &retirement)?;
    retire_active_slot(active_slot, last_terminal, active)
}

pub(in crate::native_runtime) fn release_terminal_resources(
    active: &Arc<NativeActivePlayback>,
    retirement: &TerminalRetirement,
) -> Result<(), String> {
    active.worker_quiescent.store(true, Ordering::Release);
    active.release_power_resources();
    active
        .pending
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let lease = active
        .activity_lease
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let release_verified = match &retirement.release {
        TerminalReleaseEvidence::Unverified { .. } => {
            let lease_quarantined = lease.is_some_and(PhysicalActivityLease::quarantine);
            let coordinator_quarantined =
                active.physical && active.activity.quarantine_playback(&active.session_id);
            let quarantined = lease_quarantined || coordinator_quarantined;
            if !quarantined {
                return Err("physical release quarantine could not be established".into());
            }
            if active.physical
                && let Some(result) = active
                    .terminal_result
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                && result.residual.outstanding_mask != 0
                && !active
                    .activity
                    .store_quarantined_release_residual(&active.session_id, result.residual.clone())
            {
                return Err(
                    "terminal release residual could not be committed to its quarantine".into(),
                );
            }
            false
        }
        TerminalReleaseEvidence::NotRequired | TerminalReleaseEvidence::Verified { .. } => {
            drop(lease);
            true
        }
    };
    active
        .release_verified
        .store(release_verified, Ordering::Release);
    Ok(())
}

/// Give the single post-join recovery owner one bounded cleanup-drain
/// invocation. Any unresolved journal is written back to ActivityCoordinator
/// before ownership is relinquished, so a later attempt cannot invent or
/// broaden its origin/profile/masks.
pub(in crate::native_runtime) fn recover_quarantined_release_once(
    active: &NativeActivePlayback,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
) -> Result<(), String> {
    if !active.worker_quiescent.load(Ordering::Acquire) {
        return Err("release recovery requires a joined worker".into());
    }
    let Some(residual) = active
        .activity
        .claim_quarantined_release_residual(&active.session_id)
    else {
        return Ok(());
    };
    let Some(origin) = residual.owner.as_ref() else {
        let _ = active
            .activity
            .update_quarantined_release_residual(&active.session_id, residual);
        active
            .activity
            .abandon_quarantined_recovery(&active.session_id);
        return Err("quarantined residual has no original owner proof".into());
    };
    let Some(authority) = origin.authority().cloned() else {
        let _ = active
            .activity
            .update_quarantined_release_residual(&active.session_id, residual);
        active
            .activity
            .abandon_quarantined_recovery(&active.session_id);
        return Err("quarantined residual has no retained process authority".into());
    };
    let Some(owner_pid) = origin.owner_pid() else {
        let _ = active
            .activity
            .update_quarantined_release_residual(&active.session_id, residual);
        active
            .activity
            .abandon_quarantined_recovery(&active.session_id);
        return Err("quarantined residual has no original owner PID".into());
    };
    let foreground_required = origin.foreground_required();
    let target_hwnd = origin.target_hwnd();
    let mut tracked =
        match sky_dispatch_win32::input::TrackedKeyState::from_terminal_release_residual(
            residual.profile,
            origin.clone(),
            residual.outstanding_mask,
            residual.needs_up_mask,
            residual.needs_probe_mask,
        ) {
            Ok(tracked) => tracked,
            Err(error) => {
                let _ = active
                    .activity
                    .update_quarantined_release_residual(&active.session_id, residual);
                active
                    .activity
                    .abandon_quarantined_recovery(&active.session_id);
                return Err(error);
            }
        };
    #[cfg(test)]
    tracked.set_emitter(|scan_codes, _| {
        RECOVERY_TEST_RAW_SENDS.with(|count| count.set(count.get().saturating_add(1)));
        sky_dispatch_win32::input::PlatformSendResult {
            requested: scan_codes.len() as u8,
            inserted: scan_codes.len() as u8,
            started_ticks: sky_dispatch_win32::clock::QpcTicks::ZERO,
            completed_ticks: Some(sky_dispatch_win32::clock::QpcTicks::ZERO),
            win32_error: 0,
            timing_error: None,
        }
    });
    tracked.set_probe(|_, _| sky_dispatch_win32::input::InstrumentPhysicalState::Inconclusive);
    let drain = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tracked.release_terminal_tracked_scope_with_attempt_admission_and_drain_control(
            target_hwnd,
            |mask, proof| {
                if mask == 0 || !proof.same_owner(origin) {
                    return Err("recovery release owner differs from the journal origin".into());
                }
                if authority.verify_final_process_identity(target_hwnd, owner_pid)
                    != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
                {
                    return Err("original process identity is no longer continuous".into());
                }
                if foreground_required
                    && !matches!(
                        sky_dispatch_win32::focus::foreground_window_owner_matches(
                            target_hwnd,
                            owner_pid,
                        ),
                        sky_dispatch_win32::focus::ForegroundOwnerMatch::Match
                    )
                {
                    return Err(
                        "original window is no longer foreground-owned by its process".into(),
                    );
                }
                Ok(())
            },
            || {
                if authority.continuity()
                    != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
                {
                    return Err("original process/window identity is no longer continuous".into());
                }
                let foreground = !foreground_required
                    || matches!(
                        sky_dispatch_win32::focus::foreground_window_owner_matches(
                            target_hwnd,
                            owner_pid,
                        ),
                        sky_dispatch_win32::focus::ForegroundOwnerMatch::Match
                    );
                Ok(sky_dispatch_win32::input::CleanupDrainDecision {
                    // A recovery Up is permitted only in the exact original
                    // foreground. Terminal QPC fallback remains finite.
                    terminal_requested: foreground,
                    origin_foreground: foreground,
                })
            },
        )
    }));
    let updated = terminal_residual_from_tracked(&tracked, &residual);
    let verified = matches!(&drain, Ok(outcome) if outcome.release_outcome.released_successfully
        && outcome.release_outcome.stuck_mask == 0
        && !outcome.release_outcome.verification_inconclusive
        && updated.outstanding_mask == 0);
    if verified {
        if active
            .activity
            .finish_quarantined_recovery(&active.session_id, &updated)
        {
            active.release_verified.store(true, Ordering::Release);
            let mut terminal = last_terminal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(terminal) = terminal
                .as_mut()
                .filter(|terminal| terminal.session_id == active.session_id)
            {
                terminal.terminal_release_verified = Some(true);
                terminal.release_obligation_mask = Some(0);
            }
            return Ok(());
        }
        active
            .activity
            .abandon_quarantined_recovery(&active.session_id);
        return Err("recovery owner lost its session quarantine before commit".into());
    }

    let stored = active
        .activity
        .update_quarantined_release_residual(&active.session_id, updated);
    active
        .activity
        .abandon_quarantined_recovery(&active.session_id);
    if !stored {
        return Err("unresolved recovery residual could not be re-committed".into());
    }
    match drain {
        Ok(outcome) => Err(format!(
            "release recovery remains quarantined (needs_up=0x{:04x}, needs_probe=0x{:04x}, verdict={:?}, withheld={:?})",
            outcome.needs_up_mask,
            outcome.needs_probe_mask,
            outcome.probe_verdict,
            outcome.withheld_reason,
        )),
        Err(_) => Err("release recovery panicked; journal remains quarantined".into()),
    }
}

fn terminal_residual_from_tracked(
    tracked: &sky_dispatch_win32::input::TrackedKeyState,
    prior: &sky_player::engine::TerminalReleaseResidual,
) -> sky_player::engine::TerminalReleaseResidual {
    let outstanding_mask = tracked.release_obligation_mask();
    sky_player::engine::TerminalReleaseResidual {
        outstanding_mask,
        needs_up_mask: tracked.needs_up_mask(),
        needs_probe_mask: tracked.needs_probe_mask,
        owner: prior.owner.clone(),
        profile: std::array::from_fn(|slot| tracked.instrument_key_profile().physical_key(slot)),
    }
}

fn commit_terminal_status(
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
    retirement: &TerminalRetirement,
) -> Result<(), String> {
    let slot = active_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut terminal_slot = last_terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot
        .as_ref()
        .is_some_and(|current| !Arc::ptr_eq(current, active))
    {
        return Err("playback retirement no longer owns the active slot".into());
    }
    if slot.is_none()
        && !terminal_slot
            .as_ref()
            .is_some_and(|terminal| terminal.session_id == active.session_id)
    {
        return Err("playback retirement no longer owns its session id".into());
    }

    let terminal_status = playback_terminal_status(active, retirement);
    *terminal_slot = Some(terminal_status);
    Ok(())
}

fn retire_active_slot(
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
) -> Result<(), String> {
    let mut slot = active_slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let terminal_slot = last_terminal
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot
        .as_ref()
        .is_some_and(|current| !Arc::ptr_eq(current, active))
    {
        return Err("playback retirement no longer owns the active slot".into());
    }
    if !terminal_slot
        .as_ref()
        .is_some_and(|terminal| terminal.session_id == active.session_id)
    {
        return Err("terminal state was not committed for the retiring session".into());
    }
    *slot = None;
    active.done.store(true, Ordering::Release);
    Ok(())
}

pub(in crate::native_runtime) fn terminal_presentation_completion(
    active_slot: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: Arc<NativeActivePlayback>,
    retirement: TerminalRetirement,
    terminal_state: PlaybackSessionState,
    #[cfg(all(test, feature = "tauri-test"))] retired_hook: Option<PlaybackRetiredTestHook>,
) -> PlaybackPresentationCompletion {
    let _ = (active_slot, last_terminal, retirement, terminal_state);
    Box::new(move |presentation_result| {
        if presentation_result.is_err() {
            active.event_delivery_failure.store(true, Ordering::Release);
        }
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(hook) = retired_hook {
            hook();
        }
    })
}

pub(in crate::native_runtime) fn playback_terminal_status(
    active: &Arc<NativeActivePlayback>,
    retirement: &TerminalRetirement,
) -> PlaybackTerminalStatusDto {
    let (state, outcome, failure_code, failure_message) = match &retirement.publication {
        PlaybackTerminalPublication::Finished { outcome, .. } => (
            PlaybackSessionState::Finished,
            Some(outcome.clone()),
            None,
            None,
        ),
        PlaybackTerminalPublication::Failed { code, message } => (
            PlaybackSessionState::Failed,
            None,
            Some(code.clone()),
            Some(message.clone()),
        ),
    };
    let (terminal_release_verified, release_obligation_mask) = match &retirement.release {
        TerminalReleaseEvidence::NotRequired if active.physical => (Some(true), None),
        TerminalReleaseEvidence::NotRequired => (None, None),
        TerminalReleaseEvidence::Verified { obligation_mask } => {
            (Some(true), Some(*obligation_mask))
        }
        TerminalReleaseEvidence::Unverified {
            obligation_mask, ..
        } => (Some(false), Some(*obligation_mask)),
    };
    PlaybackTerminalStatusDto {
        session_id: active.session_id.clone(),
        prepared_id: active.prepared_id.clone(),
        song_id: active.song_id.clone(),
        state,
        outcome,
        failure_code,
        failure_message,
        terminal_release_verified,
        release_obligation_mask,
    }
}

#[cfg(test)]
pub(in crate::native_runtime) fn publish_retirement_barrier(
    events: &Arc<Mutex<NativeEventHub>>,
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
    retirement: TerminalRetirement,
) -> Result<(), String> {
    release_terminal_ownership(active_slot, last_terminal, active, retirement.clone())?;
    publish_terminal_event(events, active, &retirement.publication)
}

pub(in crate::native_runtime) fn publish_playback_failed(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    code: &str,
    message: &str,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::PlaybackFailed {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: PlaybackFailedPayload {
                session_id: active.session_id.clone(),
                song_id: active.song_id.clone(),
                code: code.into(),
                message: message.into(),
            },
        })
}

pub(in crate::native_runtime) fn publish_playback_finished(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    outcome: &str,
    message: &str,
) -> Result<(), String> {
    events
        .lock()
        .map_err(|_| "native event hub lock poisoned".to_string())?
        .publish(UiEvent::PlaybackFinished {
            v: crate::DESKTOP_PROTOCOL_VERSION,
            payload: PlaybackFinishedPayload {
                session_id: active.session_id.clone(),
                song_id: active.song_id.clone(),
                outcome: outcome.into(),
                total_us: active.total_us,
                message: message.into(),
            },
        })
}
