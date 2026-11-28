use super::*;

impl NativePlaybackService {
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
        let schedule_duration_us = schedule.duration_us;
        let qpc_frequency_hz = if config.dry_run {
            None
        } else {
            Some(
                sky_dispatch_win32::clock::QpcClock::initialize()
                    .map_err(|error| format!("native QPC Prepare admission failed: {error:?}"))?
                    .frequency_hz(),
            )
        };
        let other_prepared_capacity_bytes = {
            let mut prepared = self
                .prepared
                .lock()
                .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
            prepared.try_reserve(1).map_err(|error| {
                format!("native prepared-plan cache allocation failed: {error}")
            })?;
            prepared_cache_retained_capacity_bytes(&prepared, None)?
        };
        let base_caller_retained_capacity_bytes = native_prepare_caller_retained_capacity_bytes(
            NativePrepareCapacityInput {
                song: &song,
                song_id: &song_id,
                settings_fingerprint: &settings_fingerprint,
                publication_fingerprint: &publication_fingerprint,
                source_path: &source_path,
                source_hash: &source_hash,
                plan_identity: &plan_identity,
            },
            other_prepared_capacity_bytes,
        )?;
        let base_runtime_schedule = match self.compile_prepared_schedule(
            &schedule,
            base_caller_retained_capacity_bytes,
            cancelled,
        ) {
            Ok(compiled) => Some(compiled),
            Err(_error) if config.dry_run => None,
            Err(error) => {
                return Err(format!(
                    "native playback plan could not be prepared: {error}"
                ));
            }
        };
        let prepared_dispatch_schedule = if config.dry_run {
            None
        } else {
            let compiled = base_runtime_schedule
                .as_ref()
                .expect("physical Prepare requires a compiled runtime schedule");
            Some(
                sky_player::adapter_support::PreparedDispatchSchedule::prepare_with_cancellation(
                    compiled,
                    qpc_frequency_hz.expect("physical Prepare requires QPC admission"),
                    compiled.compile_memory().caller_retained_capacity_bytes,
                    || {
                        if cancelled.load(Ordering::Acquire) {
                            Err("native dispatch materialization cancelled".to_string())
                        } else {
                            Ok(())
                        }
                    },
                )
                .map_err(|error| format!("native playback plan preparation failed: {error}"))?,
            )
        };
        let timing_advisory = if let Some(compiled) = &base_runtime_schedule {
            self.analyze_timing_variant(compiled, &config, &timing_policy, &fingerprint, cancelled)
        } else {
            timing_advisory::unavailable_compiled_variant(
                &config,
                &timing_policy,
                &fingerprint,
                cancelled,
                "The dry-run schedule could not be compiled for the generation advisory.",
            )
        };
        if cancelled.load(Ordering::Acquire) {
            return Err("playback preparation was superseded by a newer request".into());
        }
        let base_dispatch_capacity_bytes = prepared_dispatch_schedule
            .as_ref()
            .map_or(0, |plan| plan.memory_metrics().plan_capacity_bytes);
        let admission = if risk.severity == "low" {
            PlaybackAdmission::Ready
        } else {
            PlaybackAdmission::ConfirmationRequired
        };
        let prepared_id = opaque_native_id()?;
        let base_variant = NativePlaybackVariant {
            config: config.clone(),
            schedule_duration_us,
            prepared_schedule: prepared_dispatch_schedule,
            fingerprint: fingerprint.clone(),
            timing_policy: timing_policy.clone(),
        };
        let mut decisions = Vec::new();
        let mut variants = HashMap::from([(PlaybackDecision::Proceed, base_variant)]);
        let mut variant_dtos = vec![PlaybackPlanVariantDto {
            decision: PlaybackDecision::Proceed,
            config: config.clone(),
            plan_fingerprint: fingerprint.clone(),
            timing_advisory: Some(timing_advisory),
        }];
        let mut dry_run_variant = None;
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
                let base_schedule_capacity_bytes =
                    schedule_metadata_heap_capacity_bytes(&schedule)?;
                let base_runtime_schedule_capacity_bytes = base_runtime_schedule
                    .as_ref()
                    .map(|compiled| runtime_schedule_capacity_bytes(compiled, cancelled))
                    .transpose()?
                    .unwrap_or(0);
                let recommendation_caller_retained_capacity_bytes = checked_capacity_sum([
                    other_prepared_capacity_bytes,
                    base_schedule_capacity_bytes,
                    base_runtime_schedule_capacity_bytes,
                    base_dispatch_capacity_bytes,
                ])?;
                let recommendation_caller_retained_capacity_bytes =
                    native_prepare_caller_retained_capacity_bytes(
                        NativePrepareCapacityInput {
                            song: &song,
                            song_id: &song_id,
                            settings_fingerprint: &settings_fingerprint,
                            publication_fingerprint: &publication_fingerprint,
                            source_path: &source_path,
                            source_hash: &source_hash,
                            plan_identity: &plan_identity,
                        },
                        recommendation_caller_retained_capacity_bytes,
                    )?;
                let recommended_compiled = match self.compile_prepared_schedule(
                    &recommended_schedule,
                    recommendation_caller_retained_capacity_bytes,
                    cancelled,
                ) {
                    Ok(compiled) => Some(compiled),
                    Err(error) if cancelled.load(Ordering::Acquire) => {
                        return Err(format!("playback preparation was cancelled: {error}"));
                    }
                    Err(_) => None,
                };
                if let Some(recommended_compiled) = recommended_compiled {
                    let recommended_dispatch = if config.dry_run {
                        None
                    } else {
                        #[cfg(all(test, feature = "tauri-test"))]
                        if let Some(hook) = self
                            .recommendation_materialization_test_hook
                            .lock()
                            .ok()
                            .and_then(|mut hook| hook.take())
                        {
                            hook();
                        }
                        #[cfg(all(test, feature = "tauri-test"))]
                        let caller_retained_capacity_bytes = self
                            .recommended_plan_caller_capacity_override_for_test
                            .unwrap_or(
                                recommended_compiled
                                    .compile_memory()
                                    .caller_retained_capacity_bytes,
                            );
                        #[cfg(not(all(test, feature = "tauri-test")))]
                        let caller_retained_capacity_bytes = recommended_compiled
                            .compile_memory()
                            .caller_retained_capacity_bytes;
                        match sky_player::adapter_support::PreparedDispatchSchedule::prepare_with_cancellation(
                            &recommended_compiled,
                            qpc_frequency_hz.expect("physical recommendation requires QPC admission"),
                            caller_retained_capacity_bytes,
                            || {
                                if cancelled.load(Ordering::Acquire) {
                                    Err("native dispatch materialization cancelled".to_string())
                                } else {
                                    Ok(())
                                }
                            },
                        ) {
                            Ok(prepared) => Some(prepared),
                            Err(error) if cancelled.load(Ordering::Acquire) => {
                                return Err(format!("playback preparation was cancelled: {error}"));
                            }
                            Err(_) => None,
                        }
                    };
                    if config.dry_run || recommended_dispatch.is_some() {
                        let recommended_timing_advisory = self.analyze_timing_variant(
                            &recommended_compiled,
                            &recommended_config,
                            &recommended_policy,
                            &recommended_fingerprint,
                            cancelled,
                        );
                        variants.insert(
                            PlaybackDecision::UseRecommended,
                            NativePlaybackVariant {
                                config: recommended_config.clone(),
                                schedule_duration_us: recommended_schedule.duration_us,
                                prepared_schedule: recommended_dispatch,
                                fingerprint: recommended_fingerprint.clone(),
                                timing_policy: recommended_policy,
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
                }
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
                let dry_run_timing_advisory = if let Some(compiled) = &base_runtime_schedule {
                    self.analyze_timing_variant(
                        compiled,
                        &dry_run_config,
                        &timing_policy,
                        &dry_run_fingerprint,
                        cancelled,
                    )
                } else {
                    timing_advisory::unavailable_compiled_variant(
                        &dry_run_config,
                        &timing_policy,
                        &dry_run_fingerprint,
                        cancelled,
                        "The dry-run schedule could not be compiled for the offline advisory.",
                    )
                };
                if cancelled.load(Ordering::Acquire) {
                    return Err("playback preparation was superseded by a newer request".into());
                }
                dry_run_variant = Some((
                    NativePlaybackVariant {
                        config: dry_run_config.clone(),
                        schedule_duration_us,
                        prepared_schedule: None,
                        fingerprint: dry_run_fingerprint.clone(),
                        timing_policy: timing_policy.clone(),
                    },
                    PlaybackPlanVariantDto {
                        decision: PlaybackDecision::DryRun,
                        config: dry_run_config,
                        plan_fingerprint: dry_run_fingerprint,
                        timing_advisory: Some(dry_run_timing_advisory),
                    },
                ));
            }
        }
        drop(base_runtime_schedule);
        if let Some((dry_run_variant, dry_run_dto)) = dry_run_variant {
            variants.insert(PlaybackDecision::DryRun, dry_run_variant);
            variant_dtos.push(dry_run_dto);
            decisions.push(RiskDecisionDto {
                decision: PlaybackDecision::DryRun,
                label: "Run a dry-run first".into(),
            });
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
        let song_title = song.name;
        let record = NativePreparedPlan {
            song_id,
            generation,
            settings_fingerprint,
            song_title,
            source_path,
            source_hash,
            dto: dto.clone(),
            variants,
            retained_capacity_bytes: 0,
        };
        let mut record = record;
        record.retained_capacity_bytes = prepared_plan_capacity_bytes(&prepared_id, &record)?;
        let mut prepared = self
            .prepared
            .lock()
            .map_err(|_| "native prepared-plan lock poisoned".to_string())?;
        prepared.push_back((prepared_id, record));
        retain_prepared_capacity(&mut prepared);
        if prepared_cache_retained_capacity_bytes(&prepared, None)?
            > sky_player::adapter_support::MAX_RETAINED_DISPATCH_BYTES
        {
            prepared.pop_back();
            return Err("prepared-plan cache exceeds the retained dispatch capacity budget".into());
        }
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
        schedule: &sky_player::adapter_support::OnsetPlan,
        config: &PlaybackConfigDto,
        policy: &MaterializedTimingPolicy,
        fingerprint: &str,
        cancelled: &AtomicBool,
    ) -> crate::commands::GenerationTimingAdvisoryDto {
        #[cfg(test)]
        {
            let hook = self
                .prepare_before_model_test_hook
                .lock()
                .ok()
                .and_then(|mut hook| hook.take());
            timing_advisory::analyze_compiled_variant_with_model_hook(
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
        timing_advisory::analyze_compiled_variant(schedule, config, policy, fingerprint, cancelled)
    }

    fn compile_prepared_schedule(
        &self,
        schedule: &ScheduleMetadata,
        caller_retained_capacity_bytes: usize,
        cancelled: &AtomicBool,
    ) -> Result<sky_player::adapter_support::OnsetPlan, String> {
        if cancelled.load(Ordering::Acquire) {
            return Err("native schedule preparation cancelled".into());
        }
        #[cfg(test)]
        {
            let hook = self
                .prepare_compile_test_hook
                .lock()
                .ok()
                .and_then(|hook| hook.clone());
            if let Some(hook) = hook {
                return compile_dispatch_schedule_with_checkpoint(
                    schedule,
                    caller_retained_capacity_bytes,
                    cancelled,
                    |action_count| hook(action_count),
                );
            }
        }
        compile_dispatch_schedule_with_prepared_capacity(
            schedule,
            caller_retained_capacity_bytes,
            cancelled,
        )
    }
}
