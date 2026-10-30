import { useState } from 'react';
import { Tab, TabList, Tabs } from 'react-aria-components';
import type { DesktopStoreHook } from '../../state/store';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import { DiagnosticsEventsPanel } from './DiagnosticsEventsPanel';
import { DiagnosticsPerformancePanel } from './DiagnosticsPerformancePanel';
import { DiagnosticsPlaybackSummary } from './DiagnosticsPlaybackSummary';
import { DiagnosticsTimingPanel } from './DiagnosticsTimingPlot';

export function DiagnosticsView({ useStore }: { useStore: DesktopStoreHook }) {
  const diagnostics = useStore((store) => store.diagnostics);
  const sessionId = useStore((store) => store.playback.sessionId);
  const exportSenderTrace = useStore((store) => store.exportSenderTrace);
  const [traceExporting, setTraceExporting] = useState(false);
  const [traceExportError, setTraceExportError] = useState<string | null>(null);
  const [traceExportIdentity, setTraceExportIdentity] = useState<{
    song: string;
    session: string;
  } | null>(null);
  const scrollRef = useScrollVisibility<HTMLDivElement>();
  const visibleTraceExportIdentity =
    traceExportIdentity !== null &&
    (sessionId === null || sessionId === traceExportIdentity.session)
      ? traceExportIdentity
      : null;

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
      {diagnostics.enabled && <DiagnosticsPlaybackSummary useStore={useStore} />}
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
        <DiagnosticsPerformancePanel useStore={useStore} />
        <DiagnosticsTimingPanel useStore={useStore} />
        <DiagnosticsEventsPanel useStore={useStore} />
      </Tabs>
    </div>
  );
}
