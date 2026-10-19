use super::*;

impl Default for DiagnosticsPublicationGate {
    fn default() -> Self {
        Self {
            state: Mutex::new(DiagnosticsPublicationState::default()),
        }
    }
}

impl DiagnosticsPublicationGate {
    pub(in crate::native_runtime) fn is_enabled(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.enabled)
            .unwrap_or(false)
    }

    pub(in crate::native_runtime) fn set_enabled(&self, enabled: bool) -> Result<bool, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "diagnostics publication gate poisoned".to_string())?;
        state.enabled = enabled;
        state.epoch = state.epoch.wrapping_add(1);
        // This is intentionally the Python contract: re-enable starts a
        // fresh sampling window while sequence IDs remain monotonic.
        state.last_emit_at = None;
        Ok(state.enabled)
    }

    pub(in crate::native_runtime) fn try_publish<F>(
        &self,
        sample_time: Instant,
        publish: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(u64) -> Result<(), String>,
    {
        self.publish(sample_time, false, publish)
    }

    pub(in crate::native_runtime) fn force_publish<F>(
        &self,
        sample_time: Instant,
        publish: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(u64) -> Result<(), String>,
    {
        self.publish(sample_time, true, publish)
    }

    pub(in crate::native_runtime) fn publish<F>(
        &self,
        sample_time: Instant,
        force: bool,
        publish: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(u64) -> Result<(), String>,
    {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "diagnostics publication gate poisoned".to_string())?;
        if !state.enabled {
            return Ok(false);
        }
        if !force
            && state.last_emit_at.is_some_and(|last| {
                sample_time.saturating_duration_since(last) < DIAGNOSTICS_INTERVAL
            })
        {
            return Ok(false);
        }
        state.last_emit_at = Some(sample_time);
        state.sequence = state.sequence.wrapping_add(1);
        let sequence = state.sequence;
        // Keep the gate held across the bounded event publication. This is
        // the linearization point that makes a successful disable authoritative
        // even when a supervisor publication was already in flight.
        publish(sequence)?;
        Ok(true)
    }
}

#[cfg(test)]
pub(in crate::native_runtime) fn publish_diagnostics_snapshot_for_active(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    gate: &DiagnosticsPublicationGate,
) -> Result<(), String> {
    publish_diagnostics_snapshot_inner_with_cancel(events, active, gate, false, None)
}

#[cfg(test)]
pub(in crate::native_runtime) fn publish_final_diagnostics_snapshot_for_active(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    gate: &DiagnosticsPublicationGate,
) -> Result<(), String> {
    publish_diagnostics_snapshot_inner(events, active, gate, true)
}

#[cfg(test)]
pub(in crate::native_runtime) fn publish_diagnostics_snapshot_inner(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    gate: &DiagnosticsPublicationGate,
    final_sample: bool,
) -> Result<(), String> {
    publish_diagnostics_snapshot_inner_with_cancel(events, active, gate, final_sample, None)
}

pub(in crate::native_runtime) fn publish_diagnostics_snapshot_inner_cancellable(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    gate: &DiagnosticsPublicationGate,
    final_sample: bool,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    publish_diagnostics_snapshot_inner_with_cancel(
        events,
        active,
        gate,
        final_sample,
        Some(cancelled),
    )
}

pub(in crate::native_runtime) fn publish_diagnostics_snapshot_inner_with_cancel(
    events: &Arc<Mutex<NativeEventHub>>,
    active: &NativeActivePlayback,
    gate: &DiagnosticsPublicationGate,
    final_sample: bool,
    cancelled: Option<&AtomicBool>,
) -> Result<(), String> {
    let sample = active
        .player
        .as_ref()
        .map(|player| NativeDiagnosticsSample::from_player(player, active.physical))
        .unwrap_or_else(|| NativeDiagnosticsSample::unavailable(active.physical));
    let player_attached = active.player.is_some();
    let session_id = active.session_id.clone();
    let power = active.power_lifecycle_snapshot();
    let system_power = active
        .player
        .as_ref()
        .map(|player| player.system_power_snapshot());
    let completion_samples_available =
        sample.recent_latency_samples_available && !sample.recent_latencies_us.is_empty();
    let publish = |sequence| {
        if cancelled.is_some_and(|cancelled| cancelled.load(Ordering::Acquire)) {
            return Ok(());
        }
        let payload = crate::ui_events::DiagnosticsSnapshotDto {
            seq: sequence,
            physical_session: active.physical,
            player_attached,
            sender_sample_count: sample.sender_sample_count,
            max_lateness_us: sample.max_lateness_us,
            p50_ms: completion_samples_available
                .then(|| percentile_ms(&sample.recent_latencies_us, 0.50)),
            p95_ms: completion_samples_available
                .then(|| percentile_ms(&sample.recent_latencies_us, 0.95)),
            sigma_onset_ms: completion_samples_available
                .then(|| population_sigma_ms(&sample.recent_latencies_us)),
            late_2ms: sample.late_2ms,
            late_5ms: sample.late_5ms,
            late_10ms: sample.late_10ms,
            max_sendinput_pre_call_lateness_us: sample.max_sendinput_pre_call_lateness_us,
            pre_call_late_2ms: sample.pre_call_late_2ms,
            pre_call_late_5ms: sample.pre_call_late_5ms,
            pre_call_late_10ms: sample.pre_call_late_10ms,
            fps: active.timing_policy.fps,
            frame_us: active.timing_policy.frame_us,
            hold_frames: active.timing_policy.hold_frames,
            frame_base_hold_us: active.timing_policy.frame_base_hold_us,
            timing_margin_us: active.timing_policy.timing_margin_us,
            min_hold_us: active.timing_policy.min_hold_us,
            min_release_gap_us: active.timing_policy.min_release_gap_us,
            timing_margin_recommendation: active.timing_margin_recommendation.clone(),
            pre_call_lt_250us: sample.pre_call_lt_250us,
            pre_call_250_500us: sample.pre_call_250_500us,
            pre_call_500_750us: sample.pre_call_500_750us,
            pre_call_750_1000us: sample.pre_call_750_1000us,
            pre_call_1000_1500us: sample.pre_call_1000_1500us,
            pre_call_1500_2000us: sample.pre_call_1500_2000us,
            pre_call_ge_2000us: sample.pre_call_ge_2000us,
            active_keys: sample.active_keys,
            stuck_keys: sample.stuck_keys,
            keys_dropped: sample.keys_dropped,
            chord_split_events: sample.chord_split_events,
            missed_down_boundaries: sample.missed_down_boundaries,
            missed_down_keys: sample.missed_down_keys,
            missed_unobserved_backlog_boundaries: sample.missed_unobserved_backlog_boundaries,
            missed_physical_window_boundaries: 0,
            final_sender_window_expirations: 0,
            release_floor_infeasible_boundaries: 0,
            hold_floor_delay_boundaries: sample.hold_floor_delay_boundaries,
            max_hold_floor_delay_us: sample.max_hold_floor_delay_us,
            release_floor_delay_boundaries: sample.release_floor_delay_boundaries,
            max_release_floor_delay_us: sample.max_release_floor_delay_us,
            final_gate_control_rejections: sample.final_gate_control_rejections,
            final_gate_target_changes: sample.final_gate_target_changes,
            final_gate_focus_losses: sample.final_gate_focus_losses,
            final_gate_lease_expirations: sample.final_gate_lease_expirations,
            sendinput_partial_events: sample.sendinput_partial_events,
            sendinput_zero_progress_failures: sample.sendinput_zero_progress_failures,
            backend_status: sample.backend_status,
            release_max_us: sample.release_max_us,
            release_late_2ms: sample.release_late_2ms,
            session_id: Some(session_id),
            last_error: sample.last_error.clone(),
            power_request_created: power.request_created,
            power_request_active: power.request_active,
            power_request_create_failures: power.request_create_failures,
            power_request_set_failures: power.request_set_failures,
            power_request_clear_failures: power.request_clear_failures,
            power_request_close_failures: power.request_close_failures,
            suspend_resume_registered: power.registration_active,
            suspend_resume_registration_failures: power.registration_failures,
            suspend_resume_unregistration_failures: power.unregistration_failures,
            system_suspend_active: system_power.is_some_and(|snapshot| snapshot.down_blocked),
            system_suspend_notifications: system_power
                .map_or(0, |snapshot| snapshot.suspend_notifications),
            system_resume_notifications: system_power
                .map_or(0, |snapshot| snapshot.resume_notifications),
            duplicate_system_power_notifications: system_power
                .map_or(0, |snapshot| snapshot.duplicate_notifications),
        };
        events
            .lock()
            .map_err(|_| "native event hub lock poisoned".to_string())?
            .publish(UiEvent::DiagnosticsSnapshot {
                v: crate::DESKTOP_PROTOCOL_VERSION,
                payload: Box::new(payload),
            })
    };
    let _published = if final_sample {
        gate.force_publish(Instant::now(), publish)
    } else {
        gate.try_publish(Instant::now(), publish)
    }?;
    Ok(())
}

pub(in crate::native_runtime) struct NativeDiagnosticsSample {
    pub(in crate::native_runtime) max_lateness_us: Option<u64>,
    pub(in crate::native_runtime) recent_latencies_us: Vec<i64>,
    pub(in crate::native_runtime) recent_latency_samples_available: bool,
    pub(in crate::native_runtime) late_2ms: Option<u64>,
    pub(in crate::native_runtime) late_5ms: Option<u64>,
    pub(in crate::native_runtime) late_10ms: Option<u64>,
    pub(in crate::native_runtime) sender_sample_count: u64,
    pub(in crate::native_runtime) max_sendinput_pre_call_lateness_us: Option<u64>,
    pub(in crate::native_runtime) pre_call_late_2ms: u64,
    pub(in crate::native_runtime) pre_call_late_5ms: u64,
    pub(in crate::native_runtime) pre_call_late_10ms: u64,
    pub(in crate::native_runtime) pre_call_lt_250us: u64,
    pub(in crate::native_runtime) pre_call_250_500us: u64,
    pub(in crate::native_runtime) pre_call_500_750us: u64,
    pub(in crate::native_runtime) pre_call_750_1000us: u64,
    pub(in crate::native_runtime) pre_call_1000_1500us: u64,
    pub(in crate::native_runtime) pre_call_1500_2000us: u64,
    pub(in crate::native_runtime) pre_call_ge_2000us: u64,
    pub(in crate::native_runtime) active_keys: u64,
    pub(in crate::native_runtime) stuck_keys: u64,
    pub(in crate::native_runtime) keys_dropped: u64,
    pub(in crate::native_runtime) chord_split_events: u64,
    pub(in crate::native_runtime) missed_down_boundaries: u64,
    pub(in crate::native_runtime) missed_down_keys: u64,
    pub(in crate::native_runtime) missed_unobserved_backlog_boundaries: u64,
    pub(in crate::native_runtime) hold_floor_delay_boundaries: u64,
    pub(in crate::native_runtime) max_hold_floor_delay_us: u64,
    pub(in crate::native_runtime) release_floor_delay_boundaries: u64,
    pub(in crate::native_runtime) max_release_floor_delay_us: u64,
    pub(in crate::native_runtime) final_gate_control_rejections: u64,
    pub(in crate::native_runtime) final_gate_target_changes: u64,
    pub(in crate::native_runtime) final_gate_focus_losses: u64,
    pub(in crate::native_runtime) final_gate_lease_expirations: u64,
    pub(in crate::native_runtime) sendinput_partial_events: u64,
    pub(in crate::native_runtime) sendinput_zero_progress_failures: u64,
    pub(in crate::native_runtime) backend_status: DiagnosticsBackendStatus,
    pub(in crate::native_runtime) release_max_us: Option<u64>,
    pub(in crate::native_runtime) release_late_2ms: Option<u64>,
    pub(in crate::native_runtime) last_error: Option<String>,
}

impl NativeDiagnosticsSample {
    pub(in crate::native_runtime) fn unavailable(physical_session: bool) -> Self {
        let player_attached = false;
        Self {
            max_lateness_us: None,
            recent_latencies_us: Vec::new(),
            recent_latency_samples_available: false,
            late_2ms: None,
            late_5ms: None,
            late_10ms: None,
            sender_sample_count: 0,
            max_sendinput_pre_call_lateness_us: None,
            pre_call_late_2ms: 0,
            pre_call_late_5ms: 0,
            pre_call_late_10ms: 0,
            pre_call_lt_250us: 0,
            pre_call_250_500us: 0,
            pre_call_500_750us: 0,
            pre_call_750_1000us: 0,
            pre_call_1000_1500us: 0,
            pre_call_1500_2000us: 0,
            pre_call_ge_2000us: 0,
            active_keys: 0,
            stuck_keys: 0,
            keys_dropped: 0,
            chord_split_events: 0,
            missed_down_boundaries: 0,
            missed_down_keys: 0,
            missed_unobserved_backlog_boundaries: 0,
            hold_floor_delay_boundaries: 0,
            max_hold_floor_delay_us: 0,
            release_floor_delay_boundaries: 0,
            max_release_floor_delay_us: 0,
            final_gate_control_rejections: 0,
            final_gate_target_changes: 0,
            final_gate_focus_losses: 0,
            final_gate_lease_expirations: 0,
            sendinput_partial_events: 0,
            sendinput_zero_progress_failures: 0,
            backend_status: diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session,
                player_attached,
                ..DiagnosticsBackendHealth::default()
            }),
            release_max_us: None,
            release_late_2ms: None,
            last_error: (physical_session != player_attached)
                .then(|| "physical session has no attached native player".into()),
        }
    }

    pub(in crate::native_runtime) fn from_player(
        player: &NativeDispatchSession,
        physical_session: bool,
    ) -> Self {
        let snapshot = player.snapshot_lite();
        let observer_metrics_available = snapshot.recent_latency_samples_available;
        let has_last_error = snapshot.last_error.is_some();
        let (sender_sample_count, max_sendinput_pre_call_lateness_us) = sender_sample_summary(
            [
                snapshot.pre_call_lt_250us,
                snapshot.pre_call_250_500us,
                snapshot.pre_call_500_750us,
                snapshot.pre_call_750_1000us,
                snapshot.pre_call_1000_1500us,
                snapshot.pre_call_1500_2000us,
                snapshot.pre_call_ge_2000us,
            ],
            snapshot.max_sendinput_pre_call_lateness_us,
        );
        Self {
            max_lateness_us: observer_metrics_available.then_some(snapshot.max_lateness_us),
            recent_latencies_us: snapshot.recent_latencies_us,
            recent_latency_samples_available: snapshot.recent_latency_samples_available,
            late_2ms: observer_metrics_available.then_some(snapshot.late_2ms),
            late_5ms: observer_metrics_available.then_some(snapshot.late_5ms),
            late_10ms: observer_metrics_available.then_some(snapshot.late_10ms),
            sender_sample_count,
            max_sendinput_pre_call_lateness_us,
            pre_call_late_2ms: snapshot.pre_call_late_2ms,
            pre_call_late_5ms: snapshot.pre_call_late_5ms,
            pre_call_late_10ms: snapshot.pre_call_late_10ms,
            pre_call_lt_250us: snapshot.pre_call_lt_250us,
            pre_call_250_500us: snapshot.pre_call_250_500us,
            pre_call_500_750us: snapshot.pre_call_500_750us,
            pre_call_750_1000us: snapshot.pre_call_750_1000us,
            pre_call_1000_1500us: snapshot.pre_call_1000_1500us,
            pre_call_1500_2000us: snapshot.pre_call_1500_2000us,
            pre_call_ge_2000us: snapshot.pre_call_ge_2000us,
            active_keys: snapshot.active_count as u64,
            stuck_keys: snapshot.failed_release_count as u64,
            keys_dropped: snapshot.keys_dropped,
            chord_split_events: snapshot.chord_split_events,
            missed_down_boundaries: snapshot.missed_down_boundaries,
            missed_down_keys: snapshot.missed_down_keys,
            missed_unobserved_backlog_boundaries: snapshot.missed_unobserved_backlog_boundaries,
            hold_floor_delay_boundaries: snapshot.hold_floor_delay_boundaries,
            max_hold_floor_delay_us: snapshot.max_hold_floor_delay_us,
            release_floor_delay_boundaries: snapshot.release_floor_delay_boundaries,
            max_release_floor_delay_us: snapshot.max_release_floor_delay_us,
            final_gate_control_rejections: snapshot.final_gate_control_rejections,
            final_gate_target_changes: snapshot.final_gate_target_changes,
            final_gate_focus_losses: snapshot.final_gate_focus_losses,
            final_gate_lease_expirations: snapshot.final_gate_lease_expirations,
            sendinput_partial_events: snapshot.sendinput_partial_events,
            sendinput_zero_progress_failures: snapshot.sendinput_zero_progress_failures,
            last_error: snapshot.last_error,
            backend_status: diagnostics_backend_status(DiagnosticsBackendHealth {
                physical_session,
                player_attached: true,
                has_terminal_error: snapshot.has_terminal_error,
                has_terminal_failure_outcome: snapshot
                    .terminal_outcome
                    .is_some_and(EnginePollStatus::is_terminal_failure),
                has_last_error,
                failed_release_count: snapshot.failed_release_count,
                keys_dropped: snapshot.keys_dropped,
                chord_split_events: snapshot.chord_split_events,
                possibly_active_count: snapshot.possibly_active_count,
                active_count: snapshot.active_count,
            }),
            release_max_us: observer_metrics_available.then_some(snapshot.release_max_us),
            release_late_2ms: observer_metrics_available.then_some(snapshot.release_late_2ms),
        }
    }
}

