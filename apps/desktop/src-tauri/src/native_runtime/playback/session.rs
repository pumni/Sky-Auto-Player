use super::*;
use sha2::{Digest, Sha256};

mod preparation;
mod prepare_capacity;
#[cfg(test)]
pub(super) use prepare_capacity::compile_dispatch_schedule;
#[cfg(test)]
use prepare_capacity::compile_dispatch_schedule_with_checkpoint;
pub(in crate::native_runtime) use prepare_capacity::retain_prepared_capacity;
use prepare_capacity::{
    NativePrepareCapacityInput, checked_capacity_sum,
    compile_dispatch_schedule_with_prepared_capacity,
    native_prepare_caller_retained_capacity_bytes, prepared_cache_retained_capacity_bytes,
    prepared_plan_capacity_bytes, runtime_schedule_capacity_bytes,
    schedule_metadata_heap_capacity_bytes,
};
#[cfg(test)]
#[path = "session/owner_identity_poll_consumer_tests.rs"]
mod owner_identity_poll_consumer_tests;
#[cfg(test)]
#[path = "session/preparation_cancellation_tests.rs"]
mod preparation_cancellation_tests;

fn consume_owner_identity_poll_result(
    result: sky_player::engine::OwnerIdentityPollResult,
    identity_authority_invalidated: &mut bool,
    panic_release: impl FnOnce(),
) {
    if result == sky_player::engine::OwnerIdentityPollResult::CurrentInvalid
        && !*identity_authority_invalidated
    {
        *identity_authority_invalidated = true;
        panic_release();
    }
}

pub(in crate::native_runtime) struct NativeDispatchStartupContext<'a> {
    startup_cancellation: &'a AtomicBool,
    event_delivery_failure: &'a AtomicBool,
}

impl SenderTraceState {
    pub(in crate::native_runtime) fn begin_physical_session(&mut self, session_id: &str) {
        self.collecting_session_id = Some(session_id.to_string());
        self.completed = None;
    }

    pub(in crate::native_runtime) fn finish_physical_session(
        &mut self,
        session_id: &str,
        song_id: &str,
        song_title: &str,
        plan_fingerprint: &str,
        trace_json: Option<String>,
    ) {
        if self.collecting_session_id.as_deref() != Some(session_id) {
            return;
        }
        self.completed = trace_json.map(|trace_json| CompletedSenderTrace {
            session_id: session_id.to_string(),
            song_id: song_id.to_string(),
            song_title: song_title.to_string(),
            plan_fingerprint: plan_fingerprint.to_string(),
            trace_json,
        });
        self.collecting_session_id = None;
    }
}

