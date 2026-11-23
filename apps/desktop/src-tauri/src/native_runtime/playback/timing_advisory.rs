use super::*;
use crate::commands::{
    DispatchModelStatus, GenerationTimingAdvisoryDto, PlaybackConfigDto, TimingAdvisoryStatus,
};
use sky_app_core::timing::MaterializedTimingPolicy;

const MAX_TIMING_ADVISORY_GENERATIONS: usize = 50_000;
const MAX_TIMING_ADVISORY_WORK_UNITS: u64 = 3_000_000;
const MAX_TIMING_ADVISORY_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_KEY_VISITS_PER_GENERATION: u64 = 60;
const MODEL_QPC_FREQUENCY_HZ: u64 = 1_000_000;
const ESTIMATED_SENDER_SETUP_US: u64 = 10;
const ESTIMATED_PER_INPUT_US: u64 = 3;
const ESTIMATED_POST_SEND_US: u64 = 5;

#[cfg(test)]
pub(super) fn analyze_variant(
    schedule: &sky_app_core::song::ScheduleMetadata,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
) -> GenerationTimingAdvisoryDto {
    let result = empty_advisory(plan_fingerprint, config.dry_run, policy.min_hold_us);
    if let Err(reason) = validate_policy(config, policy) {
        return unavailable(result, &reason);
    }
    if schedule.actions.len() > MAX_TIMING_ADVISORY_GENERATIONS.saturating_mul(2) {
        return unavailable(
            result,
            "The authored schedule exceeds the generation advisory limit.",
        );
    }
    match super::session::compile_dispatch_schedule(schedule, cancelled) {
        Ok(compiled) => analyze_compiled_stream(&compiled, result, policy, cancelled, || {}),
        Err(error) => unavailable(
            result,
            &format!("The canonical generation schedule could not be inspected: {error}"),
        ),
    }
}

#[cfg(test)]
pub(super) fn analyze_compiled_variant_with_model_hook(
    _source_action_count: usize,
    compiled: &sky_player::adapter_support::RuntimeSchedule,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
    before_analysis: impl FnOnce(),
) -> GenerationTimingAdvisoryDto {
    analyze_compiled_variant_with_hook(
        compiled,
        config,
        policy,
        plan_fingerprint,
        cancelled,
        before_analysis,
    )
}

#[cfg(not(test))]
pub(super) fn analyze_compiled_variant(
    _source_action_count: usize,
    compiled: &sky_player::adapter_support::RuntimeSchedule,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
) -> GenerationTimingAdvisoryDto {
    analyze_compiled_variant_with_hook(compiled, config, policy, plan_fingerprint, cancelled, || {})
}

pub(super) fn unavailable_compiled_variant(
    _source_action_count: usize,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
    explanation: &str,
) -> GenerationTimingAdvisoryDto {
    let result = empty_advisory(plan_fingerprint, config.dry_run, policy.min_hold_us);
    if cancelled.load(Ordering::Acquire) {
        return unavailable(
            result,
            "analysis was cancelled before generation inspection",
        );
    }
    unavailable(result, explanation)
}

fn analyze_compiled_variant_with_hook(
    compiled: &sky_player::adapter_support::RuntimeSchedule,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
    before_analysis: impl FnOnce(),
) -> GenerationTimingAdvisoryDto {
    let result = empty_advisory(plan_fingerprint, config.dry_run, policy.min_hold_us);
    if cancelled.load(Ordering::Acquire) {
        return unavailable(
            result,
            "analysis was cancelled before generation inspection",
        );
    }
    if let Err(reason) = validate_policy(config, policy) {
        return unavailable(result, &reason);
    }
    analyze_compiled_stream(compiled, result, policy, cancelled, before_analysis)
}

