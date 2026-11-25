import { useState } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import { Tabs, TabsIndicator, TabsList, TabsTrigger } from '../ui/tabs';
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
  const [traceExportIdentity, setTraceExportIdentity] = useState<{ session: string } | null>(null);
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
      };
      if (typeof trace.session_id !== 'string' || typeof trace.song_id !== 'string') {
        throw new Error('Sender trace export is missing session identity.');
      }
      setTraceExportIdentity({ session: trace.session_id });
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
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 px-4 pb-4 pt-3"
      aria-labelledby="diagnostics-title"
      data-slot="diagnostics-view"
    >
      <h3 id="diagnostics-title" className="sr-only">
        Runtime diagnostics
      </h3>
      {diagnostics.error && (
        <p
          className="rounded-control border border-danger-border bg-danger-surface px-2 py-1.5 text-xs text-danger"
          role="alert"
        >
          {diagnostics.error}
        </p>
      )}
      {diagnostics.enabled && <DiagnosticsPlaybackSummary useStore={useStore} />}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          className="min-h-[30px] h-auto px-2.5 text-xs text-text-secondary hover:text-foreground disabled:cursor-progress disabled:opacity-70"
          type="button"
          title="Export the last available session envelope. Raw packet traces are not captured in production."
          disabled={traceExporting}
          onClick={() => void handleExportSenderTrace()}
        >
          {traceExporting ? 'Preparing sender metadata…' : 'Export sender metadata'}
        </Button>
        <span className="text-xs text-text-tertiary" role="status">
          {visibleTraceExportIdentity
            ? 'Session metadata exported; raw packet trace was not captured in production.'
            : 'Exports the last available session, when present; no raw packet trace is recorded.'}
        </span>
      </div>
      {traceExportError && (
        <p
          className="rounded-control border border-danger-border bg-danger-surface px-2 py-1.5 text-xs text-danger"
          role="alert"
        >
          {traceExportError}
        </p>
      )}
      <Tabs className="flex min-h-0 flex-1 flex-col gap-0" defaultValue="performance">
        <TabsList aria-label="Diagnostics views" variant="segmented" activateOnFocus>
          <TabsTrigger value="performance">Performance</TabsTrigger>
          <TabsTrigger value="timing">Timing</TabsTrigger>
          <TabsTrigger value="events">Events</TabsTrigger>
          <TabsIndicator />
        </TabsList>
        <DiagnosticsPerformancePanel useStore={useStore} />
        <DiagnosticsTimingPanel useStore={useStore} />
        <DiagnosticsEventsPanel useStore={useStore} />
      </Tabs>
    </div>
  );
}
