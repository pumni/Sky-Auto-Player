import { Activity } from 'lucide-react';
import type { ReactNode } from 'react';
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';

interface DiagnosticsViewProps {
  useStore: DesktopStoreHook;
}

function number(value: number | null | undefined, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : value.toFixed(digits);
}

function measure(value: number | null | undefined, unit: string, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : `${number(value, digits)} ${unit}`;
}

function count(value: number | null | undefined): string {
  return value === null || value === undefined ? 'Unavailable' : String(value);
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
  const availableSamples = samples.filter((sample) => sample.backend_status !== 'unavailable');
  if (!hasActiveSession) {
    return (
      <DiagnosticsEmptyState
        title="No active playback session"
        detail="Sender-side timing samples appear when playback starts."
      />
    );
  }
  if (availableSamples.length === 0) {
    return (
      <DiagnosticsEmptyState
        title="Timing unavailable"
        detail="Sender-side backend metrics are unavailable for this dispatch profile."
      />
    );
  }
  const values = availableSamples.map((sample) => sample.max_sendinput_pre_call_lateness_us);
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
        <title id="timing-plot-title">SendInput pre-call lateness over recent samples</title>
        <desc id="timing-plot-description">
          Sender-side maximum lateness before each SendInput call, with the effective Down cutoff
          grace threshold.
        </desc>
        <line x1="0" y1={zeroY} x2={width} y2={zeroY} className="plot-zero-axis" />
        {thresholdY !== null && (
          <>
            <line x1="0" y1={thresholdY} x2={width} y2={thresholdY} className="plot-threshold" />
            <text x={width - 4} y={Math.max(plotTop + 10, thresholdY - 4)} className="plot-label">
              Down grace {threshold} μs
            </text>
          </>
        )}
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
        {latest === null
          ? 'No sender-side timing samples yet.'
          : `Latest sender pre-call max ${latest} μs across ${values.length} samples.${
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
  const backendMetric = (value: number): string =>
    backendMetricsAvailable ? String(value) : 'Unavailable';
  const backendMeasure = (value: number, unit: string): string =>
    backendMetricsAvailable ? measure(value, unit, 0) : 'Unavailable';
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
    : senderSuppressionCount === 0 && transportFailureCount === 0
      ? { status: 'Healthy', detail: 'No Down suppression recorded this session.' }
      : {
          status: 'Attention',
          detail: `${latest?.missed_hard_late_boundaries ?? 0} hard-late boundaries; ${latest?.final_gate_focus_losses ?? 0} focus rejections; ${transportFailureCount} SendInput transport failures.`,
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
              <MetricGroup title="Timing">
                <Metric label="Completion p50" value={measure(latest.p50_ms, 'ms')} />
                <Metric label="Completion p95" value={measure(latest.p95_ms, 'ms')} />
                <Metric label="Session max" value={measure(latest.max_lateness_us, 'μs', 0)} />
                <Metric
                  label="Max pre-call lateness"
                  value={backendMeasure(latest.max_sendinput_pre_call_lateness_us, 'μs')}
                />
                <Metric label="Completion jitter σ" value={measure(latest.sigma_onset_ms, 'ms')} />
              </MetricGroup>
              <MetricGroup title="Late events">
                <Metric label="Completion > 2 ms" value={count(latest.late_2ms)} />
                <Metric label="Completion > 5 ms" value={count(latest.late_5ms)} />
                <Metric label="Completion > 10 ms" value={count(latest.late_10ms)} />
                <Metric label="Pre-call > 2 ms" value={backendMetric(latest.pre_call_late_2ms)} />
                <Metric label="Pre-call > 5 ms" value={backendMetric(latest.pre_call_late_5ms)} />
                <Metric label="Pre-call > 10 ms" value={backendMetric(latest.pre_call_late_10ms)} />
              </MetricGroup>
              <MetricGroup title="Pre-call distribution">
                <Metric label="Pre-call < 250 μs" value={backendMetric(latest.pre_call_lt_250us)} />
                <Metric
                  label="Pre-call 250–500 μs"
                  value={backendMetric(latest.pre_call_250_500us)}
                />
                <Metric
                  label="Pre-call 500–750 μs"
                  value={backendMetric(latest.pre_call_500_750us)}
                />
                <Metric
                  label="Pre-call 750–1000 μs"
                  value={backendMetric(latest.pre_call_750_1000us)}
                />
                <Metric
                  label="Pre-call 1.0–1.5 ms"
                  value={backendMetric(latest.pre_call_1000_1500us)}
                />
                <Metric
                  label="Pre-call 1.5–2.0 ms"
                  value={backendMetric(latest.pre_call_1500_2000us)}
                />
                <Metric
                  label="Pre-call ≥ 2.0 ms"
                  value={backendMetric(latest.pre_call_ge_2000us)}
                />
                <Metric
                  label="Down cutoff grace"
                  value={backendMeasure(latest.down_late_grace_us, 'μs')}
                />
              </MetricGroup>
              <MetricGroup title="Deadline admission">
                <Metric
                  label="Missed Down boundaries"
                  value={backendMetric(latest.missed_down_boundaries)}
                />
                <Metric
                  label="Hard-late Down boundaries"
                  value={backendMetric(latest.missed_hard_late_boundaries)}
                />
                <Metric label="Missed Down keys" value={backendMetric(latest.missed_down_keys)} />
                <Metric
                  label="Backlog misses"
                  value={backendMetric(latest.missed_backlog_boundaries)}
                />
                <Metric
                  label="Final cutoff misses"
                  value={backendMetric(latest.final_gate_cutoff_misses)}
                />
                <Metric
                  label="Focus gate rejections"
                  value={backendMetric(latest.final_gate_focus_losses)}
                />
                <Metric
                  label="Target changes"
                  value={backendMetric(latest.final_gate_target_changes)}
                />
                <Metric
                  label="Lease expirations"
                  value={backendMetric(latest.final_gate_lease_expirations)}
                />
                <Metric
                  label="Control rejections"
                  value={backendMetric(latest.final_gate_control_rejections)}
                />
              </MetricGroup>
              <MetricGroup title="Input transport">
                <Metric
                  label="SendInput zero-progress failures"
                  value={backendMetric(latest.sendinput_zero_progress_failures)}
                />
                <Metric
                  label="SendInput partial events"
                  value={backendMetric(latest.sendinput_partial_events)}
                />
                <Metric label="Dropped keys" value={backendMetric(latest.keys_dropped)} />
                <Metric label="Chord splits" value={backendMetric(latest.chord_split_events)} />
                <Metric label="Stuck keys" value={backendMetric(latest.stuck_keys)} />
                <Metric label="Active keys" value={backendMetric(latest.active_keys)} />
              </MetricGroup>
              <MetricGroup title="Release">
                <Metric
                  label="Max release lateness"
                  value={measure(latest.release_max_us, 'μs', 0)}
                />
                <Metric label="Release > 2 ms" value={count(latest.release_late_2ms)} />
              </MetricGroup>
              <p className="diagnostics-status">
                <Activity size={14} aria-hidden="true" />
                <span>Backend</span>
                <BackendStatus status={latest.backend_status} />
              </p>
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

function BackendStatus({
  status,
}: {
  status: DesktopStore['diagnostics']['samples'][number]['backend_status'];
}) {
  const label =
    status === 'healthy'
      ? 'Healthy'
      : status === 'degraded'
        ? 'Degraded'
        : status === 'error'
          ? 'Error'
          : 'Unavailable';
  return <span className={`diagnostics-backend-status is-${status}`}>{label}</span>;
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
