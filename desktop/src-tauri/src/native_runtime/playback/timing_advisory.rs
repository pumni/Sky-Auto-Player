use super::*;
use crate::commands::{
    PlaybackConfigDto, TimingAdvisoryAssumptionsDto, TimingAdvisoryStatus,
    WholeStreamTimingAdvisoryDto,
};
use sky_app_core::song::ScheduleMetadata;
use sky_app_core::timing::MaterializedTimingPolicy;
use sky_player::timing_model::{
    ModelPacket, ModelPacketKind, WholeStreamModelConfig, WholeStreamModelError,
    WholeStreamPacketEstimate, analyze_whole_stream_cancellable,
};

const QPC_FREQUENCY_HZ: u64 = 1_000_000;
const ESTIMATED_SENDER_SETUP_US: u64 = 10;
const ESTIMATED_PER_INPUT_US: u64 = 3;
const ESTIMATED_POST_SEND_US: u64 = 5;
const ESTIMATED_METADATA_COST_US: u64 = 1;

/// The estimate caps authored actions before compiling the stream. A compiled
/// packet cannot outnumber its source actions, and each packet has at most 15
/// key slots, so this bounds both the compiler adapter and model work.
pub(super) const MAX_TIMING_ADVISORY_PACKETS: usize = 50_000;
const MAX_TIMING_ADVISORY_ACTIONS: usize = 50_000;
const MAX_TIMING_ADVISORY_WORK_UNITS: u64 = 3_000_000;
const MAX_TIMING_ADVISORY_MEMORY_BYTES: u64 = 64 * 1024 * 1024;
const MAX_KEY_VISITS_PER_PACKET: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TimingModelAssumptions {
    qpc_frequency_hz: u64,
    sender_setup_us: u64,
    per_input_us: u64,
    post_send_us: u64,
    metadata_cost_us: u64,
}

impl Default for TimingModelAssumptions {
    fn default() -> Self {
        Self {
            qpc_frequency_hz: QPC_FREQUENCY_HZ,
            sender_setup_us: ESTIMATED_SENDER_SETUP_US,
            per_input_us: ESTIMATED_PER_INPUT_US,
            post_send_us: ESTIMATED_POST_SEND_US,
            metadata_cost_us: ESTIMATED_METADATA_COST_US,
        }
    }
}

impl TimingModelAssumptions {
    fn to_dto(self) -> TimingAdvisoryAssumptionsDto {
        TimingAdvisoryAssumptionsDto {
            qpc_frequency_hz: self.qpc_frequency_hz,
            estimated_sender_setup_us: self.sender_setup_us,
            estimated_per_input_us: self.per_input_us,
            estimated_post_send_us: self.post_send_us,
            estimated_metadata_cost_us: self.metadata_cost_us,
        }
    }

    fn model_config(self, policy: &MaterializedTimingPolicy) -> WholeStreamModelConfig {
        WholeStreamModelConfig {
            qpc_frequency_hz: self.qpc_frequency_hz,
            effective_min_hold_us: policy.min_hold_us,
            physical_release_gap_us: policy.frame_us,
            estimated_sender_setup_us: self.sender_setup_us,
            estimated_per_input_us: self.per_input_us,
            estimated_post_send_us: self.post_send_us,
            estimated_metadata_cost_us: self.metadata_cost_us,
            // Prepare has no known resumed-session barrier. A live resume
            // barrier is not guessed or projected into this authored stream.
            resume_down_barrier_ticks: None,
        }
    }
}

pub(super) fn analyze_variant(
    schedule: &ScheduleMetadata,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
) -> WholeStreamTimingAdvisoryDto {
    analyze_variant_with_assumptions(
        schedule,
        config,
        policy,
        plan_fingerprint,
        cancelled,
        TimingModelAssumptions::default(),
    )
}

#[cfg(test)]
pub(super) fn analyze_variant_with_model_hook(
    schedule: &ScheduleMetadata,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
    before_model: impl FnOnce(),
) -> WholeStreamTimingAdvisoryDto {
    analyze_variant_with_assumptions_and_hook(
        schedule,
        config,
        policy,
        plan_fingerprint,
        cancelled,
        TimingModelAssumptions::default(),
        before_model,
    )
}