impl NativeActivePlayback {
    pub(in crate::native_runtime) fn request_dry_run_terminal(
        &self,
        request: DryRunTerminalRequest,
    ) {
        if self.player.is_none() {
            let _ = self.dry_run_terminal_request.compare_exchange(
                DRY_RUN_TERMINAL_NONE,
                request as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }

    pub(in crate::native_runtime) fn dry_run_terminal_status(&self) -> Option<EnginePollStatus> {
        match self.dry_run_terminal_request.load(Ordering::Acquire) {
            value if value == DryRunTerminalRequest::Quit as u8 => Some(EnginePollStatus::Quit),
            value if value == DryRunTerminalRequest::Skipped as u8 => {
                Some(EnginePollStatus::Skipped)
            }
            _ => None,
        }
    }

    pub(in crate::native_runtime) fn release_power_request(&self) {
        if let Ok(mut owner) = self.power_request.lock()
            && let Some(mut request) = owner.take()
        {
            request.release();
        }
    }

    pub(in crate::native_runtime) fn release_power_resources(&self) {
        if let Some(player) = &self.player {
            player.deactivate_system_power();
        }
        self.release_power_request();
        if let Ok(mut owner) = self.suspend_resume_registration.lock()
            && let Some(mut registration) = owner.take()
        {
            registration.unregister();
        }
        self.power_lifecycle.mark_registration_inactive();
    }

    pub(in crate::native_runtime) fn power_lifecycle_snapshot(&self) -> PowerLifecycleSnapshot {
        self.power_lifecycle.snapshot()
    }
}

impl NativePlaybackService {
    pub(in crate::native_runtime) fn new(activity: ActivityCoordinator) -> Self {
        Self {
            prepared: Mutex::new(VecDeque::new()),
            active: Arc::new(Mutex::new(None)),
            last_terminal: Arc::new(Mutex::new(None)),
            sender_trace_state: Arc::new(Mutex::new(SenderTraceState::default())),
            diagnostics_gate: Arc::new(DiagnosticsPublicationGate::default()),
            prepare_publication: Mutex::new(PreparePublicationState::default()),
            activity,
            post_join_recovery: Arc::new(PostJoinRecoveryLifecycle::default()),
            startup_cancellation: Arc::new(AtomicBool::new(false)),
            startup_arm_gate: Mutex::new(()),
            #[cfg(test)]
            startup_failure: None,
            #[cfg(test)]
            prepare_compile_test_hook: Mutex::new(None),
            #[cfg(test)]
            settings_invalidation_count: AtomicU64::new(0),
            #[cfg(test)]
            prepare_before_publication_test_hook: Mutex::new(None),
            #[cfg(test)]
            prepare_before_model_test_hook: Mutex::new(None),
            #[cfg(all(test, feature = "tauri-test"))]
            monitor_progress_test_hook: None,
            #[cfg(all(test, feature = "tauri-test"))]
            presentation_test_hook: None,
            #[cfg(test)]
            monitor_join_test_hook: None,
            #[cfg(test)]
            supervisor_spawn_failure: false,
            #[cfg(all(test, feature = "tauri-test"))]
            test_player: None,
            #[cfg(all(test, feature = "tauri-test"))]
            prepared_player_factory_for_test: None,
            #[cfg(all(test, feature = "tauri-test"))]
            recommended_plan_caller_capacity_override_for_test: None,
            #[cfg(all(test, feature = "tauri-test"))]
            recommendation_materialization_test_hook: Mutex::new(None),
            #[cfg(all(test, feature = "tauri-test"))]
            before_arm_test_hook: None,
            #[cfg(all(test, feature = "tauri-test"))]
            #[cfg(all(test, feature = "tauri-test"))]
            monitor_retired_test_hook: None,
        }
    }

    pub(in crate::native_runtime) fn set_diagnostics_enabled(
        &self,
        enabled: bool,
        _events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<(), String> {
        let enabled = self.diagnostics_gate.set_enabled(enabled)?;
        let active_player = self
            .active
            .lock()
            .map_err(|_| "active playback lock poisoned".to_string())?
            .as_ref()
            .and_then(|active| active.player.clone());
        if let Some(player) = active_player {
            player.set_live_diagnostics_enabled(enabled);
        }
        Ok(())
    }

    pub(in crate::native_runtime) fn status(&self) -> Result<PlaybackStatusDto, String> {
        let active_slot = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?;
        let last_terminal = self
            .last_terminal
            .lock()
            .map_err(|_| "native terminal-session lock poisoned".to_string())?
            .clone();
        let active = active_slot
            .as_ref()
            .map(|active| {
                let state = *active
                    .state
                    .lock()
                    .map_err(|_| "native playback state lock poisoned".to_string())?;
                // A terminal state remains owned until the retirement barrier
                // clears the active slot. Keep it actionable as stopping until
                // that barrier is complete.
                let state = if matches!(
                    state,
                    PlaybackSessionState::Finished | PlaybackSessionState::Failed
                ) && !active.worker_quiescent.load(Ordering::Acquire)
                {
                    PlaybackSessionState::Stopping
                } else {
                    state
                };
                Ok::<PlaybackActiveStatusDto, String>(PlaybackActiveStatusDto {
                    session_id: active.session_id.clone(),
                    prepared_id: active.prepared_id.clone(),
                    song_id: active.song_id.clone(),
                    title: active.title.clone(),
                    state,
                })
            })
            .transpose()?;
        Ok(PlaybackStatusDto {
            active,
            last_terminal,
        })
    }

    pub(in crate::native_runtime) fn export_sender_trace(&self) -> Result<String, String> {
        let trace_state = self
            .sender_trace_state
            .lock()
            .map_err(|_| "sender trace state lock poisoned".to_string())?;
        let completed = trace_state
            .completed
            .as_ref()
            .ok_or_else(|| "no completed physical sender trace is available".to_string())?;
        let document: Value = serde_json::from_str(&completed.trace_json)
            .map_err(|_| "completed sender trace is malformed".to_string())?;
        if document["session_id"] != completed.session_id
            || document["song_id"] != completed.song_id
            || document["song_title"] != completed.song_title
            || document["plan_fingerprint"] != completed.plan_fingerprint
        {
            return Err("completed sender trace identity does not match its owner".to_string());
        }
        Ok(completed.trace_json.clone())
    }

    pub(in crate::native_runtime) fn start(
        &self,
        request: PlaybackStartRequest,
        settings: &ApplicationSettings,
        events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<PlaybackSessionDto, String> {
        let startup_cancellation = Arc::clone(&self.startup_cancellation);
        if startup_cancellation.load(Ordering::Acquire) {
            return Err("native playback startup was cancelled during shutdown".into());
        }
        let mut active_slot = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?;
        if active_slot.is_some() {
            return Err("another playback session is active".into());
        }
        if startup_cancellation.load(Ordering::Acquire) {
            return Err("native playback startup was cancelled during shutdown".into());
        }
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        let prepared_index = prepared
            .iter()
            .position(|(id, _)| id == &request.prepared_id)
            .ok_or_else(|| "prepared playback is stale or already consumed".to_string())?;
        let record = &prepared[prepared_index].1;
        if record.settings_fingerprint != settings_fingerprint(settings)? {
            return Err("prepared playback is stale after settings mutation".into());
        }
        let accepted = request
            .decisions
            .iter()
            .filter(|item| item.accepted)
            .collect::<Vec<_>>();
        let required = record
            .dto
            .decisions
            .iter()
            .map(|item| item.decision)
            .collect::<Vec<_>>();
        let selected = if record.dto.admission == PlaybackAdmission::Ready {
            if !request.decisions.is_empty() {
                return Err("ready playback accepts no risk decisions".into());
            }
            PlaybackDecision::Proceed
        } else {
            if accepted.len() != 1
                || request.decisions.len() != 1
                || !required.contains(&accepted[0].decision)
            {
                return Err("an exact risk decision is required".into());
            }
            accepted[0].decision
        };
        let selected_variant = record
            .variants
            .get(&selected)
            .ok_or_else(|| "selected risk decision has no prepared plan".to_string())?;
        let dry_run = selected_variant.config.dry_run;
        let session_id = opaque_native_id()?;
        let event_delivery_failure = events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .failure_flag();
        if event_delivery_failure.load(Ordering::Acquire) {
            return Err("native playback lifecycle delivery is unavailable".into());
        }
        if startup_cancellation.load(Ordering::Acquire) {
            return Err("native playback startup was cancelled during shutdown".into());
        }
        let activity_lease = if dry_run {
            None
        } else {
            Some(
                self.activity
                    .reserve_playback(&session_id)
                    .map_err(playback_activity_error)?,
            )
        };
        let record = prepared
            .remove(prepared_index)
            .expect("prepared record was found above")
            .1;
        drop(prepared);
        let NativePreparedPlan {
            song_id,
            song_title,
            mut variants,
            ..
        } = record;
        let mut variant = variants
            .remove(&selected)
            .expect("selected variant was checked before consuming the plan");
        drop(variants);
        let power_lifecycle = Arc::new(PowerLifecycleDiagnostics::default());
        let power_platform = system_power_platform();
        let mut power_request = None;
        let mut suspend_resume_registration = None;
        let physical_creation = if variant.config.dry_run {
            Ok((None, None))
        } else {
            let startup_context = NativeDispatchStartupContext {
                startup_cancellation: &startup_cancellation,
                event_delivery_failure: &event_delivery_failure,
            };
            match variant.prepared_schedule.take() {
                Some(prepared_schedule) => self.create_native_player(
                    prepared_schedule,
                    &variant.config,
                    &variant.timing_policy,
                    settings,
                    &startup_context,
                ),
                None => Err("prepared physical playback plan is missing".to_string()),
            }
            .and_then(|(player, target)| {
                let result: Result<_, String> = (|| {
                    #[cfg(all(test, feature = "tauri-test"))]
                    if self.test_player.is_some() || self.prepared_player_factory_for_test.is_some()
                    {
                        return Ok((Some(player.clone()), Some(target)));
                    }
                    (power_request, suspend_resume_registration) =
                        acquire_playback_power_resources(
                            Some(Arc::downgrade(&player)),
                            power_platform.clone(),
                            power_lifecycle.clone(),
                        )?;
                    Ok((Some(player.clone()), Some(target)))
                })();
                if result.is_err() {
                    player.deactivate_system_power();
                }
                result
            })
        };
        let (player, target_hwnd) = match physical_creation {
            Ok(value) => value,
            Err(error) => {
                // A start that cannot pass focus/target admission still has
                // an ordered starting -> failed -> failed-event trace.
                let (code, message) = physical_startup_failure(&error);
                let failed_active = Arc::new(NativeActivePlayback {
                    activity: self.activity.clone(),
                    session_id: session_id.clone(),
                    prepared_id: request.prepared_id.clone(),
                    song_id: song_id.clone(),
                    title: song_title.clone(),
                    total_us: variant.schedule_duration_us,
                    config: variant.config.clone(),
                    timing_policy: variant.timing_policy.clone(),
                    plan_fingerprint: variant.fingerprint.clone(),
                    physical: true,
                    activity_lease: Mutex::new(activity_lease),
                    target_hwnd: None,
                    state: Mutex::new(PlaybackSessionState::Starting),
                    pending: Mutex::new(None),
                    player: None,
                    terminal_result: Mutex::new(None),
                    power_lifecycle: power_lifecycle.clone(),
                    power_request: Mutex::new(power_request.take()),
                    suspend_resume_registration: Mutex::new(suspend_resume_registration.take()),
                    started_at: Instant::now(),
                    paused_since: Mutex::new(None),
                    paused_total: Mutex::new(Duration::ZERO),
                    stop_requested: AtomicBool::new(false),
                    dry_run_terminal_request: AtomicU8::new(DRY_RUN_TERMINAL_NONE),
                    done: AtomicBool::new(false),
                    worker_quiescent: AtomicBool::new(false),
                    release_verified: AtomicBool::new(false),
                    sequence: AtomicU64::new(0),
                    event_delivery_failure: Arc::clone(&event_delivery_failure),
                    presentation: OnceLock::new(),
                });
                *active_slot = Some(Arc::clone(&failed_active));
                let starting_result = publish_playback_state(
                    &events,
                    &failed_active,
                    PlaybackEventState::Starting,
                    None,
                    None,
                );
                drop(active_slot);
                if let Err(publish_error) = starting_result {
                    let retirement = TerminalRetirement {
                        quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                        release: TerminalReleaseEvidence::NotRequired,
                        publication: PlaybackTerminalPublication::Failed {
                            code: "playback_start_publication_failed".into(),
                            message: format!(
                                "Playback could not publish its starting state: {publish_error}"
                            ),
                        },
                    };
                    let _ = release_terminal_ownership(
                        &self.active,
                        &self.last_terminal,
                        &failed_active,
                        retirement,
                    );
                    return Err(format!("{code}: {message}; {error}"));
                }

                let presentation = match NativePlaybackPresentation::start(
                    &failed_active,
                    events.clone(),
                    Arc::clone(&self.diagnostics_gate),
                    #[cfg(all(test, feature = "tauri-test"))]
                    self.presentation_test_hook.clone(),
                ) {
                    Ok(presentation) => presentation,
                    Err(presentation_error) => {
                        let retirement = TerminalRetirement {
                            quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                            release: TerminalReleaseEvidence::NotRequired,
                            publication: PlaybackTerminalPublication::Failed {
                                code: "playback_presentation_start_failed".into(),
                                message: presentation_error.clone(),
                            },
                        };
                        let _ = release_terminal_ownership(
                            &self.active,
                            &self.last_terminal,
                            &failed_active,
                            retirement.clone(),
                        );
                        let completed = terminal_presentation_completion(
                            Arc::clone(&failed_active),
                            #[cfg(all(test, feature = "tauri-test"))]
                            self.monitor_retired_test_hook.clone(),
                        );
                        let _ = finalize_active_presentation(
                            &events,
                            &failed_active,
                            &self.diagnostics_gate,
                            PlaybackPresentationFinalization {
                                state: PlaybackEventState::Failed,
                                message: Some(presentation_error.clone()),
                                outcome: None,
                                publication: retirement.publication.clone(),
                            },
                            completed,
                        );
                        return Err(format!("{code}: {message}; {error}"));
                    }
                };
                let _ = failed_active.presentation.set(presentation);
                let retirement = TerminalRetirement {
                    quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                    release: TerminalReleaseEvidence::NotRequired,
                    publication: PlaybackTerminalPublication::Failed {
                        code: code.into(),
                        message: message.into(),
                    },
                };
                let _ = release_terminal_ownership(
                    &self.active,
                    &self.last_terminal,
                    &failed_active,
                    retirement.clone(),
                );
                let completed = terminal_presentation_completion(
                    Arc::clone(&failed_active),
                    #[cfg(all(test, feature = "tauri-test"))]
                    self.monitor_retired_test_hook.clone(),
                );
                let _ = finalize_active_presentation(
                    &events,
                    &failed_active,
                    &self.diagnostics_gate,
                    PlaybackPresentationFinalization {
                        state: PlaybackEventState::Failed,
                        message: Some(message.to_string()),
                        outcome: None,
                        publication: retirement.publication.clone(),
                    },
                    completed,
                );
                return Err(format!("{code}: {message}; {error}"));
            }
        };
        let active = Arc::new(NativeActivePlayback {
            activity: self.activity.clone(),
            session_id: session_id.clone(),
            prepared_id: request.prepared_id.clone(),
            song_id: song_id.clone(),
            title: song_title.clone(),
            total_us: variant.schedule_duration_us,
            config: variant.config.clone(),
            timing_policy: variant.timing_policy.clone(),
            plan_fingerprint: variant.fingerprint.clone(),
            physical: player.is_some(),
            activity_lease: Mutex::new(activity_lease),
            target_hwnd,
            state: Mutex::new(PlaybackSessionState::Starting),
            pending: Mutex::new(None),
            player,
            terminal_result: Mutex::new(None),
            power_lifecycle: power_lifecycle.clone(),
            power_request: Mutex::new(power_request),
            suspend_resume_registration: Mutex::new(suspend_resume_registration),
            started_at: Instant::now(),
            paused_since: Mutex::new(None),
            paused_total: Mutex::new(Duration::ZERO),
            stop_requested: AtomicBool::new(false),
            dry_run_terminal_request: AtomicU8::new(DRY_RUN_TERMINAL_NONE),
            done: AtomicBool::new(false),
            worker_quiescent: AtomicBool::new(false),
            release_verified: AtomicBool::new(false),
            sequence: AtomicU64::new(0),
            event_delivery_failure: Arc::clone(&event_delivery_failure),
            presentation: OnceLock::new(),
        });
        *active_slot = Some(active.clone());
        if let Some(player) = &active.player {
            let registration = match events.lock() {
                Ok(mut hub) => hub.register_playback_player(Arc::downgrade(player)),
                Err(_) => Err("native event hub lock poisoned".into()),
            };
            if let Err(error) = registration {
                active.event_delivery_failure.store(true, Ordering::Release);
                drop(active_slot);
                let _ = set_playback_state(&active, PlaybackSessionState::Failed);
                let _ = release_terminal_ownership(
                    &self.active,
                    &self.last_terminal,
                    &active,
                    TerminalRetirement {
                        quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                        release: TerminalReleaseEvidence::NotRequired,
                        publication: PlaybackTerminalPublication::Failed {
                            code: "native_event_registration_failed".into(),
                            message: error.clone(),
                        },
                    },
                );
                return Err(error);
            }
        }
        if let Some(player) = &active.player {
            player.set_live_diagnostics_enabled(self.diagnostics_gate.is_enabled());
        }
        if active.physical
            && let Ok(mut trace_state) = self.sender_trace_state.lock()
        {
            trace_state.begin_physical_session(&active.session_id);
        }
        if let Err(error) =
            publish_playback_state(&events, &active, PlaybackEventState::Starting, None, None)
        {
            drop(active_slot);
            if active.physical
                && let Ok(mut trace_state) = self.sender_trace_state.lock()
            {
                trace_state.finish_physical_session(
                    &active.session_id,
                    &active.song_id,
                    &active.title,
                    &active.plan_fingerprint,
                    None,
                );
            }
            let retirement = TerminalRetirement {
                quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                release: TerminalReleaseEvidence::NotRequired,
                publication: PlaybackTerminalPublication::Failed {
                    code: "playback_start_publication_failed".into(),
                    message: format!("Playback could not publish its starting state: {error}"),
                },
            };
            let _ =
                release_terminal_ownership(&self.active, &self.last_terminal, &active, retirement);
            return Err(error);
        }
        drop(active_slot);

        let presentation = match NativePlaybackPresentation::start(
            &active,
            events.clone(),
            Arc::clone(&self.diagnostics_gate),
            #[cfg(all(test, feature = "tauri-test"))]
            self.presentation_test_hook.clone(),
        ) {
            Ok(presentation) => presentation,
            Err(error) => {
                let retirement = TerminalRetirement {
                    quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                    release: TerminalReleaseEvidence::NotRequired,
                    publication: PlaybackTerminalPublication::Failed {
                        code: "playback_presentation_start_failed".into(),
                        message: error.clone(),
                    },
                };
                let _ = release_terminal_ownership(
                    &self.active,
                    &self.last_terminal,
                    &active,
                    retirement.clone(),
                );
                let completed = terminal_presentation_completion(
                    active.clone(),
                    #[cfg(all(test, feature = "tauri-test"))]
                    self.monitor_retired_test_hook.clone(),
                );
                let _ = finalize_active_presentation(
                    &events,
                    &active,
                    &self.diagnostics_gate,
                    PlaybackPresentationFinalization {
                        state: PlaybackEventState::Failed,
                        message: Some(error.clone()),
                        outcome: None,
                        publication: retirement.publication.clone(),
                    },
                    completed,
                );
                return Err(error);
            }
        };
        if active.presentation.set(presentation).is_err() {
            active.event_delivery_failure.store(true, Ordering::Release);
            let error = "playback presentation consumer was already attached".to_string();
            cleanup_failed_event_delivery(&active);
            let retirement = TerminalRetirement {
                quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                release: TerminalReleaseEvidence::NotRequired,
                publication: PlaybackTerminalPublication::Failed {
                    code: "playback_presentation_attachment_failed".into(),
                    message: error.clone(),
                },
            };
            let _ = release_terminal_ownership(
                &self.active,
                &self.last_terminal,
                &active,
                retirement.clone(),
            );
            let completed = terminal_presentation_completion(
                active.clone(),
                #[cfg(all(test, feature = "tauri-test"))]
                self.monitor_retired_test_hook.clone(),
            );
            let _ = finalize_active_presentation(
                &events,
                &active,
                &self.diagnostics_gate,
                PlaybackPresentationFinalization {
                    state: PlaybackEventState::Failed,
                    message: Some(error.clone()),
                    outcome: None,
                    publication: retirement.publication.clone(),
                },
                completed,
            );
            return Err(error);
        }
        let service = Arc::new(self.clone_handle());
        let active_for_thread = active.clone();
        let events_for_thread = events.clone();
        let (activation_sender, activation_receiver) = mpsc::channel();
        #[cfg(test)]
        let spawn_result = if self.supervisor_spawn_failure {
            Err(std::io::Error::other(
                "injected playback supervisor spawn failure",
            ))
        } else {
            thread::Builder::new()
                .name("sky-native-playback-supervisor".into())
                .spawn(move || {
                    service.monitor(active_for_thread, events_for_thread, activation_receiver)
                })
        };
        #[cfg(not(test))]
        let spawn_result = thread::Builder::new()
            .name("sky-native-playback-supervisor".into())
            .spawn(move || {
                service.monitor(active_for_thread, events_for_thread, activation_receiver)
            });
        if let Err(error) = spawn_result {
            if active.physical
                && let Ok(mut trace_state) = self.sender_trace_state.lock()
            {
                trace_state.finish_physical_session(
                    &active.session_id,
                    &active.song_id,
                    &active.title,
                    &active.plan_fingerprint,
                    None,
                );
            }
            let message = format!("failed to start native playback supervisor: {error}");
            let publication = PlaybackTerminalPublication::Failed {
                code: "playback_supervisor_start_failed".into(),
                message: message.clone(),
            };
            let retirement = TerminalRetirement {
                quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                release: TerminalReleaseEvidence::NotRequired,
                publication,
            };
            let _ = release_terminal_ownership(
                &self.active,
                &self.last_terminal,
                &active,
                retirement.clone(),
            );
            let completed = terminal_presentation_completion(
                active.clone(),
                #[cfg(all(test, feature = "tauri-test"))]
                self.monitor_retired_test_hook.clone(),
            );
            let _ = finalize_active_presentation(
                &events,
                &active,
                &self.diagnostics_gate,
                PlaybackPresentationFinalization {
                    state: PlaybackEventState::Failed,
                    message: Some(message.clone()),
                    outcome: None,
                    publication: retirement.publication.clone(),
                },
                completed,
            );
            return Err(format!(
                "failed to start native playback supervisor: {error}"
            ));
        }
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(hook) = &self.before_arm_test_hook {
            hook();
        }
        let _arm_gate = self
            .startup_arm_gate
            .lock()
            .map_err(|_| "native playback startup-arm lock poisoned".to_string())?;
        if startup_cancellation.load(Ordering::Acquire)
            && let Some(player) = &active.player
        {
            // A shutdown accepted before this gate won the pre-arm race.
            // Latch Quit before arm so the B worker cannot admit a Down.
            let _ = player.quit();
        }
        if let Some(player) = &active.player
            && let Err(error) = player.arm(0)
        {
            let activation = PlaybackActivation::Failed(error.clone());
            if activation_sender.send(activation).is_err() {
                let (closed_sender, activation_receiver) = mpsc::channel();
                drop(closed_sender);
                self.monitor(active.clone(), events.clone(), activation_receiver);
            }
            return Err(error);
        }
        if activation_sender.send(PlaybackActivation::Started).is_err() {
            if let Some(player) = &active.player {
                let _ = player.panic_release();
                let _ = player.quit();
            }
            let (closed_sender, activation_receiver) = mpsc::channel();
            drop(closed_sender);
            self.monitor(active.clone(), events.clone(), activation_receiver);
            return Err(
                "native playback supervisor exited before worker activation was confirmed".into(),
            );
        }
        Ok(PlaybackSessionDto {
            session_id,
            prepared_id: active.prepared_id.clone(),
            song_id: active.song_id.clone(),
            state: PlaybackSessionState::Starting,
            config: active.config.clone(),
            plan_fingerprint: active.plan_fingerprint.clone(),
        })
    }

    pub(in crate::native_runtime) fn clone_handle(&self) -> NativePlaybackServiceHandle {
        NativePlaybackServiceHandle {
            active: self.active.clone(),
            last_terminal: self.last_terminal.clone(),
            sender_trace_state: self.sender_trace_state.clone(),
            diagnostics_gate: self.diagnostics_gate.clone(),
            activity: self.activity.clone(),
            post_join_recovery: Arc::clone(&self.post_join_recovery),
            #[cfg(test)]
            monitor_join_test_hook: self.monitor_join_test_hook.clone(),
            #[cfg(all(test, feature = "tauri-test"))]
            monitor_retired_test_hook: self.monitor_retired_test_hook.clone(),
        }
    }

    pub(in crate::native_runtime) fn create_native_player(
        &self,
        prepared_schedule: sky_player::adapter_support::PreparedDispatchSchedule,
        config: &PlaybackConfigDto,
        policy: &MaterializedTimingPolicy,
        settings: &ApplicationSettings,
        startup_context: &NativeDispatchStartupContext<'_>,
    ) -> Result<(Arc<NativeDispatchSession>, isize), String> {
        #[cfg(test)]
        if let Some(error) = &self.startup_failure {
            return Err(error.clone());
        }
        if startup_context
            .event_delivery_failure
            .load(Ordering::Acquire)
        {
            return Err("native playback lifecycle delivery is unavailable".into());
        }
        if startup_context.startup_cancellation.load(Ordering::Acquire) {
            return Err("native playback startup was cancelled during shutdown".into());
        }
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(factory) = &self.prepared_player_factory_for_test {
            return factory(prepared_schedule, config, policy).map(|player| (player, 0));
        }
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(player) = &self.test_player {
            return Ok((Arc::clone(player), 0));
        }
        if startup_context
            .event_delivery_failure
            .load(Ordering::Acquire)
        {
            return Err("native playback lifecycle delivery is unavailable".into());
        }
        let target = sky_dispatch_win32::focus::find_sky_window(
            &settings.sky_process_names,
            settings.allow_title_fallback,
        )
        .ok_or_else(|| "no admissible visible Sky window was found".to_string())?;
        match target_integrity_startup_failure(
            sky_dispatch_win32::focus::target_integrity_compatibility(target),
        ) {
            Some(error) => return Err(error.to_string()),
            None => {
                // Compatible and Unknown are both allowed to continue.  The
                // integrity query is a bounded startup/control-plane hint;
                // it is never repeated by the realtime worker.
            }
        }
        if !sky_dispatch_win32::focus::focus_window_and_verify(target, Duration::from_millis(100)) {
            return Err("validated Sky window could not be focused".into());
        }
        let owner_authority = sky_dispatch_win32::focus::retain_window_identity(target)?;
        let identity = owner_authority.identity();
        let expected_process = settings.sky_process_names.is_empty()
            || settings.allow_title_fallback
            || settings
                .sky_process_names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&identity.process_image_basename));
        if !expected_process || (identity.title != "Sky" && !identity.title.starts_with("Sky")) {
            return Err("focused window no longer matches the admitted Sky process".into());
        }
        if owner_authority.continuity()
            != sky_dispatch_win32::focus::WindowIdentityContinuity::Continuous
        {
            return Err("focused window owner changed during identity capture".into());
        }
        let player = Arc::new(
            NativeDispatchSession::new_with_bound_window_identity_and_power_endpoint(
                NativeSessionOptions {
                    schedule: prepared_schedule,
                    #[cfg(feature = "tauri-test")]
                    timing_observer_for_qualification: false,
                    #[cfg(feature = "tauri-test")]
                    modifier_key_state_query_for_test: None,
                    backend: BackendConfig::Production,
                    profile: DispatchProfile::Production,
                    timing: TimingOptions {
                        game_fps: config.fps,
                        min_hold_us: policy.min_hold_us,
                        frame_us: policy.frame_us,
                        frame_base_hold_us: policy.frame_base_hold_us,
                        timing_margin_us: policy.timing_margin_us,
                        strict_timing: false,
                        strict_down_completion_late_us: 2_000,
                        strict_up_completion_late_us: 2_000,
                        input_path_warn_us: 300,
                    },
                    focus: FocusOptions {
                        require_focus: true,
                        focus_restore_grace_us: policy.focus_restore_grace_us,
                    },
                    wait: WaitOptions {
                        enable_waitable_timer: true,
                        enable_event_wait: true,
                        supervisor_lease_timeout_us:
                            sky_player::engine::DEFAULT_SUPERVISOR_LEASE_TIMEOUT_US,
                        modifier_hold_timeout_us:
                            sky_player::engine::DEFAULT_MODIFIER_HOLD_TIMEOUT_US,
                        #[cfg(feature = "tauri-test")]
                        test_spin_threshold_us: None,
                        #[cfg(feature = "tauri-test")]
                        test_wait_policy: sky_player::engine::TestWaitPolicy::LegacyTestWideSpin,
                    },
                    telemetry: TelemetryOptions {
                        mode: TelemetryMode::Off,
                        capacity: 0,
                    },
                    priority: PriorityOptions {
                        mode: PriorityMode::Auto,
                    },
                    instrument_key_profile: None,
                    #[cfg(feature = "tauri-test")]
                    startup_ordering_hook: None,
                    #[cfg(feature = "tauri-test")]
                    timer_lifecycle_context: None,
                    #[cfg(feature = "tauri-test")]
                    prepared_packet_ambiguity_mask: None,
                    #[cfg(feature = "tauri-test")]
                    preflight_user_held_mask: None,
                },
                system_power_endpoint()?,
                owner_authority,
            )?,
        );
        player.set_focus_hint(true);
        player.set_live_diagnostics_enabled(self.diagnostics_gate.is_enabled());
        Ok((player, target))
    }

    pub(in crate::native_runtime) fn join_worker(
        &self,
        active: &NativeActivePlayback,
        timeout: Duration,
    ) -> Result<bool, String> {
        #[cfg(test)]
        if let Some(hook) = &self.monitor_join_test_hook {
            return hook(timeout);
        }
        active
            .player
            .as_ref()
            .map_or(Ok(true), |player| player.join(timeout))
    }

    pub(in crate::native_runtime) fn monitor(
        &self,
        active: Arc<NativeActivePlayback>,
        events: Arc<Mutex<NativeEventHub>>,
        activation: mpsc::Receiver<PlaybackActivation>,
    ) {
        match activation.recv() {
            Ok(PlaybackActivation::Failed(error)) => {
                let message = format!("Native playback worker did not start: {error}");
                let retirement = TerminalRetirement {
                    quiescence: WorkerQuiescenceProof::NoWorkerStarted,
                    release: TerminalReleaseEvidence::NotRequired,
                    publication: PlaybackTerminalPublication::Failed {
                        code: "native_worker_start_failed".into(),
                        message: message.clone(),
                    },
                };
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                let _ = release_terminal_ownership(
                    &self.active,
                    &self.last_terminal,
                    &active,
                    retirement.clone(),
                );
                let completed = terminal_presentation_completion(
                    active.clone(),
                    #[cfg(all(test, feature = "tauri-test"))]
                    self.monitor_retired_test_hook.clone(),
                );
                let _ = finalize_active_presentation(
                    &events,
                    &active,
                    &self.diagnostics_gate,
                    PlaybackPresentationFinalization {
                        state: PlaybackEventState::Failed,
                        message: Some(message.clone()),
                        outcome: None,
                        publication: retirement.publication.clone(),
                    },
                    completed,
                );
                return;
            }
            Ok(PlaybackActivation::Started) => {}
            Err(_) => {
                // A dropped activation sender is ambiguous: the worker may
                // already have started. Request termination and keep the
                // physical reservation until the join barrier proves exit.
                active.request_dry_run_terminal(DryRunTerminalRequest::Quit);
                if let Some(player) = &active.player {
                    let _ = player.panic_release();
                    let _ = player.quit();
                }
                active.stop_requested.store(true, Ordering::Release);
            }
        }

        let supervisor_lifetime_guard = active
            .player
            .as_ref()
            .map(|player| player.supervisor_lifetime_guard());

        // WinEvent is only a quick transition hint. The periodic exact
        // foreground poll remains active as a loss/reorder/message-loop
        // fallback, and the worker keeps its own fresh final admission.
        let foreground_watcher =
            sky_dispatch_win32::focus::foreground_watcher::ForegroundWatcher::start().ok();
        let mut last_foreground_poll = Instant::now();

        let mut last_snapshot = Instant::now();
        let mut last_identity_check = Instant::now();
        let mut last_event_state = PlaybackEventState::Starting;
        let mut event_delivery_failed = active.event_delivery_failure.load(Ordering::Acquire);
        let mut identity_authority_invalidated = false;
        let mut stop_sent = false;
        let terminal_status = loop {
            if !event_delivery_failed && active.event_delivery_failure.load(Ordering::Acquire) {
                event_delivery_failed = true;
                cleanup_failed_event_delivery(&active);
            }
            if active.stop_requested.load(Ordering::Acquire) && !stop_sent {
                stop_sent = true;
                if let Some(player) = &active.player {
                    let _ = player.quit();
                }
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                if last_event_state != PlaybackEventState::Stopping {
                    if publish_active_playback_state(
                        &events,
                        &active,
                        PlaybackEventState::Stopping,
                        None,
                        None,
                    )
                    .is_err()
                    {
                        event_delivery_failed = true;
                        cleanup_failed_event_delivery(&active);
                    }
                    last_event_state = PlaybackEventState::Stopping;
                }
            }
            if let Some(player) = &active.player
                && let Some(target) = active.target_hwnd
            {
                if let Some(hint) = foreground_watcher
                    .as_ref()
                    .and_then(|watcher| watcher.take_hint())
                {
                    player.set_focus_hint(hint.hwnd == Some(target));
                }
                let loop_failed = foreground_watcher
                    .as_ref()
                    .is_some_and(|watcher| watcher.polling_fallback_required());
                if loop_failed || last_foreground_poll.elapsed() >= Duration::from_millis(100) {
                    player.set_focus_hint(sky_dispatch_win32::focus::foreground_window_matches(
                        target,
                    ));
                    last_foreground_poll = Instant::now();
                }
            }
            if !identity_authority_invalidated
                && last_identity_check.elapsed() >= Duration::from_millis(100)
            {
                if let Some(player) = &active.player {
                    consume_owner_identity_poll_result(
                        player.poll_window_identity_authority_result(),
                        &mut identity_authority_invalidated,
                        || {
                            let _ = player.panic_release();
                        },
                    );
                }
                last_identity_check = Instant::now();
            }
            let (elapsed, pre_roll_remaining, status) = if let Some(player) = &active.player {
                let state = player.poll_state();
                (state.elapsed_us, state.pre_roll_remaining_us, state.status)
            } else {
                let elapsed = dry_run_elapsed(&active);
                let paused = active
                    .state
                    .lock()
                    .map(|state| *state == PlaybackSessionState::Paused)
                    .unwrap_or(false);
                (
                    elapsed,
                    0,
                    if let Some(status) = active.dry_run_terminal_status() {
                        status
                    } else if elapsed >= active.total_us {
                        EnginePollStatus::Finished
                    } else if paused {
                        EnginePollStatus::Paused
                    } else {
                        EnginePollStatus::Playing
                    },
                )
            };
            if is_terminal_status(status) {
                break Some(status);
            }
            if active.event_delivery_failure.load(Ordering::Acquire) {
                event_delivery_failed = true;
                cleanup_failed_event_delivery(&active);
            }
            if !event_delivery_failed
                && !active.stop_requested.load(Ordering::Acquire)
                && !identity_authority_invalidated
                && let Some(player) = &active.player
            {
                let progress_result = player.publish_supervisor_progress();
                #[cfg(all(test, feature = "tauri-test"))]
                if let Some(hook) = &self.monitor_progress_test_hook {
                    hook(progress_result.is_ok());
                }
                if progress_result.is_err() {
                    active.stop_requested.store(true, Ordering::Release);
                    let _ = player.panic_release();
                    let _ = player.quit();
                }
            }
            let event_state = if active.stop_requested.load(Ordering::Acquire) {
                PlaybackEventState::Stopping
            } else {
                playback_event_state(status)
            };
            if event_state != last_event_state {
                let state = match event_state {
                    PlaybackEventState::Starting => PlaybackSessionState::Starting,
                    PlaybackEventState::Playing => PlaybackSessionState::Playing,
                    PlaybackEventState::Paused => PlaybackSessionState::Paused,
                    PlaybackEventState::Stopping => PlaybackSessionState::Stopping,
                    PlaybackEventState::Finished => PlaybackSessionState::Finished,
                    PlaybackEventState::Failed => PlaybackSessionState::Failed,
                };
                let _ = set_playback_state(&active, state);
                if matches!(
                    (
                        event_state,
                        active.pending.lock().ok().and_then(|pending| *pending)
                    ),
                    (
                        PlaybackEventState::Paused,
                        Some(PlaybackPendingControl::Pause)
                    ) | (
                        PlaybackEventState::Playing,
                        Some(PlaybackPendingControl::Resume)
                    )
                ) && let Ok(mut pending) = active.pending.lock()
                {
                    *pending = None;
                }
                if !event_delivery_failed
                    && publish_active_playback_state(&events, &active, event_state, None, None)
                        .is_err()
                {
                    event_delivery_failed = true;
                    cleanup_failed_event_delivery(&active);
                }
                last_event_state = event_state;
            }
            if last_snapshot.elapsed() >= Duration::from_millis(100) {
                if !event_delivery_failed {
                    if let Some(presentation) = active.presentation.get() {
                        presentation.publish_snapshot(&active, elapsed, pre_roll_remaining, status);
                    } else {
                        active.event_delivery_failure.store(true, Ordering::Release);
                        event_delivery_failed = true;
                        cleanup_failed_event_delivery(&active);
                    }
                }
                last_snapshot = Instant::now();
            }
            if !event_delivery_failed && active.event_delivery_failure.load(Ordering::Acquire) {
                event_delivery_failed = true;
                cleanup_failed_event_delivery(&active);
            }
            thread::sleep(Duration::from_millis(20));
        };

        let mut join_error = None;
        let mut quiescence_notice_published = false;
        let retirement = loop {
            let joined = self.join_worker(&active, PLAYBACK_JOIN_RETRY_INTERVAL);
            if let Err(error) = &joined {
                join_error.get_or_insert_with(|| error.clone());
            }
            let status = active
                .player
                .as_ref()
                .map(|player| player.poll_state().status)
                .or(terminal_status);
            let snapshot = active.player.as_ref().map(|player| player.snapshot());
            let worker_result = if matches!(joined, Ok(true)) {
                active
                    .player
                    .as_ref()
                    .and_then(|player| player.terminal_result())
            } else {
                None
            };
            if let Some(result) = worker_result.as_ref()
                && let Ok(mut stored) = active.terminal_result.lock()
            {
                *stored = Some(result.clone());
            }
            match decide_playback_handoff_with_result(
                &joined,
                status,
                snapshot.as_ref(),
                active.physical,
                worker_result.as_ref(),
            ) {
                PlaybackHandoffDecision::Retire(mut retirement) => {
                    active.worker_quiescent.store(true, Ordering::Release);
                    if let Some(guard) = &supervisor_lifetime_guard {
                        guard.disarm();
                    }
                    if let Some(error) = join_error.take() {
                        retirement.publication = PlaybackTerminalPublication::Failed {
                            code: "native_worker_join_failed".into(),
                            message: format!(
                                "Native worker termination could not be joined cleanly: {error}"
                            ),
                        };
                    }
                    break retirement;
                }
                PlaybackHandoffDecision::AwaitQuiescence => {
                    let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                    if !event_delivery_failed && !quiescence_notice_published {
                        let message = match &joined {
                            Err(error) => format!(
                                "Native worker join is unconfirmed ({error}); its physical reservation remains held while shutdown is retried."
                            ),
                            Ok(false) => "Waiting for native worker quiescence; physical activity remains reserved.".into(),
                            Ok(true) => unreachable!(),
                        };
                        if publish_active_playback_state(
                            &events,
                            &active,
                            PlaybackEventState::Stopping,
                            Some(message),
                            None,
                        )
                        .is_err()
                        {
                            event_delivery_failed = true;
                            cleanup_failed_event_delivery(&active);
                        }
                        quiescence_notice_published = true;
                    }
                    thread::sleep(PLAYBACK_JOIN_RETRY_INTERVAL);
                }
            }
        };

        let mut committed_retirement = retirement.clone();
        if active.physical
            && let Ok(mut trace_state) = self.sender_trace_state.lock()
        {
            trace_state.finish_physical_session(
                &active.session_id,
                &active.song_id,
                &active.title,
                &active.plan_fingerprint,
                completed_sender_trace(&active),
            );
        }
        let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
        if let Err(error) = release_terminal_ownership(
            &self.active,
            &self.last_terminal,
            &active,
            committed_retirement.clone(),
        ) {
            // Retirement failure is itself the committed terminal outcome.
            // Keep any current physical reservation/quarantine intact and do
            // not publish the worker's pre-commit success intention.
            active.stop_requested.store(true, Ordering::Release);
            committed_retirement.publication = terminal_retirement_commit_failure(
                &self.active,
                &self.last_terminal,
                &active,
                &committed_retirement,
                &error,
            );
        }
        let (terminal_state, terminal_event_state, terminal_message, terminal_outcome) =
            match &committed_retirement.publication {
                PlaybackTerminalPublication::Finished { outcome, message } => (
                    PlaybackSessionState::Finished,
                    PlaybackEventState::Finished,
                    Some(message.clone()),
                    Some(outcome.clone()),
                ),
                PlaybackTerminalPublication::Failed { message, .. } => (
                    PlaybackSessionState::Failed,
                    PlaybackEventState::Failed,
                    Some(message.clone()),
                    None,
                ),
            };
        if terminal_state == PlaybackSessionState::Failed {
            active.stop_requested.store(true, Ordering::Release);
        }
        let completed = terminal_presentation_completion(
            active.clone(),
            #[cfg(all(test, feature = "tauri-test"))]
            self.monitor_retired_test_hook.clone(),
        );
        let _ = finalize_active_presentation(
            &events,
            &active,
            &self.diagnostics_gate,
            PlaybackPresentationFinalization {
                state: terminal_event_state,
                message: terminal_message,
                outcome: terminal_outcome,
                publication: committed_retirement.publication.clone(),
            },
            completed,
        );
        // The active monitor's watcher owns the process-wide hook until the
        // supervisor returns. Tear it down before transferring hint/polling
        // ownership to the persistent post-join recovery thread.
        drop(foreground_watcher);
        if active.done.load(Ordering::Acquire)
            && !active.release_verified.load(Ordering::Acquire)
            && active
                .activity
                .quarantined_session_is_current(&active.session_id)
        {
            let _ = self
                .post_join_recovery
                .start(active.clone(), self.last_terminal.clone());
        }
    }

    pub(in crate::native_runtime) fn command(
        &self,
        method: &str,
        session_id: String,
        _events: Arc<Mutex<NativeEventHub>>,
    ) -> Result<PlaybackCommandAckDto, String> {
        let active = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?
            .clone();
        let Some(active) = active else {
            let terminal = self
                .last_terminal
                .lock()
                .ok()
                .and_then(|value| value.clone());
            if method == "playback.stop"
                && terminal
                    .as_ref()
                    .is_some_and(|terminal| terminal.session_id == session_id)
            {
                let terminal_state = terminal
                    .map(|terminal| terminal.state)
                    .unwrap_or(PlaybackSessionState::Failed);
                return Ok(PlaybackCommandAckDto {
                    accepted: true,
                    session_id,
                    state: terminal_state,
                    pending_command: None,
                    reason: None,
                });
            }
            return Err("there is no active playback session".into());
        };
        if active.session_id != session_id {
            return Err("session_id is stale or foreign".into());
        }
        let current = *active
            .state
            .lock()
            .map_err(|_| "native playback state lock poisoned".to_string())?;
        match method {
            "playback.stop" => {
                if !matches!(
                    current,
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                ) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *active
                            .pending
                            .lock()
                            .map_err(|_| "native playback control lock poisoned".to_string())?,
                        reason: None,
                    });
                }
                active.request_dry_run_terminal(DryRunTerminalRequest::Quit);
                active.stop_requested.store(true, Ordering::Release);
                *active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())? = None;
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
                if let Some(player) = &active.player {
                    player.quit()?;
                }
            }
            "playback.pause" => {
                if current != PlaybackSessionState::Playing {
                    return Err("pause requires a playing session".into());
                }
                let mut pending = active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())?;
                if *pending == Some(PlaybackPendingControl::Pause) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *pending,
                        reason: Some("already_pending".into()),
                    });
                }
                if pending.is_some() {
                    return Err("another playback control is awaiting acknowledgement".into());
                }
                *pending = Some(PlaybackPendingControl::Pause);
                drop(pending);
                if let Some(player) = &active.player {
                    if let Err(error) = player.pause() {
                        if let Ok(mut pending) = active.pending.lock() {
                            *pending = None;
                        }
                        return Err(error.to_string());
                    }
                } else {
                    *active
                        .paused_since
                        .lock()
                        .map_err(|_| "native playback pause lock poisoned".to_string())? =
                        Some(Instant::now());
                    set_playback_state(&active, PlaybackSessionState::Paused)?;
                }
            }
            "playback.resume" => {
                if current != PlaybackSessionState::Paused {
                    return Err("resume requires a paused session".into());
                }
                let mut pending = active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())?;
                if *pending == Some(PlaybackPendingControl::Resume) {
                    return Ok(PlaybackCommandAckDto {
                        accepted: true,
                        session_id,
                        state: current,
                        pending_command: *pending,
                        reason: Some("already_pending".into()),
                    });
                }
                if pending.is_some() {
                    return Err("another playback control is awaiting acknowledgement".into());
                }
                *pending = Some(PlaybackPendingControl::Resume);
                drop(pending);
                if let Some(player) = &active.player {
                    if let Err(error) = player.resume() {
                        if let Ok(mut pending) = active.pending.lock() {
                            *pending = None;
                        }
                        return Err(error.to_string());
                    }
                } else {
                    let now = Instant::now();
                    let paused_since = active
                        .paused_since
                        .lock()
                        .map_err(|_| "native playback pause lock poisoned".to_string())?
                        .take();
                    if let Some(paused_since) = paused_since {
                        *active
                            .paused_total
                            .lock()
                            .map_err(|_| "native playback pause lock poisoned".to_string())? +=
                            now.saturating_duration_since(paused_since);
                    }
                    set_playback_state(&active, PlaybackSessionState::Playing)?;
                }
            }
            "playback.skip" => {
                if !matches!(
                    current,
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                ) {
                    return Err("skip requires a live playback session".into());
                }
                *active
                    .pending
                    .lock()
                    .map_err(|_| "native playback control lock poisoned".to_string())? = None;
                if let Some(player) = &active.player {
                    player.skip()?;
                } else {
                    active.request_dry_run_terminal(DryRunTerminalRequest::Skipped);
                    active.stop_requested.store(true, Ordering::Release);
                }
            }
            _ => return Err(format!("unsupported native playback command: {method}")),
        }
        Ok(PlaybackCommandAckDto {
            accepted: true,
            session_id,
            state: *active
                .state
                .lock()
                .map_err(|_| "native playback state lock poisoned".to_string())?,
            pending_command: *active
                .pending
                .lock()
                .map_err(|_| "native playback control lock poisoned".to_string())?,
            reason: None,
        })
    }

    #[cfg(test)]
    fn invalidate_catalog(&self, generation: u64) {
        if let Ok(mut publication) = self.prepare_publication.lock() {
            self.invalidate_catalog_locked(&mut publication, generation, true);
        }
    }

    #[cfg(test)]
    fn invalidate_settings(&self) {
        if let Ok(mut publication) = self.prepare_publication.lock() {
            self.invalidate_settings_locked(&mut publication, None);
        }
    }

    pub(in crate::native_runtime) fn invalidate_catalog_locked(
        &self,
        publication: &mut PreparePublicationState,
        generation: u64,
        invalidate: bool,
    ) {
        let changed = publication
            .catalog_generation
            .is_some_and(|current| current != generation);
        publication.catalog_generation = Some(generation);
        if invalidate || changed {
            publication.source_epoch = publication.source_epoch.saturating_add(1);
            if let Some(active) = &publication.active_request {
                active.store(true, Ordering::Release);
            }
            if let Ok(mut prepared) = self.prepared.lock() {
                prepared.retain(|(_, record)| record.generation == generation);
            }
        }
    }

    pub(in crate::native_runtime) fn set_settings_fingerprint_locked(
        &self,
        publication: &mut PreparePublicationState,
        settings_fingerprint: String,
    ) {
        publication.settings_fingerprint = Some(settings_fingerprint);
    }

    pub(in crate::native_runtime) fn invalidate_settings_locked(
        &self,
        publication: &mut PreparePublicationState,
        settings_fingerprint: Option<String>,
    ) {
        #[cfg(test)]
        self.settings_invalidation_count
            .fetch_add(1, Ordering::Relaxed);
        publication.source_epoch = publication.source_epoch.saturating_add(1);
        if let Some(fingerprint) = settings_fingerprint {
            publication.settings_fingerprint = Some(fingerprint);
        }
        if let Some(active) = &publication.active_request {
            active.store(true, Ordering::Release);
        }
        if let Ok(mut prepared) = self.prepared.lock() {
            prepared.clear();
        }
    }

    pub(in crate::native_runtime) fn shutdown(&self, _events: Arc<Mutex<NativeEventHub>>) -> bool {
        self.post_join_recovery.shutdown();
        let arm_gate = self
            .startup_arm_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.startup_cancellation.store(true, Ordering::Release);
        if let Ok(mut publication) = self.prepare_publication.lock()
            && let Some(active) = publication.active_request.take()
        {
            active.store(true, Ordering::Release);
        }
        let active = self.active.lock().ok().and_then(|slot| slot.clone());
        if let Some(player) = active.as_ref().and_then(|active| active.player.as_ref()) {
            // The worker's own pre-arm Quit latch is the physical-send gate.
            let _ = player.quit();
        }
        drop(arm_gate);
        if let Some(active) = active {
            let current = active.state.lock().ok().map(|state| *state);
            if matches!(
                current,
                Some(
                    PlaybackSessionState::Starting
                        | PlaybackSessionState::Playing
                        | PlaybackSessionState::Paused
                )
            ) {
                active.request_dry_run_terminal(DryRunTerminalRequest::Quit);
                active.stop_requested.store(true, Ordering::Release);
                if let Ok(mut pending) = active.pending.lock() {
                    *pending = None;
                }
                let _ = set_playback_state(&active, PlaybackSessionState::Stopping);
            } else {
                active.request_dry_run_terminal(DryRunTerminalRequest::Quit);
                active.stop_requested.store(true, Ordering::Release);
            }
            if let Some(player) = &active.player {
                let _ = player.quit();
                let _ = player.panic_release();
            }
            let deadline = Instant::now() + PLAYBACK_SHUTDOWN_GRACE;
            while !active.done.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            return active.done.load(Ordering::Acquire)
                && active.release_verified.load(Ordering::Acquire)
                && !self.activity.physical_release_is_quarantined();
        }
        !self.activity.physical_release_is_quarantined()
    }
}

