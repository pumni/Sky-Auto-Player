use super::*;
use crate::commands::{GenerationTimingAdvisoryDto, PlaybackConfigDto, TimingAdvisoryStatus};
use sky_app_core::timing::MaterializedTimingPolicy;

const MAX_TIMING_ADVISORY_GENERATIONS: usize = 50_000;
const MAX_TIMING_ADVISORY_WORK_UNITS: u64 = 3_000_000;
const MAX_TIMING_ADVISORY_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_KEY_VISITS_PER_GENERATION: u64 = sky_player::adapter_support::MAX_KEYS as u64;

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
        Ok(compiled) => {
            analyze_compiled_stream(&compiled, result, policy.frame_us, cancelled, || {})
        }
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
    analyze_compiled_stream(
        compiled,
        result,
        policy.frame_us,
        cancelled,
        before_analysis,
    )
}

fn analyze_compiled_stream(
    compiled: &sky_player::adapter_support::RuntimeSchedule,
    mut result: GenerationTimingAdvisoryDto,
    frame_us: u64,
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
    let memory_bytes = match compiled.memory_metrics() {
        Ok(metrics) => metrics
            .heap_payload_bytes
            .checked_add(metrics.struct_bytes)
            .and_then(|bytes| {
                bytes
                    .max(metrics.compile_memory.phase_peak_capacity_bytes)
                    .try_into()
                    .ok()
            }),
        Err(_) => None,
    };
    if memory_bytes.is_none_or(|bytes: u64| bytes > MAX_TIMING_ADVISORY_MEMORY_BYTES) {
        return unavailable(
            result,
            "The compiled generation schedule exceeds the offline advisory memory budget.",
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
                if gap_us < frame_us {
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
    let _ = work_units;
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
    result
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
        dry_run_is_hypothetical: dry_run,
    }
}

fn unavailable(
    mut advisory: GenerationTimingAdvisoryDto,
    reason: &str,
) -> GenerationTimingAdvisoryDto {
    advisory.status = TimingAdvisoryStatus::Unavailable;
    advisory.explanation = reason.to_owned();
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