fn analyze_compiled_stream(
    compiled: &sky_player::adapter_support::RuntimeSchedule,
    mut result: GenerationTimingAdvisoryDto,
    policy: &MaterializedTimingPolicy,
    cancelled: &AtomicBool,
    before_analysis: impl FnOnce(),
) -> GenerationTimingAdvisoryDto {
    if cancelled.load(Ordering::Acquire) {
        return unavailable(
            result,
            "analysis was cancelled before generation inspection",
        );
    }
    let generation_count = match usize::try_from(compiled.generation_count) {
        Ok(count) => count,
        Err(_) => return unavailable(result, "The generation count exceeds addressable memory."),
    };
    result.total_authored_timestamp_group_count =
        Some(compiled.authored_timestamp_group_count as u64);
    result.down_generation_count = Some(compiled.generation_count);
    result.unmatched_up_count = Some(compiled.unmatched_ups.len() as u64);
    if generation_count > MAX_TIMING_ADVISORY_GENERATIONS {
        return unavailable(
            result,
            "The compiled generation schedule exceeds the offline advisory limit.",
        );
    }
    let work_units = match u64::try_from(generation_count)
        .ok()
        .and_then(|count| count.checked_mul(MAX_KEY_VISITS_PER_GENERATION))
    {
        Some(work) if work <= MAX_TIMING_ADVISORY_WORK_UNITS => work,
        _ => {
            return unavailable(
                result,
                "The compiled generation schedule exceeds the offline advisory work budget.",
            );
        }
    };
    let compiled_memory_bytes = match compiled.memory_metrics() {
        Ok(metrics) => metrics
            .heap_payload_bytes
            .checked_add(metrics.struct_bytes)
            .map(|bytes| bytes.max(metrics.compile_memory.phase_peak_capacity_bytes)),
        Err(_) => None,
    };
    let executor_memory_bytes =
        sky_player::adapter_support::DispatchExecutor::estimated_memory_bytes(compiled).ok();
    let total_memory_bytes = compiled_memory_bytes
        .zip(executor_memory_bytes)
        .and_then(|(compiled, executor)| compiled.checked_add(executor));
    if total_memory_bytes.is_none_or(|bytes| {
        bytes > usize::try_from(MAX_TIMING_ADVISORY_MEMORY_BYTES).unwrap_or(usize::MAX)
    }) {
        return unavailable(
            result,
            "The prepared generation schedule and executor simulation exceed the offline advisory memory budget.",
        );
    }
    result.work_limit_units = MAX_TIMING_ADVISORY_WORK_UNITS;
    result.working_memory_limit_bytes = MAX_TIMING_ADVISORY_MEMORY_BYTES;

    // Test callers can stop at the canonical adapter boundary to exercise
    // cancellation while generation pairs are being inspected.
    before_analysis();
    if cancelled.load(Ordering::Acquire) {
        return unavailable(result, "analysis was cancelled before paired-Up inspection");
    }

    let mut prior_up_by_slot = [None; sky_player::adapter_support::MAX_KEYS];
    for (index, chord) in compiled.down_chords.iter().enumerate() {
        if index % 256 == 0 && cancelled.load(Ordering::Acquire) {
            return unavailable(
                result,
                "analysis was cancelled while inspecting generations",
            );
        }
        if chord.chord_index != index {
            return unavailable(
                result,
                "The canonical Down chord identifiers are not contiguous.",
            );
        }
        for generation in &chord.intents {
            let slot = usize::from(generation.key_slot);
            let Some(prior_up) = prior_up_by_slot.get_mut(slot) else {
                return unavailable(result, "A generation refers to an invalid key slot.");
            };
            if let Some(up_us) = *prior_up {
                let Some(gap_us) = chord.scheduled_us.checked_sub(up_us) else {
                    return unavailable(
                        result,
                        "A same-key Down precedes the prior paired authored Up.",
                    );
                };
                if gap_us < policy.frame_us {
                    result.short_up_gap_repeat_count += 1;
                }
            }
            if generation.paired_up_offset_us < chord.scheduled_us {
                return unavailable(
                    result,
                    "A generation's paired authored Up precedes its Down.",
                );
            }
            *prior_up = Some(generation.paired_up_offset_us);
        }
    }
    result.status = if result.short_up_gap_repeat_count > 0 {
        TimingAdvisoryStatus::ShortUpGapRisk
    } else {
        TimingAdvisoryStatus::NoShortUpGapRisk
    };
    result.explanation = if result.short_up_gap_repeat_count > 0 {
        format!(
            "{} feasible same-key repeat(s) have an authored Up-to-Down gap shorter than one frame.",
            result.short_up_gap_repeat_count
        )
    } else {
        "No feasible same-key repeat has an authored Up-to-Down gap shorter than one frame.".into()
    };

    let simulated = match simulate_dispatch_schedule(compiled, policy, cancelled) {
        Ok(summary) => summary,
        Err(reason) => {
            if cancelled.load(Ordering::Acquire) {
                return unavailable(
                    result,
                    "analysis was cancelled while simulating the executor schedule",
                );
            }
            return dispatch_model_unavailable(result, &reason);
        }
    };
    result.modeled_send_count = simulated.send_count;
    result.modeled_negative_slack_send_count = simulated.negative_slack_send_count;
    result.modeled_h_floor_displaced_send_count = simulated.h_floor_displaced_send_count;
    result.max_modeled_h_floor_displacement_us = simulated.max_h_floor_displacement_us;
    result.modeled_head_of_line_send_count = simulated.head_of_line_send_count;
    result.max_modeled_head_of_line_delay_us = simulated.max_head_of_line_delay_us;
    result.max_modeled_lateness_us = simulated.max_lateness_us;
    result.dispatch_model_status = if simulated.negative_slack_send_count > 0
        || simulated.h_floor_displaced_send_count > 0
        || simulated.head_of_line_send_count > 0
    {
        DispatchModelStatus::ModeledRisk
    } else {
        DispatchModelStatus::NoModeledRisk
    };
    result.dispatch_model_explanation = match result.dispatch_model_status {
        DispatchModelStatus::NoModeledRisk => format!(
            "The illustrative executor model found no lateness, H-floor displacement, or head-of-line delay across {} sends.",
            simulated.send_count
        ),
        DispatchModelStatus::ModeledRisk => format!(
            "The illustrative executor model found {} late send(s), {} H-floor displacement(s), and {} head-of-line delay(s).",
            simulated.negative_slack_send_count,
            simulated.h_floor_displaced_send_count,
            simulated.head_of_line_send_count
        ),
        DispatchModelStatus::NotAnalyzed | DispatchModelStatus::Unavailable => {
            "The illustrative executor model could not be analyzed.".into()
        }
    };
    let _ = work_units;
    result
}