pub(in crate::native_runtime) fn target_integrity_startup_failure(
    compatibility: sky_dispatch_win32::focus::TargetIntegrityCompatibility,
) -> Option<&'static str> {
    match compatibility {
        sky_dispatch_win32::focus::TargetIntegrityCompatibility::Mismatch => Some(
            "target Sky process has higher integrity than Sky Auto Player; Windows UIPI can block SendInput",
        ),
        sky_dispatch_win32::focus::TargetIntegrityCompatibility::Compatible
        | sky_dispatch_win32::focus::TargetIntegrityCompatibility::Unknown => None,
    }
}

#[derive(Clone)]
pub(in crate::native_runtime) struct NativePlaybackServiceHandle {
    pub(in crate::native_runtime) active: Arc<Mutex<Option<Arc<NativeActivePlayback>>>>,
    pub(in crate::native_runtime) last_terminal: Arc<Mutex<Option<PlaybackTerminalStatusDto>>>,
    pub(in crate::native_runtime) sender_trace_state: Arc<Mutex<SenderTraceState>>,
    pub(in crate::native_runtime) diagnostics_gate: Arc<DiagnosticsPublicationGate>,
    pub(in crate::native_runtime) activity: ActivityCoordinator,
    pub(in crate::native_runtime) post_join_recovery: Arc<PostJoinRecoveryLifecycle>,
    #[cfg(test)]
    pub(in crate::native_runtime) monitor_join_test_hook: Option<PlaybackJoinTestHook>,
    #[cfg(all(test, feature = "tauri-test"))]
    pub(in crate::native_runtime) monitor_retired_test_hook: Option<PlaybackRetiredTestHook>,
}