pub(in crate::native_runtime) fn sender_sample_summary(
    buckets: [u64; 7],
    max_lateness_us: u64,
) -> (u64, Option<u64>) {
    let sample_count = buckets.into_iter().fold(0_u64, u64::saturating_add);
    (sample_count, (sample_count > 0).then_some(max_lateness_us))
}

#[derive(Clone, Copy, Debug, Default)]
pub(in crate::native_runtime) struct DiagnosticsBackendHealth {
    pub(in crate::native_runtime) physical_session: bool,
    pub(in crate::native_runtime) player_attached: bool,
    pub(in crate::native_runtime) has_terminal_error: bool,
    pub(in crate::native_runtime) has_terminal_failure_outcome: bool,
    pub(in crate::native_runtime) has_last_error: bool,
    pub(in crate::native_runtime) failed_release_count: usize,
    pub(in crate::native_runtime) keys_dropped: u64,
    pub(in crate::native_runtime) chord_split_events: u64,
    pub(in crate::native_runtime) possibly_active_count: usize,
    pub(in crate::native_runtime) active_count: usize,
}

pub(in crate::native_runtime) fn diagnostics_backend_status(
    health: DiagnosticsBackendHealth,
) -> crate::ui_events::DiagnosticsBackendStatus {
    if health.physical_session != health.player_attached {
        crate::ui_events::DiagnosticsBackendStatus::Error
    } else if !health.player_attached {
        crate::ui_events::DiagnosticsBackendStatus::Unavailable
    } else if health.has_terminal_error
        || health.has_terminal_failure_outcome
        || health.has_last_error
        || health.failed_release_count > 0
    {
        crate::ui_events::DiagnosticsBackendStatus::Error
    } else if health.keys_dropped > 0
        || health.chord_split_events > 0
        || health.possibly_active_count > health.active_count
    {
        crate::ui_events::DiagnosticsBackendStatus::Degraded
    } else {
        crate::ui_events::DiagnosticsBackendStatus::Healthy
    }
}
