import { Activity, X } from 'lucide-react';
import { Tab, TabList, TabPanel, Tabs } from 'react-aria-components';
import { useEffect, useRef, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface DiagnosticsPanelProps {
  useStore: DesktopStoreHook;
  mode: 'pane' | 'overlay';
  restoreFocusRef?: RefObject<HTMLButtonElement | null>;
}

function number(value: number | null | undefined, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : value.toFixed(digits);
}

function TimingPlot({ samples }: { samples: DesktopStore['diagnostics']['samples'] }) {
  const width = 560;
  const height = 112;
  const values = samples.slice(-600).map((sample) => Math.max(0, sample.max_lateness_us));
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
        <title id="timing-plot-title">Maximum timing lateness over recent samples</title>
        <line x1="0" y1={height - 4} x2={width} y2={height - 4} className="plot-axis" />
        {points && <polyline points={points} className="plot-line" />}
      </svg>
      <figcaption>
        {latest === null
          ? 'No timing samples yet.'
          : `Latest maximum lateness ${number(latest, 0)} microseconds across ${values.length} samples.`}
      </figcaption>
    </figure>
  );
}

export function DiagnosticsPanel({ useStore, mode, restoreFocusRef }: DiagnosticsPanelProps) {
  const diagnostics = useStore((store) => store.diagnostics);
  const close = useStore((store) => store.setDiagnosticsOpen);
  const surfaceRef = useRef<HTMLElement>(null);
  const overlayWasOpen = useRef(false);
  const pane = mode === 'pane';

  useEffect(() => {
    if (pane || !diagnostics.open) return;
    overlayWasOpen.current = true;
    const frame = window.requestAnimationFrame(() => surfaceRef.current?.focus());
    return () => {
      window.cancelAnimationFrame(frame);
      if (!overlayWasOpen.current) return;
      overlayWasOpen.current = false;
      window.queueMicrotask(() => restoreFocusRef?.current?.focus());
    };
  }, [diagnostics.open, pane, restoreFocusRef]);

  if (!diagnostics.open) return null;
  const latest = diagnostics.samples[diagnostics.samples.length - 1];
  return (
    <section
      ref={surfaceRef}
      className={`diagnostics-surface diagnostics-${mode}`}
      role={pane ? 'region' : 'dialog'}
      aria-label="Diagnostics"
      tabIndex={pane ? undefined : -1}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault();
          close(false);
          return;
        }
        if (pane || event.key !== 'Tab') return;
        const focusable = Array.from(
          surfaceRef.current?.querySelectorAll<HTMLElement>(
            'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
          ) ?? [],
        ).filter((element) => !element.hasAttribute('disabled'));
        if (focusable.length === 0) {
          event.preventDefault();
          surfaceRef.current?.focus();
          return;
        }
        const first = focusable[0]!;
        const last = focusable[focusable.length - 1]!;
        const activeElement = document.activeElement;
        if (event.shiftKey && (activeElement === first || activeElement === surfaceRef.current)) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }}
    >
      <div className="diagnostics-heading">
        <div>
          <p className="eyebrow">RUNTIME TELEMETRY</p>
          <h2>
            <Activity size={17} aria-hidden="true" /> Diagnostics
          </h2>
        </div>
        <button
          className="icon-button"
          type="button"
          aria-label="Close diagnostics"
          onClick={() => close(false)}
        >
          <X size={16} aria-hidden="true" />
        </button>
      </div>
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
          <Tab id="logs">Logs</Tab>
        </TabList>
        <TabPanel id="performance" className="diagnostics-panel">
          <div className="diagnostics-metrics">
            <Metric
              label="Max lateness"
              value={latest ? `${number(latest.max_lateness_us, 0)} μs` : '—'}
            />
            <Metric label="P50" value={latest ? `${number(latest.p50_ms)} ms` : '—'} />
            <Metric label="P95" value={latest ? `${number(latest.p95_ms)} ms` : '—'} />
            <Metric label="Sigma" value={latest ? `${number(latest.sigma_onset_ms)} ms` : '—'} />
            <Metric label="Dropped" value={latest ? String(latest.keys_dropped) : '—'} />
            <Metric label="Stuck" value={latest ? String(latest.stuck_keys) : '—'} />
          </div>
          <p className="diagnostics-status">
            {latest
              ? `Backend: ${latest.backend_status}`
              : 'Diagnostics are waiting for the native session.'}
          </p>
        </TabPanel>
        <TabPanel id="timing" className="diagnostics-panel">
          <TimingPlot samples={diagnostics.samples} />
        </TabPanel>
        <TabPanel id="events" className="diagnostics-panel diagnostics-scroll">
          {diagnostics.events.length === 0 ? (
            <p className="muted">No events recorded.</p>
          ) : (
            <ol className="diagnostics-lines">
              {diagnostics.events
                .slice()
                .reverse()
                .map((line) => (
                  <li key={line.seq}>
                    <span>#{line.seq}</span>
                    <strong>{line.name}</strong>
                    <em>{line.detail}</em>
                  </li>
                ))}
            </ol>
          )}
        </TabPanel>
        <TabPanel id="logs" className="diagnostics-panel diagnostics-scroll">
          {diagnostics.logs.length === 0 ? (
            <p className="muted">No logs recorded.</p>
          ) : (
            <ol className="diagnostics-lines">
              {diagnostics.logs
                .slice()
                .reverse()
                .map((line) => (
                  <li key={line.seq}>
                    <span>#{line.seq}</span>
                    <strong className={`log-${line.level}`}>{line.level}</strong>
                    <em>{line.message}</em>
                  </li>
                ))}
            </ol>
          )}
        </TabPanel>
      </Tabs>
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
