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
    if schedule.chords.len() > MAX_TIMING_ADVISORY_GENERATIONS {
        return unavailable(
            result,
            "The authored schedule exceeds the generation advisory limit.",
        );
    }
    match super::session::compile_dispatch_schedule(schedule, cancelled) {
        Ok(compiled) => analyze_compiled_stream(&compiled, result, policy, cancelled, || {}),
        Err(error) => unavailable(
            result,
            &format!("The canonical onset plan could not be inspected: {error}"),
        ),
    }
}

#[cfg(test)]
pub(super) fn analyze_compiled_variant_with_model_hook(
    compiled: &sky_player::adapter_support::OnsetPlan,
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
    compiled: &sky_player::adapter_support::OnsetPlan,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
) -> GenerationTimingAdvisoryDto {
    analyze_compiled_variant_with_hook(compiled, config, policy, plan_fingerprint, cancelled, || {})
}

pub(super) fn unavailable_compiled_variant(
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
    compiled: &sky_player::adapter_support::OnsetPlan,
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

/// Count the distinct union of authored onset timestamps (`chord.scheduled_us`)
/// and nominal release timestamps (`chord.scheduled_us + hold_us`) in O(n) time
/// and O(1) extra space using two sorted cursors, with bounded cancellation checks.
#[cfg(test)]
pub(crate) fn count_authored_timestamp_groups(
    chords: &[sky_player::adapter_support::OnsetChord],
    hold_us: u64,
) -> Result<u64, String> {
    count_authored_timestamp_groups_with_cancellation(chords, hold_us, &AtomicBool::new(false))
}

pub(crate) fn count_authored_timestamp_groups_with_cancellation(
    chords: &[sky_player::adapter_support::OnsetChord],
    hold_us: u64,
    cancelled: &AtomicBool,
) -> Result<u64, String> {
    const CHECKPOINT_INTERVAL: usize = 4096;
    let n = chords.len();
    let mut i = 0usize;
    let mut j = 0usize;
    let mut count = 0u64;
    let mut steps = 0usize;
    while i < n && j < n {
        if steps.is_multiple_of(CHECKPOINT_INTERVAL) && cancelled.load(Ordering::Acquire) {
            return Err("analysis was cancelled during timestamp group scan".to_string());
        }
        steps += 1;
        let onset_us = chords[i].scheduled_us;
        let release_us = chords[j]
            .scheduled_us
            .checked_add(hold_us)
            .ok_or_else(|| "nominal release timestamp overflow".to_string())?;
        match onset_us.cmp(&release_us) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| "timestamp group count overflow".to_string())?;
    }
    while j < n {
        if steps.is_multiple_of(CHECKPOINT_INTERVAL) && cancelled.load(Ordering::Acquire) {
            return Err("analysis was cancelled during timestamp group scan".to_string());
        }
        steps += 1;
        chords[j]
            .scheduled_us
            .checked_add(hold_us)
            .ok_or_else(|| "nominal release timestamp overflow".to_string())?;
        j += 1;
        count = count
            .checked_add(1)
            .ok_or_else(|| "timestamp group count overflow".to_string())?;
    }
    while i < n {
        if steps.is_multiple_of(CHECKPOINT_INTERVAL) && cancelled.load(Ordering::Acquire) {
            return Err("analysis was cancelled during timestamp group scan".to_string());
        }
        steps += 1;
        i += 1;
        count = count
            .checked_add(1)
            .ok_or_else(|| "timestamp group count overflow".to_string())?;
    }
    Ok(count)
}