fn analyze_variant_with_assumptions(
    schedule: &ScheduleMetadata,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
    assumptions: TimingModelAssumptions,
) -> WholeStreamTimingAdvisoryDto {
    analyze_variant_with_assumptions_and_hook(
        schedule,
        config,
        policy,
        plan_fingerprint,
        cancelled,
        assumptions,
        || {},
    )
}

fn analyze_variant_with_assumptions_and_hook(
    schedule: &ScheduleMetadata,
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    plan_fingerprint: &str,
    cancelled: &AtomicBool,
    assumptions: TimingModelAssumptions,
    before_model: impl FnOnce(),
) -> WholeStreamTimingAdvisoryDto {
    let mut result = empty_advisory(plan_fingerprint, assumptions, config.dry_run, None);
    if cancelled.load(Ordering::Acquire) {
        return unavailable(result, "analysis was cancelled before packet preparation");
    }
    if let Err(reason) = validate_assumptions(config, policy, assumptions) {
        result.status = TimingAdvisoryStatus::InvalidAssumptions;
        result.explanation = reason;
        return result;
    }
    result.effective_hold_floor_h_us = Some(policy.min_hold_us);
    result.authored_release_gap_floor_g_us = Some(policy.min_release_gap_us);
    result.physical_release_gap_floor_g_us = Some(policy.frame_us);
    result.structural_same_key_cycle_minimum_us =
        policy.min_hold_us.checked_add(policy.min_release_gap_us);
    if schedule.actions.len() > MAX_TIMING_ADVISORY_ACTIONS {
        return unavailable(
            result,
            &format!(
                "The schedule has {} authored actions; the offline advisory limit is {}.",
                schedule.actions.len(),
                MAX_TIMING_ADVISORY_ACTIONS
            ),
        );
    }
    let estimated_work = match u64::try_from(schedule.actions.len())
        .ok()
        .and_then(|count| count.checked_mul(MAX_KEY_VISITS_PER_PACKET))
    {
        Some(work) if work <= MAX_TIMING_ADVISORY_WORK_UNITS => work,
        _ => {
            return unavailable(
                result,
                "The schedule exceeds the offline advisory work budget.",
            );
        }
    };
    let _estimated_memory = match estimated_peak_bytes(schedule.actions.len()) {
        Some(bytes) if bytes <= MAX_TIMING_ADVISORY_MEMORY_BYTES => bytes,
        _ => {
            return unavailable(
                result,
                "The schedule exceeds the offline advisory memory budget.",
            );
        }
    };
    if cancelled.load(Ordering::Acquire) {
        return unavailable(result, "analysis was cancelled before packet compilation");
    }

    let compiled = match super::session::compile_dispatch_schedule(schedule) {
        Ok(compiled) => compiled,
        Err(error) => {
            return unavailable(
                result,
                &format!("The canonical packet stream could not be modeled: {error}"),
            );
        }
    };
    if cancelled.load(Ordering::Acquire) {
        return unavailable(result, "analysis was cancelled after packet compilation");
    }
    if compiled.packets.len() > MAX_TIMING_ADVISORY_PACKETS {
        return unavailable(
            result,
            "The compiled packet stream exceeds the offline advisory limit.",
        );
    }
    let actual_work = match u64::try_from(compiled.packets.len())
        .ok()
        .and_then(|count| count.checked_mul(MAX_KEY_VISITS_PER_PACKET))
    {
        Some(work) if work <= MAX_TIMING_ADVISORY_WORK_UNITS => work,
        _ => {
            return unavailable(
                result,
                "The compiled packet stream exceeds the offline advisory work budget.",
            );
        }
    };
    if actual_work > estimated_work {
        return unavailable(
            result,
            "The compiled packet stream exceeded its preflight work bound.",
        );
    }
    result.total_packet_count = Some(compiled.packets.len() as u64);

    let mut model_packets = Vec::with_capacity(compiled.packets.len());
    for (index, packet) in compiled.packets.iter().enumerate() {
        if index % 256 == 0 && cancelled.load(Ordering::Acquire) {
            result.packet_count_inspected = index as u64;
            return unavailable(result, "analysis was cancelled while preparing packets");
        }
        if usize::try_from(packet.packet_id).ok() != Some(index) {
            result.packet_count_inspected = index as u64;
            return unavailable(
                result,
                "The canonical packet identifiers are not contiguous.",
            );
        }
        result.packet_count_inspected = (index + 1) as u64;
        let kind = if packet.up_mask == 0 && packet.down_mask == 0 {
            ModelPacketKind::Metadata
        } else {
            ModelPacketKind::Physical
        };
        model_packets.push(ModelPacket {
            // The prepared authored timeline is relative to its epoch and
            // already expressed in microseconds. At 1 MHz, ticks are µs.
            authored_target_ticks: packet.scheduled_us,
            up_mask: packet.up_mask,
            down_mask: packet.down_mask,
            kind,
        });
    }

    // Test callers can stop at the adapter/model boundary to deterministically
    // exercise cancellation while an advisory analysis is in flight.
    before_model();
    let report = match analyze_whole_stream_cancellable(
        &model_packets,
        assumptions.model_config(policy),
        cancelled,
    ) {
        Ok(report) => report,
        Err(WholeStreamModelError::Cancelled) => {
            return unavailable(result, "analysis was cancelled while simulating packets");
        }
        Err(WholeStreamModelError::ZeroQpcFrequency) => {
            result.status = TimingAdvisoryStatus::InvalidAssumptions;
            result.explanation = "The model QPC frequency must be nonzero.".into();
            return result;
        }
        Err(WholeStreamModelError::ArithmeticOverflow) => {
            result.status = TimingAdvisoryStatus::InvalidAssumptions;
            result.explanation = "The timing model assumptions overflow checked arithmetic.".into();
            return result;
        }
        Err(error) => {
            return unavailable(
                result,
                &format!("The compiled stream could not be modeled: {error}"),
            );
        }
    };
    summarize_report(&mut result, &report.packets);
    result.packet_count_analyzed = report.packet_count as u64;
    result.status = if result.negative_slack_packet_count != 0
        || result.h_floor_displaced_packet_count != 0
        || result.g_floor_displaced_packet_count != 0
        || result.head_of_line_packet_count != 0
    {
        TimingAdvisoryStatus::ModeledRisk
    } else {
        TimingAdvisoryStatus::NoModeledRisk
    };
    result.explanation = match result.status {
        TimingAdvisoryStatus::NoModeledRisk => format!(
            "No lateness, H/G floor displacement, or head-of-line delay was modeled across {} packets.",
            result.packet_count_analyzed
        ),
        TimingAdvisoryStatus::ModeledRisk => format!(
            "The illustrative model found {} negative-slack packet(s), {} H-floor and {} G-floor displacement(s), and {} head-of-line delay(s).",
            result.negative_slack_packet_count,
            result.h_floor_displaced_packet_count,
            result.g_floor_displaced_packet_count,
            result.head_of_line_packet_count
        ),
        _ => result.explanation,
    };
    result
}

