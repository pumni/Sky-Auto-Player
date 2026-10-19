use super::*;

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

pub(in crate::native_runtime) fn terminal_retirement_after_worker(
    status: EnginePollStatus,
    snapshot: &EngineSnapshot,
    physical: bool,
) -> TerminalRetirement {
    let release = terminal_release_evidence_after_startup(physical, snapshot);
    let publication = terminal_publication_after_worker(
        status,
        &release,
        snapshot.terminal_error.as_deref(),
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

pub(in crate::native_runtime) fn decide_playback_handoff(
    joined: &Result<bool, String>,
    status: Option<EnginePollStatus>,
    snapshot: Option<&EngineSnapshot>,
    physical: bool,
) -> PlaybackHandoffDecision {
    if !matches!(joined, Ok(true)) {
        return PlaybackHandoffDecision::AwaitQuiescence;
    }
    match (status, snapshot) {
        (Some(status), Some(snapshot)) if status.is_terminal() => PlaybackHandoffDecision::Retire(
            terminal_retirement_after_worker(status, snapshot, physical),
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
    release_terminal_resources(active, &retirement)?;
    clear_terminal_ownership(active_slot, last_terminal, active, retirement)
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
        .map_err(|_| "native playback control lock poisoned".to_string())?
        .take();
    let lease = active
        .activity_lease
        .lock()
        .map_err(|_| "native playback activity lock poisoned".to_string())?
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

pub(in crate::native_runtime) fn clear_terminal_ownership(
    active_slot: &Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: &Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: &Arc<NativeActivePlayback>,
    retirement: TerminalRetirement,
) -> Result<(), String> {
    let mut slot = active_slot
        .lock()
        .map_err(|_| "native active-playback lock poisoned".to_string())?;
    let mut terminal_slot = last_terminal
        .lock()
        .map_err(|_| "native terminal-session lock poisoned".to_string())?;
    if !slot
        .as_ref()
        .is_some_and(|current| Arc::ptr_eq(current, active))
    {
        return Err("playback retirement no longer owns the active slot".into());
    }

    let terminal_status = playback_terminal_status(active, &retirement);
    *slot = None;
    *terminal_slot = Some(terminal_status);
    active.done.store(true, Ordering::Release);
    Ok(())
}

pub(in crate::native_runtime) fn terminal_presentation_completion(
    active_slot: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    last_terminal: Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    active: Arc<NativeActivePlayback>,
    mut retirement: TerminalRetirement,
    terminal_state: PlaybackSessionState,
    #[cfg(all(test, feature = "tauri-test"))] retired_hook: Option<PlaybackRetiredTestHook>,
) -> PlaybackPresentationCompletion {
    Box::new(move |presentation_result| {
        if presentation_result.is_err() {
            active.event_delivery_failure.store(true, Ordering::Release);
        }
        if set_playback_state(&active, terminal_state).is_err() {
            retirement.publication = PlaybackTerminalPublication::Failed {
                code: "playback_state_update_failed".into(),
                message: "Playback terminal state could not be committed.".into(),
            };
            active.stop_requested.store(true, Ordering::Release);
            active.event_delivery_failure.store(true, Ordering::Release);
        }
        loop {
            if clear_terminal_ownership(&active_slot, &last_terminal, &active, retirement.clone())
                .is_ok()
            {
                break;
            }
            thread::sleep(PLAYBACK_JOIN_RETRY_INTERVAL);
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
