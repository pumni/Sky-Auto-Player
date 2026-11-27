use super::*;
use crate::commands::{
    DispatchModelScenarioDto, DispatchModelStatus, GenerationTimingAdvisoryDto, PlaybackConfigDto,
    TimingAdvisoryStatus,
};
use sky_app_core::timing::MaterializedTimingPolicy;

const MAX_TIMING_ADVISORY_GENERATIONS: usize = 50_000;
const MAX_TIMING_ADVISORY_WORK_UNITS: u64 = 3_000_000;
const MAX_TIMING_ADVISORY_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const MODEL_QPC_FREQUENCY_HZ: u64 = 1_000_000;
const MODEL_COST_DISCLAIMER: &str = "Illustrative modeled cost only — not measured SendInput latency or a predictive gameplay guarantee.";

#[derive(Debug, Clone, Copy)]
struct ModelScenarioSpec {
    scenario_id: &'static str,
    label: &'static str,
    sender_setup_us: u64,
    per_input_us: u64,
    post_send_us: u64,
}

const MODEL_SCENARIOS: [ModelScenarioSpec; 3] = [
    ModelScenarioSpec {
        scenario_id: "ideal",
        label: "Ideal (illustrative modeled cost: 0 µs)",
        sender_setup_us: 0,
        per_input_us: 0,
        post_send_us: 0,
    },
    ModelScenarioSpec {
        scenario_id: "reference",
        label: "Reference (illustrative modeled cost: 10 µs + 3 µs/event + 5 µs)",
        sender_setup_us: 10,
        per_input_us: 3,
        post_send_us: 5,
    },
    ModelScenarioSpec {
        scenario_id: "stress",
        label: "Stress (illustrative modeled cost: 100 µs + 10 µs/event + 50 µs)",
        sender_setup_us: 100,
        per_input_us: 10,
        post_send_us: 50,
    },
];

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
    result.work_limit_units = MAX_TIMING_ADVISORY_WORK_UNITS;
    result.working_memory_limit_bytes = MAX_TIMING_ADVISORY_MEMORY_BYTES;

    // Test callers can stop at the canonical adapter boundary to exercise
    // cancellation while onset intervals are being inspected.
    before_analysis();
    if cancelled.load(Ordering::Acquire) {
        return unavailable(result, "analysis was cancelled before onset inspection");
    }

    let mut work_units_used = 0u64;
    let mut prior_onset_by_slot: [Option<u64>; sky_player::adapter_support::MAX_KEYS] =
        [None; sky_player::adapter_support::MAX_KEYS];
    let mut min_same_key_slack_us: Option<u64> = None;
    let mut same_key_repeat_count = 0u64;
    let mut zero_slack_repeat_count = 0u64;

    for (index, chord) in compiled.chords().iter().enumerate() {
        if index.is_multiple_of(256) && cancelled.load(Ordering::Acquire) {
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
            work_units_used = match work_units_used.checked_add(1) {
                Some(used) if used <= MAX_TIMING_ADVISORY_WORK_UNITS => used,
                _ => {
                    return unavailable(
                        result,
                        "The compiled generation schedule exceeds the offline advisory work budget.",
                    );
                }
            };
            let slot = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            let Some(prior_onset) = prior_onset_by_slot.get_mut(slot) else {
                return unavailable(result, "A generation refers to an invalid key slot.");
            };
            if let Some(prev_onset_us) = *prior_onset {
                let Some(nominal_up_us) = prev_onset_us.checked_add(policy.min_hold_us) else {
                    return unavailable(result, "A nominal release timestamp overflowed.");
                };
                let Some(slack_us) = chord.scheduled_us.checked_sub(nominal_up_us) else {
                    return unavailable(
                        result,
                        "A same-key onset interval is shorter than the required hold floor.",
                    );
                };
                same_key_repeat_count = match same_key_repeat_count.checked_add(1) {
                    Some(count) => count,
                    None => {
                        return unavailable(result, "Same-key repeat count overflowed.");
                    }
                };
                if slack_us == 0 {
                    zero_slack_repeat_count = match zero_slack_repeat_count.checked_add(1) {
                        Some(count) => count,
                        None => {
                            return unavailable(result, "Zero-slack repeat count overflowed.");
                        }
                    };
                }
                min_same_key_slack_us =
                    Some(min_same_key_slack_us.map_or(slack_us, |current| current.min(slack_us)));
            }
            *prior_onset = Some(chord.scheduled_us);
        }
    }

    result.min_same_key_slack_us = min_same_key_slack_us;
    result.same_key_repeat_count = same_key_repeat_count;
    result.zero_slack_repeat_count = zero_slack_repeat_count;
    result.status = TimingAdvisoryStatus::Analyzed;
    result.explanation = if same_key_repeat_count == 0 {
        "No feasible same-key repeats in this schedule.".into()
    } else if zero_slack_repeat_count > 0 {
        format!(
            "{zero_slack_repeat_count} of {same_key_repeat_count} same-key repeat(s) have zero nominal scheduling slack (I = H; minimum slack 0 µs). Hold-only allows I = H; transport overhead can displace the following Down."
        )
    } else {
        format!(
            "{same_key_repeat_count} same-key repeat(s); minimum nominal scheduling slack (I - H) is {} µs.",
            min_same_key_slack_us.unwrap_or(0)
        )
    };

    // Memory admission check before running the illustrative offline model.
    // Nominal slack above remains valid even if the model exceeds work/memory caps.
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
        .and_then(|(compiled_bytes, executor_bytes)| compiled_bytes.checked_add(executor_bytes));
    if total_memory_bytes.is_none_or(|bytes| {
        bytes > usize::try_from(MAX_TIMING_ADVISORY_MEMORY_BYTES).unwrap_or(usize::MAX)
    }) {
        return dispatch_model_unavailable(
            result,
            "The prepared generation schedule and executor simulation exceed the offline advisory memory budget.",
        );
    }

    let scenarios = match simulate_all_scenarios(compiled, policy, cancelled, &mut work_units_used)
    {
        Ok(scenarios) => scenarios,
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

    let reference = scenarios
        .iter()
        .find(|scenario| scenario.scenario_id == "reference");
    let has_displacement = scenarios.iter().any(|scenario| {
        scenario.modeled_late_down_count > 0
            || scenario.modeled_h_floor_displaced_up_slot_count > 0
            || scenario.modeled_late_up_packet_count > 0
    });
    result.dispatch_model_status = if has_displacement {
        DispatchModelStatus::ModeledDisplacement
    } else {
        DispatchModelStatus::NoModeledDisplacement
    };
    result.dispatch_model_explanation = match (result.dispatch_model_status, reference) {
        (DispatchModelStatus::NoModeledDisplacement, Some(ref_scenario)) => format!(
            "Illustrative modeled scenarios found no Down lateness or Up H-floor displacement across {} sends.",
            ref_scenario.modeled_send_count
        ),
        (DispatchModelStatus::ModeledDisplacement, Some(ref_scenario)) => format!(
            "Reference illustrative model: {} late Down(s) (max {} µs), {} H-floor Up slot(s) (max {} µs), {} cursor-order Down(s) (max {} µs).",
            ref_scenario.modeled_late_down_count,
            ref_scenario.max_modeled_down_lateness_us,
            ref_scenario.modeled_h_floor_displaced_up_slot_count,
            ref_scenario.max_modeled_h_floor_displacement_us,
            ref_scenario.modeled_cursor_order_displaced_down_count,
            ref_scenario.max_modeled_cursor_order_displacement_us,
        ),
        _ => "The illustrative executor model could not be analyzed.".into(),
    };
    result.scenarios = scenarios;
    result
}