fn validate_assumptions(
    config: &PlaybackConfigDto,
    policy: &MaterializedTimingPolicy,
    assumptions: TimingModelAssumptions,
) -> Result<(), String> {
    if assumptions.qpc_frequency_hz == 0 {
        return Err("The model QPC frequency must be nonzero.".into());
    }
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
    let expected_authored_gap = policy
        .frame_us
        .checked_add(policy.timing_margin_us)
        .ok_or_else(|| "The authored release gap overflows.".to_string())?;
    if policy.frame_us == 0
        || policy.min_hold_us != expected_hold
        || policy.min_release_gap_us != expected_authored_gap
    {
        return Err("The materialized H/G timing floors are inconsistent.".into());
    }
    policy
        .min_hold_us
        .checked_add(policy.min_release_gap_us)
        .ok_or_else(|| "The structural same-key cycle floor overflows.".to_string())?;
    Ok(())
}

fn estimated_peak_bytes(action_count: usize) -> Option<u64> {
    use sky_player::adapter_support::{
        CompactIntent, CompiledBatch, CompiledPacket, KeyActionInput, MAX_REASON_BYTES,
    };

    let per_action = size_of::<KeyActionInput>()
        .checked_add(size_of::<CompiledBatch>())?
        .checked_add(size_of::<CompiledPacket>())?
        .checked_add(size_of::<ModelPacket>())?
        .checked_add(size_of::<WholeStreamPacketEstimate>())?
        .checked_add(size_of::<i64>())?
        // Covers three copies of a maximum-length reason across Arc, reason
        // table, and map storage, plus SmallVec spill and hash allocation.
        .checked_add(3 * MAX_REASON_BYTES)?
        .checked_add(256)?;
    let intents = action_count
        .checked_mul(15)?
        .checked_mul(size_of::<CompactIntent>())?;
    let total = action_count.checked_mul(per_action)?.checked_add(intents)?;
    u64::try_from(total).ok()
}