impl NativePlaybackServiceHandle {
    pub(in crate::native_runtime) fn monitor(
        &self,
        active: Arc<NativeActivePlayback>,
        events: Arc<Mutex<NativeEventHub>>,
        activation: mpsc::Receiver<PlaybackActivation>,
    ) {
        let service = NativePlaybackService {
            prepared: Mutex::new(VecDeque::new()),
            active: self.active.clone(),
            last_terminal: self.last_terminal.clone(),
            sender_trace_state: self.sender_trace_state.clone(),
            diagnostics_gate: self.diagnostics_gate.clone(),
            prepare_publication: Mutex::new(PreparePublicationState::default()),
            activity: self.activity.clone(),
            post_join_recovery: Arc::clone(&self.post_join_recovery),
            startup_cancellation: Arc::new(AtomicBool::new(false)),
            startup_arm_gate: Mutex::new(()),
            #[cfg(test)]
            startup_failure: None,
            #[cfg(test)]
            prepare_compile_test_hook: Mutex::new(None),
            #[cfg(test)]
            settings_invalidation_count: AtomicU64::new(0),
            #[cfg(test)]
            prepare_before_publication_test_hook: Mutex::new(None),
            #[cfg(test)]
            prepare_before_model_test_hook: Mutex::new(None),
            #[cfg(all(test, feature = "tauri-test"))]
            monitor_progress_test_hook: None,
            #[cfg(all(test, feature = "tauri-test"))]
            presentation_test_hook: None,
            #[cfg(test)]
            monitor_join_test_hook: self.monitor_join_test_hook.clone(),
            #[cfg(all(test, feature = "tauri-test"))]
            monitor_retired_test_hook: self.monitor_retired_test_hook.clone(),
            #[cfg(test)]
            supervisor_spawn_failure: false,
            #[cfg(all(test, feature = "tauri-test"))]
            test_player: None,
            #[cfg(all(test, feature = "tauri-test"))]
            prepared_player_factory_for_test: None,
            #[cfg(all(test, feature = "tauri-test"))]
            recommended_plan_caller_capacity_override_for_test: None,
            #[cfg(all(test, feature = "tauri-test"))]
            recommendation_materialization_test_hook: Mutex::new(None),
            #[cfg(all(test, feature = "tauri-test"))]
            before_arm_test_hook: None,
        };
        service.monitor(active, events, activation);
    }
}

