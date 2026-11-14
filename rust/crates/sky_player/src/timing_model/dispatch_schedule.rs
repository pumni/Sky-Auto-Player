use super::{
    INSTRUMENT_MASK, SlackClass, WholeStreamModelConfig, WholeStreamModelError,
    WholeStreamModelReport, WholeStreamPacketEstimate, check_cancelled, percentiles,
    signed_difference, us_to_ticks_ceil,
};
use sky_dispatch_core::executor::{DispatchDecision, DispatchExecutor};
use sky_dispatch_core::model::RuntimeSchedule;
use sky_dispatch_core::time::{DurationTicks, QpcTicks, TimelineTicks};
use std::sync::atomic::{AtomicBool, Ordering};

/// Simulate the physical packet order selected by the production scheduler,
/// advancing absolute QPC by the configured sender and post-send costs.
pub fn analyze_dispatch_schedule_cancellable(
    schedule: &RuntimeSchedule,
    config: WholeStreamModelConfig,
    cancelled: &AtomicBool,
) -> Result<WholeStreamModelReport, WholeStreamModelError> {
    check_cancelled(Some(cancelled))?;
    if config.qpc_frequency_hz == 0 {
        return Err(WholeStreamModelError::ZeroQpcFrequency);
    }
    let hold_ticks = us_to_ticks_ceil(config.effective_min_hold_us, config.qpc_frequency_hz)?;
    let release_gap_ticks =
        us_to_ticks_ceil(config.physical_release_gap_us, config.qpc_frequency_hz)?;
    let sender_setup_ticks =
        us_to_ticks_ceil(config.estimated_sender_setup_us, config.qpc_frequency_hz)?;
    let per_input_ticks = us_to_ticks_ceil(config.estimated_per_input_us, config.qpc_frequency_hz)?;
    let post_send_ticks = us_to_ticks_ceil(config.estimated_post_send_us, config.qpc_frequency_hz)?;
    let mut executor = DispatchExecutor::new_with_cancellation(
        schedule,
        DurationTicks::from_raw(hold_ticks),
        DurationTicks::from_raw(release_gap_ticks),
        |us| us_to_ticks_ceil(us, config.qpc_frequency_hz).map(TimelineTicks::from_raw),
        || {
            if cancelled.load(Ordering::Acquire) {
                Err("cancelled".to_string())
            } else {
                Ok(())
            }
        },
    )
    .map_err(|_| {
        if cancelled.load(Ordering::Acquire) {
            WholeStreamModelError::Cancelled
        } else {
            WholeStreamModelError::ArithmeticOverflow
        }
    })?;
    let epoch = QpcTicks::ZERO;
    if let Some(resume_barrier) = config.resume_down_barrier_ticks {
        let cleanup_completion = resume_barrier
            .checked_sub(release_gap_ticks)
            .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
        executor
            .on_verified_cleanup_at(QpcTicks::from_raw(cleanup_completion))
            .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?;
    }

    let mut rows = Vec::new();
    rows.try_reserve_exact(schedule.packets.len())
        .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?;
    let mut slack_values = Vec::new();
    slack_values
        .try_reserve_exact(schedule.packets.len())
        .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?;
    let mut now = epoch;
    let mut previous_worker_free = None;
    let max_iterations = schedule
        .packets
        .len()
        .checked_mul(3)
        .and_then(|count| count.checked_add(16))
        .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
    let mut iterations = 0usize;

    loop {
        if iterations.is_multiple_of(256) {
            check_cancelled(Some(cancelled))?;
        }
        iterations = iterations
            .checked_add(1)
            .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
        if iterations > max_iterations {
            return Err(WholeStreamModelError::InvalidPacketKind);
        }
        match executor
            .evaluate_at(now, epoch)
            .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?
        {
            DispatchDecision::Finished => break,
            DispatchDecision::Wait { deadline_qpc } if deadline_qpc > now => {
                now = deadline_qpc;
            }
            DispatchDecision::Wait { .. } => return Err(WholeStreamModelError::InvalidPacketKind),
            DispatchDecision::Send {
                up_mask,
                down_chord,
                timing_window,
            } => {
                let down_mask = down_chord.map_or(0, |chord| chord.down_mask);
                if (up_mask | down_mask) & !INSTRUMENT_MASK != 0
                    || up_mask & down_mask != 0
                    || up_mask | down_mask == 0
                {
                    return Err(WholeStreamModelError::InvalidPacketMask);
                }

                let authored_target = timing_window.authored_target_qpc.as_u64();
                let up_timings = executor.up_slot_timings(up_mask);
                let mut h_constraint = 0u64;
                let mut h_displacement = 0u64;
                for timing in &up_timings {
                    let up_target = epoch
                        .checked_add_duration(DurationTicks::from_raw(
                            timing.authored_up_offset_ticks.as_u64(),
                        ))
                        .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?
                        .as_u64();
                    let floor = timing.release_not_before_qpc.as_u64();
                    h_constraint = h_constraint.max(floor);
                    h_displacement = h_displacement.max(
                        floor
                            .max(up_target)
                            .checked_sub(up_target)
                            .ok_or(WholeStreamModelError::ArithmeticOverflow)?,
                    );
                }
                let mut g_constraint = 0u64;
                let mut g_displacement = 0u64;
                if let Some(chord) = down_chord {
                    let down_target = epoch
                        .checked_add_duration(DurationTicks::from_raw(
                            chord.scheduled_ticks.as_u64(),
                        ))
                        .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?
                        .as_u64();
                    for intent in chord.intents {
                        let floor = executor.slots()[intent.key_slot as usize]
                            .next_down_not_before_qpc
                            .as_u64();
                        g_constraint = g_constraint.max(floor);
                        g_displacement = g_displacement.max(
                            floor
                                .max(down_target)
                                .checked_sub(down_target)
                                .ok_or(WholeStreamModelError::ArithmeticOverflow)?,
                        );
                    }
                }
                let resume_constraint = if down_mask != 0 {
                    executor
                        .resume_barrier_qpc()
                        .map(QpcTicks::as_u64)
                        .unwrap_or(0)
                } else {
                    0
                };
                let resume_displacement = resume_constraint
                    .max(authored_target)
                    .checked_sub(authored_target)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
                let floor_ready = authored_target
                    .max(h_constraint)
                    .max(g_constraint)
                    .max(resume_constraint);
                let constraint_ready = h_constraint
                    .max(g_constraint)
                    .max(resume_constraint)
                    .max(previous_worker_free.unwrap_or(0));
                let ready = now.as_u64().max(floor_ready);
                let slack = signed_difference(authored_target, constraint_ready)?;
                let total_floor_displacement = floor_ready
                    .checked_sub(authored_target)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
                let hol_delay = previous_worker_free
                    .map(|free| free.saturating_sub(floor_ready))
                    .unwrap_or(0);
                let modeled_lateness = ready
                    .checked_sub(authored_target)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
                let event_count = u64::from(up_mask.count_ones() + down_mask.count_ones());
                let sender_envelope = sender_setup_ticks
                    .checked_add(
                        per_input_ticks
                            .checked_mul(event_count)
                            .ok_or(WholeStreamModelError::ArithmeticOverflow)?,
                    )
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
                let sender_completion = ready
                    .checked_add(sender_envelope)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
                let worker_free = sender_completion
                    .checked_add(post_send_ticks)
                    .ok_or(WholeStreamModelError::ArithmeticOverflow)?;
                executor
                    .on_send_completed_at(up_mask, down_mask, QpcTicks::from_raw(sender_completion))
                    .map_err(|_| WholeStreamModelError::ArithmeticOverflow)?;
                let packet_index = rows.len();
                rows.push(WholeStreamPacketEstimate {
                    packet_index,
                    authored_target_ticks: authored_target,
                    modeled_ready_ticks: ready,
                    modeled_sender_completion_ticks: Some(sender_completion),
                    modeled_worker_free_ticks: worker_free,
                    h_floor_displacement_ticks: h_displacement,
                    g_floor_displacement_ticks: g_displacement,
                    resume_floor_displacement_ticks: resume_displacement,
                    total_floor_displacement_ticks: total_floor_displacement,
                    hol_delay_lower_bound_ticks: hol_delay,
                    modeled_lateness_ticks: modeled_lateness,
                    signed_slack_ticks: slack,
                    slack_class: if slack > 0 {
                        SlackClass::Positive
                    } else if slack == 0 {
                        SlackClass::Zero
                    } else {
                        SlackClass::Negative
                    },
                });
                slack_values.push(slack);
                previous_worker_free = Some(worker_free);
                now = QpcTicks::from_raw(worker_free);
            }
        }
    }

    check_cancelled(Some(cancelled))?;
    let slack_ticks = percentiles(&mut slack_values);
    let mut positive_slack_count = 0u64;
    let mut zero_slack_count = 0u64;
    let mut negative_slack_count = 0u64;
    for row in &rows {
        match row.slack_class {
            SlackClass::Positive => positive_slack_count += 1,
            SlackClass::Zero => zero_slack_count += 1,
            SlackClass::Negative => negative_slack_count += 1,
        }
    }
    Ok(WholeStreamModelReport {
        model_kind: "offline_whole_stream_risk_estimate",
        guarantee: false,
        packet_count: rows.len(),
        positive_slack_count,
        zero_slack_count,
        negative_slack_count,
        slack_ticks,
        packets: rows,
    })
}
