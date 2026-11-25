import { useMemo } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { TabsContent } from '../ui/tabs';
import {
  ScrollAreaContent,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';
import { formatEventTime } from './diagnosticsPresentation';

export function DiagnosticsEventsPanel({ useStore }: { useStore: DesktopStoreHook }) {
  const events = useStore((store) => store.diagnostics.events);
  const reversedEvents = useMemo(() => events.slice().reverse(), [events]);

  return (
    <TabsContent value="events" className="flex min-h-[116px] min-w-0 flex-1 flex-col">
      <ScrollAreaRoot className="min-h-0 min-w-0 flex-1">
        <ScrollAreaViewport className="size-full" role="region" aria-label="Diagnostics events">
          <ScrollAreaContent className="min-w-0 pt-3 pb-4">
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
          </ScrollAreaContent>
        </ScrollAreaViewport>
        <ScrollAreaScrollbar orientation="vertical">
          <ScrollAreaThumb />
        </ScrollAreaScrollbar>
      </ScrollAreaRoot>
    </TabsContent>
  );
}