fn simulate_all_scenarios(
    plan: &sky_player::adapter_support::OnsetPlan,
    policy: &MaterializedTimingPolicy,
    cancelled: &AtomicBool,
    work_units_used: &mut u64,
) -> Result<Vec<DispatchModelScenarioDto>, String> {
    if plan.hold_us() != policy.min_hold_us {
        return Err(format!(
            "onset plan hold_us ({}) does not match policy min_hold_us ({})",
            plan.hold_us(),
            policy.min_hold_us
        ));
    }
    let compiled_metrics = plan
        .memory_metrics()
        .map_err(|error| format!("compiled plan memory accounting failed: {error}"))?;
    let retained_schedule_bytes = compiled_metrics
        .heap_payload_bytes
        .checked_add(compiled_metrics.struct_bytes)
        .ok_or_else(|| "compiled plan memory accounting overflow".to_string())?
        .max(compiled_metrics.compile_memory.phase_peak_capacity_bytes);

    let mut results = Vec::with_capacity(MODEL_SCENARIOS.len());
    for spec in MODEL_SCENARIOS {
        let scenario = simulate_single_scenario(
            plan,
            retained_schedule_bytes,
            spec,
            cancelled,
            work_units_used,
        )?;
        results.push(scenario);
    }
    Ok(results)
}