fn empty_advisory(
    plan_fingerprint: &str,
    assumptions: TimingModelAssumptions,
    dry_run: bool,
    total_packet_count: Option<u64>,
) -> WholeStreamTimingAdvisoryDto {
    WholeStreamTimingAdvisoryDto {
        status: TimingAdvisoryStatus::NotAnalyzed,
        plan_fingerprint: plan_fingerprint.to_owned(),
        explanation: "The offline timing advisory has not been computed.".into(),
        total_packet_count,
        packet_count_inspected: 0,
        packet_count_analyzed: 0,
        packet_limit: MAX_TIMING_ADVISORY_PACKETS as u64,
        work_limit_units: MAX_TIMING_ADVISORY_WORK_UNITS,
        working_memory_limit_bytes: MAX_TIMING_ADVISORY_MEMORY_BYTES,
        negative_slack_packet_count: 0,
        h_floor_displaced_packet_count: 0,
        max_h_floor_displacement_us: 0,
        g_floor_displaced_packet_count: 0,
        max_g_floor_displacement_us: 0,
        head_of_line_packet_count: 0,
        max_head_of_line_delay_us: 0,
        effective_hold_floor_h_us: None,
        authored_release_gap_floor_g_us: None,
        physical_release_gap_floor_g_us: None,
        structural_same_key_cycle_minimum_us: None,
        assumptions: assumptions.to_dto(),
        receiver_acknowledged: false,
        dry_run_is_hypothetical: dry_run,
    }
}

fn unavailable(
    mut advisory: WholeStreamTimingAdvisoryDto,
    reason: &str,
) -> WholeStreamTimingAdvisoryDto {
    advisory.status = TimingAdvisoryStatus::Unavailable;
    advisory.explanation = reason.to_owned();
    advisory
}

