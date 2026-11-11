use super::*;
use sha2::{Digest, Sha256};

pub(in crate::native_runtime) struct NativeDispatchStartupContext<'a> {
    caller_retained_capacity_bytes: usize,
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
            player.close_window_identity_authority();
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
            startup_cancellation: Arc::new(AtomicBool::new(false)),
            startup_arm_gate: Mutex::new(()),
            #[cfg(test)]
            startup_failure: None,
            #[cfg(test)]
            startup_compile_test_hook: Mutex::new(None),
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
            before_arm_test_hook: None,
            #[cfg(all(test, feature = "tauri-test"))]
            post_compile_prepublication_test_hook: None,
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

    pub(in crate::native_runtime) fn begin_preparation(&self) -> Result<Arc<AtomicBool>, String> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let mut active = self
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare cancellation lock poisoned".to_string())?;
        if let Some(previous) = active.active_request.replace(Arc::clone(&cancelled)) {
            previous.store(true, Ordering::Release);
        }
        Ok(cancelled)
    }

    pub(in crate::native_runtime) fn finish_preparation(&self, cancelled: &Arc<AtomicBool>) {
        if let Ok(mut active) = self.prepare_publication.lock()
            && active
                .active_request
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, cancelled))
        {
            active.active_request = None;
        }
    }

    /// Snapshot source identity while holding the same short gate used by
    /// source mutation, request supersession, and cache publication. `read`
    /// may take catalog/settings locks but must not perform expensive work.
    pub(in crate::native_runtime) fn capture_preparation_sources<T>(
        &self,
        cancelled: &Arc<AtomicBool>,
        read: impl FnOnce() -> Result<(u64, String, T), String>,
    ) -> Result<(u64, T), String> {
        let mut publication = self
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        Self::ensure_current_prepare(&publication, cancelled)?;
        let (catalog_generation, settings_fingerprint, snapshot) = read()?;
        self.synchronize_prepare_source_locked(
            &mut publication,
            catalog_generation,
            settings_fingerprint,
        );
        Ok((publication.source_epoch, snapshot))
    }

    fn synchronize_prepare_source_locked(
        &self,
        publication: &mut PreparePublicationState,
        catalog_generation: u64,
        settings_fingerprint: String,
    ) {
        let catalog_changed = publication
            .catalog_generation
            .is_some_and(|current| current != catalog_generation);
        let settings_changed = publication
            .settings_fingerprint
            .as_ref()
            .is_some_and(|current| current != &settings_fingerprint);
        if catalog_changed || settings_changed {
            publication.source_epoch = publication.source_epoch.saturating_add(1);
            if settings_changed {
                #[cfg(test)]
                self.settings_invalidation_count
                    .fetch_add(1, Ordering::Relaxed);
                if let Ok(mut prepared) = self.prepared.lock() {
                    prepared.clear();
                }
            } else if catalog_changed && let Ok(mut prepared) = self.prepared.lock() {
                prepared.retain(|(_, record)| record.generation == catalog_generation);
            }
        }
        publication.catalog_generation = Some(catalog_generation);
        publication.settings_fingerprint = Some(settings_fingerprint);
    }

    fn ensure_current_prepare(
        publication: &PreparePublicationState,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        if cancelled.load(Ordering::Acquire)
            || !publication
                .active_request
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, cancelled))
        {
            return Err("playback preparation was superseded by a newer request".into());
        }
        Ok(())
    }

    fn ensure_prepare_source(
        publication: &PreparePublicationState,
        cancelled: &Arc<AtomicBool>,
        source_epoch: u64,
        generation: u64,
        settings_fingerprint: &str,
    ) -> Result<(), String> {
        Self::ensure_current_prepare(publication, cancelled)?;
        if publication.source_epoch != source_epoch
            || publication.catalog_generation != Some(generation)
            || publication.settings_fingerprint.as_deref() != Some(settings_fingerprint)
        {
            return Err("playback preparation source changed before publication".into());
        }
        Ok(())
    }

    #[cfg(test)]
    pub(in crate::native_runtime) fn prepare(
        &self,
        mut input: NativePreparedInput,
    ) -> Result<PreparedPlaybackDto, String> {
        let cancelled = self.begin_preparation()?;
        let captured = self.capture_preparation_sources(&cancelled, || {
            Ok((input.generation, input.settings_fingerprint.clone(), ()))
        });
        let result = match captured {
            Ok((source_epoch, ())) => {
                input.source_epoch = source_epoch;
                self.prepare_with_token(input, &cancelled)
            }
            Err(error) => Err(error),
        };
        self.finish_preparation(&cancelled);
        result
    }

    pub(in crate::native_runtime) fn prepare_with_token(
        &self,
        input: NativePreparedInput,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<PreparedPlaybackDto, String> {
        let result = self.prepare_with_cancel(input, cancelled);
        self.finish_preparation(cancelled);
        result
    }

    fn prepare_with_cancel(
        &self,
        input: NativePreparedInput,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<PreparedPlaybackDto, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("playback preparation was superseded by a newer request".into());
        }
        let NativePreparedInput {
            song_id,
            generation,
            config,
            song,
            sheet_analysis,
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation,
            settings_fingerprint,
            publication_fingerprint,
            source_path,
            source_hash,
            plan_identity,
            plan_source,
            source_epoch,
        } = input;
        if sheet_analysis.as_ref().is_some_and(|analysis| {
            analysis.readiness == crate::commands::SheetReadinessDto::RequiresConfirmation
                && !analysis.confirmation_received
        }) {
            self.validate_prepare_source(
                cancelled,
                source_epoch,
                generation,
                &publication_fingerprint,
            )?;
            let detail = song_detail(
                &song_id,
                &song,
                &schedule,
                &risk,
                sheet_analysis.clone(),
                &config,
            );
            let risk_summary = risk_summary_for_sheet(&risk, sheet_analysis.as_ref(), &config);
            return Ok(PreparedPlaybackDto {
                prepared_id: None,
                song: detail,
                config,
                admission: PlaybackAdmission::ConfirmationRequired,
                risk: risk_summary,
                decisions: Vec::new(),
                plan_fingerprint: None,
                variants: Vec::new(),
                error_code: Some("conversion_confirmation_required".into()),
                error_message: Some(
                    "Review the listed conversion losses and explicitly accept them before preparing playback.".into(),
                ),
            });
        }
        let detail = song_detail(
            &song_id,
            &song,
            &schedule,
            &risk,
            sheet_analysis.clone(),
            &config,
        );
        if !config.dry_run && schedule.impossible_same_key_repeats > 0 {
            let message = format!(
                "Detected {} infeasible same-key repeat(s): the authored interval is shorter than the configured hold.",
                schedule.impossible_same_key_repeats
            );
            self.validate_prepare_source(
                cancelled,
                source_epoch,
                generation,
                &publication_fingerprint,
            )?;
            let mut blocked = blocked_playback_dto(
                &song_id,
                config,
                "validation_failed",
                message,
                schedule.recommended_tempo_scale,
                schedule.recommended_hold_frames,
            );
            blocked.song = detail;
            return Ok(blocked);
        }
        let fingerprint_song_id = format!("{song_id}\0{plan_identity}");
        let fingerprint =
            plan_fingerprint(&fingerprint_song_id, &config, &schedule, &timing_policy)?;
        let timing_advisory = self.analyze_timing_variant(
            &schedule,
            &config,
            &timing_policy,
            &fingerprint,
            cancelled,
        );
        if cancelled.load(Ordering::Acquire) {
            return Err("playback preparation was superseded by a newer request".into());
        }
        let admission = if risk.severity == "low" {
            PlaybackAdmission::Ready
        } else {
            PlaybackAdmission::ConfirmationRequired
        };
        let prepared_id = opaque_native_id()?;
        let base_variant = NativePlaybackVariant {
            config: config.clone(),
            schedule: schedule.clone(),
            fingerprint: fingerprint.clone(),
            timing_policy: timing_policy.clone(),
            timing_margin_recommendation: timing_margin_recommendation.clone(),
        };
        let mut decisions = Vec::new();
        let mut variants = HashMap::from([(PlaybackDecision::Proceed, base_variant)]);
        let mut variant_dtos = vec![PlaybackPlanVariantDto {
            decision: PlaybackDecision::Proceed,
            config: config.clone(),
            plan_fingerprint: fingerprint.clone(),
            timing_advisory: Some(timing_advisory),
        }];
        if risk.severity != "low" {
            decisions.push(RiskDecisionDto {
                decision: PlaybackDecision::Proceed,
                label: "Proceed with current settings".into(),
            });
            if plan_source.is_none()
                && let (Some(hold_frames), Some(tempo_scale)) =
                    (risk.suggested_hold_frames, risk.suggested_tempo_scale)
                && (hold_frames != config.hold_frames || tempo_scale != config.tempo_scale)
                && let Ok(recommended_policy) = MaterializedTimingPolicy::from_user_margin(
                    config.fps,
                    hold_frames,
                    config.timing_margin_us,
                )
                && let Ok(recommended_schedule) = recommended_schedule(
                    &song,
                    plan_source.as_ref(),
                    tempo_scale,
                    &recommended_policy,
                )
                && (config.dry_run || recommended_schedule.impossible_same_key_repeats == 0)
            {
                let recommended_config = PlaybackConfigDto {
                    hold_frames,
                    timing_margin_us: config.timing_margin_us,
                    tempo_scale,
                    fps: config.fps,
                    dry_run: config.dry_run,
                };
                let recommended_fingerprint = plan_fingerprint(
                    &fingerprint_song_id,
                    &recommended_config,
                    &recommended_schedule,
                    &recommended_policy,
                )?;
                let recommended_timing_advisory = self.analyze_timing_variant(
                    &recommended_schedule,
                    &recommended_config,
                    &recommended_policy,
                    &recommended_fingerprint,
                    cancelled,
                );
                if cancelled.load(Ordering::Acquire) {
                    return Err("playback preparation was superseded by a newer request".into());
                }
                variants.insert(
                    PlaybackDecision::UseRecommended,
                    NativePlaybackVariant {
                        config: recommended_config.clone(),
                        schedule: recommended_schedule,
                        fingerprint: recommended_fingerprint.clone(),
                        timing_policy: recommended_policy,
                        timing_margin_recommendation: timing_margin_recommendation.clone(),
                    },
                );
                variant_dtos.push(PlaybackPlanVariantDto {
                    decision: PlaybackDecision::UseRecommended,
                    config: recommended_config,
                    plan_fingerprint: recommended_fingerprint,
                    timing_advisory: Some(recommended_timing_advisory),
                });
                decisions.push(RiskDecisionDto {
                    decision: PlaybackDecision::UseRecommended,
                    label: "Use recommended settings".into(),
                });
            }
            if !config.dry_run {
                let dry_run_config = PlaybackConfigDto {
                    dry_run: true,
                    ..config.clone()
                };
                let dry_run_fingerprint = plan_fingerprint(
                    &fingerprint_song_id,
                    &dry_run_config,
                    &schedule,
                    &timing_policy,
                )?;
                let dry_run_timing_advisory = self.analyze_timing_variant(
                    &schedule,
                    &dry_run_config,
                    &timing_policy,
                    &dry_run_fingerprint,
                    cancelled,
                );
                if cancelled.load(Ordering::Acquire) {
                    return Err("playback preparation was superseded by a newer request".into());
                }
                variants.insert(
                    PlaybackDecision::DryRun,
                    NativePlaybackVariant {
                        config: dry_run_config.clone(),
                        schedule: schedule.clone(),
                        fingerprint: dry_run_fingerprint.clone(),
                        timing_policy: timing_policy.clone(),
                        timing_margin_recommendation: timing_margin_recommendation.clone(),
                    },
                );
                variant_dtos.push(PlaybackPlanVariantDto {
                    decision: PlaybackDecision::DryRun,
                    config: dry_run_config,
                    plan_fingerprint: dry_run_fingerprint,
                    timing_advisory: Some(dry_run_timing_advisory),
                });
                decisions.push(RiskDecisionDto {
                    decision: PlaybackDecision::DryRun,
                    label: "Run a dry-run first".into(),
                });
            }
        }
        let prepared_risk = risk_summary_for_sheet(&risk, sheet_analysis.as_ref(), &config);
        let dto = PreparedPlaybackDto {
            prepared_id: Some(prepared_id.clone()),
            song: detail,
            config,
            admission,
            risk: prepared_risk,
            decisions,
            plan_fingerprint: Some(fingerprint.clone()),
            variants: variant_dtos,
            error_code: None,
            error_message: None,
        };
        #[cfg(test)]
        let publication_hook = self
            .prepare_before_publication_test_hook
            .lock()
            .map_err(|_| "Prepare test hook lock poisoned".to_string())?
            .take();
        #[cfg(test)]
        if let Some(hook) = publication_hook {
            hook();
        }
        let publication = self
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        Self::ensure_prepare_source(
            &publication,
            cancelled,
            source_epoch,
            generation,
            &publication_fingerprint,
        )?;
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        prepared.push_back((
            prepared_id,
            NativePreparedPlan {
                song_id,
                generation,
                settings_fingerprint,
                song,
                source_path,
                source_hash,
                dto: dto.clone(),
                variants,
            },
        ));
        retain_prepared_capacity(&mut prepared);
        Ok(dto)
    }

    pub(in crate::native_runtime) fn invalidate_prepared_for_selection(
        &self,
    ) -> Result<(), String> {
        let mut publication = self
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        publication.source_epoch = publication.source_epoch.saturating_add(1);
        if let Some(active) = publication.active_request.take() {
            active.store(true, Ordering::Release);
        }
        self.prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?
            .clear();
        Ok(())
    }

    pub(in crate::native_runtime) fn prepared_source_identity(
        &self,
        prepared_id: &str,
    ) -> Result<Option<(PathBuf, String)>, String> {
        let prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        Ok(prepared
            .iter()
            .find(|(id, _)| id == prepared_id)
            .map(|(_, record)| (record.source_path.clone(), record.source_hash.clone())))
    }

    pub(in crate::native_runtime) fn invalidate_prepared_source(
        &self,
        prepared_id: &str,
    ) -> Result<(), String> {
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        prepared.retain(|(id, _)| id != prepared_id);
        Ok(())
    }

    fn validate_prepare_source(
        &self,
        cancelled: &Arc<AtomicBool>,
        source_epoch: u64,
        generation: u64,
        settings_fingerprint: &str,
    ) -> Result<(), String> {
        let publication = self
            .prepare_publication
            .lock()
            .map_err(|_| "native Prepare publication lock poisoned".to_string())?;
        Self::ensure_prepare_source(
            &publication,
            cancelled,
            source_epoch,
            generation,
            settings_fingerprint,
        )
    }

    fn analyze_timing_variant(
        &self,
        schedule: &ScheduleMetadata,
        config: &PlaybackConfigDto,
        policy: &MaterializedTimingPolicy,
        fingerprint: &str,
        cancelled: &AtomicBool,
    ) -> crate::commands::WholeStreamTimingAdvisoryDto {
        #[cfg(test)]
        {
            let hook = self
                .prepare_before_model_test_hook
                .lock()
                .ok()
                .and_then(|mut hook| hook.take());
            timing_advisory::analyze_variant_with_model_hook(
                schedule,
                config,
                policy,
                fingerprint,
                cancelled,
                move || {
                    if let Some(hook) = hook {
                        hook();
                    }
                },
            )
        }
        #[cfg(not(test))]
        timing_advisory::analyze_variant(schedule, config, policy, fingerprint, cancelled)
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
        let other_prepared_capacity_bytes =
            prepared_cache_retained_capacity_bytes(&prepared, Some(prepared_index))?;
        let caller_retained_capacity_bytes = native_start_caller_retained_capacity_bytes(
            selected_variant,
            &record.song_id,
            &record.song.name,
            &request.prepared_id,
            &session_id,
            other_prepared_capacity_bytes,
        )?;
        let record = prepared
            .remove(prepared_index)
            .expect("prepared record was found above")
            .1;
        drop(prepared);
        let NativePreparedPlan {
            song_id,
            song,
            mut variants,
            ..
        } = record;
        let song_title = song.name;
        drop(song.notes);
        let variant = variants
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
                caller_retained_capacity_bytes,
                startup_cancellation: &startup_cancellation,
                event_delivery_failure: &event_delivery_failure,
            };
            self.create_native_player(
                &variant.schedule,
                &variant.config,
                &variant.timing_policy,
                settings,
                &startup_context,
            )
            .and_then(|(player, target)| {
                let result: Result<_, String> = (|| {
                    #[cfg(all(test, feature = "tauri-test"))]
                    if self.test_player.is_some() {
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
                    total_us: variant.schedule.duration_us,
                    config: variant.config.clone(),
                    timing_policy: variant.timing_policy.clone(),
                    timing_margin_recommendation: variant.timing_margin_recommendation.clone(),
                    plan_fingerprint: variant.fingerprint.clone(),
                    physical: true,
                    activity_lease: Mutex::new(activity_lease),
                    target_hwnd: None,
                    state: Mutex::new(PlaybackSessionState::Starting),
                    pending: Mutex::new(None),
                    player: None,
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
                        let _ = release_terminal_resources(&failed_active, &retirement);
                        let completed = terminal_presentation_completion(
                            self.active.clone(),
                            self.last_terminal.clone(),
                            Arc::clone(&failed_active),
                            retirement.clone(),
                            PlaybackSessionState::Failed,
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
                let _ = release_terminal_resources(&failed_active, &retirement);
                let completed = terminal_presentation_completion(
                    self.active.clone(),
                    self.last_terminal.clone(),
                    Arc::clone(&failed_active),
                    retirement.clone(),
                    PlaybackSessionState::Failed,
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
            total_us: variant.schedule.duration_us,
            config: variant.config.clone(),
            timing_policy: variant.timing_policy.clone(),
            timing_margin_recommendation: variant.timing_margin_recommendation.clone(),
            plan_fingerprint: variant.fingerprint.clone(),
            physical: player.is_some(),
            activity_lease: Mutex::new(activity_lease),
            target_hwnd,
            state: Mutex::new(PlaybackSessionState::Starting),
            pending: Mutex::new(None),
            player,
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
                let _ = release_terminal_resources(&active, &retirement);
                let completed = terminal_presentation_completion(
                    self.active.clone(),
                    self.last_terminal.clone(),
                    active.clone(),
                    retirement.clone(),
                    PlaybackSessionState::Failed,
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
            let _ = release_terminal_resources(&active, &retirement);
            let completed = terminal_presentation_completion(
                self.active.clone(),
                self.last_terminal.clone(),
                active.clone(),
                retirement.clone(),
                PlaybackSessionState::Failed,
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
            let _ = release_terminal_resources(&active, &retirement);
            let completed = terminal_presentation_completion(
                self.active.clone(),
                self.last_terminal.clone(),
                active.clone(),
                retirement.clone(),
                PlaybackSessionState::Failed,
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
            #[cfg(test)]
            monitor_join_test_hook: self.monitor_join_test_hook.clone(),
            #[cfg(all(test, feature = "tauri-test"))]
            monitor_retired_test_hook: self.monitor_retired_test_hook.clone(),
        }
    }

    pub(in crate::native_runtime) fn create_native_player(
        &self,
        schedule: &ScheduleMetadata,
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
        let runtime_schedule = {
            #[cfg(test)]
            {
                let hook = self
                    .startup_compile_test_hook
                    .lock()
                    .ok()
                    .and_then(|hook| hook.clone());
                if let Some(hook) = hook {
                    compile_dispatch_schedule_with_checkpoint(
                        schedule,
                        startup_context.caller_retained_capacity_bytes,
                        startup_context.startup_cancellation,
                        |index| hook(index),
                    )?
                } else {
                    compile_dispatch_schedule_with_prepared_capacity(
                        schedule,
                        startup_context.caller_retained_capacity_bytes,
                        startup_context.startup_cancellation,
                    )?
                }
            }
            #[cfg(not(test))]
            {
                compile_dispatch_schedule_with_prepared_capacity(
                    schedule,
                    startup_context.caller_retained_capacity_bytes,
                    startup_context.startup_cancellation,
                )?
            }
        };
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(hook) = &self.post_compile_prepublication_test_hook {
            hook();
        }
        if startup_context.startup_cancellation.load(Ordering::Acquire) {
            return Err("native playback startup was cancelled during shutdown".into());
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
        let player = Arc::new(NativeDispatchSession::new_with_power_endpoint(
            NativeSessionOptions {
                schedule: runtime_schedule,
                #[cfg(feature = "tauri-test")]
                startup_calibration_placement:
                    sky_player::engine::StartupCalibrationPlacement::BeforeSchedulingGuards,
                #[cfg(feature = "tauri-test")]
                timing_observer_for_qualification: false,
                #[cfg(feature = "tauri-test")]
                modifier_key_state_query_for_test: None,
                backend: BackendConfig::Production,
                profile: DispatchProfile::Production,
                timing: TimingOptions {
                    game_fps: config.fps,
                    min_hold_us: policy.min_hold_us,
                    min_release_gap_us: policy.min_release_gap_us,
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
                    guard_mode: sky_player::engine::WaitGuardMode::default(),
                    #[cfg(feature = "tauri-test")]
                    test_spin_threshold_us: None,
                    #[cfg(feature = "tauri-test")]
                    test_wait_policy: sky_player::engine::TestWaitPolicy::LegacyTestWideSpin,
                },
                telemetry: TelemetryOptions {
                    mode: TelemetryMode::Ring,
                    // Keep fixed-width schema 19 records inside the bounded
                    // sender-trace capture budget before export.
                    capacity: SENDER_TRACE_CAPTURE_CAPACITY,
                },
                priority: PriorityOptions {
                    mode: PriorityMode::Auto,
                },
                instrument_key_profile: None,
                #[cfg(feature = "tauri-test")]
                startup_ordering_hook: None,
                #[cfg(feature = "tauri-test")]
                restore_race_hook: None,
                #[cfg(feature = "tauri-test")]
                focus_pause_hook: None,
                #[cfg(feature = "tauri-test")]
                timer_lifecycle_context: None,
                #[cfg(feature = "tauri-test")]
                prepared_packet_ambiguity_mask: None,
                #[cfg(feature = "tauri-test")]
                preflight_user_held_mask: None,
            },
            system_power_endpoint()?,
        )?);
        player.set_target_hwnd(target);
        player.bind_window_identity_authority(owner_authority)?;
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
                let _ = release_terminal_resources(&active, &retirement);
                let completed = terminal_presentation_completion(
                    self.active.clone(),
                    self.last_terminal.clone(),
                    active.clone(),
                    retirement.clone(),
                    PlaybackSessionState::Failed,
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
                // The supervisor owns only a transition hint. The worker
                // still performs its fresh exact-HWND final admission.
                let focused = sky_dispatch_win32::focus::foreground_window_matches(target);
                player.set_focus_hint(focused);
            }
            if last_identity_check.elapsed() >= Duration::from_millis(100) {
                if let Some(player) = &active.player
                    && !player.poll_window_identity_authority()
                {
                    identity_authority_invalidated = true;
                    let _ = player.panic_release();
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
            match decide_playback_handoff(&joined, status, snapshot.as_ref(), active.physical) {
                PlaybackHandoffDecision::Retire(mut retirement) => {
                    active.worker_quiescent.store(true, Ordering::Release);
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

        let (terminal_state, terminal_event_state, terminal_message, terminal_outcome) =
            match &retirement.publication {
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
        let mut retirement_notice_published = false;
        loop {
            match release_terminal_resources(&active, &retirement) {
                Ok(()) => break,
                Err(error) => {
                    if !event_delivery_failed && !retirement_notice_published {
                        retirement_notice_published = publish_active_playback_state(
                            &events,
                            &active,
                            PlaybackEventState::Stopping,
                            Some(format!(
                                "The native worker is quiescent, but session retirement is still pending ({error})."
                            )),
                            None,
                        )
                        .is_ok();
                        if !retirement_notice_published {
                            event_delivery_failed = true;
                        }
                    }
                    thread::sleep(PLAYBACK_JOIN_RETRY_INTERVAL);
                }
            }
        }
        let completed = terminal_presentation_completion(
            self.active.clone(),
            self.last_terminal.clone(),
            active.clone(),
            retirement.clone(),
            terminal_state,
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
                publication: retirement.publication.clone(),
            },
            completed,
        );
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
        let arm_gate = self
            .startup_arm_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.startup_cancellation.store(true, Ordering::Release);
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
            startup_cancellation: Arc::new(AtomicBool::new(false)),
            startup_arm_gate: Mutex::new(()),
            #[cfg(test)]
            startup_failure: None,
            #[cfg(test)]
            startup_compile_test_hook: Mutex::new(None),
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
            before_arm_test_hook: None,
            #[cfg(all(test, feature = "tauri-test"))]
            post_compile_prepublication_test_hook: None,
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
        "export_schema_version": 4,
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
            "release_gap_us": active.timing_policy.min_release_gap_us,
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

pub(in crate::native_runtime) fn retain_prepared_capacity<T>(prepared: &mut VecDeque<T>) {
    while prepared.len() > MAX_PREPARED_PLANS {
        prepared.pop_front();
    }
}

pub(in crate::native_runtime) fn plan_fingerprint(
    song_id: &str,
    config: &PlaybackConfigDto,
    schedule: &ScheduleMetadata,
    policy: &MaterializedTimingPolicy,
) -> Result<String, String> {
    let actions = schedule
        .actions
        .iter()
        .map(|action| {
            serde_json::json!([
                match action.kind {
                    ActionKind::Down => "down",
                    ActionKind::Up => "up",
                },
                action.at_us,
                action.scan_codes,
            ])
        })
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
            "min_release_gap_us": policy.min_release_gap_us,
        },
        "actions": actions,
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

pub(super) fn compile_dispatch_schedule(
    schedule: &ScheduleMetadata,
    cancelled: &AtomicBool,
) -> Result<sky_player::adapter_support::RuntimeSchedule, String> {
    compile_dispatch_schedule_with_checkpoint(schedule, 0, cancelled, |_| {})
}

fn compile_dispatch_schedule_with_checkpoint(
    schedule: &ScheduleMetadata,
    caller_retained_capacity_bytes: usize,
    cancelled: &AtomicBool,
    mut checkpoint: impl FnMut(usize),
) -> Result<sky_player::adapter_support::RuntimeSchedule, String> {
    if cancelled.load(Ordering::Acquire) {
        return Err("native schedule compilation cancelled".into());
    }
    if schedule.actions.len() > sky_player::adapter_support::MAX_COMPILED_ACTIONS {
        return Err("native schedule exceeds compiler action limit".into());
    }
    let mut actions = Vec::new();
    actions
        .try_reserve(schedule.actions.len())
        .map_err(|error| format!("native action allocation failed: {error}"))?;
    for (index, action) in schedule.actions.iter().enumerate() {
        if index.is_multiple_of(256) && cancelled.load(Ordering::Acquire) {
            return Err("native schedule compilation cancelled".into());
        }
        if action.scan_codes.is_empty()
            || action.scan_codes.len() > sky_app_core::song::SKY_SCAN_CODES.len()
        {
            return Err(format!("native action {index} has invalid scan code count"));
        }
        if action.reason.len() > sky_player::adapter_support::MAX_COMPILED_REASON_BYTES {
            return Err(format!(
                "native action {index} reason exceeds compiler limit"
            ));
        }
        let mut scan_codes = SmallVec::<[u16; 4]>::new();
        scan_codes
            .try_reserve(action.scan_codes.len())
            .map_err(|error| format!("native scan code allocation failed: {error}"))?;
        scan_codes.extend_from_slice(&action.scan_codes);
        actions.push(KeyActionInput {
            source_action_index: u32::try_from(index)
                .map_err(|_| "native action index overflow")?,
            kind: match action.kind {
                ActionKind::Down => DispatchActionKind::Down,
                ActionKind::Up => DispatchActionKind::Up,
            },
            scheduled_us: action.at_us,
            scan_codes,
            reason: Arc::<str>::from(action.reason.as_str()),
        });
        if (index + 1).is_multiple_of(256) {
            checkpoint(index + 1);
        }
    }
    let caller_retained_capacity_bytes = caller_retained_capacity_bytes
        .checked_add(schedule_metadata_heap_capacity_bytes(schedule)?)
        .ok_or_else(|| "native caller-retained startup capacity overflow".to_string())?;
    sky_player::adapter_support::compile_runtime_intents_with_memory_accounting(
        &actions,
        &sky_app_core::song::SKY_SCAN_CODES,
        actions.capacity(),
        caller_retained_capacity_bytes,
        || cancelled.load(Ordering::Acquire),
    )
    .map_err(|error| format!("native schedule compilation failed: {error}"))
}

fn compile_dispatch_schedule_with_prepared_capacity(
    schedule: &ScheduleMetadata,
    caller_retained_capacity_bytes: usize,
    cancelled: &AtomicBool,
) -> Result<sky_player::adapter_support::RuntimeSchedule, String> {
    compile_dispatch_schedule_with_checkpoint(
        schedule,
        caller_retained_capacity_bytes,
        cancelled,
        |_| {},
    )
}

fn checked_capacity_bytes(count: usize, element_size: usize) -> Result<usize, String> {
    count
        .checked_mul(element_size)
        .ok_or_else(|| "native startup capacity size overflow".to_string())
}

fn checked_capacity_sum(values: impl IntoIterator<Item = usize>) -> Result<usize, String> {
    values.into_iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value)
            .ok_or_else(|| "native startup capacity sum overflow".to_string())
    })
}

/// Heap payload retained by one source schedule. Struct headers are counted
/// by the containing prepared variant; allocator metadata is excluded.
fn schedule_metadata_heap_capacity_bytes(schedule: &ScheduleMetadata) -> Result<usize, String> {
    let action_headers = checked_capacity_bytes(
        schedule.actions.capacity(),
        std::mem::size_of::<sky_app_core::song::KeyAction>(),
    )?;
    let action_payloads = schedule.actions.iter().try_fold(0usize, |total, action| {
        let scan_codes =
            checked_capacity_bytes(action.scan_codes.capacity(), std::mem::size_of::<u16>())?;
        total
            .checked_add(scan_codes)
            .and_then(|bytes| bytes.checked_add(action.reason.capacity()))
            .ok_or_else(|| "native action source capacity overflow".to_string())
    })?;
    let warning_headers =
        checked_capacity_bytes(schedule.warnings.capacity(), std::mem::size_of::<String>())?;
    let warning_payloads = schedule
        .warnings
        .iter()
        .try_fold(0usize, |total, warning| {
            total
                .checked_add(warning.capacity())
                .ok_or_else(|| "native warning source capacity overflow".to_string())
        })?;
    checked_capacity_sum([
        action_headers,
        action_payloads,
        warning_headers,
        warning_payloads,
    ])
}

fn native_start_caller_retained_capacity_bytes(
    variant: &NativePlaybackVariant,
    song_id: &String,
    song_title: &String,
    prepared_id: &String,
    session_id: &String,
    other_prepared_capacity_bytes: usize,
) -> Result<usize, String> {
    // The selected source schedule is counted by the compiler adapter. This
    // covers the variant header and its other heap strings, the complete
    // remaining prepared-plan cache payload, and current plus subsequently
    // cloned active-session identifiers.
    let copied_session_strings = checked_capacity_sum([
        song_id.capacity(),
        song_title.capacity(),
        song_id.capacity(),
        song_title.capacity(),
        song_id.capacity(),
        prepared_id.capacity(),
        prepared_id.capacity(),
        prepared_id.capacity(),
        session_id.capacity(),
        session_id.capacity(),
        variant.fingerprint.capacity(),
        variant.fingerprint.capacity(),
    ])?;
    checked_capacity_sum([
        std::mem::size_of::<NativePlaybackVariant>(),
        variant.timing_margin_recommendation.source.capacity(),
        copied_session_strings,
        other_prepared_capacity_bytes,
    ])
}

fn optional_string_capacity(value: Option<&String>) -> usize {
    value.map_or(0, String::capacity)
}

fn string_vec_capacity_bytes(values: &[String], capacity: usize) -> Result<usize, String> {
    let headers = checked_capacity_bytes(capacity, std::mem::size_of::<String>())?;
    let payloads = values.iter().try_fold(0usize, |total, value| {
        total
            .checked_add(value.capacity())
            .ok_or_else(|| "prepared string-vector capacity overflow".to_string())
    })?;
    checked_capacity_sum([headers, payloads])
}

fn risk_summary_capacity_bytes(risk: &crate::commands::RiskSummaryDto) -> Result<usize, String> {
    checked_capacity_sum([
        risk.level.capacity(),
        risk.headline.capacity(),
        string_vec_capacity_bytes(&risk.reasons, risk.reasons.capacity())?,
        string_vec_capacity_bytes(&risk.recommendations, risk.recommendations.capacity())?,
    ])
}

fn sheet_analysis_capacity_bytes(
    analysis: Option<&crate::commands::SheetAnalysisDto>,
) -> Result<usize, String> {
    let Some(analysis) = analysis else {
        return Ok(0);
    };
    let track_headers = checked_capacity_bytes(
        analysis.tracks.capacity(),
        std::mem::size_of::<crate::commands::SheetTrackDto>(),
    )?;
    let track_strings = analysis.tracks.iter().try_fold(0usize, |total, track| {
        total
            .checked_add(optional_string_capacity(track.instrument.as_ref()))
            .and_then(|total| {
                total.checked_add(optional_string_capacity(track.effective_pitch.as_ref()))
            })
            .ok_or_else(|| "prepared sheet-track capacity overflow".to_string())
    })?;
    checked_capacity_sum([
        analysis.format_label.capacity(),
        track_headers,
        track_strings,
        string_vec_capacity_bytes(&analysis.loss_reasons, analysis.loss_reasons.capacity())?,
        optional_string_capacity(analysis.error_message.as_ref()),
        optional_string_capacity(analysis.confirmation_fingerprint.as_ref()),
    ])
}

fn whole_stream_advisory_capacity_bytes(
    advisory: Option<&crate::commands::WholeStreamTimingAdvisoryDto>,
) -> Result<usize, String> {
    let Some(advisory) = advisory else {
        return Ok(0);
    };
    checked_capacity_sum([
        advisory.plan_fingerprint.capacity(),
        advisory.explanation.capacity(),
    ])
}

fn prepared_dto_capacity_bytes(
    dto: &crate::commands::PreparedPlaybackDto,
) -> Result<usize, String> {
    let song = &dto.song;
    let mut total = checked_capacity_sum([
        optional_string_capacity(dto.prepared_id.as_ref()),
        song.song_id.capacity(),
        song.title.capacity(),
        song.format_label.capacity(),
        risk_summary_capacity_bytes(&song.risk)?,
        sheet_analysis_capacity_bytes(song.sheet_analysis.as_ref())?,
        risk_summary_capacity_bytes(&dto.risk)?,
        checked_capacity_bytes(
            dto.decisions.capacity(),
            std::mem::size_of::<crate::commands::RiskDecisionDto>(),
        )?,
        optional_string_capacity(dto.plan_fingerprint.as_ref()),
        checked_capacity_bytes(
            dto.variants.capacity(),
            std::mem::size_of::<crate::commands::PlaybackPlanVariantDto>(),
        )?,
        optional_string_capacity(dto.error_code.as_ref()),
        optional_string_capacity(dto.error_message.as_ref()),
    ])?;
    if let Some(recommendation) = &song.recommendation {
        total = total
            .checked_add(recommendation.summary.capacity())
            .ok_or_else(|| "prepared recommendation capacity overflow".to_string())?;
    }
    for decision in &dto.decisions {
        total = total
            .checked_add(decision.label.capacity())
            .ok_or_else(|| "prepared decision capacity overflow".to_string())?;
    }
    for variant in &dto.variants {
        let advisory_bytes =
            whole_stream_advisory_capacity_bytes(variant.timing_advisory.as_ref())?;
        total = checked_capacity_sum([total, variant.plan_fingerprint.capacity(), advisory_bytes])?;
    }
    Ok(total)
}

fn prepared_plan_capacity_bytes(id: &String, record: &NativePreparedPlan) -> Result<usize, String> {
    let note_headers = checked_capacity_bytes(
        record.song.notes.capacity(),
        std::mem::size_of::<sky_app_core::song::Note>(),
    )?;
    let note_keys = record.song.notes.iter().try_fold(0usize, |total, note| {
        total
            .checked_add(note.key.capacity())
            .ok_or_else(|| "prepared song-key capacity overflow".to_string())
    })?;
    let variant_entry_size =
        std::mem::size_of::<(crate::commands::PlaybackDecision, NativePlaybackVariant)>();
    let variant_bucket_count = if record.variants.capacity() == 0 {
        0
    } else {
        record
            .variants
            .capacity()
            .checked_mul(8)
            .and_then(|value| value.checked_add(6))
            .map(|value| value / 7)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| "prepared variant-map capacity overflow".to_string())?
    };
    let variant_map_bytes = if variant_bucket_count == 0 {
        0
    } else {
        checked_capacity_bytes(variant_bucket_count, variant_entry_size.saturating_add(1))?
            .checked_add(16 + std::mem::align_of::<usize>() - 1)
            .ok_or_else(|| "prepared variant-map bytes overflow".to_string())?
    };
    let mut variant_payloads = 0usize;
    for variant in record.variants.values() {
        variant_payloads = checked_capacity_sum([
            variant_payloads,
            variant.fingerprint.capacity(),
            variant.timing_margin_recommendation.source.capacity(),
            schedule_metadata_heap_capacity_bytes(&variant.schedule)?,
        ])?;
    }
    checked_capacity_sum([
        id.capacity(),
        record.song_id.capacity(),
        record.settings_fingerprint.capacity(),
        record.song.name.capacity(),
        note_headers,
        note_keys,
        record.source_path.capacity(),
        record.source_hash.capacity(),
        variant_map_bytes,
        variant_payloads,
        prepared_dto_capacity_bytes(&record.dto)?,
    ])
}

fn prepared_cache_retained_capacity_bytes(
    prepared: &std::collections::VecDeque<(String, NativePreparedPlan)>,
    excluded_index: Option<usize>,
) -> Result<usize, String> {
    let queue_buffer_bytes = checked_capacity_bytes(
        prepared.capacity(),
        std::mem::size_of::<(String, NativePreparedPlan)>(),
    )?;
    prepared
        .iter()
        .enumerate()
        .filter(|(index, _)| Some(*index) != excluded_index)
        .try_fold(0usize, |total, (_, (id, record))| {
            let plan_bytes = prepared_plan_capacity_bytes(id, record)?;
            total
                .checked_add(plan_bytes)
                .ok_or_else(|| "prepared-cache retained capacity overflow".to_string())
        })
        .and_then(|record_bytes| {
            queue_buffer_bytes
                .checked_add(record_bytes)
                .ok_or_else(|| "prepared-cache queue capacity overflow".to_string())
        })
}

#[cfg(test)]
mod preparation_cancellation_tests {
    use super::super::{
        NativePreparedInput, analyze_schedule_with_context, build_schedule_with_policy,
    };
    use super::{
        NativePlaybackService, compile_dispatch_schedule_with_checkpoint,
        schedule_metadata_heap_capacity_bytes,
    };
    use crate::app_state::ActivityCoordinator;
    use crate::commands::{PlaybackConfigDto, TimingMarginRecommendationDto};
    use crate::native_runtime::NativeEventHub;
    use sky_app_core::settings::ApplicationSettings;
    use sky_app_core::song::parse_song_json;
    use sky_app_core::timing::MaterializedTimingPolicy;
    use std::path::PathBuf;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Barrier, Mutex, mpsc};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn valid_one_million_single_key_actions_are_rejected_by_combined_startup_budget() {
        const ACTION_COUNT: usize = 1_000_000;
        let mut actions = Vec::with_capacity(ACTION_COUNT);
        for index in 0..(ACTION_COUNT / 2) {
            let start = (index as u64) * 2;
            actions.push(sky_app_core::song::KeyAction {
                at_us: start,
                scan_codes: vec![0x15],
                kind: sky_app_core::song::ActionKind::Down,
                reason: "one-key".into(),
            });
            actions.push(sky_app_core::song::KeyAction {
                at_us: start + 1,
                scan_codes: vec![0x15],
                kind: sky_app_core::song::ActionKind::Up,
                reason: "one-key".into(),
            });
        }
        let schedule = sky_app_core::song::ScheduleMetadata {
            actions,
            source_duration_us: ACTION_COUNT as u64,
            playback_duration_us: ACTION_COUNT as u64,
            duration_us: ACTION_COUNT as u64,
            note_count: ACTION_COUNT / 2,
            deduplicated_note_count: ACTION_COUNT / 2,
            duplicate_note_count: 0,
            compressed_holds: 0,
            impossible_same_key_repeats: 0,
            risky_same_key_repeats: 0,
            max_polyphony: 1,
            shortest_same_key_interval_us: None,
            min_same_key_up_gap_us: None,
            recommended_hold_frames: None,
            recommended_tempo_scale: None,
            warnings: Vec::new(),
        };
        let source_capacity_bytes =
            schedule_metadata_heap_capacity_bytes(&schedule).expect("source schedule capacity");
        let compiled = compile_dispatch_schedule_with_checkpoint(
            &schedule,
            0,
            &std::sync::atomic::AtomicBool::new(false),
            |_| {},
        )
        .expect("one-million-action source fits the compiler phase bound");
        assert_eq!(compiled.generation_count, (ACTION_COUNT / 2) as u64);
        let compile_memory = compiled.compile_memory;
        let schedule_heap_bytes = compiled
            .memory_metrics()
            .expect("compiled schedule capacity")
            .heap_payload_bytes;
        let error = sky_player::adapter_support::RuntimeDispatchCoordinator::try_new_ticks(
            compiled,
            0,
            sky_player::adapter_support::DurationTicks::ZERO,
            |microseconds| {
                Ok::<_, sky_player::adapter_support::CoordinatorError>(
                    sky_player::adapter_support::TimelineTicks::from_raw(microseconds),
                )
            },
        )
        .expect_err("combined schedule, coordinator, and executor exceeds 512 MiB");
        let error = error.to_string();
        eprintln!(
            "dispatch_memory workload=valid_one_key actions={ACTION_COUNT} source_schedule_capacity_bytes={source_capacity_bytes} caller_retained_capacity_bytes={} compiler_input_capacity_bytes={} compiler_temporary_capacity_bytes={} compiled_schedule_capacity_bytes={schedule_heap_bytes} compiler_phase_peak_capacity_bytes={} policy_bytes={} disposition=combined_dispatch_rejected memory_basis=calculated_size_and_capacity lifetime_model=source_schedule_overlaps_compile_input_output_and_coordinator_materialization;compiler_input_and_temp_drop_before_coordinator_materialization complete_application_process_peak=not_measured windows_process_memory=not_measured reason={error}",
            compile_memory.caller_retained_capacity_bytes,
            compile_memory.input_capacity_bytes,
            compile_memory.temporary_capacity_bytes,
            compile_memory.phase_peak_capacity_bytes,
            sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES,
        );
        assert!(error.contains("exceeds dispatch budget"), "{error}");
        assert!(
            error.contains("536870912 bytes"),
            "the fixed 512 MiB admission limit must remain visible: {error}"
        );
        assert!(
            !error.to_ascii_lowercase().contains("out of memory")
                && error.contains("exceeds dispatch budget of 536870912 bytes"),
            "budget rejection must not be mislabeled as OOM: {error}"
        );
    }

    fn input(generation: u64, settings_fingerprint: &str) -> NativePreparedInput {
        let song = parse_song_json(
            br#"{"name":"Prepare publication race","songNotes":[{"time":0,"key":"Key0"}]}"#,
            "prepare-publication-race",
        )
        .expect("fixture song");
        let config = PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run: false,
        };
        let timing_policy = MaterializedTimingPolicy::from_user_margin(
            config.fps,
            config.hold_frames,
            config.timing_margin_us,
        )
        .expect("fixture timing policy");
        let schedule = build_schedule_with_policy(&song, config.tempo_scale, &timing_policy)
            .expect("fixture schedule");
        let risk = analyze_schedule_with_context(
            &schedule,
            Some(&song.notes),
            config.hold_frames,
            config.tempo_scale,
        );
        NativePreparedInput {
            song_id: "prepare-publication-race".into(),
            generation,
            config,
            song,
            sheet_analysis: None,
            schedule,
            risk,
            timing_policy,
            timing_margin_recommendation: TimingMarginRecommendationDto {
                recommended_timing_margin_us: Some(800),
                qualified: false,
                source: "default_fallback".into(),
            },
            settings_fingerprint: settings_fingerprint.into(),
            publication_fingerprint: settings_fingerprint.into(),
            source_path: PathBuf::new(),
            source_hash: String::new(),
            plan_identity: "test-plan".into(),
            plan_source: None,
            source_epoch: 0,
        }
    }

    #[test]
    fn native_schedule_compilation_honors_cancelled_request() {
        let prepared = input(1, "cancelled");
        let cancelled = std::sync::atomic::AtomicBool::new(true);
        let error = super::compile_dispatch_schedule(&prepared.schedule, &cancelled).unwrap_err();
        assert!(error.contains("cancelled"));
        cancelled.store(false, std::sync::atomic::Ordering::Release);
        assert!(super::compile_dispatch_schedule(&prepared.schedule, &cancelled).is_ok());
    }

    #[test]
    fn shutdown_cancels_an_in_flight_startup_compiler_and_releases_activity() {
        use sky_app_core::song::{ActionKind as SourceActionKind, KeyAction};

        let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
        let settings = ApplicationSettings::default();
        let fingerprint =
            super::super::settings_fingerprint(&settings).expect("default settings fingerprint");
        let mut prepared = input(1, &fingerprint);
        let actions = (0..150u64)
            .flat_map(|index| {
                let base = index * 100_000;
                [
                    KeyAction {
                        at_us: base,
                        scan_codes: vec![0x15],
                        kind: SourceActionKind::Down,
                        reason: "startup-cancel-down".into(),
                    },
                    KeyAction {
                        at_us: base + 50_000,
                        scan_codes: vec![0x15],
                        kind: SourceActionKind::Up,
                        reason: "startup-cancel-up".into(),
                    },
                ]
            })
            .collect::<Vec<_>>();
        prepared.schedule.duration_us = actions.last().expect("last action").at_us;
        prepared.schedule.playback_duration_us = prepared.schedule.duration_us;
        prepared.schedule.source_duration_us = prepared.schedule.duration_us;
        prepared.schedule.actions = actions;
        let prepared = service
            .prepare(prepared)
            .expect("prepare physical schedule");
        let prepared_id = prepared.prepared_id.expect("prepared identity");

        let (compiler_entered_tx, compiler_entered_rx) = mpsc::channel();
        let (compiler_release_tx, compiler_release_rx) = mpsc::channel();
        let compiler_release_rx = Mutex::new(compiler_release_rx);
        *service
            .startup_compile_test_hook
            .lock()
            .expect("startup compiler hook") = Some(Arc::new(move |action_count| {
            if action_count == 256 {
                compiler_entered_tx
                    .send(action_count)
                    .expect("startup test is waiting at the compiler checkpoint");
                compiler_release_rx
                    .lock()
                    .expect("startup compiler release lock")
                    .recv()
                    .expect("startup test releases the compiler checkpoint");
            }
        }));

        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let start_service = Arc::clone(&service);
        let start_events = Arc::clone(&events);
        let start_thread = thread::spawn(move || {
            start_service.start(
                super::super::NativePlaybackStartRequest {
                    prepared_id,
                    decisions: Vec::new(),
                }
                .into_public(),
                &settings,
                start_events,
            )
        });

        match compiler_entered_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(256) => {}
            Ok(action_count) => panic!("unexpected compiler checkpoint {action_count}"),
            Err(error) => {
                let startup_result = start_thread.join().expect("startup thread");
                panic!(
                    "startup compiler did not reach the in-flight checkpoint: {error}; result={startup_result:?}"
                );
            }
        }
        let shutdown_service = Arc::clone(&service);
        let shutdown_events = Arc::clone(&events);
        let shutdown_thread = thread::spawn(move || shutdown_service.shutdown(shutdown_events));
        let cancellation_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !service.startup_cancellation.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < cancellation_deadline,
                "shutdown did not publish startup cancellation"
            );
            thread::yield_now();
        }
        compiler_release_tx
            .send(())
            .expect("compiler is paused at its in-flight checkpoint");

        let error = start_thread
            .join()
            .expect("startup thread")
            .expect_err("shutdown cancels startup compilation");
        assert!(error.contains("native schedule compilation cancelled"));
        assert!(shutdown_thread.join().expect("shutdown thread"));
        assert!(!service.activity.physical_release_is_quarantined());
        let lease = service
            .activity
            .reserve_playback("post-startup-cancellation")
            .expect("startup cancellation releases its physical activity lease");
        drop(lease);
    }

    #[cfg(feature = "tauri-test")]
    #[test]
    fn shutdown_after_compilation_before_publication_retires_start_without_dispatch() {
        let activity = ActivityCoordinator::default();
        let mut service = NativePlaybackService::new(activity.clone());
        let (compiled_tx, compiled_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let release_rx = Mutex::new(release_rx);
        service.post_compile_prepublication_test_hook = Some(Arc::new(move || {
            compiled_tx
                .send(())
                .expect("test waits for the post-compile barrier");
            release_rx
                .lock()
                .expect("post-compile release lock")
                .recv()
                .expect("test releases the post-compile barrier");
        }));
        let service = Arc::new(service);
        let settings = ApplicationSettings::default();
        let fingerprint =
            super::super::settings_fingerprint(&settings).expect("settings fingerprint");
        let prepared = service
            .prepare(input(1, &fingerprint))
            .expect("prepare physical plan");
        let prepared_id = prepared.prepared_id.expect("prepared identity");

        let events = Arc::new(Mutex::new(NativeEventHub::default()));
        let start_service = Arc::clone(&service);
        let start_events = Arc::clone(&events);
        let start_settings = settings.clone();
        let start_prepared_id = prepared_id.clone();
        let start_thread = thread::spawn(move || {
            start_service.start(
                super::super::NativePlaybackStartRequest {
                    prepared_id: start_prepared_id,
                    decisions: Vec::new(),
                }
                .into_public(),
                &start_settings,
                start_events,
            )
        });

        compiled_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("compile reaches post-compile/pre-publication barrier");
        let shutdown_service = Arc::clone(&service);
        let shutdown_events = Arc::clone(&events);
        let shutdown_thread = thread::spawn(move || shutdown_service.shutdown(shutdown_events));
        let cancellation_deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !service.startup_cancellation.load(Ordering::Acquire) {
            assert!(
                std::time::Instant::now() < cancellation_deadline,
                "shutdown did not accept cancellation at the pre-publication boundary"
            );
            thread::yield_now();
        }
        release_tx.send(()).expect("release the completed compiler");

        let error = start_thread
            .join()
            .expect("startup thread")
            .expect_err("shutdown wins before active publication");
        assert!(error.contains("native playback startup was cancelled during shutdown"));
        assert!(shutdown_thread.join().expect("shutdown thread"));
        assert!(!service.activity.physical_release_is_quarantined());
        assert!(service.active.lock().expect("active slot").is_none());
        let lease = activity
            .reserve_playback("after-cancelled-start")
            .expect("cancelled startup releases activity ownership");
        drop(lease);

        let restart_prepared = service
            .prepare(input(2, &fingerprint))
            .expect("shutdown does not silently clear the prepared plan");
        let restart_error = service
            .start(
                super::super::NativePlaybackStartRequest {
                    prepared_id: restart_prepared.prepared_id.expect("prepared identity"),
                    decisions: Vec::new(),
                }
                .into_public(),
                &settings,
                events,
            )
            .expect_err("sticky shutdown disables restart");
        assert!(restart_error.contains("startup was cancelled during shutdown"));
    }

    #[test]
    fn newer_prepare_cancels_old_analysis_without_clearing_new_owner() {
        let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
        let (started, started_rx) = mpsc::channel();
        let (continue_old, continue_old_rx) = mpsc::channel();
        let old_service = Arc::clone(&service);
        let old_request = thread::spawn(move || {
            let cancellation = old_service
                .begin_preparation()
                .expect("first Prepare cancellation token");
            started
                .send(Arc::clone(&cancellation))
                .expect("publish token");
            continue_old_rx.recv().expect("release first request");
            let was_cancelled = cancellation.load(std::sync::atomic::Ordering::Acquire);
            old_service.finish_preparation(&cancellation);
            was_cancelled
        });

        let old_token = started_rx.recv().expect("first Prepare started");
        let new_token = service
            .begin_preparation()
            .expect("newer Prepare cancellation token");
        assert!(old_token.load(std::sync::atomic::Ordering::Acquire));
        assert!(!new_token.load(std::sync::atomic::Ordering::Acquire));

        continue_old.send(()).expect("resume old request");
        assert!(old_request.join().expect("first Prepare thread"));
        let active = service
            .prepare_publication
            .lock()
            .expect("Prepare cancellation owner lock");
        assert!(
            active
                .active_request
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &new_token))
        );
        drop(active);
        service.finish_preparation(&new_token);
        assert!(
            service
                .prepare_publication
                .lock()
                .expect("Prepare cancellation owner lock")
                .active_request
                .is_none()
        );
    }

    #[test]
    fn superseding_request_before_cache_commit_cannot_publish_or_return_old_plan() {
        let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let entered_hook = Arc::clone(&entered);
        let release_hook = Arc::clone(&release);
        *service
            .prepare_before_publication_test_hook
            .lock()
            .expect("publication hook") = Some(Arc::new(move || {
            entered_hook.wait();
            release_hook.wait();
        }));

        let old_service = Arc::clone(&service);
        let old = thread::spawn(move || old_service.prepare(input(1, "settings-a")));
        entered.wait();

        // The old request has completed model work and is paused immediately
        // before the atomic final check plus cache insertion.
        let latest = service
            .prepare(input(1, "settings-a"))
            .expect("newer request publishes");
        release.wait();

        let old_result = old.join().expect("old Prepare thread");
        assert!(
            old_result
                .expect_err("superseded request must not return an accepted plan")
                .contains("superseded")
        );
        let prepared_id = latest.prepared_id.expect("latest prepared identity");
        let cache = service.prepared.lock().expect("prepared cache");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].0, prepared_id);
    }

    #[test]
    fn settings_invalidation_while_model_is_queued_cancels_stale_publication() {
        let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let entered_hook = Arc::clone(&entered);
        let release_hook = Arc::clone(&release);
        *service
            .prepare_before_model_test_hook
            .lock()
            .expect("model hook") = Some(Arc::new(move || {
            entered_hook.wait();
            release_hook.wait();
        }));

        let old_service = Arc::clone(&service);
        let old = thread::spawn(move || old_service.prepare(input(1, "settings-a")));
        entered.wait();
        service.invalidate_settings();
        release.wait();

        assert!(
            old.join()
                .expect("old Prepare thread")
                .expect_err("settings invalidation rejects in-flight Prepare")
                .contains("superseded")
        );
        assert!(service.prepared.lock().expect("prepared cache").is_empty());

        let latest = service
            .prepare(input(1, "settings-a"))
            .expect("request after invalidation remains usable");
        let prepared_id = latest.prepared_id.expect("latest prepared identity");
        assert_eq!(service.prepared.lock().expect("prepared cache").len(), 1);
        assert!(service.prepared.lock().expect("prepared cache")[0].0 == prepared_id);
    }

    #[test]
    fn catalog_generation_invalidation_cancels_outstanding_prepare_and_keeps_new_generation() {
        let service = Arc::new(NativePlaybackService::new(ActivityCoordinator::default()));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let entered_hook = Arc::clone(&entered);
        let release_hook = Arc::clone(&release);
        *service
            .prepare_before_model_test_hook
            .lock()
            .expect("model hook") = Some(Arc::new(move || {
            entered_hook.wait();
            release_hook.wait();
        }));

        let old_service = Arc::clone(&service);
        let old = thread::spawn(move || old_service.prepare(input(1, "settings-a")));
        entered.wait();
        service.invalidate_catalog(2);
        release.wait();

        assert!(
            old.join()
                .expect("old Prepare thread")
                .expect_err("catalog reload rejects old-generation Prepare")
                .contains("superseded")
        );
        assert!(service.prepared.lock().expect("prepared cache").is_empty());

        let latest = service
            .prepare(input(2, "settings-a"))
            .expect("new-generation request remains usable");
        let prepared_id = latest.prepared_id.expect("latest prepared identity");
        let cache = service.prepared.lock().expect("prepared cache");
        assert_eq!(cache.len(), 1);
        assert_eq!(cache[0].0, prepared_id);
        assert_eq!(cache[0].1.generation, 2);
    }
}