pub(in crate::native_runtime) fn completed_sender_trace(
    active: &NativeActivePlayback,
) -> Option<String> {
    let player = active.player.as_ref()?;
    let telemetry: Value = serde_json::from_str(&player.take_telemetry_json().ok()?).ok()?;
    sender_trace_export_json(active, &telemetry).ok()
}

pub(in crate::native_runtime) fn sender_trace_export_json(
    active: &NativeActivePlayback,
    telemetry: &Value,
) -> Result<String, serde_json::Error> {
    let records = telemetry
        .get("records")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let up_slot_records = telemetry
        .get("up_slot_records")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let up_slot_records_dropped = telemetry
        .get("up_slot_records_dropped")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let up_slot_records_expected = telemetry
        .get("up_slot_records_expected")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let up_slot_capture_mode = telemetry
        .get("up_slot_capture_mode")
        .and_then(Value::as_str)
        .unwrap_or("unavailable");
    let loss_counts = serde_json::json!({
        "observer_queue": telemetry.get("up_slot_records_dropped_by_observer_queue").and_then(Value::as_u64).unwrap_or_default(),
        "pool_storage": telemetry.get("up_slot_records_dropped_by_pool_storage").and_then(Value::as_u64).unwrap_or_default(),
        "telemetry_ring": telemetry.get("up_slot_records_dropped_by_telemetry_ring").and_then(Value::as_u64).unwrap_or_default(),
        "sidecar_capacity": telemetry.get("up_slot_records_dropped_by_sidecar_capacity").and_then(Value::as_u64).unwrap_or_default(),
        "not_captured_by_mode": telemetry.get("up_slot_records_not_captured_by_mode").and_then(Value::as_u64).unwrap_or_default(),
    });
    let up_slot_attribution_missing_records = records
        .iter()
        .filter(|record| {
            record
                .get("up_slot_attribution_missing")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    serde_json::to_string(&serde_json::json!({
        "export_schema_version": 6,
        "raw_packet_trace_status": "not_captured",
        "session_id": active.session_id,
        "song_id": active.song_id,
        "song_title": active.title,
        "plan_fingerprint": active.plan_fingerprint,
        "timing_policy": {
            "fps": active.timing_policy.fps,
            "hold_frames": active.timing_policy.hold_frames,
            "frame_period_us": active.timing_policy.frame_us,
            "frame_base_hold_us": active.timing_policy.frame_base_hold_us,
            "timing_margin_us": active.timing_policy.timing_margin_us,
            "target_hold_us": active.timing_policy.min_hold_us,
        },
        "analytics": {
            "aggregate_scope": "packet_trace_records",
            "per_key_p95_p99_supported": false,
            "per_key_p95_p99_refusal_reason": "per-slot sidecar evidence is exported as records but is not consumed by the aggregate summary",
            "up_slot_sidecar_consumed_by_summary": false,
            "up_slot_capture_mode": up_slot_capture_mode,
            "up_slot_records_expected": up_slot_records_expected,
            "up_slot_sidecar_record_count": up_slot_records,
            "up_slot_sidecar_dropped_count": up_slot_records_dropped,
            "up_slot_sidecar_loss_counts": loss_counts,
            "up_slot_sidecar_not_captured_by_mode_count": telemetry.get("up_slot_records_not_captured_by_mode").and_then(Value::as_u64).unwrap_or_default(),
            "up_slot_sidecar_accounting": "expected = captured + dropped_by_cause + not_captured_by_mode",
            "per_key_measurement_semantics": "only retained sidecar records contain per-slot evidence; packet aggregates are not expanded or imputed into per-key samples",
            "up_slot_attribution_missing_packet_count": up_slot_attribution_missing_records,
        },
        "telemetry": telemetry,
    }))
}

pub(in crate::native_runtime) fn playback_event_state(
    status: EnginePollStatus,
) -> PlaybackEventState {
    match status {
        EnginePollStatus::Ready | EnginePollStatus::Preparing | EnginePollStatus::Preroll => {
            PlaybackEventState::Starting
        }
        EnginePollStatus::Playing => PlaybackEventState::Playing,
        EnginePollStatus::Paused => PlaybackEventState::Paused,
        EnginePollStatus::Finished | EnginePollStatus::Skipped | EnginePollStatus::Quit => {
            PlaybackEventState::Finished
        }
        EnginePollStatus::SystemSuspended => PlaybackEventState::Failed,
        EnginePollStatus::Error
        | EnginePollStatus::Panicked
        | EnginePollStatus::Poisoned
        | EnginePollStatus::Invalid => PlaybackEventState::Failed,
    }
}

pub(in crate::native_runtime) fn is_terminal_status(status: EnginePollStatus) -> bool {
    matches!(
        status,
        EnginePollStatus::Finished
            | EnginePollStatus::Skipped
            | EnginePollStatus::Quit
            | EnginePollStatus::SystemSuspended
            | EnginePollStatus::Error
            | EnginePollStatus::Panicked
            | EnginePollStatus::Poisoned
            | EnginePollStatus::Invalid
    )
}

pub(in crate::native_runtime) fn is_failure_status(status: EnginePollStatus) -> bool {
    matches!(
        status,
        EnginePollStatus::Error
            | EnginePollStatus::Panicked
            | EnginePollStatus::Poisoned
            | EnginePollStatus::Invalid
    )
}

pub(in crate::native_runtime) fn terminal_success_outcome(
    status: EnginePollStatus,
) -> &'static str {
    match status {
        EnginePollStatus::Skipped => "skipped",
        EnginePollStatus::Quit => "quit",
        EnginePollStatus::Finished => "finished",
        EnginePollStatus::SystemSuspended => "system_suspend",
        _ => "finished",
    }
}

pub(in crate::native_runtime) fn cleanup_failed_event_delivery(active: &NativeActivePlayback) {
    active.request_dry_run_terminal(DryRunTerminalRequest::Quit);
    active.stop_requested.store(true, Ordering::Release);
    if !active.worker_quiescent.load(Ordering::Acquire) {
        let _ = set_playback_state(active, PlaybackSessionState::Stopping);
    }
    if let Some(player) = &active.player {
        let _ = player.panic_release();
        let _ = player.quit();
    }
}

pub(in crate::native_runtime) fn validate_prepared_id(prepared_id: &str) -> Result<(), String> {
    if prepared_id.len() != 32
        || !prepared_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid_params: prepared_id is invalid".into());
    }
    Ok(())
}

