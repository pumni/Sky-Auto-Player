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

function TimingPlot({ samples }: { samples: DesktopStore['diagnostics']['samples'] }) {
  const width = 560;
  const height = 112;
  const hasSession = samples.length > 0 && samples.at(-1)?.session_id !== null;
  const values = hasSession ? samples.map((sample) => Math.max(0, sample.p95_ms)) : [];
  const maximum = Math.max(1, ...values);
  const points = values
    .map((value, index) => {
      const x = values.length <= 1 ? 0 : (index / (values.length - 1)) * width;
      const y = height - (value / maximum) * (height - 8) - 4;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(' ');
  const latest = values.length ? values[values.length - 1] : null;
  return (
    <figure className="diagnostics-plot">
      <svg viewBox={`0 0 ${width} ${height}`} role="img" aria-labelledby="timing-plot-title">
        <title id="timing-plot-title">Completion p95 lateness over recent samples</title>
        <line x1="0" y1={height - 4} x2={width} y2={height - 4} className="plot-axis" />
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
        {latest === null
          ? hasSession
            ? 'No timing samples yet.'
            : 'No active playback session.'
          : `Latest completion p95 ${number(latest)} ms across ${values.length} samples.`}
      </figcaption>
    </figure>
  );
}

export function DiagnosticsView({ useStore }: DiagnosticsViewProps) {
  const diagnostics = useStore((store: DesktopStore) => store.diagnostics);
  const scrollRef = useScrollVisibility<HTMLDivElement>();
  const eventsScrollRef = useScrollVisibility<HTMLDivElement>();
  const latest = diagnostics.samples[diagnostics.samples.length - 1];
  const activeSession = latest?.session_id !== null && latest?.session_id !== undefined;
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
                <Metric label="Completion p50" value={`${number(latest.p50_ms)} ms`} />
                <Metric label="Completion p95" value={`${number(latest.p95_ms)} ms`} />
                <Metric label="Session max" value={`${number(latest.max_lateness_us, 0)} μs`} />
                <Metric label="Completion jitter σ" value={`${number(latest.sigma_onset_ms)} ms`} />
              </MetricGroup>
              <MetricGroup title="Late events">
                <Metric label="> 2 ms" value={String(latest.late_2ms)} />
                <Metric label="> 5 ms" value={String(latest.late_5ms)} />
                <Metric label="> 10 ms" value={String(latest.late_10ms)} />
              </MetricGroup>
              <MetricGroup title="Input health">
                <Metric label="Dropped keys" value={String(latest.keys_dropped)} />
                <Metric label="Chord splits" value={String(latest.chord_split_events)} />
                <Metric label="Stuck keys" value={String(latest.stuck_keys)} />
                <Metric label="Active keys" value={String(latest.active_keys)} />
              </MetricGroup>
              <MetricGroup title="Release">
                <Metric
                  label="Max release lateness"
                  value={
                    latest.release_max_us === null
                      ? 'Unavailable'
                      : `${number(latest.release_max_us, 0)} μs`
                  }
                />
                <Metric
                  label="Release > 2 ms"
                  value={
                    latest.release_late_2ms === null
                      ? 'Unavailable'
                      : String(latest.release_late_2ms)
                  }
                />
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
          <TimingPlot samples={diagnostics.samples} />
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
          : 'No session';
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