fn simulate_single_scenario(
    plan: &sky_player::adapter_support::OnsetPlan,
    retained_schedule_bytes: usize,
    spec: ModelScenarioSpec,
    cancelled: &AtomicBool,
    work_units_used: &mut u64,
) -> Result<DispatchModelScenarioDto, String> {
    let model_qpc_hz = std::num::NonZeroU64::new(MODEL_QPC_FREQUENCY_HZ)
        .ok_or_else(|| "model QPC frequency must be nonzero".to_string())?;
    // Sequential per-scenario preparation keeps peak working memory bounded to
    // one PreparedDispatchSchedule + shared immutable OnsetPlan at a time.
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

    let mut dto = DispatchModelScenarioDto {
        scenario_id: spec.scenario_id.to_owned(),
        label: spec.label.to_owned(),
        estimated_sender_setup_us: spec.sender_setup_us,
        estimated_per_input_us: spec.per_input_us,
        estimated_post_send_us: spec.post_send_us,
        modeled_send_count: 0,
        modeled_down_packet_count: 0,
        modeled_up_packet_count: 0,
        modeled_late_down_count: 0,
        max_modeled_down_lateness_us: 0,
        modeled_key_readiness_displaced_down_count: 0,
        max_modeled_key_readiness_displacement_us: 0,
        modeled_cursor_order_displaced_down_count: 0,
        max_modeled_cursor_order_displacement_us: 0,
        modeled_worker_occupancy_displaced_down_count: 0,
        max_modeled_worker_occupancy_residual_us: 0,
        modeled_up_slot_count: 0,
        modeled_h_floor_displaced_up_slot_count: 0,
        max_modeled_h_floor_displacement_us: 0,
        modeled_late_up_packet_count: 0,
        max_modeled_up_lateness_us: 0,
        max_modeled_up_pre_call_above_floor_us: 0,
    };

    let mut now = epoch;
    // Offline-only attribution state (not tracked in runtime executor):
    // - slot_key_ready_us: modeled worker-free timestamp after the previous Up
    //   on each slot (same-key Up must commit before the next Down).
    // - prior_down_pre_call: modeled pre-call timestamp of the previous Down chord.
    let mut slot_key_ready_us = [0u64; sky_player::adapter_support::MAX_KEYS];
    let mut prior_down_pre_call: Option<u64> = None;
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
        *work_units_used = work_units_used
            .checked_add(1)
            .ok_or_else(|| "executor simulation work budget overflow".to_string())?;
        if *work_units_used > MAX_TIMING_ADVISORY_WORK_UNITS {
            return Err(
                "the illustrative executor scenarios exceed the offline advisory work budget"
                    .into(),
            );
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
                ..
            } => {
                let down_mask = down_chord.map_or(0, |chord| chord.key_mask);
                if up_mask & down_mask != 0 || up_mask | down_mask == 0 {
                    return Err("the executor returned an invalid physical send mask".into());
                }
                let actual_pre_call = now.as_u64();

                // 1. Musical Up direction attribution (independent of Down in mixed packets).
                if up_mask != 0 {
                    dto.modeled_up_packet_count = dto
                        .modeled_up_packet_count
                        .checked_add(1)
                        .ok_or_else(|| "modeled Up packet count overflow".to_string())?;
                    let mut packet_max_up_lateness = 0u64;
                    let mut packet_max_up_above_floor = 0u64;
                    for timing in executor.up_slot_timings(up_mask) {
                        *work_units_used = work_units_used.checked_add(1).ok_or_else(|| {
                            "executor simulation work budget overflow".to_string()
                        })?;
                        if *work_units_used > MAX_TIMING_ADVISORY_WORK_UNITS {
                            return Err(
                                "the illustrative executor scenarios exceed the offline advisory work budget".into(),
                            );
                        }
                        dto.modeled_up_slot_count = dto
                            .modeled_up_slot_count
                            .checked_add(1)
                            .ok_or_else(|| "modeled Up slot count overflow".to_string())?;
                        let nominal_up = timing.nominal_up_offset_ticks.as_u64();
                        let physical_up_floor = timing.release_not_before_qpc.as_u64();
                        let effective_floor = nominal_up.max(physical_up_floor);
                        let h_floor_disp = effective_floor.saturating_sub(nominal_up);
                        if h_floor_disp > 0 {
                            dto.modeled_h_floor_displaced_up_slot_count = dto
                                .modeled_h_floor_displaced_up_slot_count
                                .checked_add(1)
                                .ok_or_else(|| "H-floor Up slot count overflow".to_string())?;
                            dto.max_modeled_h_floor_displacement_us =
                                dto.max_modeled_h_floor_displacement_us.max(h_floor_disp);
                        }
                        let slot_lateness = actual_pre_call.saturating_sub(nominal_up);
                        let slot_above_floor = actual_pre_call.saturating_sub(effective_floor);
                        packet_max_up_lateness = packet_max_up_lateness.max(slot_lateness);
                        packet_max_up_above_floor = packet_max_up_above_floor.max(slot_above_floor);
                    }
                    if packet_max_up_lateness > 0 {
                        dto.modeled_late_up_packet_count = dto
                            .modeled_late_up_packet_count
                            .checked_add(1)
                            .ok_or_else(|| "late Up packet count overflow".to_string())?;
                    }
                    dto.max_modeled_up_lateness_us =
                        dto.max_modeled_up_lateness_us.max(packet_max_up_lateness);
                    dto.max_modeled_up_pre_call_above_floor_us = dto
                        .max_modeled_up_pre_call_above_floor_us
                        .max(packet_max_up_above_floor);
                }

                // 2. Musical Down chord attribution (uses Down chord's own authored onset,
                //    non-double-counted decomposition: key_readiness + cursor_order +
                //    worker_occupancy == down_lateness).
                if let Some(chord) = down_chord {
                    dto.modeled_down_packet_count = dto
                        .modeled_down_packet_count
                        .checked_add(1)
                        .ok_or_else(|| "modeled Down packet count overflow".to_string())?;
                    let authored = chord.onset_ticks.as_u64();
                    let mut key_ready = 0u64;
                    let mut remaining = chord.key_mask;
                    while remaining != 0 {
                        let slot = remaining.trailing_zeros() as usize;
                        remaining &= remaining - 1;
                        key_ready = key_ready.max(slot_key_ready_us[slot]);
                    }
                    let prior_down = prior_down_pre_call.unwrap_or(0);
                    let down_lateness = actual_pre_call.checked_sub(authored).ok_or_else(|| {
                        "executor model produced negative Down lateness".to_string()
                    })?;
                    let key_readiness_disp = key_ready.saturating_sub(authored);
                    let after_key_ready = authored.max(key_ready);
                    let cursor_order_disp = prior_down.saturating_sub(after_key_ready);
                    let after_cursor_order = after_key_ready.max(prior_down);
                    let worker_occupancy_res = actual_pre_call.saturating_sub(after_cursor_order);

                    debug_assert_eq!(
                        key_readiness_disp + cursor_order_disp + worker_occupancy_res,
                        down_lateness
                    );

                    if down_lateness > 0 {
                        dto.modeled_late_down_count = dto
                            .modeled_late_down_count
                            .checked_add(1)
                            .ok_or_else(|| "late Down count overflow".to_string())?;
                        dto.max_modeled_down_lateness_us =
                            dto.max_modeled_down_lateness_us.max(down_lateness);
                    }
                    if key_readiness_disp > 0 {
                        dto.modeled_key_readiness_displaced_down_count = dto
                            .modeled_key_readiness_displaced_down_count
                            .checked_add(1)
                            .ok_or_else(|| "key-readiness count overflow".to_string())?;
                        dto.max_modeled_key_readiness_displacement_us = dto
                            .max_modeled_key_readiness_displacement_us
                            .max(key_readiness_disp);
                    }
                    if cursor_order_disp > 0 {
                        dto.modeled_cursor_order_displaced_down_count = dto
                            .modeled_cursor_order_displaced_down_count
                            .checked_add(1)
                            .ok_or_else(|| "cursor-order count overflow".to_string())?;
                        dto.max_modeled_cursor_order_displacement_us = dto
                            .max_modeled_cursor_order_displacement_us
                            .max(cursor_order_disp);
                    }
                    if worker_occupancy_res > 0 {
                        dto.modeled_worker_occupancy_displaced_down_count = dto
                            .modeled_worker_occupancy_displaced_down_count
                            .checked_add(1)
                            .ok_or_else(|| "worker-occupancy count overflow".to_string())?;
                        dto.max_modeled_worker_occupancy_residual_us = dto
                            .max_modeled_worker_occupancy_residual_us
                            .max(worker_occupancy_res);
                    }
                    prior_down_pre_call = Some(actual_pre_call);
                }

                dto.modeled_send_count = dto
                    .modeled_send_count
                    .checked_add(1)
                    .ok_or_else(|| "modeled send count overflow".to_string())?;

                let event_count = u64::from(up_mask.count_ones() + down_mask.count_ones());
                let sender_envelope = spec
                    .sender_setup_us
                    .checked_add(
                        spec.per_input_us
                            .checked_mul(event_count)
                            .ok_or_else(|| "estimated SendInput cost overflow".to_string())?,
                    )
                    .ok_or_else(|| "estimated sender envelope overflow".to_string())?;
                let sender_completion = actual_pre_call
                    .checked_add(sender_envelope)
                    .ok_or_else(|| "estimated sender completion overflow".to_string())?;
                let worker_free = sender_completion
                    .checked_add(spec.post_send_us)
                    .ok_or_else(|| "estimated post-send time overflow".to_string())?;

                executor.on_send_completed_at(
                    up_mask,
                    down_mask,
                    sky_player::adapter_support::QpcTicks::from_raw(sender_completion),
                )?;

                let mut released = up_mask;
                while released != 0 {
                    let slot = released.trailing_zeros() as usize;
                    released &= released - 1;
                    slot_key_ready_us[slot] = worker_free;
                }
                now = sky_player::adapter_support::QpcTicks::from_raw(worker_free);
            }
        }
    }
    if cancelled.load(Ordering::Acquire) {
        return Err("cancelled".into());
    }
    Ok(dto)
}