fn analyze_compiled_stream(
    compiled: &sky_player::adapter_support::OnsetPlan,
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
    let generation_count = match usize::try_from(compiled.generation_count()) {
        Ok(count) => count,
        Err(_) => return unavailable(result, "The generation count exceeds addressable memory."),
    };
    let timestamp_group_count = match count_authored_timestamp_groups_with_cancellation(
        compiled.chords(),
        compiled.hold_us(),
        cancelled,
    ) {
        Ok(count) => count,
        Err(reason) => return unavailable(result, &reason),
    };
    result.total_authored_timestamp_group_count = Some(timestamp_group_count);
    result.down_generation_count = Some(compiled.generation_count());
    result.unmatched_up_count = Some(0);
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
    // cancellation while onset intervals are being inspected.
    before_analysis();
    if cancelled.load(Ordering::Acquire) {
        return unavailable(result, "analysis was cancelled before onset inspection");
    }

    let mut prior_onset_by_slot: [Option<u64>; sky_player::adapter_support::MAX_KEYS] =
        [None; sky_player::adapter_support::MAX_KEYS];
    for (index, chord) in compiled.chords().iter().enumerate() {
        if index % 256 == 0 && cancelled.load(Ordering::Acquire) {
            return unavailable(
                result,
                "analysis was cancelled while inspecting generations",
            );
        }
        if chord.chord_index != index {
            return unavailable(
                result,
                "The canonical onset chord identifiers are not contiguous.",
            );
        }
        let mut remaining = chord.key_mask;
        while remaining != 0 {
            let slot = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let Some(prior_onset) = prior_onset_by_slot.get_mut(slot) else {
                return unavailable(result, "A generation refers to an invalid key slot.");
            };
            if let Some(prev_onset_us) = *prior_onset {
                let Some(nominal_up_us) = prev_onset_us.checked_add(policy.min_hold_us) else {
                    return unavailable(result, "A nominal release timestamp overflowed.");
                };
                let Some(gap_us) = chord.scheduled_us.checked_sub(nominal_up_us) else {
                    return unavailable(
                        result,
                        "A same-key onset interval is shorter than the required hold floor.",
                    );
                };
                if gap_us < policy.frame_us {
                    result.short_up_gap_repeat_count += 1;
                }
            }
            *prior_onset = Some(chord.scheduled_us);
        }
    }
    result.status = if result.short_up_gap_repeat_count > 0 {
        TimingAdvisoryStatus::ShortUpGapRisk
    } else {
        TimingAdvisoryStatus::NoShortUpGapRisk
    };
    result.explanation = if result.short_up_gap_repeat_count > 0 {
        format!(
            "{} feasible same-key repeat(s) have a nominal Up-to-Down gap shorter than one frame.",
            result.short_up_gap_repeat_count
        )
    } else {
        "No feasible same-key repeat has a nominal Up-to-Down gap shorter than one frame.".into()
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
    plan: &sky_player::adapter_support::OnsetPlan,
    policy: &MaterializedTimingPolicy,
    cancelled: &AtomicBool,
) -> Result<DispatchSimulationSummary, String> {
    if plan.hold_us() != policy.min_hold_us {
        return Err(format!(
            "onset plan hold_us ({}) does not match policy min_hold_us ({})",
            plan.hold_us(),
            policy.min_hold_us
        ));
    }
    let model_qpc_hz = std::num::NonZeroU64::new(MODEL_QPC_FREQUENCY_HZ)
        .ok_or_else(|| "model QPC frequency must be nonzero".to_string())?;
    let prepared =
        sky_player::adapter_support::PreparedDispatchSchedule::prepare_with_cancellation(
            plan,
            model_qpc_hz,
            0,
            || {
                if cancelled.load(Ordering::Acquire) {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            },
        )?;
    let mut executor =
        sky_player::adapter_support::DispatchExecutor::from_prepared_schedule(prepared);
    let compiled_metrics = plan
        .memory_metrics()
        .map_err(|error| format!("compiled plan memory accounting failed: {error}"))?;
    let retained_schedule_bytes = compiled_metrics
        .heap_payload_bytes
        .checked_add(compiled_metrics.struct_bytes)
        .ok_or_else(|| "compiled plan memory accounting overflow".to_string())?
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
        return Err(
            "the prepared onset plan and executor exceed the offline advisory memory budget".into(),
        );
    }
    let epoch = sky_player::adapter_support::QpcTicks::ZERO;
    let max_iterations = usize::try_from(plan.generation_count())
        .ok()
        .and_then(|count| count.checked_mul(4))
        .and_then(|count| count.checked_add(2))
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
                ..
            } => {
                let down_mask = down_chord.map_or(0, |chord| chord.key_mask);
                if up_mask & down_mask != 0 || up_mask | down_mask == 0 {
                    return Err("the executor returned an invalid physical send mask".into());
                }
                let authored_target = timing_window.authored_target_qpc.as_u64();
                let mut h_constraint = 0u64;
                let mut h_displacement = 0u64;
                for timing in executor.up_slot_timings(up_mask) {
                    let nominal_up = timing.nominal_up_offset_ticks.as_u64();
                    let physical_up_floor = timing.release_not_before_qpc.as_u64();
                    let floor = nominal_up.max(physical_up_floor);
                    h_constraint = h_constraint.max(floor);
                    h_displacement = h_displacement.max(floor.saturating_sub(nominal_up));
                }
                let floor_ready = authored_target.max(h_constraint);
                let constraint_ready = h_constraint.max(previous_worker_free.unwrap_or(0));
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
    use sky_app_core::song::{OnsetChord, ScheduleMetadata};

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

    fn schedule(chords: &[(u64, u16)], hold_us: u64) -> ScheduleMetadata {
        let chords = chords
            .iter()
            .map(|&(at_us, scan_code)| OnsetChord {
                at_us,
                scan_codes: vec![scan_code],
            })
            .collect::<Vec<_>>();
        ScheduleMetadata {
            chords,
            hold_us,
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
    fn reports_canonical_generation_and_onset_group_counts() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[(0, 0x15), (17_467, 0x15)], policy.min_hold_us);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "same-key-plan",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.status, TimingAdvisoryStatus::ShortUpGapRisk);
        assert_eq!(advisory.total_authored_timestamp_group_count, Some(3));
        assert_eq!(advisory.down_generation_count, Some(2));
        assert_eq!(advisory.unmatched_up_count, Some(0));
        assert_eq!(advisory.short_up_gap_repeat_count, 1);
        assert_eq!(advisory.effective_hold_floor_h_us, Some(policy.min_hold_us));
        assert_eq!(
            advisory.dispatch_model_status,
            DispatchModelStatus::ModeledRisk
        );
        assert!(advisory.max_modeled_h_floor_displacement_us > 0);
    }

    #[test]
    fn authored_timestamp_group_union_counts_empty_single_coincident_disjoint_and_overflow() {
        use sky_player::adapter_support::OnsetChord;

        let chord = |chord_index, scheduled_us| OnsetChord {
            chord_index,
            scheduled_us,
            key_mask: 1,
            source_action_index: chord_index as u32,
        };
        assert_eq!(count_authored_timestamp_groups(&[], 17_467).unwrap(), 0);
        assert_eq!(
            count_authored_timestamp_groups(&[chord(0, 1_000)], 17_467).unwrap(),
            2
        );
        assert_eq!(
            count_authored_timestamp_groups(&[chord(0, 1_000)], 0).unwrap(),
            1
        );
        // Coincident: onsets {0, H} U releases {H, 2H} = {0, H, 2H} -> 3 groups
        assert_eq!(
            count_authored_timestamp_groups(&[chord(0, 0), chord(1, 17_467)], 17_467).unwrap(),
            3
        );
        // Disjoint: onsets {0, 40_000} U releases {17_467, 57_467} -> 4 groups
        assert_eq!(
            count_authored_timestamp_groups(&[chord(0, 0), chord(1, 40_000)], 17_467).unwrap(),
            4
        );
        // Overflow: scheduled_us + hold_us > u64::MAX
        assert!(count_authored_timestamp_groups(&[chord(0, u64::MAX)], 1).is_err());
    }

    #[test]
    fn exact_one_frame_nominal_up_gap_is_not_short_gap_risk() {
        let config = config(false);
        let policy = policy(&config);
        let frame_us = policy.frame_us;
        let h = policy.min_hold_us;
        let authored = schedule(&[(0, 0x15), (h + frame_us, 0x15)], h);
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
    fn sparse_nine_note_schedule_finishes_the_bounded_executor_model() {
        let config = config(false);
        let policy = policy(&config);
        let chords = (1..=9_u64)
            .map(|note| (note * 100_000, 0x15))
            .collect::<Vec<_>>();
        let authored = schedule(&chords, policy.min_hold_us);

        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "sparse-nine-note-plan",
            &AtomicBool::new(false),
        );

        assert_eq!(advisory.status, TimingAdvisoryStatus::NoShortUpGapRisk);
        assert_eq!(advisory.down_generation_count, Some(9));
        assert_eq!(advisory.unmatched_up_count, Some(0));
        assert_eq!(advisory.modeled_send_count, 18);
        assert_ne!(
            advisory.dispatch_model_status,
            DispatchModelStatus::Unavailable
        );
    }

    #[test]
    fn executor_model_reports_late_up_and_hol_independently_of_one_frame_nominal_gap() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        let frame = policy.frame_us;
        let authored = schedule(&[(0, 0x15), (10, 0x16), (h + frame, 0x15)], h);

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
        let authored = schedule(&[(0, 0x15)], policy.min_hold_us);
        let cancelled = analyze_variant(
            &authored,
            &config,
            &policy,
            "cancelled-plan",
            &AtomicBool::new(true),
        );
        assert_eq!(cancelled.status, TimingAdvisoryStatus::Unavailable);

        let invalid_order = schedule(&[(20_000, 0x15), (10_000, 0x15)], policy.min_hold_us);
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
        let authored = schedule(&[(0, 0x15)], policy.min_hold_us);
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
