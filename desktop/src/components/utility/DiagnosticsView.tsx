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
  const height = 112;
  const values = samples.flatMap((sample) => (sample.p95_ms === null ? [] : [sample.p95_ms]));
  const minimum = Math.min(0, ...values);
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
  return (
    <figure className="diagnostics-plot">
      <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-labelledby="timing-plot-title">
        <title id="timing-plot-title">Completion p95 residual over recent samples</title>
        <line x1="0" y1={zeroY} x2={width} y2={zeroY} className="plot-zero-axis" />
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
        {latest === null
          ? !hasActiveSession
            ? 'No active playback session.'
            : samples.length === 0
              ? 'No timing samples yet.'
              : 'Completion p95 distribution unavailable for this dispatch profile.'
          : `Latest completion p95 residual ${number(latest)} ms across ${values.length} samples.`}
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
  const backendMetricsAvailable = latest?.backend_status !== 'unavailable';
  const backendMetric = (value: number): string =>
    backendMetricsAvailable ? String(value) : 'Unavailable';
  const backendMeasure = (value: number, unit: string): string =>
    backendMetricsAvailable ? measure(value, unit, 0) : 'Unavailable';
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
              <MetricGroup title="Input health">
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
