import { useState, type ReactNode } from 'react';
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { playbackIssuePresentation, playbackSessionStatusConcern } from '../../state/store';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import { timingMarginRecommendationSourceLabel } from '../timingMarginSource';

interface DiagnosticsViewProps {
  useStore: DesktopStoreHook;
}

function number(value: number | null | undefined, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : value.toFixed(digits);
}

function measure(value: number | null | undefined, unit: string, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : `${number(value, digits)} ${unit}`;
}

function formatEventTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

type PlaybackSummaryState = Pick<
  DesktopStore['playback'],
  | 'error'
  | 'sessionId'
  | 'startRequestId'
  | 'state'
  | 'statusRetryPending'
  | 'transportOperation'
  | 'prepared'
>;

function playbackStatusLabel(playback: PlaybackSummaryState): string {
  const issue = playbackIssuePresentation(playback);
  const statusConcern = playbackSessionStatusConcern(playback);
  if (statusConcern === 'recoverable_status') {
    return 'Native playback status uncertain';
  }
  if (statusConcern === 'conflict') return 'Native playback session conflict';
  if (issue?.kind === 'target_failure' || issue?.kind === 'playback_failure') {
    return 'Playback failed';
  }
  if (playback.prepared?.admission === 'confirmation_required') return 'Confirmation required';

  switch (playback.transportOperation) {
    case 'preparing':
      return 'Preparing playback';
    case 'starting':
      return 'Starting playback';
    case 'pausing':
      return 'Pausing playback';
    case 'resuming':
      return 'Resuming playback';
    case 'stopping':
      return 'Stopping playback';
    case 'advancing':
      return 'Starting next song';
    case 'restarting':
      return 'Restarting song';
  }

  if (playback.startRequestId !== null) return 'Waiting for native playback confirmation';
  switch (playback.state) {
    case 'starting':
      return 'Starting playback';
    case 'playing':
      return 'Playing';
    case 'paused':
      return 'Paused';
    case 'stopping':
      return 'Stopping playback';
    case 'finished':
      return 'Playback finished';
    case 'failed':
      return 'Playback failed';
    default:
      return playback.prepared ? 'Prepared' : 'No active playback session';
  }
}

function PlaybackSessionSummary({ useStore }: DiagnosticsViewProps) {
  const error = useStore((store) => store.playback.error);
  const sessionId = useStore((store) => store.playback.sessionId);
  const startRequestId = useStore((store) => store.playback.startRequestId);
  const state = useStore((store) => store.playback.state);
  const statusRetryPending = useStore((store) => store.playback.statusRetryPending);
  const transportOperation = useStore((store) => store.playback.transportOperation);
  const prepared = useStore((store) => store.playback.prepared);
  const currentSongId = useStore((store) => store.playback.currentSong?.songId ?? null);
  const currentSongTitle = useStore((store) => store.playback.currentSong?.title);
  const hasCurrentSong = useStore((store) => store.playback.currentSong !== null);
  const dryRun = useStore(
    (store) => store.playback.context?.dryRun ?? store.playback.prepared?.config.dry_run ?? null,
  );
  const playback: PlaybackSummaryState = {
    error,
    sessionId,
    startRequestId,
    state,
    statusRetryPending,
    transportOperation,
    prepared,
  };
  const status = playbackStatusLabel(playback);
  const isActive = ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const sessionStatusConcern = playbackSessionStatusConcern(playback);
  const hasPlaybackContext =
    playback.state !== 'idle' ||
    playback.transportOperation !== null ||
    playback.startRequestId !== null ||
    playback.prepared !== null ||
    hasCurrentSong ||
    playback.error !== null;

  if (!hasPlaybackContext) return null;

  return (
    <section className="diagnostics-playback-summary" aria-label="Playback identity and status">
      <strong>Playback status: {status}</strong>
      {hasCurrentSong && (
        <span>
          {sessionStatusConcern !== null
            ? 'Last confirmed session song'
            : isActive
              ? 'Now Playing'
              : 'Last session song'}
          : {currentSongTitle}
        </span>
      )}
      {playback.prepared?.song && playback.prepared.song.song_id !== currentSongId && (
        <span>Prepared: {playback.prepared.song.title}</span>
      )}
      {dryRun !== null && (isActive || playback.prepared) && (
        <span>Mode: {dryRun ? 'Dry run · no physical input sent' : 'Physical input'}</span>
      )}
    </section>
  );
}

