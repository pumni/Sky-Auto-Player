import { useState, type ReactNode } from 'react';
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
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
}: {
  samples: DesktopStore['diagnostics']['samples'];
  hasActiveSession: boolean;
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
  const threshold = availableSamples.at(-1)?.down_late_grace_us ?? null;
  const minimum = 0;
  const maximum = Math.max(0, ...values, threshold ?? 0);
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
  const thresholdY = threshold === null ? null : yFor(threshold);
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
          after recovery. Pre-call timing covers physical sends, while the fixed Down late cutoff
          applies only to Down-bearing sends.
        </desc>
        <line x1="0" y1={zeroY} x2={width} y2={zeroY} className="plot-zero-axis" />
        {thresholdY !== null && (
          <>
            <line x1="0" y1={thresholdY} x2={width} y2={thresholdY} className="plot-threshold" />
            <text x={width - 4} y={Math.max(plotTop + 10, thresholdY - 4)} className="plot-label">
              Late Down tolerance {threshold} μs
            </text>
          </>
        )}
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
        {latest === null
          ? 'No sender-side timing samples yet.'
          : `Session max pre-call lateness observed at the latest diagnostics snapshot: ${latest} μs across ${values.length} snapshots. This cumulative value does not decrease after recovery. The fixed Down late cutoff applies only to Down-bearing sends.${
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
  const playback = useStore((store: DesktopStore) => store.playback);
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
  const playbackIsActive = ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const activeSession =
    diagnostics.enabled &&
    playbackIsActive &&
    playback.sessionId !== null &&
    latest?.session_id === playback.sessionId;
  const backendMetricsAvailable = latest !== undefined && latest.backend_status !== 'unavailable';
  const visibleTraceExportIdentity =
    traceExportIdentity !== null &&
    (playback.sessionId === null || playback.sessionId === traceExportIdentity.session)
      ? traceExportIdentity
      : null;
  const playerMetric = (value: number): string =>
    !backendMetricsAvailable || latest?.player_attached !== true ? 'Unavailable' : String(value);
  const senderSampleMetric = (value: number | null | undefined): string =>
    !backendMetricsAvailable
      ? 'Unavailable'
      : latest?.player_attached !== true
        ? 'Unavailable'
        : latest?.sender_sample_count === 0
          ? 'No samples'
          : value === null || value === undefined
            ? 'Unavailable'
            : String(value);
  const backendMeasure = (value: number | null, unit: string): string =>
    !backendMetricsAvailable
      ? 'Unavailable'
      : latest?.player_attached !== true
        ? 'Unavailable'
        : latest?.sender_sample_count === 0
          ? 'No samples'
          : measure(value, unit, 0);
  const senderSuppressionCount = latest
    ? latest.missed_down_boundaries +
      latest.missed_down_keys +
      latest.missed_backlog_boundaries +
      latest.missed_hard_late_boundaries +
      latest.final_gate_cutoff_misses +
      latest.final_gate_control_rejections +
      latest.final_gate_target_changes +
      latest.final_gate_focus_losses +
      latest.final_gate_lease_expirations
    : 0;
  const transportFailureCount = latest
    ? latest.sendinput_partial_events + latest.sendinput_zero_progress_failures
    : 0;
  const senderSummary = !backendMetricsAvailable
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
                    detail: `${latest?.missed_hard_late_boundaries ?? 0} hard-late boundaries; ${latest?.final_gate_focus_losses ?? 0} focus rejections; ${transportFailureCount} SendInput transport failures.`,
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
          ) : !activeSession || !latest ? (
            <DiagnosticsEmptyState
              title="No active playback session"
              detail="Runtime timing and input-health metrics appear when playback starts."
            />
          ) : (
            <>
              <section className="diagnostics-sender-summary" aria-label="Sender-side status">
                <strong>Sender-side status: {senderSummary.status}</strong>
                <span>{senderSummary.detail}</span>
              </section>
              <MetricGroup title="Sender availability">
                <Metric label="Physical session" value={latest.physical_session ? 'Yes' : 'No'} />
                <Metric label="Player attached" value={latest.player_attached ? 'Yes' : 'No'} />
                <Metric
                  label="Sender samples"
                  value={senderSampleMetric(latest.sender_sample_count)}
                />
                <Metric label="Sender backend" value={backendStatusLabel(latest.backend_status)} />
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
                <Metric label="Late Down tolerance" value={`${latest.down_late_grace_us} µs`} />
                <Metric
                  label="Recommended sender margin"
                  value={`${latest.timing_margin_recommendation.recommended_timing_margin_us} µs`}
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
                  label="Hard-late Down boundaries"
                  value={playerMetric(latest.missed_hard_late_boundaries)}
                />
                <Metric label="Missed Down keys" value={playerMetric(latest.missed_down_keys)} />
                <Metric
                  label="Backlog misses"
                  value={playerMetric(latest.missed_backlog_boundaries)}
                />
                <Metric
                  label="Final cutoff misses"
                  value={playerMetric(latest.final_gate_cutoff_misses)}
                />
                <Metric
                  label="Focus gate rejections"
                  value={playerMetric(latest.final_gate_focus_losses)}
                />
                <Metric
                  label="Target changes"
                  value={playerMetric(latest.final_gate_target_changes)}
                />
                <Metric
                  label="Lease expirations"
                  value={playerMetric(latest.final_gate_lease_expirations)}
                />
                <Metric
                  label="Control rejections"
                  value={playerMetric(latest.final_gate_control_rejections)}
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
            hasActiveSession={activeSession}
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
