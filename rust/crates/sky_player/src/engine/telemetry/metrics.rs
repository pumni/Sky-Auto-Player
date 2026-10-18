use parking_lot::{Mutex, RwLock};
#[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
use sky_dispatch_core::coordinator::GenerationAccounting;
use sky_dispatch_core::time::DurationTicks;
use sky_dispatch_win32::clock::QpcClock;
use sky_dispatch_win32::input::ReleaseAllOutcome;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

#[derive(Debug, Clone, Default)]
pub(crate) struct RecentLatencyRing {
    values: [i32; 32],
    next: u8,
    len: u8,
}

fn signed_qpc_delta(value: u64, baseline: u64) -> Option<i64> {
    i64::try_from(i128::from(value) - i128::from(baseline)).ok()
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PrecisionPacketTimingObservation {
    pub(crate) authored_target_qpc: u64,
    pub(crate) packet_not_before_qpc: u64,
    pub(crate) pre_call_qpc: Option<u64>,
    pub(crate) sendinput_completion_qpc: Option<u64>,
    pub(crate) previous_sendinput_completion_qpc: Option<u64>,
    pub(crate) previous_sender_completion_known: bool,
    pub(crate) transport_anomaly: bool,
}

impl RecentLatencyRing {
    pub(crate) fn push(&mut self, value: i64) {
        self.values[usize::from(self.next)] =
            value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        self.next = (self.next + 1) % self.values.len() as u8;
        self.len = self.len.saturating_add(1).min(self.values.len() as u8);
    }

    pub(crate) fn to_vec(&self) -> Vec<i64> {
        let len = usize::from(self.len);
        let start = if self.len == self.values.len() as u8 {
            usize::from(self.next)
        } else {
            0
        };
        (0..len)
            .map(|offset| i64::from(self.values[(start + offset) % self.values.len()]))
            .collect()
    }

    fn extend_from(&mut self, other: &Self) {
        let len = usize::from(other.len);
        let start = if other.len == other.values.len() as u8 {
            usize::from(other.next)
        } else {
            0
        };
        for offset in 0..len {
            self.push(i64::from(
                other.values[(start + offset) % other.values.len()],
            ));
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkerMetricsLocal {
    /// Legacy worker-publication field. Live session snapshots derive
    /// playback progress from the shared transition-only clock projection.
    pub elapsed_us: u64,
    pub total_us: u64,
    pub lateness_us: u64,
    pub max_lateness_us: u64,
    pub late_2ms: u64,
    pub late_5ms: u64,
    pub late_10ms: u64,
    /// Essential production scalar evidence measured immediately before the
    /// prepared SendInput call.
    ///
    /// This remains the public snapshot field.  The producer updates the
    /// tick-domain companion below and publication derives this value once.
    pub max_sendinput_pre_call_lateness_us: u64,
    pub(crate) max_sendinput_pre_call_lateness_ticks: u64,
    /// Mutually-exclusive production pre-call lateness buckets. These are
    /// scalar-only and use the same QPC domain as the legacy thresholds.
    pub pre_call_lt_250us: u64,
    pub pre_call_250_500us: u64,
    pub pre_call_500_750us: u64,
    pub pre_call_750_1000us: u64,
    pub pre_call_1000_1500us: u64,
    pub pre_call_1500_2000us: u64,
    pub pre_call_ge_2000us: u64,
    pub pre_call_late_2ms: u64,
    pub pre_call_late_5ms: u64,
    pub pre_call_late_10ms: u64,
    /// Always-on bounded timing observations captured at the sender return
    /// boundary; these are independent of the diagnostic observer queue.
    pub precision_packet_attempts: u64,
    pub pre_call_qpc_unavailable_attempts: u64,
    pub sendinput_completion_qpc_unavailable_attempts: u64,
    pub authored_lateness_samples: u64,
    pub max_authored_lateness_ticks: i64,
    pub max_authored_lateness_abs_ticks: u64,
    pub physical_floor_displacement_samples: u64,
    pub max_physical_floor_displacement_ticks: u64,
    pub residual_lateness_samples: u64,
    pub max_residual_lateness_ticks: i64,
    pub max_residual_lateness_abs_ticks: u64,
    pub hol_delay_lower_bound_samples: u64,
    pub max_hol_delay_lower_bound_ticks: u64,
    pub unattributed_lateness_samples: u64,
    pub max_unattributed_lateness_ticks: u64,
    pub sendinput_envelope_samples: u64,
    pub max_sendinput_envelope_ticks: u64,
    pub overdue_packet_observations: u64,
    pub transport_anomalies: u64,
    pub timing_arithmetic_anomalies: u64,
    pub release_max_us: u64,
    pub release_late_2ms: u64,
    pub active_count: u64,
    pub possibly_active_count: u64,
    pub failed_release_count: u64,
    pub keys_dropped: u64,
    pub chord_split_events: u64,
    pub chord_integrity_lost: u64,
    pub sendinput_partial_events: u64,
    pub sendinput_zero_progress_failures: u64,
    pub chords_rejected: u64,
    pub authored_conflict_events: u64,
    pub authored_chords_rejected: u64,
    pub authored_keys_rejected: u64,
    /// Production musical deadline recovery evidence. These are scalar-only
    /// worker counters and never allocate on the dispatch path.
    pub missed_down_boundaries: u64,
    pub missed_down_keys: u64,
    pub missed_unobserved_backlog_boundaries: u64,
    /// Deprecated compatibility counter; physical-window rejection is retired
    /// and this remains zero in production.
    pub missed_physical_window_boundaries: u64,
    /// Prepared-normal acceptance counters. These are distinct from
    /// authored/dynamic recovery counters so the deterministic report can
    /// prove the prepared miss resolver did not enter dynamic policy.
    pub prepared_up_prefix_recovery_sends: u64,
    pub prepared_normal_backlog_boundaries: u64,
    /// Deprecated compatibility counter; prepared sender expiry is retired and
    /// this remains zero in production.
    pub prepared_normal_sender_expirations: u64,
    /// Final control and modifier gate rejections observed before the
    /// authoritative SendInput timestamp.
    pub final_gate_control_rejections: u64,
    /// Final target and bound-owner authority rejections observed before the
    /// authoritative SendInput timestamp.
    pub final_gate_target_changes: u64,
    pub final_gate_focus_losses: u64,
    /// Final supervisor-lease gate rejections. The compatibility field name
    /// includes both expired and unproven lease outcomes.
    pub final_gate_lease_expirations: u64,
    /// Deprecated compatibility counter; sender-window rejection is retired
    /// and this remains zero in production.
    pub final_sender_window_expirations: u64,
    /// Deprecated compatibility counter; release floors never reject a Down
    /// and this remains zero in production.
    pub release_floor_infeasible_boundaries: u64,
    pub max_missed_lateness_ticks: u64,
    /// Counts one authored packet boundary when its fixed physical floor is
    /// later than the authored target. QPC timestamps and masks below retain
    /// the latest contributing boundary without per-note storage.
    pub hold_floor_delay_boundaries: u64,
    pub(crate) max_hold_floor_delay_ticks: u64,
    pub max_hold_floor_delay_us: u64,
    pub last_hold_floor_delay_mask: u16,
    pub last_hold_floor_authored_target_qpc_ticks: u64,
    pub last_hold_floor_not_before_qpc_ticks: u64,
    pub release_floor_delay_boundaries: u64,
    pub(crate) max_release_floor_delay_ticks: u64,
    pub max_release_floor_delay_us: u64,
    pub last_release_floor_delay_mask: u16,
    pub last_release_floor_authored_target_qpc_ticks: u64,
    pub last_release_floor_not_before_qpc_ticks: u64,
    /// Worker-local fixed-size production forensics. Availability is false
    /// until a successful physical packet has supplied evidence.
    pub production_forensics_available: bool,
    pub production_forensics_version: u32,
    pub production_hold_pair_samples: u64,
    pub production_min_hold_start_after_down_completion_ticks: u64,
    pub production_hold_floor_violation_count: u64,
    pub production_release_floor_samples: u64,
    pub production_min_down_start_after_up_completion_ticks: u64,
    pub production_release_floor_violation_count: u64,
    pub production_hold_floor_ticks: u64,
    pub production_release_floor_ticks: u64,
    /// Corruption/forensics evidence for a packet that violated the
    /// disjoint Up/Down mask contract before realtime execution.
    pub production_same_key_overlap_forensics_count: u64,
    pub production_anchor_overwrite_count: u64,
    pub production_unmatched_up_count: u64,
    pub production_anomaly_ring_overwrite_count: u64,
    pub production_forensics_anomaly_count: u64,
    pub production_structural_anomaly_count: u64,
    pub production_timing_diagnostic_count: u64,
    /// Fixed-width evidence for the most recently classified missed Down.
    /// This is populated after classification and remains absent from the
    /// lightweight live snapshot.
    pub last_missed_down_valid: bool,
    pub last_missed_down_reason_code: u8,
    pub last_missed_down_source_action_index: u32,
    pub last_missed_down_mask: u16,
    pub last_missed_down_lateness_ticks: u64,
    pub keys_inserted_before_failure: u64,
    pub keys_rolled_back: u64,
    pub rollback_residue_keys: u64,
    /// Saturation diagnostics indexed by physical event count. Index 0 is
    /// unused; index 15 is the explicit `15_plus` overflow bucket.
    pub lead_saturation_count_down: [u64; 16],
    pub lead_saturation_count_up: [u64; 16],
    pub positive_residual_at_cap: u64,
    pub recovered_zero_progress_retries: u64,
    pub recovered_partial_up_retries: u64,
    pub recovered_zero_progress_but_late: u64,
    /// Startup-only calibration provenance. These scalar values are written
    /// before dispatch and are never touched by the precision path.
    pub startup_calibration_executed: bool,
    pub startup_calibration_sample_count: u64,
    pub startup_calibration_duration_us: u64,
    pub startup_calibration_result: u8,
    pub startup_wake_error_robust_us: u64,
    pub spin_threshold_source: u8,
    pub effective_spin_threshold_us: u64,
    pub wake_error_p50_us: u64,
    pub wake_error_p95_us: u64,
    pub wake_error_p99_us: u64,
    pub wake_error_max_us: u64,
    pub spin_time_us: u64,
    pub playback_wall_time_us: u64,
    pub spin_duty_cycle_ppm: u64,
    pub worker_cpu_time_us: u64,
    pub process_cpu_time_us: u64,
    pub power_throttling_disabled: bool,
    pub input_path_degraded: bool,
    pub sendinput_path_degraded: bool,
    pub core_post_send_degraded: bool,
    pub post_send_metrics_available: bool,
    pub observer_degraded: bool,
    pub wait_path_degraded: bool,
    pub sendinput_warn_threshold_us: u64,
    pub core_post_send_warn_threshold_us: u64,
    pub observer_warn_threshold_us: u64,
    pub wait_warn_threshold_us: u64,
    pub sendinput_degraded_samples: u64,
    pub core_post_send_degraded_samples: u64,
    pub observer_degraded_samples: u64,
    pub wait_degraded_samples: u64,
    pub wait_backend_failures: u64,
    pub wait_clock_failures: u64,
    pub wait_interrupted_count: u64,
    pub sendinput_window_bad_count: u64,
    pub core_post_send_window_bad_count: u64,
    pub observer_window_bad_count: u64,
    pub wait_window_bad_count: u64,
    pub sendinput_window_sample_count: u64,
    pub core_post_send_window_sample_count: u64,
    pub observer_window_sample_count: u64,
    pub wait_window_sample_count: u64,
    pub timeline_rebase_count: u64,
    pub timeline_rebase_total_us: u64,
    pub timeline_rebase_max_us: u64,
    pub timeline_rebase_total_ticks: DurationTicks,
    pub timeline_rebase_max_ticks: DurationTicks,
    /// Deprecated compatibility field. Authored dispatch never rebases its
    /// timeline, so production always leaves this at zero.
    pub timeline_rebase_last_reason: u8,
    pub dispatch_occupancy_max_us: u64,
    pub send_down_degraded_samples: u64,
    pub send_up_degraded_samples: u64,
    pub send_mixed_degraded_samples: u64,
    pub send_down_warn_threshold_us: u64,
    pub send_up_warn_threshold_us: u64,
    pub send_mixed_warn_threshold_us: u64,
    pub wait_target_error_us: u64,
    pub idle_wake_count: u64,
    /// Planned physical wait gaps classified against the existing cold-gap
    /// threshold. These are observer-side evidence only; wait policy is not
    /// changed by the classification.
    pub wait_planned_gap_hot_count: u64,
    pub wait_planned_gap_cold_count: u64,
    pub(crate) wait_planned_gap_max_ticks: u64,
    pub wait_planned_gap_max_us: u64,
    pub(crate) wait_planned_gap_hot_lateness_max_ticks: u64,
    pub wait_planned_gap_hot_lateness_max_us: u64,
    pub(crate) wait_planned_gap_cold_lateness_max_ticks: u64,
    pub wait_planned_gap_cold_lateness_max_us: u64,
    pub(crate) physical_target_to_wake_max_ticks: u64,
    pub physical_target_to_wake_max_us: u64,
    /// Time between `sender_completed` and `dispatch_ready` on the hard
    /// critical path (typed QPC derivation), in microseconds.
    pub core_post_send_max_us: u64,
    /// Decomposition of the physical handoff after the wait wake.
    pub wake_to_final_policy_max_us: u64,
    pub final_policy_to_pre_call_max_us: u64,
    pub sendinput_duration_max_us: u64,
    /// Peak QPC duration from a blocking deadline wake to the next
    /// `SendInput` call entry (us). Only dispatches that followed a blocking
    /// deadline wait contribute a sample.
    pub wake_to_send_max_us: u64,
    /// Peak wall-clock duration of a single deferred observer drain step.
    pub observer_duration_max_us: u64,
    /// How many `DispatchObservation` samples were dropped because the fixed
    /// diagnostic observer queue was full (drop-new policy).
    pub observer_dropped_samples: u64,
    /// Largest queue occupancy observed immediately after a successful enqueue.
    pub observer_queue_high_watermark: u64,
    pub(crate) recent_latencies: RecentLatencyRing,
}

impl WorkerMetricsLocal {
    /// Record fixed-width evidence for one physical sender attempt. Missing or
    /// unrepresentable intervals are left unavailable by omitting their sample
    /// count; no timestamp is guessed or replaced with a schedule value.
    pub(crate) fn observe_precision_packet_timing(
        &mut self,
        observation: PrecisionPacketTimingObservation,
    ) {
        let PrecisionPacketTimingObservation {
            authored_target_qpc,
            packet_not_before_qpc,
            pre_call_qpc,
            sendinput_completion_qpc,
            previous_sendinput_completion_qpc,
            previous_sender_completion_known,
            transport_anomaly,
        } = observation;
        self.precision_packet_attempts = self.precision_packet_attempts.saturating_add(1);
        self.pre_call_qpc_unavailable_attempts = self
            .pre_call_qpc_unavailable_attempts
            .saturating_add(u64::from(pre_call_qpc.is_none()));
        self.sendinput_completion_qpc_unavailable_attempts = self
            .sendinput_completion_qpc_unavailable_attempts
            .saturating_add(u64::from(sendinput_completion_qpc.is_none()));
        self.transport_anomalies = self
            .transport_anomalies
            .saturating_add(u64::from(transport_anomaly));

        match packet_not_before_qpc.checked_sub(authored_target_qpc) {
            Some(displacement) => {
                self.physical_floor_displacement_samples =
                    self.physical_floor_displacement_samples.saturating_add(1);
                self.max_physical_floor_displacement_ticks =
                    self.max_physical_floor_displacement_ticks.max(displacement);
            }
            None => {
                self.timing_arithmetic_anomalies =
                    self.timing_arithmetic_anomalies.saturating_add(1);
            }
        }

        if let Some(pre_call) = pre_call_qpc {
            if let Some(lateness) = signed_qpc_delta(pre_call, authored_target_qpc) {
                self.authored_lateness_samples = self.authored_lateness_samples.saturating_add(1);
                self.max_authored_lateness_ticks = if self.authored_lateness_samples == 1 {
                    lateness
                } else {
                    self.max_authored_lateness_ticks.max(lateness)
                };
                self.max_authored_lateness_abs_ticks = self
                    .max_authored_lateness_abs_ticks
                    .max(lateness.unsigned_abs());
                self.overdue_packet_observations = self
                    .overdue_packet_observations
                    .saturating_add(u64::from(lateness > 0));
            } else {
                self.timing_arithmetic_anomalies =
                    self.timing_arithmetic_anomalies.saturating_add(1);
            }

            if let Some(residual) = signed_qpc_delta(pre_call, packet_not_before_qpc) {
                self.residual_lateness_samples = self.residual_lateness_samples.saturating_add(1);
                self.max_residual_lateness_ticks = if self.residual_lateness_samples == 1 {
                    residual
                } else {
                    self.max_residual_lateness_ticks.max(residual)
                };
                self.max_residual_lateness_abs_ticks = self
                    .max_residual_lateness_abs_ticks
                    .max(residual.unsigned_abs());

                if previous_sender_completion_known {
                    let hol = previous_sendinput_completion_qpc
                        .map(|previous| {
                            previous
                                .saturating_sub(packet_not_before_qpc)
                                .min(pre_call.saturating_sub(packet_not_before_qpc))
                        })
                        .unwrap_or(0);
                    self.hol_delay_lower_bound_samples =
                        self.hol_delay_lower_bound_samples.saturating_add(1);
                    self.max_hol_delay_lower_bound_ticks =
                        self.max_hol_delay_lower_bound_ticks.max(hol);
                    let unattributed = residual.max(0).unsigned_abs().saturating_sub(hol);
                    self.unattributed_lateness_samples =
                        self.unattributed_lateness_samples.saturating_add(1);
                    self.max_unattributed_lateness_ticks =
                        self.max_unattributed_lateness_ticks.max(unattributed);
                }
            } else {
                self.timing_arithmetic_anomalies =
                    self.timing_arithmetic_anomalies.saturating_add(1);
            }
        }

        if let (Some(pre_call), Some(completed)) = (pre_call_qpc, sendinput_completion_qpc) {
            match completed.checked_sub(pre_call) {
                Some(envelope) => {
                    self.sendinput_envelope_samples =
                        self.sendinput_envelope_samples.saturating_add(1);
                    self.max_sendinput_envelope_ticks =
                        self.max_sendinput_envelope_ticks.max(envelope);
                }
                None => {
                    self.timing_arithmetic_anomalies =
                        self.timing_arithmetic_anomalies.saturating_add(1);
                }
            }
        }
    }

    /// Merge fields owned by the dedicated observer consumer into the
    /// dispatch snapshot projection. Each source snapshot starts from its own
    /// zero state, so producer-owned counters remain separate until readout.
    pub(crate) fn merge_observer(&mut self, observer: &Self) {
        self.total_us = self.total_us.saturating_add(observer.total_us);
        self.lateness_us = observer.lateness_us;
        self.max_lateness_us = self.max_lateness_us.max(observer.max_lateness_us);
        self.late_2ms = self.late_2ms.saturating_add(observer.late_2ms);
        self.late_5ms = self.late_5ms.saturating_add(observer.late_5ms);
        self.late_10ms = self.late_10ms.saturating_add(observer.late_10ms);
        self.release_max_us = self.release_max_us.max(observer.release_max_us);
        self.release_late_2ms = self
            .release_late_2ms
            .saturating_add(observer.release_late_2ms);
        self.recovered_zero_progress_retries = self
            .recovered_zero_progress_retries
            .saturating_add(observer.recovered_zero_progress_retries);
        self.recovered_partial_up_retries = self
            .recovered_partial_up_retries
            .saturating_add(observer.recovered_partial_up_retries);
        self.recovered_zero_progress_but_late = self
            .recovered_zero_progress_but_late
            .saturating_add(observer.recovered_zero_progress_but_late);
        self.effective_spin_threshold_us = self
            .effective_spin_threshold_us
            .max(observer.effective_spin_threshold_us);
        self.post_send_metrics_available |= observer.post_send_metrics_available;
        self.sendinput_warn_threshold_us = self
            .sendinput_warn_threshold_us
            .max(observer.sendinput_warn_threshold_us);
        self.core_post_send_warn_threshold_us = self
            .core_post_send_warn_threshold_us
            .max(observer.core_post_send_warn_threshold_us);
        self.observer_warn_threshold_us = self
            .observer_warn_threshold_us
            .max(observer.observer_warn_threshold_us);
        self.wait_warn_threshold_us = self
            .wait_warn_threshold_us
            .max(observer.wait_warn_threshold_us);
        self.input_path_degraded |= observer.input_path_degraded;
        self.sendinput_path_degraded |= observer.sendinput_path_degraded;
        self.core_post_send_degraded |= observer.core_post_send_degraded;
        self.observer_degraded |= observer.observer_degraded;
        self.wait_path_degraded |= observer.wait_path_degraded;
        self.sendinput_degraded_samples = self
            .sendinput_degraded_samples
            .saturating_add(observer.sendinput_degraded_samples);
        self.core_post_send_degraded_samples = self
            .core_post_send_degraded_samples
            .saturating_add(observer.core_post_send_degraded_samples);
        self.observer_degraded_samples = self
            .observer_degraded_samples
            .saturating_add(observer.observer_degraded_samples);
        self.wait_degraded_samples = self
            .wait_degraded_samples
            .saturating_add(observer.wait_degraded_samples);
        self.sendinput_window_bad_count = observer.sendinput_window_bad_count;
        self.core_post_send_window_bad_count = observer.core_post_send_window_bad_count;
        self.observer_window_bad_count = observer.observer_window_bad_count;
        self.wait_window_bad_count = observer.wait_window_bad_count;
        self.sendinput_window_sample_count = observer.sendinput_window_sample_count;
        self.core_post_send_window_sample_count = observer.core_post_send_window_sample_count;
        self.observer_window_sample_count = observer.observer_window_sample_count;
        self.wait_window_sample_count = observer.wait_window_sample_count;
        self.send_down_degraded_samples = self
            .send_down_degraded_samples
            .saturating_add(observer.send_down_degraded_samples);
        self.send_up_degraded_samples = self
            .send_up_degraded_samples
            .saturating_add(observer.send_up_degraded_samples);
        self.send_mixed_degraded_samples = self
            .send_mixed_degraded_samples
            .saturating_add(observer.send_mixed_degraded_samples);
        self.wait_planned_gap_hot_count = self
            .wait_planned_gap_hot_count
            .saturating_add(observer.wait_planned_gap_hot_count);
        self.wait_planned_gap_cold_count = self
            .wait_planned_gap_cold_count
            .saturating_add(observer.wait_planned_gap_cold_count);
        self.wait_planned_gap_max_ticks = self
            .wait_planned_gap_max_ticks
            .max(observer.wait_planned_gap_max_ticks);
        self.wait_planned_gap_hot_lateness_max_ticks = self
            .wait_planned_gap_hot_lateness_max_ticks
            .max(observer.wait_planned_gap_hot_lateness_max_ticks);
        self.wait_planned_gap_cold_lateness_max_ticks = self
            .wait_planned_gap_cold_lateness_max_ticks
            .max(observer.wait_planned_gap_cold_lateness_max_ticks);
        self.physical_target_to_wake_max_ticks = self
            .physical_target_to_wake_max_ticks
            .max(observer.physical_target_to_wake_max_ticks);
        self.wait_planned_gap_hot_lateness_max_us = self
            .wait_planned_gap_hot_lateness_max_us
            .max(observer.wait_planned_gap_hot_lateness_max_us);
        self.wait_planned_gap_cold_lateness_max_us = self
            .wait_planned_gap_cold_lateness_max_us
            .max(observer.wait_planned_gap_cold_lateness_max_us);
        self.physical_target_to_wake_max_us = self
            .physical_target_to_wake_max_us
            .max(observer.physical_target_to_wake_max_us);
        self.dispatch_occupancy_max_us = self
            .dispatch_occupancy_max_us
            .max(observer.dispatch_occupancy_max_us);
        self.core_post_send_max_us = self
            .core_post_send_max_us
            .max(observer.core_post_send_max_us);
        self.wake_to_final_policy_max_us = self
            .wake_to_final_policy_max_us
            .max(observer.wake_to_final_policy_max_us);
        self.final_policy_to_pre_call_max_us = self
            .final_policy_to_pre_call_max_us
            .max(observer.final_policy_to_pre_call_max_us);
        self.sendinput_duration_max_us = self
            .sendinput_duration_max_us
            .max(observer.sendinput_duration_max_us);
        self.wake_to_send_max_us = self.wake_to_send_max_us.max(observer.wake_to_send_max_us);
        self.observer_duration_max_us = self
            .observer_duration_max_us
            .max(observer.observer_duration_max_us);
        self.timeline_rebase_count = self
            .timeline_rebase_count
            .max(observer.timeline_rebase_count);
        self.timeline_rebase_total_us = self
            .timeline_rebase_total_us
            .max(observer.timeline_rebase_total_us);
        self.timeline_rebase_max_us = self
            .timeline_rebase_max_us
            .max(observer.timeline_rebase_max_us);
        self.timeline_rebase_total_ticks = self
            .timeline_rebase_total_ticks
            .max(observer.timeline_rebase_total_ticks);
        self.timeline_rebase_max_ticks = self
            .timeline_rebase_max_ticks
            .max(observer.timeline_rebase_max_ticks);
        if observer.timeline_rebase_last_reason != 0 {
            self.timeline_rebase_last_reason = observer.timeline_rebase_last_reason;
        }
        self.observer_dropped_samples = self
            .observer_dropped_samples
            .saturating_add(observer.observer_dropped_samples);
        self.observer_queue_high_watermark = self
            .observer_queue_high_watermark
            .max(observer.observer_queue_high_watermark);
        self.recent_latencies
            .extend_from(&observer.recent_latencies);
    }
}

#[derive(Default)]
pub(crate) struct SnapshotBuffer {
    dispatch: RwLock<WorkerMetricsLocal>,
    observer: RwLock<WorkerMetricsLocal>,
    terminal_published: AtomicBool,
}

impl SnapshotBuffer {
    pub(crate) fn load(&self) -> WorkerMetricsLocal {
        let dispatch = self.dispatch.read();
        let observer = self.observer.read();
        if self.terminal_published.load(Ordering::Acquire) {
            return dispatch.clone();
        }
        let mut value = dispatch.clone();
        value.merge_observer(&observer);
        value
    }

    /// Dispatch publication never waits for a snapshot reader. If readout
    /// currently owns the slot, this point-in-time publication is dropped;
    /// the worker's next publication carries its cumulative metrics.
    pub(crate) fn try_publish_dispatch(&self, local: WorkerMetricsLocal) -> bool {
        let Some(mut slot) = self.dispatch.try_write() else {
            return false;
        };
        *slot = local;
        true
    }

    /// Observer metrics have a separate writer-owned slot. They cannot race
    /// with or replace dispatch-owned counters.
    pub(crate) fn try_publish_observer(&self, local: WorkerMetricsLocal) -> bool {
        let Some(mut slot) = self.observer.try_write() else {
            return false;
        };
        *slot = local;
        true
    }

    /// Terminal publication replaces the live dispatch projection and clears
    /// the observer slot in one lock acquisition order. Readers hold both
    /// read guards together, so they see either the prior live projection or
    /// the merged terminal projection without double-counting observer data.
    fn try_publish_terminal(&self, local: &mut WorkerMetricsLocal) -> bool {
        let Some(mut dispatch) = self.dispatch.try_write() else {
            return false;
        };
        let Some(mut observer) = self.observer.try_write() else {
            return false;
        };
        *dispatch = std::mem::take(local);
        *observer = WorkerMetricsLocal::default();
        self.terminal_published.store(true, Ordering::Release);
        true
    }
}

#[derive(Default)]
pub(crate) struct SharedMetrics {
    pub(crate) snapshot: SnapshotBuffer,
    pub(crate) last_publish_us: AtomicU64,
    pub(crate) last_observer_publish_us: AtomicU64,
    pub(crate) live_diagnostics_enabled: AtomicBool,
    pub(crate) is_paused: AtomicBool,
    pub(crate) panicked: AtomicBool,
    pub(crate) last_error: Mutex<Option<String>>,
    /// Startup-only provenance. These strings are written before dispatch
    /// begins and read from snapshots, never from the precision path.
    pub(crate) requested_priority_mode: Mutex<String>,
    pub(crate) requested_wait_policy: Mutex<String>,
    pub(crate) effective_wait_policy: Mutex<String>,
    pub(crate) wait_strategy_acquired: Mutex<String>,
    pub(crate) terminal_error: Mutex<Option<String>>,
    pub(crate) terminal_trigger_reason: Mutex<Option<String>>,
    pub(crate) secondary_errors: Mutex<Vec<String>>,
    pub(crate) generation_status_counts: Mutex<HashMap<String, u64>>,
    #[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
    pub(crate) generation_accounting: Mutex<GenerationAccounting>,
    pub(crate) abort_counts_by_reason: Mutex<HashMap<String, u64>>,
    pub(crate) terminal_release_outcome: Mutex<Option<ReleaseAllOutcome>>,
    pub(crate) terminal_release_obligation_mask: AtomicU32,
    pub(crate) terminal_release_attempts_total: AtomicU64,
    #[cfg(test)]
    pub(crate) publish_count: AtomicU64,
}

pub(crate) fn try_publish_metrics(
    local: &WorkerMetricsLocal,
    shared: &SharedMetrics,
    qpc_clock: QpcClock,
    now_us: u64,
    force: bool,
) -> bool {
    try_publish_metrics_from(
        local,
        shared,
        qpc_clock,
        now_us,
        force,
        SnapshotOwner::Dispatch,
    )
}

pub(crate) fn try_publish_observer_metrics(
    local: &WorkerMetricsLocal,
    shared: &SharedMetrics,
    qpc_clock: QpcClock,
    now_us: u64,
    force: bool,
) -> bool {
    try_publish_metrics_from(
        local,
        shared,
        qpc_clock,
        now_us,
        force,
        SnapshotOwner::Observer,
    )
}

#[derive(Clone, Copy)]
enum SnapshotOwner {
    Dispatch,
    Observer,
}

fn try_publish_metrics_from(
    local: &WorkerMetricsLocal,
    shared: &SharedMetrics,
    qpc_clock: QpcClock,
    now_us: u64,
    force: bool,
    owner: SnapshotOwner,
) -> bool {
    let last_publish_us = match owner {
        SnapshotOwner::Dispatch => &shared.last_publish_us,
        SnapshotOwner::Observer => &shared.last_observer_publish_us,
    };
    let last = last_publish_us.load(Ordering::Relaxed);
    if !force && now_us.saturating_sub(last) < 50_000 {
        return true;
    }
    let published = project_metrics_for_snapshot(local, qpc_clock);
    let published_ok = match owner {
        SnapshotOwner::Dispatch => shared.snapshot.try_publish_dispatch(published),
        SnapshotOwner::Observer => shared.snapshot.try_publish_observer(published),
    };
    if published_ok {
        last_publish_us.store(now_us, Ordering::Relaxed);
        #[cfg(test)]
        if matches!(owner, SnapshotOwner::Dispatch) {
            shared.publish_count.fetch_add(1, Ordering::Relaxed);
        }
        true
    } else {
        false
    }
}

fn project_metrics_for_snapshot(
    local: &WorkerMetricsLocal,
    qpc_clock: QpcClock,
) -> WorkerMetricsLocal {
    let mut published = local.clone();
    published.max_sendinput_pre_call_lateness_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(
            local.max_sendinput_pre_call_lateness_ticks,
        ))
        .unwrap_or_default();
    published.max_hold_floor_delay_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(local.max_hold_floor_delay_ticks))
        .unwrap_or_default();
    published.max_release_floor_delay_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(local.max_release_floor_delay_ticks))
        .unwrap_or_default();
    published.wait_planned_gap_hot_lateness_max_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(
            local.wait_planned_gap_hot_lateness_max_ticks,
        ))
        .unwrap_or_default();
    published.wait_planned_gap_max_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(local.wait_planned_gap_max_ticks))
        .unwrap_or_default();
    published.wait_planned_gap_cold_lateness_max_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(
            local.wait_planned_gap_cold_lateness_max_ticks,
        ))
        .unwrap_or_default();
    published.physical_target_to_wake_max_us = qpc_clock
        .duration_to_us(DurationTicks::from_raw(
            local.physical_target_to_wake_max_ticks,
        ))
        .unwrap_or_default();
    published
}

/// Publish the terminal metrics before the lifecycle outcome becomes visible.
///
/// A live snapshot reader may hold either source slot. Terminal cleanup is
/// outside the precision dispatch path, so it can wait for both nonblocking
/// write acquisitions and publish one merged terminal snapshot.
pub(crate) fn publish_terminal_metrics(
    local: &WorkerMetricsLocal,
    shared: &SharedMetrics,
    qpc_clock: QpcClock,
    now_us: u64,
) {
    let mut published = project_metrics_for_snapshot(local, qpc_clock);
    loop {
        if shared.snapshot.try_publish_terminal(&mut published) {
            break;
        }
        std::thread::yield_now();
    }
    shared.last_publish_us.store(now_us, Ordering::Relaxed);
    #[cfg(test)]
    shared.publish_count.fetch_add(1, Ordering::Relaxed);
}

pub(crate) fn cpu_metrics_sample_due(now_us: u64, last_sample_us: u64, interval_us: u64) -> bool {
    now_us.saturating_sub(last_sample_us) >= interval_us
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