#[derive(Default)]
struct DispatchSimulationSummary {
    send_count: u64,
    negative_slack_send_count: u64,
    h_floor_displaced_send_count: u64,
    max_h_floor_displacement_us: u64,
    head_of_line_send_count: u64,
    max_head_of_line_delay_us: u64,
    max_lateness_us: u64,
}

fn simulate_dispatch_schedule(
    schedule: &sky_player::adapter_support::RuntimeSchedule,
    policy: &MaterializedTimingPolicy,
    cancelled: &AtomicBool,
) -> Result<DispatchSimulationSummary, String> {
    let hold_ticks = us_to_ticks_ceil(policy.min_hold_us, MODEL_QPC_FREQUENCY_HZ)?;
    let mut executor = sky_player::adapter_support::DispatchExecutor::new_with_cancellation(
        schedule,
        sky_player::adapter_support::DurationTicks::from_raw(hold_ticks),
        sky_player::adapter_support::DurationTicks::ZERO,
        |us| {
            us_to_ticks_ceil(us, MODEL_QPC_FREQUENCY_HZ)
                .map(sky_player::adapter_support::TimelineTicks::from_raw)
        },
        || {
            if cancelled.load(Ordering::Acquire) {
                Err("cancelled".into())
            } else {
                Ok(())
            }
        },
    )?;
    let compiled_metrics = schedule
        .memory_metrics()
        .map_err(|error| format!("compiled schedule memory accounting failed: {error}"))?;
    let retained_schedule_bytes = compiled_metrics
        .heap_payload_bytes
        .checked_add(compiled_metrics.struct_bytes)
        .ok_or_else(|| "compiled schedule memory accounting overflow".to_string())?
        .max(compiled_metrics.compile_memory.phase_peak_capacity_bytes);
    let actual_executor_bytes = executor
        .try_memory_metrics()
        .map_err(|error| format!("executor memory accounting failed: {error}"))?
        .total_bytes;
    if retained_schedule_bytes
        .checked_add(actual_executor_bytes)
        .is_none_or(|bytes| {
            bytes > usize::try_from(MAX_TIMING_ADVISORY_MEMORY_BYTES).unwrap_or(usize::MAX)
        })
    {
        return Err("the prepared generation schedule and executor exceed the offline advisory memory budget".into());
    }
    let epoch = sky_player::adapter_support::QpcTicks::ZERO;
    let max_iterations = usize::try_from(schedule.generation_count)
        .ok()
        .and_then(|count| count.checked_mul(2))
        .and_then(|count| count.checked_add(16))
        .ok_or_else(|| "executor simulation iteration bound overflow".to_string())?;
    let mut summary = DispatchSimulationSummary::default();
    let mut now = epoch;
    let mut previous_worker_free = None;
    let mut iterations = 0usize;

    loop {
        if iterations.is_multiple_of(256) && cancelled.load(Ordering::Acquire) {
            return Err("cancelled".into());
        }
        iterations = iterations
            .checked_add(1)
            .ok_or_else(|| "executor simulation iteration overflow".to_string())?;
        if iterations > max_iterations {
            return Err("the executor simulation exceeded its bounded iteration count".into());
        }
        match executor
            .evaluate_at(now, epoch)
            .map_err(|error| format!("executor timeline arithmetic failed: {error}"))?
        {
            sky_player::adapter_support::DispatchDecision::Finished => break,
            sky_player::adapter_support::DispatchDecision::Wait { deadline_qpc }
                if deadline_qpc > now =>
            {
                now = deadline_qpc;
            }
            sky_player::adapter_support::DispatchDecision::Wait { .. } => {
                return Err("the executor returned a non-advancing wait deadline".into());
            }
            sky_player::adapter_support::DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => {
                let down_mask = down_chord.map_or(0, |chord| chord.down_mask);
                if up_mask & down_mask != 0 || up_mask | down_mask == 0 {
                    return Err("the executor returned an invalid physical send mask".into());
                }
                let authored_target = timing_window.authored_target_qpc.as_u64();
                let mut h_constraint = 0u64;
                let mut h_displacement = 0u64;
                for timing in executor.up_slot_timings(up_mask) {
                    let authored_up = timing.authored_up_offset_ticks.as_u64();
                    let prepared_up_floor = timing.paired_up_not_before_offset_ticks.as_u64();
                    let physical_up_floor = timing.release_not_before_qpc.as_u64();
                    let floor = authored_up.max(prepared_up_floor).max(physical_up_floor);
                    h_constraint = h_constraint.max(floor);
                    h_displacement = h_displacement.max(floor.saturating_sub(authored_up));
                }
                let resume_constraint = if down_mask != 0 {
                    executor
                        .resume_barrier_qpc()
                        .map_or(0, |barrier| barrier.as_u64())
                } else {
                    0
                };
                let floor_ready = authored_target.max(h_constraint).max(resume_constraint);
                let constraint_ready = h_constraint
                    .max(resume_constraint)
                    .max(previous_worker_free.unwrap_or(0));
                let ready = now.as_u64().max(floor_ready);
                let slack = i128::from(authored_target) - i128::from(constraint_ready);
                let lateness = ready
                    .checked_sub(authored_target)
                    .ok_or_else(|| "executor model produced negative lateness".to_string())?;
                if slack < 0 {
                    summary.negative_slack_send_count = summary
                        .negative_slack_send_count
                        .checked_add(1)
                        .ok_or_else(|| "negative-slack send count overflow".to_string())?;
                }
                if h_displacement > 0 {
                    summary.h_floor_displaced_send_count = summary
                        .h_floor_displaced_send_count
                        .checked_add(1)
                        .ok_or_else(|| "H-floor send count overflow".to_string())?;
                    summary.max_h_floor_displacement_us =
                        summary.max_h_floor_displacement_us.max(h_displacement);
                }
                let hol_delay = previous_worker_free
                    .map(|free| free.saturating_sub(floor_ready))
                    .unwrap_or(0);
                if hol_delay > 0 {
                    summary.head_of_line_send_count = summary
                        .head_of_line_send_count
                        .checked_add(1)
                        .ok_or_else(|| "head-of-line send count overflow".to_string())?;
                    summary.max_head_of_line_delay_us =
                        summary.max_head_of_line_delay_us.max(hol_delay);
                }
                summary.send_count = summary
                    .send_count
                    .checked_add(1)
                    .ok_or_else(|| "modeled send count overflow".to_string())?;
                summary.max_lateness_us = summary.max_lateness_us.max(lateness);

                let event_count = u64::from(up_mask.count_ones() + down_mask.count_ones());
                let sender_envelope = ESTIMATED_SENDER_SETUP_US
                    .checked_add(
                        ESTIMATED_PER_INPUT_US
                            .checked_mul(event_count)
                            .ok_or_else(|| "estimated SendInput cost overflow".to_string())?,
                    )
                    .ok_or_else(|| "estimated sender envelope overflow".to_string())?;
                let sender_completion = ready
                    .checked_add(sender_envelope)
                    .ok_or_else(|| "estimated sender completion overflow".to_string())?;
                let worker_free = sender_completion
                    .checked_add(ESTIMATED_POST_SEND_US)
                    .ok_or_else(|| "estimated post-send time overflow".to_string())?;
                executor.on_send_completed_at(
                    up_mask,
                    down_mask,
                    sky_player::adapter_support::QpcTicks::from_raw(sender_completion),
                )?;
                previous_worker_free = Some(worker_free);
                now = sky_player::adapter_support::QpcTicks::from_raw(worker_free);
            }
        }
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    Ok(summary)
}

fn us_to_ticks_ceil(microseconds: u64, frequency_hz: u64) -> Result<u64, String> {
    let numerator = u128::from(microseconds)
        .checked_mul(u128::from(frequency_hz))
        .and_then(|value| value.checked_add(999_999))
        .ok_or_else(|| "microseconds-to-QPC conversion overflow".to_string())?;
    u64::try_from(numerator / 1_000_000)
        .map_err(|_| "microseconds-to-QPC result exceeds the timeline range".to_string())
}

fn dispatch_model_unavailable(
    mut advisory: GenerationTimingAdvisoryDto,
    reason: &str,
) -> GenerationTimingAdvisoryDto {
    advisory.dispatch_model_status = DispatchModelStatus::Unavailable;
    advisory.dispatch_model_explanation = reason.to_owned();
    advisory
}

fn validate_policy(
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
) -> Result<(), String> {
    if config.fps != policy.fps
        || config.hold_frames != policy.hold_frames
        || config.timing_margin_us != policy.timing_margin_us
    {
        return Err(
            "The playback configuration and materialized timing policy do not match.".into(),
        );
    }
    let expected_hold = policy
        .frame_base_hold_us
        .checked_add(policy.timing_margin_us)
        .ok_or_else(|| "The effective hold floor overflows.".to_string())?;
    if policy.frame_us == 0 || policy.min_hold_us != expected_hold {
        return Err("The materialized H timing floor is inconsistent.".into());
    }
    Ok(())
}

fn empty_advisory(
    plan_fingerprint: &str,
    dry_run: bool,
    effective_hold_floor_h_us: u64,
) -> GenerationTimingAdvisoryDto {
    GenerationTimingAdvisoryDto {
        status: TimingAdvisoryStatus::NotAnalyzed,
        plan_fingerprint: plan_fingerprint.to_owned(),
        explanation: "The prepared generation advisory has not been computed.".into(),
        total_authored_timestamp_group_count: None,
        down_generation_count: None,
        unmatched_up_count: None,
        generation_limit: MAX_TIMING_ADVISORY_GENERATIONS as u64,
        work_limit_units: MAX_TIMING_ADVISORY_WORK_UNITS,
        working_memory_limit_bytes: MAX_TIMING_ADVISORY_MEMORY_BYTES,
        short_up_gap_repeat_count: 0,
        effective_hold_floor_h_us: Some(effective_hold_floor_h_us),
        dispatch_model_status: DispatchModelStatus::NotAnalyzed,
        dispatch_model_explanation: "The illustrative executor model has not been computed.".into(),
        modeled_send_count: 0,
        modeled_negative_slack_send_count: 0,
        modeled_h_floor_displaced_send_count: 0,
        max_modeled_h_floor_displacement_us: 0,
        modeled_head_of_line_send_count: 0,
        max_modeled_head_of_line_delay_us: 0,
        max_modeled_lateness_us: 0,
        estimated_sender_setup_us: ESTIMATED_SENDER_SETUP_US,
        estimated_per_input_us: ESTIMATED_PER_INPUT_US,
        estimated_post_send_us: ESTIMATED_POST_SEND_US,
        dry_run_is_hypothetical: dry_run,
    }
}

fn unavailable(
    mut advisory: GenerationTimingAdvisoryDto,
    reason: &str,
) -> GenerationTimingAdvisoryDto {
    advisory.status = TimingAdvisoryStatus::Unavailable;
    advisory.explanation = reason.to_owned();
    advisory.dispatch_model_status = DispatchModelStatus::Unavailable;
    advisory.dispatch_model_explanation = reason.to_owned();
    advisory
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_app_core::song::{ActionKind, KeyAction, ScheduleMetadata};

    fn config(dry_run: bool) -> PlaybackConfigDto {
        PlaybackConfigDto {
            hold_frames: 1.0,
            timing_margin_us: 800,
            tempo_scale: 1.0,
            fps: 60,
            dry_run,
        }
    }

    fn policy(config: &PlaybackConfigDto) -> MaterializedTimingPolicy {
        MaterializedTimingPolicy::from_user_margin(
            config.fps,
            config.hold_frames,
            config.timing_margin_us,
        )
        .expect("valid timing policy")
    }

    fn schedule(actions: &[(ActionKind, u64, u16)]) -> ScheduleMetadata {
        let actions = actions
            .iter()
            .enumerate()
            .map(|(index, (kind, at_us, scan_code))| KeyAction {
                at_us: *at_us,
                scan_codes: vec![*scan_code],
                kind: *kind,
                reason: format!("action-{index}"),
            })
            .collect::<Vec<_>>();
        ScheduleMetadata {
            actions,
            source_duration_us: 0,
            playback_duration_us: 0,
            duration_us: 0,
            note_count: 0,
            deduplicated_note_count: 0,
            duplicate_note_count: 0,
            impossible_same_key_repeats: 0,
            max_polyphony: 0,
            shortest_same_key_interval_us: None,
            min_same_key_up_gap_us: None,
            short_up_gap_repeat_count: 0,
            recommended_hold_frames: None,
            recommended_tempo_scale: None,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn reports_canonical_generation_and_authored_group_counts() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[
            (ActionKind::Down, 0, 0x15),
            (ActionKind::Up, 17_467, 0x15),
            (ActionKind::Down, 17_467, 0x15),
            (ActionKind::Up, 34_934, 0x15),
            (ActionKind::Up, 40_000, 0x16),
        ]);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "same-key-plan",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.status, TimingAdvisoryStatus::ShortUpGapRisk);
        assert_eq!(advisory.total_authored_timestamp_group_count, Some(4));
        assert_eq!(advisory.down_generation_count, Some(2));
        assert_eq!(advisory.unmatched_up_count, Some(1));
        assert_eq!(advisory.short_up_gap_repeat_count, 1);
        assert_eq!(advisory.effective_hold_floor_h_us, Some(policy.min_hold_us));
        assert_eq!(
            advisory.dispatch_model_status,
            DispatchModelStatus::ModeledRisk
        );
        assert!(advisory.max_modeled_h_floor_displacement_us > 0);
    }

    #[test]
    fn exact_one_frame_authored_up_gap_is_not_short_gap_risk() {
        let config = config(false);
        let policy = policy(&config);
        let frame_us = policy.frame_us;
        let up_us = policy.min_hold_us;
        let authored = schedule(&[
            (ActionKind::Down, 0, 0x15),
            (ActionKind::Up, up_us, 0x15),
            (ActionKind::Down, up_us + frame_us, 0x15),
            (ActionKind::Up, up_us * 2 + frame_us, 0x15),
        ]);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "one-frame-gap",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.status, TimingAdvisoryStatus::NoShortUpGapRisk);
        assert_eq!(advisory.short_up_gap_repeat_count, 0);
    }

    #[test]
    fn executor_model_reports_late_up_and_hol_independently_of_one_frame_authored_gap() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        let frame = policy.frame_us;
        let authored = schedule(&[
            (ActionKind::Down, 0, 0x15),
            (ActionKind::Down, 10, 0x16),
            (ActionKind::Up, h, 0x15),
            (ActionKind::Up, h + 10, 0x16),
            (ActionKind::Down, h + frame, 0x15),
            (ActionKind::Up, h * 2 + frame, 0x15),
        ]);

        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "modeled-pressure-with-one-frame-gap",
            &AtomicBool::new(false),
        );

        assert_eq!(advisory.status, TimingAdvisoryStatus::NoShortUpGapRisk);
        assert_eq!(advisory.short_up_gap_repeat_count, 0);
        assert_eq!(
            advisory.dispatch_model_status,
            DispatchModelStatus::ModeledRisk
        );
        assert!(advisory.modeled_negative_slack_send_count > 0);
        assert!(advisory.modeled_h_floor_displaced_send_count > 0);
        assert!(advisory.max_modeled_h_floor_displacement_us > 0);
        assert!(advisory.modeled_head_of_line_send_count > 0);
        assert!(advisory.max_modeled_head_of_line_delay_us > 0);
        assert!(advisory.max_modeled_lateness_us > 0);
    }

    #[test]
    fn cancelled_and_invalid_schedules_fail_closed_as_advisory_only() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[(ActionKind::Down, 0, 0x15), (ActionKind::Up, 20_000, 0x15)]);
        let cancelled = analyze_variant(
            &authored,
            &config,
            &policy,
            "cancelled-plan",
            &AtomicBool::new(true),
        );
        assert_eq!(cancelled.status, TimingAdvisoryStatus::Unavailable);

        let invalid_order = schedule(&[
            (ActionKind::Down, 20_000, 0x15),
            (ActionKind::Up, 10_000, 0x15),
        ]);
        let invalid = analyze_variant(
            &invalid_order,
            &config,
            &policy,
            "invalid-order-plan",
            &AtomicBool::new(false),
        );
        assert_eq!(invalid.status, TimingAdvisoryStatus::Unavailable);
        assert!(invalid.explanation.contains("could not be inspected"));
    }

    #[test]
    fn dry_run_advisory_is_fingerprint_specific_and_hypothetical() {
        let config = config(true);
        let policy = policy(&config);
        let authored = schedule(&[(ActionKind::Down, 0, 0x15), (ActionKind::Up, 50_000, 0x15)]);
        let first = analyze_variant(
            &authored,
            &config,
            &policy,
            "dry-run-a",
            &AtomicBool::new(false),
        );
        let second = analyze_variant(
            &authored,
            &config,
            &policy,
            "dry-run-b",
            &AtomicBool::new(false),
        );
        assert!(first.dry_run_is_hypothetical);
        assert_eq!(first.plan_fingerprint, "dry-run-a");
        assert_ne!(first.plan_fingerprint, second.plan_fingerprint);
        assert_eq!(first.status, TimingAdvisoryStatus::NoShortUpGapRisk);
    }
}
