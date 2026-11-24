import { useMemo } from 'react';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import type { DesktopStoreHook } from '../../state/store';
import { TabsContent } from '../ui/tabs';
import { formatEventTime } from './diagnosticsPresentation';

export function DiagnosticsEventsPanel({ useStore }: { useStore: DesktopStoreHook }) {
  const events = useStore((store) => store.diagnostics.events);
  const eventsScrollRef = useScrollVisibility<HTMLDivElement>();
  const reversedEvents = useMemo(() => events.slice().reverse(), [events]);

  return (
    <TabsContent
      ref={eventsScrollRef}
      value="events"
      className="min-h-[116px] min-w-0 flex-1 overflow-auto pt-3"
    >
      {events.length === 0 ? (
        <p className="text-xs text-muted-foreground">No events recorded.</p>
      ) : (
        <ol className="m-0 grid list-none gap-1.5 p-0">
          {reversedEvents.map((line) => (
            <li
              className="grid min-w-0 grid-cols-[72px_124px_minmax(0,1fr)] items-baseline gap-1.5 text-xs text-text-secondary @max-[360px]/utility:grid-cols-[72px_96px_minmax(0,1fr)] @max-[360px]/utility:gap-1"
              key={line.seq}
            >
              <time
                className="font-mono tabular-nums text-text-tertiary"
                dateTime={new Date(line.timestamp).toISOString()}
              >
                {formatEventTime(line.timestamp)}
              </time>
              <strong className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[11px] font-semibold text-text-secondary">
                {line.name}
              </strong>
              <em className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap not-italic">
                {line.detail}
              </em>
            </li>
          ))}
        </ol>
      )}
    </TabsContent>
  );
}