pub(in crate::native_runtime) fn validate_playback_start_request(
    request: &NativePlaybackStartRequest,
) -> Result<(), String> {
    validate_prepared_id(&request.prepared_id)?;
    if request.decisions.len() > MAX_DECISION_COUNT {
        return Err("invalid_params: decisions must be a bounded array".into());
    }
    Ok(())
}

pub(in crate::native_runtime) fn validate_public_playback_start_request(
    request: &PlaybackStartRequest,
) -> Result<(), String> {
    validate_prepared_id(&request.prepared_id)?;
    if request.decisions.len() > MAX_DECISION_COUNT {
        return Err("invalid_params: decisions must be a bounded array".into());
    }
    Ok(())
}

pub(in crate::native_runtime) fn opaque_native_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("secure native identifier generation failed: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(in crate::native_runtime) fn plan_fingerprint(
    song_id: &str,
    config: &PlaybackConfigDto,
    schedule: &ScheduleMetadata,
    policy: &MaterializedTimingPolicy,
) -> Result<String, String> {
    let chords = schedule
        .chords
        .iter()
        .map(|chord| serde_json::json!([chord.at_us, chord.scan_codes]))
        .collect::<Vec<_>>();
    // Construct every nested object with json! rather than serializing the
    // Rust DTO directly. This keeps the fingerprint field order independent
    // from the DTO declaration order.
    let payload = serde_json::json!({
        "song_id": song_id,
        "config": {
            "dry_run": config.dry_run,
            "fps": config.fps,
            "hold_frames": config.hold_frames,
            "timing_margin_us": config.timing_margin_us,
            "tempo_scale": config.tempo_scale,
        },
        "policy": {
            "fps": policy.fps,
            "frame_base_hold_us": policy.frame_base_hold_us,
            "timing_margin_us": policy.timing_margin_us,
            "min_hold_us": policy.min_hold_us,
        },
        "chords": chords,
    });
    let bytes = serde_json::to_vec(&payload).map_err(json_error)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(in crate::native_runtime) fn settings_fingerprint(
    settings: &ApplicationSettings,
) -> Result<String, String> {
    // Appearance and Auto Play do not affect the prepared schedule. The
    // settings.patch path conservatively invalidates plans for every other
    // field; keep this payload explicit so its identity is stable.
    let payload = format!(
        "{{\"fps\": {}, \"hold\": {}, \"telemetry\": {}, \"tempo\": {}, \"timing_margin_us\": {}}}",
        settings.playback_defaults.fps,
        settings.playback_defaults.hold_frames,
        settings.telemetry_enabled,
        settings.playback_defaults.tempo_scale,
        settings.playback_defaults.timing_margin_us,
    );
    let digest = Sha256::digest(payload.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub(in crate::native_runtime) fn risk_summary(risk: &RiskReport) -> RiskSummaryDto {
    RiskSummaryDto {
        level: risk.severity.clone(),
        headline: match risk.severity.as_str() {
            "low" => "Low timing risk".into(),
            "medium" => "Medium timing risk".into(),
            "high" => "High timing risk".into(),
            _ => risk.reason.clone(),
        },
        reasons: if risk.reason.is_empty() {
            Vec::new()
        } else {
            vec![risk.reason.clone()]
        },
        recommendations: risk.recommendations.clone(),
        min_same_key_slack_us: risk.min_same_key_slack_us,
        same_key_repeat_count: risk.same_key_repeat_count as u64,
        zero_slack_repeat_count: risk.zero_slack_repeat_count as u64,
    }
}

fn risk_summary_for_sheet(
    risk: &RiskReport,
    sheet_analysis: Option<&crate::commands::SheetAnalysisDto>,
    config: &PlaybackConfigDto,
) -> RiskSummaryDto {
    let mut summary = risk_summary(risk);
    let versioned = sheet_analysis.is_some_and(|analysis| analysis.schema_version.is_some());
    if versioned
        && super::super::sheet::timing_reconsent_required(
            risk,
            config.hold_frames,
            config.tempo_scale,
        )
    {
        let note = super::super::sheet::TIMING_RECONSENT_NOTE;
        if let Some(reason) = summary.reasons.first_mut() {
            reason.push(' ');
            reason.push_str(note);
        } else {
            summary.reasons.push(note.to_owned());
        }
        summary.recommendations.push(note.to_owned());
    }
    summary
}

pub(in crate::native_runtime) fn blocked_playback_dto(
    song_id: &str,
    config: PlaybackConfigDto,
    code: &str,
    message: String,
    recommended_tempo_scale: Option<f64>,
    recommended_hold_frames: Option<f64>,
) -> PreparedPlaybackDto {
    // Represent blocked preparation as a typed DTO with a stable error code,
    // rather than as a transport error or implementation-specific response.
    let recommendations = [
        recommended_tempo_scale.map(|tempo| format!("Try tempo {tempo:.2}×")),
        recommended_hold_frames.map(|hold| format!("Try hold {hold:.2} frames")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let risk = RiskSummaryDto {
        level: "high".into(),
        headline: "Playback blocked".into(),
        reasons: vec![message.clone()],
        recommendations,
        min_same_key_slack_us: None,
        same_key_repeat_count: 0,
        zero_slack_repeat_count: 0,
    };
    PreparedPlaybackDto {
        prepared_id: None,
        song: SongDetailDto {
            song_id: song_id.into(),
            title: song_id.into(),
            duration_us: 0,
            note_count: 0,
            format_label: "UNKNOWN".into(),
            risk: risk.clone(),
            recommendation: None,
            sheet_analysis: None,
        },
        config,
        admission: PlaybackAdmission::Blocked,
        risk,
        decisions: Vec::new(),
        plan_fingerprint: None,
        variants: Vec::new(),
        error_code: Some(code.into()),
        error_message: Some(message),
    }
}

pub(in crate::native_runtime) fn song_detail(
    song_id: &str,
    song: &Song,
    schedule: &ScheduleMetadata,
    risk: &RiskReport,
    sheet_analysis: Option<crate::commands::SheetAnalysisDto>,
    config: &PlaybackConfigDto,
) -> SongDetailDto {
    let versioned = sheet_analysis
        .as_ref()
        .is_some_and(|analysis| analysis.schema_version.is_some());
    let timing_reconsent_required = versioned
        && super::super::sheet::timing_reconsent_required(
            risk,
            config.hold_frames,
            config.tempo_scale,
        );
    let summary = risk
        .recommendations
        .first()
        .cloned()
        .unwrap_or_else(|| "Keep the selected settings.".into());
    let recommendation = crate::commands::PlaybackRecommendationDto {
        recommended_hold_frames: risk.suggested_hold_frames,
        recommended_tempo_scale: risk.suggested_tempo_scale,
        summary: if timing_reconsent_required {
            format!("{summary} {}", super::super::sheet::TIMING_RECONSENT_NOTE)
        } else {
            summary
        },
    };
    SongDetailDto {
        song_id: song_id.to_owned(),
        title: song.name.clone(),
        duration_us: schedule.source_duration_us,
        note_count: sheet_analysis
            .as_ref()
            .and_then(|analysis| analysis.source_note_count)
            .unwrap_or(song.notes.len() as u64),
        format_label: "SHEET".into(),
        risk: risk_summary_for_sheet(risk, sheet_analysis.as_ref(), config),
        recommendation: Some(recommendation),
        sheet_analysis,
    }
}

fn recommended_schedule(
    song: &Song,
    plan_source: Option<&super::NativeSheetSource>,
    tempo_scale: f64,
    policy: &MaterializedTimingPolicy,
) -> Result<ScheduleMetadata, String> {
    let Some(source) = plan_source else {
        return build_schedule_with_policy(song, tempo_scale, policy)
            .map_err(|error| error.to_string());
    };
    let options = super::super::sheet::planner_options(
        source.conversion_mode,
        source.target_pitch,
        tempo_scale,
    );
    match build_song_plan(&source.bytes, &source.fallback_name, &options, policy)
        .map_err(|error| error.to_string())?
    {
        SongPlan::PianoConversion(plan) => plan
            .schedule
            .ok_or_else(|| "recommended converted schedule contains no playable notes".into()),
        SongPlan::LegacyCompatibility(plan) => Ok(plan.schedule),
    }
}