fn dispatch_model_unavailable(
    mut advisory: GenerationTimingAdvisoryDto,
    reason: &str,
) -> GenerationTimingAdvisoryDto {
    advisory.dispatch_model_status = DispatchModelStatus::Unavailable;
    advisory.dispatch_model_explanation = reason.to_owned();
    advisory.scenarios.clear();
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
        min_same_key_slack_us: None,
        same_key_repeat_count: 0,
        zero_slack_repeat_count: 0,
        effective_hold_floor_h_us: Some(effective_hold_floor_h_us),
        dispatch_model_status: DispatchModelStatus::NotAnalyzed,
        dispatch_model_explanation: "The illustrative executor model has not been computed.".into(),
        model_cost_disclaimer: MODEL_COST_DISCLAIMER.into(),
        scenarios: Vec::new(),
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
    advisory.scenarios.clear();
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

    fn schedule_chords(chords: &[(u64, &[u16])], hold_us: u64) -> ScheduleMetadata {
        let chords = chords
            .iter()
            .map(|&(at_us, scan_codes)| OnsetChord {
                at_us,
                scan_codes: scan_codes.to_vec(),
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
            min_same_key_slack_us: None,
            same_key_repeat_count: 0,
            zero_slack_repeat_count: 0,
            recommended_hold_frames: None,
            recommended_tempo_scale: None,
            warnings: Vec::new(),
        }
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
            min_same_key_slack_us: None,
            same_key_repeat_count: 0,
            zero_slack_repeat_count: 0,
            recommended_hold_frames: None,
            recommended_tempo_scale: None,
            warnings: Vec::new(),
        }
    }

    fn scenario<'a>(
        advisory: &'a GenerationTimingAdvisoryDto,
        id: &str,
    ) -> &'a DispatchModelScenarioDto {
        advisory
            .scenarios
            .iter()
            .find(|s| s.scenario_id == id)
            .unwrap_or_else(|| panic!("missing scenario {id}"))
    }

    #[test]
    fn fixture_one_key_zero_slack_ideal_reference_and_stress_hand_calculated() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us; // 17_467
        let authored = schedule(&[(0, 0x15), (h, 0x15)], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "one-key-zero-slack",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.status, TimingAdvisoryStatus::Analyzed);
        assert_eq!(advisory.total_authored_timestamp_group_count, Some(3));
        assert_eq!(advisory.down_generation_count, Some(2));
        assert_eq!(advisory.unmatched_up_count, Some(0));
        assert_eq!(advisory.min_same_key_slack_us, Some(0));
        assert_eq!(advisory.same_key_repeat_count, 1);
        assert_eq!(advisory.zero_slack_repeat_count, 1);
        assert_eq!(advisory.effective_hold_floor_h_us, Some(h));
        assert_eq!(advisory.scenarios.len(), 3);
        assert!(
            advisory
                .model_cost_disclaimer
                .contains("Illustrative modeled cost")
        );

        // Ideal (0/0/0): zero lateness, zero H-floor displacement.
        let ideal = scenario(&advisory, "ideal");
        assert_eq!(ideal.modeled_send_count, 4);
        assert_eq!(ideal.modeled_down_packet_count, 2);
        assert_eq!(ideal.modeled_up_packet_count, 2);
        assert_eq!(ideal.modeled_late_down_count, 0);
        assert_eq!(ideal.max_modeled_down_lateness_us, 0);
        assert_eq!(ideal.modeled_key_readiness_displaced_down_count, 0);
        assert_eq!(ideal.modeled_cursor_order_displaced_down_count, 0);
        assert_eq!(ideal.modeled_worker_occupancy_displaced_down_count, 0);
        assert_eq!(ideal.modeled_up_slot_count, 2);
        assert_eq!(ideal.modeled_h_floor_displaced_up_slot_count, 0);
        assert_eq!(ideal.modeled_late_up_packet_count, 0);

        // Reference (10/3/5):
        // Down0@0: completion=13, worker_free=18.
        // Up0: nominal=H, floor=H+13 -> pre_call=H+13 (h_floor_disp=13, up_lateness=13, above_floor=0),
        //      completion=H+26, worker_free=H+31 -> key_ready=H+31.
        // Down1@H: pre_call=H+31 -> key_readiness_disp=31, cursor_order=0, worker_occ=0, down_lateness=31.
        //      completion=H+44, worker_free=H+49.
        // Up1: nominal=2H, floor=2H+44 -> pre_call=2H+44 (h_floor_disp=44, up_lateness=44, above_floor=0).
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_send_count, 4);
        assert_eq!(reference.modeled_down_packet_count, 2);
        assert_eq!(reference.modeled_up_packet_count, 2);
        assert_eq!(reference.modeled_late_down_count, 1);
        assert_eq!(reference.max_modeled_down_lateness_us, 31);
        assert_eq!(reference.modeled_key_readiness_displaced_down_count, 1);
        assert_eq!(reference.max_modeled_key_readiness_displacement_us, 31);
        assert_eq!(reference.modeled_cursor_order_displaced_down_count, 0);
        assert_eq!(reference.max_modeled_cursor_order_displacement_us, 0);
        assert_eq!(reference.modeled_worker_occupancy_displaced_down_count, 0);
        assert_eq!(reference.max_modeled_worker_occupancy_residual_us, 0);
        assert_eq!(reference.modeled_up_slot_count, 2);
        assert_eq!(reference.modeled_h_floor_displaced_up_slot_count, 2);
        assert_eq!(reference.max_modeled_h_floor_displacement_us, 44);
        assert_eq!(reference.modeled_late_up_packet_count, 2);
        assert_eq!(reference.max_modeled_up_lateness_us, 44);
        assert_eq!(reference.max_modeled_up_pre_call_above_floor_us, 0);

        // Stress (100/10/50):
        // Down0@0: completion=110, worker_free=160.
        // Up0: floor=H+110 -> completion=H+220, worker_free=H+270 -> key_ready=H+270.
        // Down1@H: pre_call=H+270 -> key_readiness_disp=270, down_lateness=270.
        //      completion=H+380.
        // Up1: nominal=2H, floor=2H+380 -> h_floor_disp=380, up_lateness=380.
        let stress = scenario(&advisory, "stress");
        assert_eq!(stress.max_modeled_down_lateness_us, 270);
        assert_eq!(stress.max_modeled_key_readiness_displacement_us, 270);
        assert_eq!(stress.max_modeled_h_floor_displacement_us, 380);
    }

    #[test]
    fn fixture_positive_slack_absorbs_reference_cost_with_zero_down_lateness() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        let authored = schedule(&[(0, 0x15), (h + 500, 0x15)], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "positive-slack",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.status, TimingAdvisoryStatus::Analyzed);
        assert_eq!(advisory.min_same_key_slack_us, Some(500));
        assert_eq!(advisory.same_key_repeat_count, 1);
        assert_eq!(advisory.zero_slack_repeat_count, 0);

        // Reference: Up0 worker_free = H+31 <= H+500 -> Down1 starts right on time at H+500.
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_late_down_count, 0);
        assert_eq!(reference.max_modeled_down_lateness_us, 0);
        assert_eq!(reference.modeled_key_readiness_displaced_down_count, 0);
        assert_eq!(reference.modeled_h_floor_displaced_up_slot_count, 2);
        assert_eq!(reference.max_modeled_h_floor_displacement_us, 13);
    }

    #[test]
    fn fixture_transport_cost_drift_accumulates_across_zero_slack_chain() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        let authored = schedule(&[(0, 0x15), (h, 0x15), (2 * h, 0x15)], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "drift-chain",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.same_key_repeat_count, 2);
        assert_eq!(advisory.zero_slack_repeat_count, 2);
        assert_eq!(advisory.min_same_key_slack_us, Some(0));

        // Reference (10/3/5):
        // Down0@0 -> Up0@H+13 (free H+31) -> Down1@H+31 (lateness=31, free H+49)
        // -> Up1@2H+44 (free 2H+62) -> Down2@2H+62 (lateness=62, key_readiness=62).
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_late_down_count, 2);
        assert_eq!(reference.max_modeled_down_lateness_us, 62);
        assert_eq!(reference.modeled_key_readiness_displaced_down_count, 2);
        assert_eq!(reference.max_modeled_key_readiness_displacement_us, 62);
        assert_eq!(reference.modeled_cursor_order_displaced_down_count, 0);
        assert_eq!(reference.modeled_worker_occupancy_displaced_down_count, 0);
    }

    #[test]
    fn fixture_independent_chord_behind_blocked_chord_separates_cursor_order_and_worker_occupancy()
    {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        // Key0@0, Key0@H (blocked by Key0 Up until H+31), Key1@(H+10) (independent key!).
        let authored = schedule(&[(0, 0x15), (h, 0x15), (h + 10, 0x16)], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "cursor-order-vs-worker-occ",
            &AtomicBool::new(false),
        );
        // Reference (10/3/5):
        // Down0(Key0@0): pre_call=0, free=18.
        // Up0(Key0): pre_call=H+13, free=H+31 -> key_ready[Key0]=H+31.
        // Down1(Key0@H): pre_call=H+31 (key_ready=31, cursor=0, worker_occ=0), free=H+49.
        // Down2(Key1@H+10): pre_call=H+49, authored=H+10, key_ready[Key1]=0, prior_down=H+31.
        //   key_readiness_disp = 0
        //   cursor_order_disp = (H+31) - (H+10) = 21
        //   worker_occupancy_res = (H+49) - (H+31) = 18
        //   down_lateness = 39 == 0 + 21 + 18!
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_key_readiness_displaced_down_count, 1);
        assert_eq!(reference.max_modeled_key_readiness_displacement_us, 31);
        assert_eq!(reference.modeled_cursor_order_displaced_down_count, 1);
        assert_eq!(reference.max_modeled_cursor_order_displacement_us, 21);
        assert_eq!(reference.modeled_worker_occupancy_displaced_down_count, 1);
        assert_eq!(reference.max_modeled_worker_occupancy_residual_us, 18);
        assert_eq!(reference.max_modeled_down_lateness_us, 39);
    }

    #[test]
    fn fixture_mixed_up_and_down_retains_independent_authored_targets() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        // Key0@0 -> Up0 floor is H+13 (nominal H).
        // Key1@(H+13) -> combined into one mixed Up(Key0)+Down(Key1) packet at H+13!
        let authored = schedule(&[(0, 0x15), (h + 13, 0x16)], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "mixed-independent-targets",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.same_key_repeat_count, 0);
        assert_eq!(advisory.min_same_key_slack_us, None);

        // Reference (10/3/5):
        // Packet 0: Down(Key0)@0 -> completion=13, free=18.
        // Packet 1 (MIXED): Up(Key0) + Down(Key1) at pre_call=H+13.
        //   Up(Key0): nominal=H, floor=H+13 -> h_floor_disp=13, up_lateness=13, above_floor=0.
        //   Down(Key1): authored=H+13 -> down_lateness=0!
        //   2 events -> envelope=16, completion=H+29, free=H+34.
        // Packet 2: Up(Key1): nominal=2H+13, floor=2H+29 -> h_floor_disp=16, up_lateness=16, above_floor=0.
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_send_count, 3);
        assert_eq!(reference.modeled_down_packet_count, 2);
        assert_eq!(reference.modeled_up_packet_count, 2);
        assert_eq!(reference.modeled_late_down_count, 0);
        assert_eq!(reference.max_modeled_down_lateness_us, 0);
        assert_eq!(reference.modeled_h_floor_displaced_up_slot_count, 2);
        assert_eq!(reference.max_modeled_h_floor_displacement_us, 16);
        assert_eq!(reference.modeled_late_up_packet_count, 2);
        assert_eq!(reference.max_modeled_up_lateness_us, 16);
        assert_eq!(reference.max_modeled_up_pre_call_above_floor_us, 0);
    }

    #[test]
    fn fixture_late_up_without_down_reports_pre_call_above_floor() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        // Key0@0 -> Up0 floor = H+13 (nominal H).
        // Key1@(H+5) -> Down(Key1) at H+5 finishes at worker_free = H+23,
        // delaying Up(Key0) from floor H+13 to actual pre_call H+23!
        let authored = schedule(&[(0, 0x15), (h + 5, 0x16)], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "late-up-without-down",
            &AtomicBool::new(false),
        );
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_send_count, 4);
        assert_eq!(reference.modeled_late_down_count, 0);
        assert_eq!(reference.max_modeled_down_lateness_us, 0);
        assert_eq!(reference.modeled_late_up_packet_count, 2);
        // Up0: nominal=H, floor=H+13, actual=H+23 -> h_floor_disp=13, above_floor=10, up_lateness=23.
        assert_eq!(reference.max_modeled_h_floor_displacement_us, 13);
        assert_eq!(reference.max_modeled_up_pre_call_above_floor_us, 10);
        assert_eq!(reference.max_modeled_up_lateness_us, 23);
    }

    #[test]
    fn fixture_same_time_multi_key_chord_scales_per_input_cost() {
        let config = config(false);
        let policy = policy(&config);
        let h = policy.min_hold_us;
        let authored = schedule_chords(&[(0, &[0x15, 0x16])], h);
        let advisory = analyze_variant(
            &authored,
            &config,
            &policy,
            "two-key-chord",
            &AtomicBool::new(false),
        );
        assert_eq!(advisory.same_key_repeat_count, 0);
        assert_eq!(advisory.min_same_key_slack_us, None);

        // Reference: 2 keys -> envelope = 10 + 3*2 = 16 -> Up floor = H+16 (2 Up slots).
        let reference = scenario(&advisory, "reference");
        assert_eq!(reference.modeled_send_count, 2);
        assert_eq!(reference.modeled_down_packet_count, 1);
        assert_eq!(reference.modeled_up_packet_count, 1);
        assert_eq!(reference.modeled_up_slot_count, 2);
        assert_eq!(reference.modeled_h_floor_displaced_up_slot_count, 2);
        assert_eq!(reference.max_modeled_h_floor_displacement_us, 16);
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
        assert_eq!(
            count_authored_timestamp_groups(&[chord(0, 0), chord(1, 17_467)], 17_467).unwrap(),
            3
        );
        assert_eq!(
            count_authored_timestamp_groups(&[chord(0, 0), chord(1, 40_000)], 17_467).unwrap(),
            4
        );
        assert!(count_authored_timestamp_groups(&[chord(0, u64::MAX)], 1).is_err());
    }

    #[test]
    fn sparse_nine_note_schedule_finishes_all_three_scenarios() {
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

        assert_eq!(advisory.status, TimingAdvisoryStatus::Analyzed);
        assert_eq!(advisory.down_generation_count, Some(9));
        assert_eq!(advisory.unmatched_up_count, Some(0));
        assert_eq!(advisory.same_key_repeat_count, 8);
        assert_eq!(advisory.zero_slack_repeat_count, 0);
        assert_eq!(
            advisory.min_same_key_slack_us,
            Some(100_000 - policy.min_hold_us)
        );
        assert_eq!(advisory.scenarios.len(), 3);
        assert_eq!(scenario(&advisory, "reference").modeled_send_count, 18);
    }

    #[test]
    fn cancellation_and_invalid_policy_fail_closed_without_partial_scenarios() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[(0, 0x15), (policy.min_hold_us, 0x15)], policy.min_hold_us);

        let cancelled = AtomicBool::new(true);
        let cancelled_advisory =
            analyze_variant(&authored, &config, &policy, "cancelled", &cancelled);
        assert_eq!(cancelled_advisory.status, TimingAdvisoryStatus::Unavailable);
        assert_eq!(
            cancelled_advisory.dispatch_model_status,
            DispatchModelStatus::Unavailable
        );
        assert!(cancelled_advisory.scenarios.is_empty());

        let mut mismatched = config.clone();
        mismatched.fps = 120;
        let invalid_advisory = analyze_variant(
            &authored,
            &mismatched,
            &policy,
            "mismatched-policy",
            &AtomicBool::new(false),
        );
        assert_eq!(invalid_advisory.status, TimingAdvisoryStatus::Unavailable);
        assert_eq!(
            invalid_advisory.dispatch_model_status,
            DispatchModelStatus::Unavailable
        );
        assert!(invalid_advisory.scenarios.is_empty());
    }
}
