impl NativeDispatchSession {
    pub fn snapshot(&self) -> EngineSnapshot {
        let lifecycle = self.shared.lifecycle.lifecycle.load(Ordering::Acquire);
        let (((elapsed_us, paused), pre_roll_remaining_us), startup) = self.live_projection();
        let status = self
            .poll_status(lifecycle, paused, pre_roll_remaining_us, startup)
            .as_str();
        let local = self.shared.publication.metrics.snapshot.load();
        let startup_armed = startup.armed_published;
        let startup_ready = startup.ready_published;
        EngineSnapshot {
            elapsed_us,
            total_us: local.total_us,
            pre_roll_remaining_us,
            lateness_us: local.lateness_us,
            max_lateness_us: local.max_lateness_us,
            late_2ms: local.late_2ms,
            late_5ms: local.late_5ms,
            late_10ms: local.late_10ms,
            max_sendinput_pre_call_lateness_us: local.max_sendinput_pre_call_lateness_us,
            precision_packet_attempts: local.precision_packet_attempts,
            pre_call_qpc_unavailable_attempts: local.pre_call_qpc_unavailable_attempts,
            sendinput_completion_qpc_unavailable_attempts: local
                .sendinput_completion_qpc_unavailable_attempts,
            authored_lateness_samples: local.authored_lateness_samples,
            max_authored_lateness_ticks: local.max_authored_lateness_ticks,
            max_authored_lateness_abs_ticks: local.max_authored_lateness_abs_ticks,
            physical_floor_displacement_samples: local.physical_floor_displacement_samples,
            max_physical_floor_displacement_ticks: local.max_physical_floor_displacement_ticks,
            residual_lateness_samples: local.residual_lateness_samples,
            max_residual_lateness_ticks: local.max_residual_lateness_ticks,
            max_residual_lateness_abs_ticks: local.max_residual_lateness_abs_ticks,
            hol_delay_lower_bound_samples: local.hol_delay_lower_bound_samples,
            max_hol_delay_lower_bound_ticks: local.max_hol_delay_lower_bound_ticks,
            unattributed_lateness_samples: local.unattributed_lateness_samples,
            max_unattributed_lateness_ticks: local.max_unattributed_lateness_ticks,
            sendinput_envelope_samples: local.sendinput_envelope_samples,
            max_sendinput_envelope_ticks: local.max_sendinput_envelope_ticks,
            overdue_packet_observations: local.overdue_packet_observations,
            transport_anomalies: local.transport_anomalies,
            timing_arithmetic_anomalies: local.timing_arithmetic_anomalies,
            pre_call_lt_250us: local.pre_call_lt_250us,
            pre_call_250_500us: local.pre_call_250_500us,
            pre_call_500_750us: local.pre_call_500_750us,
            pre_call_750_1000us: local.pre_call_750_1000us,
            pre_call_1000_1500us: local.pre_call_1000_1500us,
            pre_call_1500_2000us: local.pre_call_1500_2000us,
            pre_call_ge_2000us: local.pre_call_ge_2000us,
            pre_call_late_2ms: local.pre_call_late_2ms,
            pre_call_late_5ms: local.pre_call_late_5ms,
            pre_call_late_10ms: local.pre_call_late_10ms,
            release_max_us: local.release_max_us,
            release_late_2ms: local.release_late_2ms,
            recent_latencies_us: local.recent_latencies.to_vec(),
            recent_latency_samples_available: self.observer_enabled,
            is_running: lifecycle == LIFECYCLE_RUNNING,
            is_finished: matches!(lifecycle, LIFECYCLE_FINISHED | LIFECYCLE_POISONED),
            is_paused: paused,
            status: status.to_string(),
            active_count: local.active_count as usize,
            possibly_active_count: local.possibly_active_count as usize,
            failed_release_count: local.failed_release_count as usize,
            last_error: self.shared.publication.metrics.last_error.lock().clone(),
            keys_dropped: local.keys_dropped,
            chord_split_events: local.chord_split_events,
            sendinput_partial_events: local.sendinput_partial_events,
            sendinput_zero_progress_failures: local.sendinput_zero_progress_failures,
            chords_rejected: local.chords_rejected,
            authored_conflict_events: local.authored_conflict_events,
            authored_chords_rejected: local.authored_chords_rejected,
            authored_keys_rejected: local.authored_keys_rejected,
            // Compatibility-only DTO fields; the live DispatchExecutor has no missed-Down path.
            missed_down_boundaries: 0,
            missed_down_keys: 0,
            missed_unobserved_backlog_boundaries: 0,
            missed_physical_window_boundaries: 0,
            final_gate_control_rejections: local.final_gate_control_rejections,
            final_gate_target_changes: local.final_gate_target_changes,
            final_gate_focus_losses: local.final_gate_focus_losses,
            final_gate_lease_expirations: local.final_gate_lease_expirations,
            final_sender_window_expirations: 0,
            release_floor_infeasible_boundaries: 0,
            hold_floor_delay_boundaries: local.hold_floor_delay_boundaries,
            max_hold_floor_delay_us: local.max_hold_floor_delay_us,
            last_hold_floor_delay_mask: local.last_hold_floor_delay_mask,
            release_floor_delay_boundaries: local.release_floor_delay_boundaries,
            max_release_floor_delay_us: local.max_release_floor_delay_us,
            last_release_floor_delay_mask: local.last_release_floor_delay_mask,
            max_missed_lateness_ticks: 0,
            keys_inserted_before_failure: local.keys_inserted_before_failure,
            keys_rolled_back: local.keys_rolled_back,
            rollback_residue_keys: local.rollback_residue_keys,
            production_forensics_available: local.production_forensics_available,
            production_forensics_version: local.production_forensics_version,
            production_hold_pair_samples: local.production_hold_pair_samples,
            production_min_hold_start_after_down_completion_ticks: local
                .production_min_hold_start_after_down_completion_ticks,
            production_hold_floor_violation_count: local.production_hold_floor_violation_count,
            production_release_floor_samples: local.production_release_floor_samples,
            production_min_down_start_after_up_completion_ticks: local
                .production_min_down_start_after_up_completion_ticks,
            production_release_floor_violation_count: local
                .production_release_floor_violation_count,
            production_hold_floor_ticks: local.production_hold_floor_ticks,
            production_release_floor_ticks: local.production_release_floor_ticks,
            production_same_key_overlap_forensics_count: local
                .production_same_key_overlap_forensics_count,
            production_anchor_overwrite_count: local.production_anchor_overwrite_count,
            production_unmatched_up_count: local.production_unmatched_up_count,
            production_anomaly_ring_overwrite_count: local.production_anomaly_ring_overwrite_count,
            production_forensics_anomaly_count: local.production_forensics_anomaly_count,
            production_structural_anomaly_count: local.production_structural_anomaly_count,
            production_timing_diagnostic_count: local.production_timing_diagnostic_count,
            last_missed_down_reason: None,
            last_missed_down_source_action_index: None,
            last_missed_down_mask: 0,
            last_missed_down_lateness_ticks: 0,
            chord_integrity_lost: local.chord_integrity_lost,
            lead_saturation_count_down: local.lead_saturation_count_down.to_vec(),
            lead_saturation_count_up: local.lead_saturation_count_up.to_vec(),
            positive_residual_at_cap: local.positive_residual_at_cap,
            recovered_zero_progress_but_late: local.recovered_zero_progress_but_late,
            recovered_zero_progress_retries: local.recovered_zero_progress_retries,
            recovered_partial_up_retries: local.recovered_partial_up_retries,
            outcome: self.terminal_outcome().map(str::to_string),
            requested_rt_priority_mode: self
                .shared
                .publication
                .metrics
                .requested_priority_mode
                .lock()
                .clone(),
            rt_priority_acquired: self.shared.publication.priority_acquired.lock().clone(),
            startup_priority_mode_acquired: self
                .shared
                .publication
                .startup_priority_mode_acquired
                .lock()
                .clone(),
            startup_power_guard_acquired: self
                .shared
                .publication
                .startup_power_guard_acquired
                .load(Ordering::Acquire),
            requested_wait_policy: self
                .shared
                .publication
                .metrics
                .requested_wait_policy
                .lock()
                .clone(),
            effective_wait_policy: self
                .shared
                .publication
                .metrics
                .effective_wait_policy
                .lock()
                .clone(),
            startup_calibration_executed: local.startup_calibration_executed,
            startup_calibration_sample_count: local.startup_calibration_sample_count,
            startup_calibration_duration_us: local.startup_calibration_duration_us,
            startup_calibration_result: crate::engine::config::StartupCalibrationResult::from_raw(
                local.startup_calibration_result,
            )
            .label()
            .to_string(),
            startup_wake_error_p50_us: local.wake_error_p50_us,
            startup_wake_error_p95_us: local.wake_error_p95_us,
            startup_wake_error_p99_us: local.wake_error_p99_us,
            startup_wake_error_max_us: local.wake_error_max_us,
            startup_wake_error_robust_us: local.startup_wake_error_robust_us,
            effective_spin_threshold_us: local.effective_spin_threshold_us,
            spin_threshold_source: crate::engine::config::SpinThresholdSource::from_raw(
                local.spin_threshold_source,
            )
            .label()
            .to_string(),
            wake_error_p50_us: local.wake_error_p50_us,
            wake_error_p95_us: local.wake_error_p95_us,
            wake_error_p99_us: local.wake_error_p99_us,
            wake_error_max_us: local.wake_error_max_us,
            spin_time_us: (local.idle_wake_count > 0).then_some(local.spin_time_us),
            spin_time_available: local.idle_wake_count > 0,
            spin_time_complete: local.spin_observation_expected_samples > 0
                && local.spin_observation_pending_samples == 0
                && local.spin_observation_dropped_samples == 0
                && local.spin_observation_missing_samples == 0,
            spin_observation_expected_samples: local.spin_observation_expected_samples,
            spin_observation_dropped_samples: local.spin_observation_dropped_samples,
            spin_observation_missing_samples: local.spin_observation_missing_samples,
            spin_observation_pending_samples: local.spin_observation_pending_samples,
            playback_wall_time_us: local.playback_wall_time_us,
            spin_duty_cycle_ppm: (local.idle_wake_count > 0 && local.playback_wall_time_us > 0)
                .then_some(local.spin_duty_cycle_ppm),
            worker_cpu_time_us: local.worker_cpu_time_us,
            process_cpu_time_us: local.process_cpu_time_us,
            wait_strategy_acquired: self
                .shared
                .publication
                .metrics
                .wait_strategy_acquired
                .lock()
                .clone(),
            power_throttling_disabled: local.power_throttling_disabled,
            input_path_degraded: local.input_path_degraded,
            sendinput_path_degraded: local.sendinput_path_degraded,
            core_post_send_degraded: local.core_post_send_degraded,
            post_send_metrics_available: local.post_send_metrics_available,
            observer_degraded: local.observer_degraded,
            wait_path_degraded: local.wait_path_degraded,
            sendinput_warn_threshold_us: local.sendinput_warn_threshold_us,
            core_post_send_warn_threshold_us: local.core_post_send_warn_threshold_us,
            observer_warn_threshold_us: local.observer_warn_threshold_us,
            wait_warn_threshold_us: local.wait_warn_threshold_us,
            sendinput_degraded_samples: local.sendinput_degraded_samples,
            core_post_send_degraded_samples: local.core_post_send_degraded_samples,
            observer_degraded_samples: local.observer_degraded_samples,
            wait_degraded_samples: local.wait_degraded_samples,
            wait_backend_failures: local.wait_backend_failures,
            wait_clock_failures: local.wait_clock_failures,
            wait_interrupted_count: local.wait_interrupted_count,
            sendinput_window_bad_count: local.sendinput_window_bad_count,
            core_post_send_window_bad_count: local.core_post_send_window_bad_count,
            observer_window_bad_count: local.observer_window_bad_count,
            wait_window_bad_count: local.wait_window_bad_count,
            sendinput_window_sample_count: local.sendinput_window_sample_count,
            core_post_send_window_sample_count: local.core_post_send_window_sample_count,
            observer_window_sample_count: local.observer_window_sample_count,
            wait_window_sample_count: local.wait_window_sample_count,
            timeline_rebase_count: local.timeline_rebase_count,
            timeline_rebase_total_us: local.timeline_rebase_total_us,
            timeline_rebase_max_us: local.timeline_rebase_max_us,
            timeline_rebase_last_reason: timeline_rebase_reason(local.timeline_rebase_last_reason),
            dispatch_occupancy_max_us: local.dispatch_occupancy_max_us,
            send_down_degraded_samples: local.send_down_degraded_samples,
            send_up_degraded_samples: local.send_up_degraded_samples,
            send_mixed_degraded_samples: local.send_mixed_degraded_samples,
            send_down_warn_threshold_us: local.send_down_warn_threshold_us,
            send_up_warn_threshold_us: local.send_up_warn_threshold_us,
            send_mixed_warn_threshold_us: local.send_mixed_warn_threshold_us,
            wait_target_error_us: local.wait_target_error_us,
            idle_wake_count: local.idle_wake_count,
            wait_planned_gap_max_us: local.wait_planned_gap_max_us,
            wait_planned_gap_hot_count: local.wait_planned_gap_hot_count,
            wait_planned_gap_cold_count: local.wait_planned_gap_cold_count,
            wait_planned_gap_hot_lateness_max_us: local.wait_planned_gap_hot_lateness_max_us,
            wait_planned_gap_cold_lateness_max_us: local.wait_planned_gap_cold_lateness_max_us,
            physical_target_to_wake_max_us: local.physical_target_to_wake_max_us,
            core_post_send_max_us: local.core_post_send_max_us,
            wake_to_final_policy_max_us: local.wake_to_final_policy_max_us,
            final_policy_to_pre_call_max_us: local.final_policy_to_pre_call_max_us,
            sendinput_duration_max_us: local.sendinput_duration_max_us,
            wake_to_send_max_us: local.wake_to_send_max_us,
            observer_duration_max_us: local.observer_duration_max_us,
            observer_dropped_samples: local.observer_dropped_samples,
            observer_queue_high_watermark: local.observer_queue_high_watermark,
            terminal_error: self
                .shared
                .publication
                .metrics
                .terminal_error
                .lock()
                .clone(),
            terminal_trigger_reason: self
                .shared
                .publication
                .metrics
                .terminal_trigger_reason
                .lock()
                .clone(),
            secondary_errors: self
                .shared
                .publication
                .metrics
                .secondary_errors
                .lock()
                .clone(),
            generation_count: self.generation_count,
            generation_status_counts: self
                .shared
                .publication
                .metrics
                .generation_status_counts
                .lock()
                .clone(),
            #[cfg(any(test, feature = "test-support", feature = "real-input-acceptance"))]
            generation_accounting: self.generation_accounting_for_test(),
            final_release_obligation_mask: self
                .shared
                .publication
                .metrics
                .terminal_release_obligation_mask
                .load(Ordering::Acquire) as u16,
            abort_counts_by_reason: self
                .shared
                .publication
                .metrics
                .abort_counts_by_reason
                .lock()
                .clone(),
            release_outcome: self
                .shared
                .publication
                .metrics
                .terminal_release_outcome
                .lock()
                .clone(),
            release_attempts_total: self
                .shared
                .publication
                .metrics
                .terminal_release_attempts_total
                .load(Ordering::Acquire),
            startup_armed,
            startup_ready,
            startup_latency_us: startup_ready.then(|| {
                self.shared
                    .publication
                    .startup_latency_us
                    .load(Ordering::Relaxed)
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn startup_ticks(&self) -> (u64, u64) {
        (
            self.shared
                .publication
                .startup_requested_ticks
                .load(Ordering::Acquire),
            self.shared
                .publication
                .startup_ready_ticks
                .load(Ordering::Relaxed),
        )
    }

    #[cfg(test)]
    pub(super) fn shared_for_test(&self) -> &SessionShared {
        &self.shared
    }

}
