use super::*;

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
            #[cfg(test)]
            startup_failure: None,
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
            let detail = song_detail(&song_id, &song, &schedule, &risk, sheet_analysis.clone());
            return Ok(PreparedPlaybackDto {
                prepared_id: None,
                song: detail,
                config,
                admission: PlaybackAdmission::ConfirmationRequired,
                risk: risk_summary(&risk),
                decisions: Vec::new(),
                plan_fingerprint: None,
                variants: Vec::new(),
                error_code: Some("conversion_confirmation_required".into()),
                error_message: Some(
                    "Review the listed conversion losses and explicitly accept them before preparing playback.".into(),
                ),
            });
        }
        let detail = song_detail(&song_id, &song, &schedule, &risk, sheet_analysis.clone());
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
            if let (Some(hold_frames), Some(tempo_scale)) =
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
        let dto = PreparedPlaybackDto {
            prepared_id: Some(prepared_id.clone()),
            song: detail,
            config,
            admission,
            risk: risk_summary(&risk),
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
        let mut active_slot = self
            .active
            .lock()
            .map_err(|_| "native active-playback lock poisoned".to_string())?;
        if active_slot.is_some() {
            return Err("another playback session is active".into());
        }
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        let record = prepared
            .iter()
            .find(|(id, _)| id == &request.prepared_id)
            .map(|(_, record)| record.clone())
            .ok_or_else(|| "prepared playback is stale or already consumed".to_string())?;
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
        let variant = record
            .variants
            .get(&selected)
            .cloned()
            .ok_or_else(|| "selected risk decision has no prepared plan".to_string())?;
        let session_id = opaque_native_id()?;
        let event_delivery_failure = events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .failure_flag();
        if event_delivery_failure.load(Ordering::Acquire) {
            return Err("native playback lifecycle delivery is unavailable".into());
        }
        let activity_lease = if variant.config.dry_run {
            None
        } else {
            Some(
                self.activity
                    .reserve_playback(&session_id)
                    .map_err(playback_activity_error)?,
            )
        };
        let prepared_index = prepared
            .iter()
            .position(|(id, _)| id == &request.prepared_id)
            .expect("prepared record was found above");
        prepared.remove(prepared_index);
        drop(prepared);
        let power_lifecycle = Arc::new(PowerLifecycleDiagnostics::default());
        let power_platform = system_power_platform();
        let mut power_request = None;
        let mut suspend_resume_registration = None;
        let physical_creation = if variant.config.dry_run {
            Ok((None, None))
        } else {
            self.create_native_player(
                &variant.schedule,
                &variant.config,
                &variant.timing_policy,
                settings,
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
                // Match the Python supervisor's observable failure path:
                // a start that cannot pass focus/target admission still
                // has an ordered starting -> failed -> failed-event trace.
                let (code, message) = physical_startup_failure(&error);
                let failed_active = Arc::new(NativeActivePlayback {
                    activity: self.activity.clone(),
                    session_id: session_id.clone(),
                    prepared_id: request.prepared_id.clone(),
                    song_id: record.song_id.clone(),
                    title: record.song.name.clone(),
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
            song_id: record.song_id.clone(),
            title: record.song.name.clone(),
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
    ) -> Result<(Arc<NativeDispatchSession>, isize), String> {
        #[cfg(test)]
        if let Some(error) = &self.startup_failure {
            return Err(error.clone());
        }
        #[cfg(all(test, feature = "tauri-test"))]
        if let Some(player) = &self.test_player {
            return Ok((Arc::clone(player), 0));
        }
        let runtime_schedule = compile_dispatch_schedule(schedule)?;
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
        let active = self.active.lock().ok().and_then(|slot| slot.clone());
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
            #[cfg(test)]
            startup_failure: None,
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
    serde_json::to_string(&serde_json::json!({
        "export_schema_version": 2,
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
    // Rust DTO directly.  The Python oracle uses
    // json.dumps(..., sort_keys=True, separators=(",", ":")); DTO struct
    // declaration order is not the canonical fingerprint order.
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

pub(in crate::native_runtime) fn blocked_playback_dto(
    song_id: &str,
    config: PlaybackConfigDto,
    code: &str,
    message: String,
    recommended_tempo_scale: Option<f64>,
    recommended_hold_frames: Option<f64>,
) -> PreparedPlaybackDto {
    // Keep blocked preparation responses aligned with the Python
    // DesktopPlaybackService: the failure is represented as a typed blocked
    // DTO, not as a new transport error or an implementation-specific code.
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
) -> SongDetailDto {
    let recommendation = crate::commands::PlaybackRecommendationDto {
        recommended_hold_frames: risk.suggested_hold_frames,
        recommended_tempo_scale: risk.suggested_tempo_scale,
        summary: risk
            .recommendations
            .first()
            .cloned()
            .unwrap_or_else(|| "Keep the selected settings.".into()),
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
        risk: risk_summary(risk),
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
    let options = super::planner_options(source.conversion_mode, source.target_pitch, tempo_scale);
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
) -> Result<sky_player::adapter_support::RuntimeSchedule, String> {
    let actions = schedule
        .actions
        .iter()
        .enumerate()
        .map(|(index, action)| KeyActionInput {
            source_action_index: index as u32,
            kind: match action.kind {
                ActionKind::Down => DispatchActionKind::Down,
                ActionKind::Up => DispatchActionKind::Up,
            },
            scheduled_us: action.at_us,
            scan_codes: action
                .scan_codes
                .iter()
                .copied()
                .collect::<SmallVec<[u16; 4]>>(),
            reason: Arc::<str>::from(action.reason.as_str()),
        })
        .collect::<Vec<_>>();
    compile_runtime_intents(&actions, &sky_app_core::song::SKY_SCAN_CODES)
        .map_err(|error| format!("native schedule compilation failed: {error}"))
}

#[cfg(test)]
mod preparation_cancellation_tests {
    use super::super::{
        NativePreparedInput, analyze_schedule_with_context, build_schedule_with_policy,
    };
    use super::NativePlaybackService;
    use crate::app_state::ActivityCoordinator;
    use crate::commands::{PlaybackConfigDto, TimingMarginRecommendationDto};
    use sky_app_core::song::parse_song_json;
    use sky_app_core::timing::MaterializedTimingPolicy;
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier, mpsc};
    use std::thread;

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