fn summarize_report(
    advisory: &mut WholeStreamTimingAdvisoryDto,
    rows: &[WholeStreamPacketEstimate],
) {
    for row in rows {
        if row.signed_slack_ticks < 0 {
            advisory.negative_slack_packet_count += 1;
        }
        if row.h_floor_displacement_ticks > 0 {
            advisory.h_floor_displaced_packet_count += 1;
            advisory.max_h_floor_displacement_us = advisory
                .max_h_floor_displacement_us
                .max(row.h_floor_displacement_ticks);
        }
        if row.t_floor_displacement_ticks > 0 {
            advisory.g_floor_displaced_packet_count += 1;
            advisory.max_g_floor_displacement_us = advisory
                .max_g_floor_displacement_us
                .max(row.t_floor_displacement_ticks);
        }
        if row.hol_delay_lower_bound_ticks > 0 {
            advisory.head_of_line_packet_count += 1;
            advisory.max_head_of_line_delay_us = advisory
                .max_head_of_line_delay_us
                .max(row.hol_delay_lower_bound_ticks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_app_core::song::{ActionKind, KeyAction, Song, build_schedule_with_policy};

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

    fn schedule(actions: &[(ActionKind, u64, Vec<u16>)]) -> ScheduleMetadata {
        let actions = actions
            .iter()
            .map(|(kind, at_us, scan_codes)| KeyAction {
                at_us: *at_us,
                scan_codes: scan_codes.clone(),
                kind: *kind,
                reason: "timing-advisory-test".into(),
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
            compressed_holds: 0,
            impossible_same_key_repeats: 0,
            risky_same_key_repeats: 0,
            max_polyphony: 0,
            shortest_same_key_interval_us: None,
            min_same_key_up_gap_us: None,
            recommended_hold_frames: None,
            recommended_tempo_scale: None,
            warnings: Vec::new(),
        }
    }

    fn analyze(
        schedule: &ScheduleMetadata,
        config: &PlaybackConfigDto,
        policy: &MaterializedTimingPolicy,
        fingerprint: &str,
    ) -> WholeStreamTimingAdvisoryDto {
        analyze_variant(
            schedule,
            config,
            policy,
            fingerprint,
            &AtomicBool::new(false),
        )
    }

    #[test]
    fn empty_sparse_mixed_and_fifteen_key_packets_keep_canonical_boundaries() {
        let config = config(false);
        let policy = policy(&config);
        let empty = schedule(&[]);
        let empty_result = analyze(&empty, &config, &policy, "empty-plan");
        assert_eq!(empty_result.status, TimingAdvisoryStatus::NoModeledRisk);
        assert_eq!(empty_result.total_packet_count, Some(0));

        let mixed_schedule = schedule(&[
            (ActionKind::Down, 0, vec![0x15]),
            (ActionKind::Up, 1_000, vec![0x15]),
            (ActionKind::Down, 1_000, vec![0x16]),
            (ActionKind::Up, 40_000, vec![0x16]),
        ]);
        let mixed = analyze(&mixed_schedule, &config, &policy, "mixed-plan");
        assert_eq!(mixed.total_packet_count, Some(3));
        assert_eq!(mixed.packet_count_inspected, 3);
        assert_eq!(mixed.packet_count_analyzed, 3);
        assert_eq!(mixed.plan_fingerprint, "mixed-plan");

        let sparse_schedule = schedule(&[
            (ActionKind::Down, 0, vec![0x15]),
            (ActionKind::Up, 100_000, vec![0x15]),
        ]);
        let sparse = analyze(&sparse_schedule, &config, &policy, "sparse-plan");
        assert_eq!(sparse.status, TimingAdvisoryStatus::NoModeledRisk);
        assert_eq!(sparse.negative_slack_packet_count, 0);

        let chord_codes = sky_app_core::song::SKY_SCAN_CODES
            .iter()
            .take(15)
            .copied()
            .collect::<Vec<_>>();
        let two_key_codes = chord_codes.iter().take(2).copied().collect::<Vec<_>>();
        let two_key_chord_schedule = schedule(&[
            (ActionKind::Down, 0, two_key_codes.clone()),
            (ActionKind::Up, policy.min_hold_us, two_key_codes),
        ]);
        let two_key_chord = analyze(
            &two_key_chord_schedule,
            &config,
            &policy,
            "two-key-chord-plan",
        );
        assert_eq!(two_key_chord.total_packet_count, Some(2));

        let chord_schedule = schedule(&[
            (ActionKind::Down, 0, chord_codes.clone()),
            (ActionKind::Up, policy.min_hold_us, chord_codes),
        ]);
        let chord = analyze(&chord_schedule, &config, &policy, "chord-plan");
        assert_eq!(chord.total_packet_count, Some(2));
        assert_eq!(chord.status, TimingAdvisoryStatus::ModeledRisk);
    }

    #[test]
    fn dense_alternating_keys_keep_authored_packet_count_and_report_modeled_pressure() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[
            (ActionKind::Down, 0, vec![0x15]),
            (ActionKind::Up, 500, vec![0x15]),
            (ActionKind::Down, 600, vec![0x16]),
            (ActionKind::Up, 1_000, vec![0x16]),
            (ActionKind::Down, 1_100, vec![0x15]),
            (ActionKind::Up, 1_500, vec![0x15]),
            (ActionKind::Down, 1_600, vec![0x16]),
            (ActionKind::Up, 2_000, vec![0x16]),
        ]);
        let advisory = analyze(&authored, &config, &policy, "dense-alternation-plan");
        assert_eq!(advisory.total_packet_count, Some(8));
        assert_eq!(advisory.packet_count_analyzed, 8);
        assert_eq!(advisory.status, TimingAdvisoryStatus::ModeledRisk);
        assert_eq!(authored.actions.len(), 8);
    }

    #[test]
    fn supported_fps_hold_profiles_and_margin_edges_preserve_each_materialized_floor() {
        for fps in sky_app_core::settings::VALID_FPS {
            for hold_frames in sky_app_core::settings::HOLD_FRAME_OPTIONS {
                for timing_margin_us in [0, sky_app_core::settings::MAX_TIMING_MARGIN_US] {
                    let config = PlaybackConfigDto {
                        fps,
                        hold_frames,
                        timing_margin_us,
                        ..config(false)
                    };
                    let policy = policy(&config);
                    let advisory = analyze(&schedule(&[]), &config, &policy, "boundary-plan");
                    assert_eq!(advisory.status, TimingAdvisoryStatus::NoModeledRisk);
                    assert_eq!(advisory.effective_hold_floor_h_us, Some(policy.min_hold_us));
                    assert_eq!(
                        advisory.authored_release_gap_floor_g_us,
                        Some(policy.min_release_gap_us)
                    );
                    assert_eq!(
                        advisory.physical_release_gap_floor_g_us,
                        Some(policy.frame_us)
                    );
                    assert_eq!(
                        advisory.structural_same_key_cycle_minimum_us,
                        Some(policy.min_hold_us + policy.min_release_gap_us)
                    );
                }
            }
        }
    }

    #[test]
    fn near_floor_same_key_cycle_models_h_and_physical_g_without_changing_authored_actions() {
        let config = config(false);
        let policy = policy(&config);
        let cycle = policy
            .min_hold_us
            .checked_add(policy.min_release_gap_us)
            .expect("authored cycle floor");
        let authored = schedule(&[
            (ActionKind::Down, 0, vec![0x15]),
            (ActionKind::Up, policy.min_hold_us, vec![0x15]),
            (ActionKind::Down, cycle, vec![0x15]),
            (ActionKind::Up, cycle + policy.min_hold_us, vec![0x15]),
        ]);
        let before = authored.clone();
        let first = analyze(&authored, &config, &policy, "same-key-plan");
        let second = analyze(&authored, &config, &policy, "same-key-plan");
        assert_eq!(first, second);
        assert_eq!(authored, before);
        assert_eq!(first.effective_hold_floor_h_us, Some(policy.min_hold_us));
        assert_eq!(
            first.authored_release_gap_floor_g_us,
            Some(policy.min_release_gap_us)
        );
        assert_eq!(first.physical_release_gap_floor_g_us, Some(policy.frame_us));
        assert_eq!(
            first.structural_same_key_cycle_minimum_us,
            Some(policy.min_hold_us + policy.min_release_gap_us)
        );
        assert_eq!(authored.actions[2].at_us, cycle);
    }

    #[test]
    fn unrelated_key_head_of_line_pressure_is_reported_without_admission_changes() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[
            (ActionKind::Down, 0, vec![0x15, 0x16]),
            (ActionKind::Up, 80_000, vec![0x15, 0x16]),
            (ActionKind::Down, 80_001, vec![0x17]),
            (ActionKind::Up, 120_000, vec![0x17]),
        ]);
        let advisory = analyze(&authored, &config, &policy, "hol-plan");
        assert_eq!(advisory.status, TimingAdvisoryStatus::ModeledRisk);
        assert!(advisory.head_of_line_packet_count > 0);
        assert!(advisory.max_head_of_line_delay_us > 0);
        assert_eq!(authored.impossible_same_key_repeats, 0);
    }

    #[test]
    fn validates_floor_assumptions_and_preserves_explicit_unavailable_statuses() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[]);
        let mut mismatched = config.clone();
        mismatched.fps = 30;
        let invalid = analyze(&authored, &mismatched, &policy, "invalid-plan");
        assert_eq!(invalid.status, TimingAdvisoryStatus::InvalidAssumptions);

        let too_large = schedule(
            &(0..=MAX_TIMING_ADVISORY_PACKETS)
                .map(|index| {
                    (
                        if index % 2 == 0 {
                            ActionKind::Down
                        } else {
                            ActionKind::Up
                        },
                        index as u64,
                        vec![0x15],
                    )
                })
                .collect::<Vec<_>>(),
        );
        let unavailable = analyze(&too_large, &config, &policy, "large-plan");
        assert_eq!(unavailable.status, TimingAdvisoryStatus::Unavailable);
        assert_eq!(unavailable.packet_count_inspected, 0);
        assert_eq!(unavailable.packet_limit, MAX_TIMING_ADVISORY_PACKETS as u64);
    }

    #[test]
    fn cancellation_and_invalid_authored_order_fail_closed_as_advisory_only() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[
            (ActionKind::Down, 0, vec![0x15]),
            (ActionKind::Up, 20_000, vec![0x15]),
        ]);
        let cancelled = analyze_variant_with_assumptions(
            &authored,
            &config,
            &policy,
            "cancelled-plan",
            &AtomicBool::new(true),
            TimingModelAssumptions::default(),
        );
        assert_eq!(cancelled.status, TimingAdvisoryStatus::Unavailable);

        let mut invalid_order = authored.clone();
        invalid_order.actions[1].at_us = 0;
        let invalid = analyze(&invalid_order, &config, &policy, "invalid-order-plan");
        assert_eq!(invalid.status, TimingAdvisoryStatus::Unavailable);
        assert!(invalid.explanation.contains("could not be modeled"));
    }

    #[test]
    fn physical_assumption_overflow_is_reported_as_invalid_assumptions() {
        let config = config(false);
        let policy = policy(&config);
        let authored = schedule(&[]);
        let advisory = analyze_variant_with_assumptions(
            &authored,
            &config,
            &policy,
            "overflow-plan",
            &AtomicBool::new(false),
            TimingModelAssumptions {
                qpc_frequency_hz: u64::MAX,
                sender_setup_us: u64::MAX,
                per_input_us: 0,
                post_send_us: 0,
                metadata_cost_us: 0,
            },
        );
        assert_eq!(advisory.status, TimingAdvisoryStatus::InvalidAssumptions);
    }

    #[test]
    fn dry_run_marks_physical_projection_as_hypothetical_and_fingerprint_specific() {
        let config = config(true);
        let policy = policy(&config);
        let authored = schedule(&[
            (ActionKind::Down, 0, vec![0x15]),
            (ActionKind::Up, 50_000, vec![0x15]),
        ]);
        let first = analyze(&authored, &config, &policy, "dry-run-a");
        let second = analyze(&authored, &config, &policy, "dry-run-b");
        assert!(first.dry_run_is_hypothetical);
        assert_eq!(first.plan_fingerprint, "dry-run-a");
        assert_ne!(first.plan_fingerprint, second.plan_fingerprint);
        assert_eq!(
            first.status,
            TimingAdvisoryStatus::NoModeledRisk,
            "long authored hold leaves modeled slack under the fixed assumptions"
        );
    }

    #[test]
    fn adapter_can_construct_an_empty_model_from_a_real_domain_song() {
        let config = config(false);
        let policy = policy(&config);
        let song = Song {
            name: "empty".into(),
            notes: Vec::new(),
        };
        let authored = build_schedule_with_policy(&song, config.tempo_scale, &policy)
            .expect("empty domain song schedule");
        let advisory = analyze(&authored, &config, &policy, "domain-empty");
        assert_eq!(advisory.status, TimingAdvisoryStatus::NoModeledRisk);
        assert_eq!(advisory.total_packet_count, Some(0));
    }
}
