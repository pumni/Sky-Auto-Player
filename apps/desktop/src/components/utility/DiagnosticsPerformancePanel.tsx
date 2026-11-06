import { TabPanel } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { timingMarginRecommendationSourceLabel } from '../timingMarginSource';
import { backendStatusLabel, measure, playbackStatusLabel } from './diagnosticsPresentation';
import {
  DiagnosticsEmptyState,
  DiagnosticsMetric as Metric,
  DiagnosticsMetricGroup as MetricGroup,
} from './DiagnosticsReadouts';

export function DiagnosticsPerformancePanel({ useStore }: { useStore: DesktopStoreHook }) {
  const diagnostics = useStore((store: DesktopStore) => store.diagnostics);
  const sessionId = useStore((store: DesktopStore) => store.playback.sessionId);
  const playbackState = useStore((store: DesktopStore) => store.playback.state);
  const playbackStartPending = useStore(
    (store: DesktopStore) => store.playback.startRequestId !== null,
  );
  const transportOperation = useStore((store: DesktopStore) => store.playback.transportOperation);
  const preparedAdmission = useStore(
    (store: DesktopStore) => store.playback.prepared?.admission ?? null,
  );
  const playbackError = useStore((store: DesktopStore) => store.playback.error);
  const playbackStatus = useStore((store: DesktopStore) =>
    playbackStatusLabel({
      error: store.playback.error,
      sessionId: store.playback.sessionId,
      startRequestId: store.playback.startRequestId,
      state: store.playback.state,
      statusRetryPending: store.playback.statusRetryPending,
      transportOperation: store.playback.transportOperation,
      prepared: store.playback.prepared,
    }),
  );
  const playbackIsActive = useStore((store: DesktopStore) =>
    ['starting', 'playing', 'paused', 'stopping'].includes(store.playback.state),
  );
  const dryRun = useStore(
    (store: DesktopStore) =>
      store.playback.context?.dryRun ?? store.playback.prepared?.config.dry_run ?? false,
  );
  const latest = diagnostics.samples[diagnostics.samples.length - 1];
  const nativeSessionActive = diagnostics.enabled && playbackIsActive && sessionId !== null;
  const activeSession = nativeSessionActive && latest?.session_id === sessionId;
  const backendMetricsAvailable = latest !== undefined && latest.backend_status !== 'unavailable';
  const playerMetric = (value: number): string =>
    dryRun
      ? 'Not measured'
      : !backendMetricsAvailable || latest?.player_attached !== true
        ? 'Unavailable'
        : String(value);
  const senderSampleMetric = (value: number | null | undefined): string =>
    dryRun
      ? 'Not measured'
      : !backendMetricsAvailable
        ? 'Unavailable'
        : latest?.player_attached !== true
          ? 'Unavailable'
          : latest?.sender_sample_count === 0
            ? 'No samples'
            : value === null || value === undefined
              ? 'Unavailable'
              : String(value);
  const backendMeasure = (value: number | null, unit: string): string =>
    dryRun
      ? 'Not measured'
      : !backendMetricsAvailable
        ? 'Unavailable'
        : latest?.player_attached !== true
          ? 'Unavailable'
          : latest?.sender_sample_count === 0
            ? 'No samples'
            : measure(value, unit, 0);
  const senderSuppressionCount = latest
    ? latest.missed_down_boundaries +
      latest.final_gate_control_rejections +
      latest.final_gate_target_changes +
      latest.final_gate_focus_losses +
      latest.final_gate_lease_expirations
    : 0;
  const transportFailureCount = latest
    ? latest.sendinput_partial_events + latest.sendinput_zero_progress_failures
    : 0;
  const senderSummary = dryRun
    ? {
        status: 'Not measured',
        detail:
          'Dry-run playback sends no physical input; sender health and timing are not measured.',
      }
    : !backendMetricsAvailable
      ? {
          status: 'Unavailable',
          detail: 'Sender-side diagnostics are unavailable for this dispatch profile.',
        }
      : latest?.physical_session && !latest.player_attached
        ? {
            status: 'Error',
            detail: 'The physical session is active, but no native player is attached.',
          }
        : latest?.last_error
          ? {
              status: 'Error',
              detail: `Last error: ${latest.last_error}`,
            }
          : latest?.backend_status === 'error'
            ? {
                status: 'Error',
                detail: 'Sender-side backend reported an error.',
              }
            : latest?.backend_status === 'degraded'
              ? {
                  status: 'Attention',
                  detail: 'Sender-side backend reported degraded health.',
                }
              : latest?.player_attached && latest.sender_sample_count === 0
                ? {
                    status: 'Waiting',
                    detail:
                      'The physical player is attached; no SendInput call has been sampled yet.',
                  }
                : senderSuppressionCount === 0 && transportFailureCount === 0
                  ? { status: 'Healthy', detail: 'No Down suppression recorded this session.' }
                  : {
                      status: 'Attention',
                      detail: `${latest?.missed_physical_window_boundaries ?? 0} physical-window expirations; ${latest?.final_sender_window_expirations ?? 0} sender-window expirations; ${latest?.final_gate_focus_losses ?? 0} focus rejections; ${transportFailureCount} SendInput transport failures.`,
                    };
  return (
    <TabPanel id="performance" className="diagnostics-panel">
      {diagnostics.error ? (
        <DiagnosticsEmptyState
          title="Diagnostics error"
          detail={diagnostics.error}
          status="error"
        />
      ) : !diagnostics.enabled ? (
        <DiagnosticsEmptyState
          title="Diagnostics disabled"
          detail="Open Runtime diagnostics to inspect sender-side timing and input-health metrics."
        />
      ) : !nativeSessionActive ? (
        <DiagnosticsEmptyState
          title={playbackState === 'finished' ? 'No active playback session' : playbackStatus}
          detail={
            playbackStartPending || transportOperation === 'starting'
              ? 'Waiting for native playback confirmation before showing session diagnostics.'
              : transportOperation === 'preparing'
                ? 'Playback is being prepared; session diagnostics appear after it starts.'
                : preparedAdmission === 'confirmation_required'
                  ? 'Review and confirm playback before a native session can start.'
                  : playbackState === 'failed'
                    ? (playbackError ?? 'Playback failed before an active session was confirmed.')
                    : 'Runtime timing and input-health metrics appear when playback starts.'
          }
        />
      ) : !activeSession || !latest ? (
        <DiagnosticsEmptyState
          title="Waiting for session diagnostics"
          detail="Native playback is active; its matching diagnostics snapshot has not arrived yet."
        />
      ) : (
        <>
          <section className="diagnostics-sender-summary" aria-label="Sender-side status">
            <strong>Sender-side status: {senderSummary.status}</strong>
            <span>{senderSummary.detail}</span>
          </section>
          <MetricGroup title="Sender availability">
            <Metric
              label="Physical session"
              value={dryRun ? 'Not applicable (dry run)' : latest.physical_session ? 'Yes' : 'No'}
            />
            <Metric
              label="Player attached"
              value={dryRun ? 'Not applicable (dry run)' : latest.player_attached ? 'Yes' : 'No'}
            />
            <Metric label="Sender samples" value={senderSampleMetric(latest.sender_sample_count)} />
            <Metric
              label="Sender backend"
              value={dryRun ? 'Not measured' : backendStatusLabel(latest.backend_status)}
            />
          </MetricGroup>
          <MetricGroup title="Timing">
            <Metric label="Completion p50" value={backendMeasure(latest.p50_ms, 'ms')} />
            <Metric label="Completion p95" value={backendMeasure(latest.p95_ms, 'ms')} />
            <Metric label="Session max" value={backendMeasure(latest.max_lateness_us, 'μs')} />
            <Metric
              label="Max pre-call lateness"
              value={backendMeasure(latest.max_sendinput_pre_call_lateness_us, 'μs')}
            />
            <Metric
              label="Completion jitter σ"
              value={backendMeasure(latest.sigma_onset_ms, 'ms')}
            />
          </MetricGroup>
          <MetricGroup title="Frozen session timing">
            <Metric label="FPS" value={String(latest.fps)} />
            <Metric label="Frame period" value={`${(latest.frame_us / 1_000).toFixed(3)} ms`} />
            <Metric
              label="Base hold"
              value={`${(latest.frame_base_hold_us / 1_000).toFixed(3)} ms`}
            />
            <Metric label="Configured Timing Margin" value={`${latest.timing_margin_us} µs`} />
            <Metric label="Target hold" value={`${(latest.min_hold_us / 1_000).toFixed(3)} ms`} />
            <Metric
              label="Release gap"
              value={`${(latest.min_release_gap_us / 1_000).toFixed(3)} ms`}
            />
            <Metric
              label="Recommended sender margin"
              value={
                latest.timing_margin_recommendation.recommended_timing_margin_us === null
                  ? 'Unavailable'
                  : `${latest.timing_margin_recommendation.recommended_timing_margin_us} µs`
              }
            />
            <Metric
              label="Recommendation source"
              value={timingMarginRecommendationSourceLabel(
                latest.timing_margin_recommendation.source,
              )}
            />
          </MetricGroup>
          <MetricGroup title="Late events">
            <Metric label="Completion > 2 ms" value={senderSampleMetric(latest.late_2ms)} />
            <Metric label="Completion > 5 ms" value={senderSampleMetric(latest.late_5ms)} />
            <Metric label="Completion > 10 ms" value={senderSampleMetric(latest.late_10ms)} />
            <Metric label="Pre-call > 2 ms" value={senderSampleMetric(latest.pre_call_late_2ms)} />
            <Metric label="Pre-call > 5 ms" value={senderSampleMetric(latest.pre_call_late_5ms)} />
            <Metric
              label="Pre-call > 10 ms"
              value={senderSampleMetric(latest.pre_call_late_10ms)}
            />
          </MetricGroup>
          <MetricGroup title="Pre-call distribution">
            <Metric
              label="Pre-call < 250 μs"
              value={senderSampleMetric(latest.pre_call_lt_250us)}
            />
            <Metric
              label="Pre-call 250–500 μs"
              value={senderSampleMetric(latest.pre_call_250_500us)}
            />
            <Metric
              label="Pre-call 500–750 μs"
              value={senderSampleMetric(latest.pre_call_500_750us)}
            />
            <Metric
              label="Pre-call 750–1000 μs"
              value={senderSampleMetric(latest.pre_call_750_1000us)}
            />
            <Metric
              label="Pre-call 1.0–1.5 ms"
              value={senderSampleMetric(latest.pre_call_1000_1500us)}
            />
            <Metric
              label="Pre-call 1.5–2.0 ms"
              value={senderSampleMetric(latest.pre_call_1500_2000us)}
            />
            <Metric
              label="Pre-call ≥ 2.0 ms"
              value={senderSampleMetric(latest.pre_call_ge_2000us)}
            />
          </MetricGroup>
          <MetricGroup title="Deadline admission">
            <Metric
              label="Missed Down boundaries"
              value={playerMetric(latest.missed_down_boundaries)}
            />
            <Metric
              label="Physical-window expirations"
              value={playerMetric(latest.missed_physical_window_boundaries)}
            />
            <Metric label="Missed Down keys" value={playerMetric(latest.missed_down_keys)} />
            <Metric
              label="Unobserved backlog boundaries"
              value={playerMetric(latest.missed_unobserved_backlog_boundaries)}
            />
            <Metric
              label="Sender-window expirations"
              value={playerMetric(latest.final_sender_window_expirations)}
            />
            <Metric
              label="Focus gate rejections"
              value={playerMetric(latest.final_gate_focus_losses)}
            />
            <Metric
              label="Target / owner rejections"
              value={playerMetric(latest.final_gate_target_changes)}
            />
            <Metric
              label="Lease gate rejections"
              value={playerMetric(latest.final_gate_lease_expirations)}
            />
            <Metric
              label="Control / modifier rejections"
              value={playerMetric(latest.final_gate_control_rejections)}
            />
          </MetricGroup>
          <MetricGroup title="Physical floor delays">
            <Metric
              label="Hold-floor delayed boundaries"
              value={playerMetric(latest.hold_floor_delay_boundaries)}
            />
            <Metric
              label="Maximum hold-floor delay"
              value={dryRun ? 'Not measured' : measure(latest.max_hold_floor_delay_us, 'µs', 0)}
            />
            <Metric
              label="Release-floor delayed boundaries"
              value={playerMetric(latest.release_floor_delay_boundaries)}
            />
            <Metric
              label="Release-floor infeasible boundaries"
              value={playerMetric(latest.release_floor_infeasible_boundaries)}
            />
            <Metric
              label="Maximum release-floor delay"
              value={dryRun ? 'Not measured' : measure(latest.max_release_floor_delay_us, 'µs', 0)}
            />
          </MetricGroup>
          <MetricGroup title="Input transport">
            <Metric
              label="SendInput zero-progress failures"
              value={playerMetric(latest.sendinput_zero_progress_failures)}
            />
            <Metric
              label="SendInput partial events"
              value={playerMetric(latest.sendinput_partial_events)}
            />
            <Metric label="Dropped keys" value={playerMetric(latest.keys_dropped)} />
            <Metric label="Chord splits" value={playerMetric(latest.chord_split_events)} />
            <Metric label="Stuck keys" value={playerMetric(latest.stuck_keys)} />
            <Metric label="Active keys" value={playerMetric(latest.active_keys)} />
          </MetricGroup>
          <MetricGroup title="Windows power lifecycle">
            <Metric
              label="Suspend/resume notifications"
              value={latest.suspend_resume_registered ? 'Registered' : 'Unavailable'}
            />
            <Metric
              label="System power state"
              value={latest.system_suspend_active ? 'Suspended / re-admission blocked' : 'Running'}
            />
            <Metric
              label="Suspend notifications"
              value={String(latest.system_suspend_notifications)}
            />
            <Metric
              label="Resume notifications"
              value={String(latest.system_resume_notifications)}
            />
            <Metric
              label="Duplicate notifications"
              value={String(latest.duplicate_system_power_notifications)}
            />
            <Metric
              label="Power request active"
              value={latest.power_request_active ? 'Yes' : 'No'}
            />
            <Metric
              label="Power request create failures"
              value={String(latest.power_request_create_failures)}
            />
            <Metric
              label="Power request set failures"
              value={String(latest.power_request_set_failures)}
            />
            <Metric
              label="Power request clear failures"
              value={String(latest.power_request_clear_failures)}
            />
            <Metric
              label="Power request close failures"
              value={String(latest.power_request_close_failures)}
            />
            <Metric
              label="Notification registration failures"
              value={String(latest.suspend_resume_registration_failures)}
            />
            <Metric
              label="Notification unregistration failures"
              value={String(latest.suspend_resume_unregistration_failures)}
            />
          </MetricGroup>
          <MetricGroup title="Release">
            <Metric
              label="Max release lateness"
              value={backendMeasure(latest.release_max_us, 'μs')}
            />
            <Metric label="Release > 2 ms" value={senderSampleMetric(latest.release_late_2ms)} />
          </MetricGroup>
        </>
      )}
    </TabPanel>
  );
}