function DiagnosticsEmptyState({
  title,
  detail,
  status = 'empty',
}: {
  title: string;
  detail: string;
  status?: 'empty' | 'error';
}) {
  return (
    <div className={`diagnostics-empty-state is-${status}`} role="status">
      <strong>{title}</strong>
      <span>{detail}</span>
    </div>
  );
}

function TimingPlot({
  samples,
  hasActiveSession,
  dryRun,
}: {
  samples: DesktopStore['diagnostics']['samples'];
  hasActiveSession: boolean;
  dryRun: boolean;
}) {
  const width = 560;
  const height = 132;
  const availableSamples = samples.filter(
    (sample) =>
      sample.backend_status !== 'unavailable' &&
      sample.sender_sample_count > 0 &&
      sample.max_sendinput_pre_call_lateness_us !== null,
  );
  if (!hasActiveSession) {
    return (
      <DiagnosticsEmptyState
        title="No active playback session"
        detail="Sender-side timing samples appear when playback starts."
      />
    );
  }
  if (dryRun) {
    return (
      <DiagnosticsEmptyState
        title="No physical timing samples in dry run"
        detail="Dry-run playback sends no physical input, so sender timing is not measured."
      />
    );
  }
  if (samples.length === 0) {
    return (
      <DiagnosticsEmptyState
        title="Waiting for session diagnostics"
        detail="Native playback is active; its matching diagnostics snapshot has not arrived yet."
      />
    );
  }
  if (availableSamples.length === 0) {
    const latest = samples.at(-1);
    const playerAttached = latest?.player_attached === true;
    const backendUnavailable = latest === undefined || latest.backend_status === 'unavailable';
    const physicalPlayerMissing = latest?.physical_session === true && !playerAttached;
    const backendError = latest?.backend_status === 'error';
    const noSenderSamples =
      !backendUnavailable &&
      !physicalPlayerMissing &&
      !backendError &&
      playerAttached &&
      latest.sender_sample_count === 0;
    return (
      <DiagnosticsEmptyState
        title={
          backendUnavailable
            ? 'Timing unavailable'
            : physicalPlayerMissing
              ? 'Player not attached'
              : backendError
                ? 'Sender timing error'
                : noSenderSamples
                  ? 'No sender samples yet'
                  : 'Timing unavailable'
        }
        detail={
          backendUnavailable
            ? latest === undefined
              ? 'No sender diagnostics snapshot is available for this session yet.'
              : 'Sender-side backend metrics are unavailable for this dispatch profile.'
            : physicalPlayerMissing
              ? 'A physical session is active, but its native player is not attached.'
              : backendError
                ? `The sender backend reported an error: ${latest?.last_error ?? 'no sender sample is available.'}`
                : noSenderSamples
                  ? 'The physical player is attached; no SendInput call has been sampled yet.'
                  : 'Sender-side timing measurements are unavailable for this session.'
        }
      />
    );
  }
  const values = availableSamples.map((sample) => sample.max_sendinput_pre_call_lateness_us!);
  const minimum = 0;
  const maximum = Math.max(0, ...values);
  const range = Math.max(1, maximum - minimum);
  const plotTop = 4;
  const plotBottom = height - 4;
  const yFor = (value: number) => plotBottom - ((value - minimum) / range) * (plotBottom - plotTop);
  const points = values
    .map((value, index) => {
      const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * width;
      return `${x.toFixed(1)},${yFor(value).toFixed(1)}`;
    })
    .join(' ');
  const latest = values.length ? values[values.length - 1] : null;
  const zeroY = yFor(0);
  const latestSample = availableSamples[availableSamples.length - 1];
  return (
    <figure className="diagnostics-plot">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-labelledby="timing-plot-title timing-plot-description"
      >
        <title id="timing-plot-title">
          Session max SendInput pre-call lateness observed at each diagnostics snapshot
        </title>
        <desc id="timing-plot-description">
          Cumulative session maximum observed at each diagnostics snapshot; it does not decrease
          after recovery. Completion latency is reported separately.
        </desc>
        <line x1="0" y1={zeroY} x2={width} y2={zeroY} className="plot-zero-axis" />
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
        {latest === null
          ? 'No sender-side timing samples yet.'
          : `Session max pre-call lateness observed at the latest diagnostics snapshot: ${latest} μs across ${values.length} snapshots. This cumulative value does not decrease after recovery.${
              latestSample?.p95_ms === null || latestSample?.p95_ms === undefined
                ? ''
                : ` Completion p95 observer value ${number(latestSample.p95_ms)} ms.`
            }`}
      </figcaption>
    </figure>
  );
}

