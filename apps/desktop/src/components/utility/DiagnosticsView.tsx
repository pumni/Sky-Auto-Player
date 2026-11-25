import { useState } from 'react';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
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
      ref={scrollRef}
      className="flex min-h-0 min-w-0 flex-1 flex-col gap-0 overflow-auto px-4 pb-4"
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
      <div className="mb-2.5 flex flex-wrap items-center gap-2">
        <Button
          variant="outline"
          size="sm"
          className="min-h-[30px] h-auto bg-surface px-2.5 text-xs text-text-secondary hover:bg-control-hover hover:text-foreground disabled:cursor-progress disabled:opacity-70"
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
        <TabsList
          className="flex flex-none gap-0.5 overflow-x-auto border-b border-border"
          aria-label="Diagnostics views"
          activateOnFocus
        >
          <TabsTrigger
            className="whitespace-nowrap rounded-none border-0 px-2.5 py-2 text-xs text-text-tertiary duration-tab hover:bg-control-hover hover:text-foreground data-active:bg-surface data-active:text-accent-ink motion-reduce:transition-none"
            value="performance"
          >
            Performance
          </TabsTrigger>
          <TabsTrigger
            className="whitespace-nowrap rounded-none border-0 px-2.5 py-2 text-xs text-text-tertiary duration-tab hover:bg-control-hover hover:text-foreground data-active:bg-surface data-active:text-accent-ink motion-reduce:transition-none"
            value="timing"
          >
            Timing
          </TabsTrigger>
          <TabsTrigger
            className="whitespace-nowrap rounded-none border-0 px-2.5 py-2 text-xs text-text-tertiary duration-tab hover:bg-control-hover hover:text-foreground data-active:bg-surface data-active:text-accent-ink motion-reduce:transition-none"
            value="events"
          >
            Events
          </TabsTrigger>
          <TabsIndicator />
        </TabsList>
        <DiagnosticsPerformancePanel useStore={useStore} />
        <DiagnosticsTimingPanel useStore={useStore} />
        <DiagnosticsEventsPanel useStore={useStore} />
      </Tabs>
    </div>
  );
}