export function DiagnosticsView({ useStore }: DiagnosticsViewProps) {
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
  const exportSenderTrace = useStore((store: DesktopStore) => store.exportSenderTrace);
  const [traceExporting, setTraceExporting] = useState(false);
  const [traceExportError, setTraceExportError] = useState<string | null>(null);
  const [traceExportIdentity, setTraceExportIdentity] = useState<{
    song: string;
    session: string;
  } | null>(null);
  const scrollRef = useScrollVisibility<HTMLDivElement>();
  const eventsScrollRef = useScrollVisibility<HTMLDivElement>();
  const latest = diagnostics.samples[diagnostics.samples.length - 1];
  const nativeSessionActive = diagnostics.enabled && playbackIsActive && sessionId !== null;
  const activeSession = nativeSessionActive && latest?.session_id === sessionId;
  const backendMetricsAvailable = latest !== undefined && latest.backend_status !== 'unavailable';
  const visibleTraceExportIdentity =
    traceExportIdentity !== null &&
    (sessionId === null || sessionId === traceExportIdentity.session)
      ? traceExportIdentity
      : null;
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
  const handleExportSenderTrace = async () => {
    setTraceExporting(true);
    setTraceExportError(null);
    setTraceExportIdentity(null);
    try {
      const content = await exportSenderTrace();
      const trace = JSON.parse(content) as {
        session_id?: unknown;
        song_id?: unknown;
        song_title?: unknown;
      };
      if (typeof trace.session_id !== 'string' || typeof trace.song_id !== 'string') {
        throw new Error('Sender trace export is missing session identity.');
      }
      setTraceExportIdentity({
        song:
          typeof trace.song_title === 'string' && trace.song_title.length > 0
            ? trace.song_title
            : trace.song_id,
        session: trace.session_id,
      });
      const objectUrl = URL.createObjectURL(new Blob([content], { type: 'application/json' }));
      const anchor = document.createElement('a');
      anchor.href = objectUrl;
      anchor.download = 'sky-sender-trace.json';
      anchor.click();
      window.setTimeout(() => URL.revokeObjectURL(objectUrl), 0);
    } catch (error) {
      setTraceExportError(error instanceof Error ? error.message : String(error));
    } finally {
      setTraceExporting(false);
    }
  };
  return (
    <div
      ref={scrollRef}
      className="diagnostics-view scroll-surface"
      aria-labelledby="diagnostics-title"
    >
      <h3 id="diagnostics-title" className="visually-hidden">
        Runtime diagnostics
      </h3>
      {diagnostics.error && (
        <p className="inline-error" role="alert">
          {diagnostics.error}
        </p>
      )}
      {diagnostics.enabled && <PlaybackSessionSummary useStore={useStore} />}
      <div className="diagnostics-export">
        <button
          className="diagnostics-export-button"
          type="button"
          disabled={traceExporting}
          onClick={() => void handleExportSenderTrace()}
        >
          {traceExporting ? 'Preparing sender trace…' : 'Export last sender trace'}
        </button>
        <span role="status">
          {visibleTraceExportIdentity
            ? `Trace available: Song ${visibleTraceExportIdentity.song} · Session ${visibleTraceExportIdentity.session}`
            : 'Available after a completed physical playback session.'}
        </span>
      </div>
      {traceExportError && (
        <p className="inline-error" role="alert">
          {traceExportError}
        </p>
      )}
      <Tabs className="diagnostics-tabs" defaultSelectedKey="performance">
        <TabList aria-label="Diagnostics views">
          <Tab id="performance">Performance</Tab>
          <Tab id="timing">Timing</Tab>
          <Tab id="events">Events</Tab>
        </TabList>
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
                        ? (playbackError ??
                          'Playback failed before an active session was confirmed.')
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
                  value={
                    dryRun ? 'Not applicable (dry run)' : latest.physical_session ? 'Yes' : 'No'
                  }
                />
                <Metric
                  label="Player attached"
                  value={
                    dryRun ? 'Not applicable (dry run)' : latest.player_attached ? 'Yes' : 'No'
                  }
                />
                <Metric
                  label="Sender samples"
                  value={senderSampleMetric(latest.sender_sample_count)}
                />
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
                <Metric
                  label="Target hold"
                  value={`${(latest.min_hold_us / 1_000).toFixed(3)} ms`}
                />
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
                <Metric
                  label="Pre-call > 2 ms"
                  value={senderSampleMetric(latest.pre_call_late_2ms)}
                />
                <Metric
                  label="Pre-call > 5 ms"
                  value={senderSampleMetric(latest.pre_call_late_5ms)}
                />
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
                  value={
                    dryRun ? 'Not measured' : measure(latest.max_release_floor_delay_us, 'µs', 0)
                  }
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
                  value={
                    latest.system_suspend_active ? 'Suspended / re-admission blocked' : 'Running'
                  }
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
                <Metric
                  label="Release > 2 ms"
                  value={senderSampleMetric(latest.release_late_2ms)}
                />
              </MetricGroup>
            </>
          )}
        </TabPanel>
        <TabPanel id="timing" className="diagnostics-panel">
          <TimingPlot
            samples={activeSession ? diagnostics.samples : []}
            hasActiveSession={nativeSessionActive}
            dryRun={nativeSessionActive && dryRun}
          />
        </TabPanel>
        <TabPanel
          ref={eventsScrollRef}
          id="events"
          className="diagnostics-panel diagnostics-scroll scroll-surface"
        >
          {diagnostics.events.length === 0 ? (
            <p className="muted">No events recorded.</p>
          ) : (
            <ol className="diagnostics-lines">
              {diagnostics.events
                .slice()
                .reverse()
                .map((line) => (
                  <li key={line.seq}>
                    <time dateTime={new Date(line.timestamp).toISOString()}>
                      {formatEventTime(line.timestamp)}
                    </time>
                    <strong>{line.name}</strong>
                    <em>{line.detail}</em>
                  </li>
                ))}
            </ol>
          )}
        </TabPanel>
      </Tabs>
    </div>
  );
}

function backendStatusLabel(
  status: DesktopStore['diagnostics']['samples'][number]['backend_status'],
): string {
  return status === 'healthy'
    ? 'Healthy'
    : status === 'degraded'
      ? 'Degraded'
      : status === 'error'
        ? 'Error'
        : 'Unavailable';
}

function MetricGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="diagnostics-metric-group" aria-label={title}>
      <h4>{title}</h4>
      <div className="diagnostics-metrics">{children}</div>
    </section>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div className="diagnostics-metric">
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}
