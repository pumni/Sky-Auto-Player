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
import { formatEventTime, knownEventLabel } from './diagnosticsPresentation';

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
                {reversedEvents.map((line) => {
                  const readableLabel = knownEventLabel(line.name);
                  return (
                    <li
                      className="grid min-w-0 gap-1.5 border-b border-border/70 pb-2 text-xs text-text-secondary last:border-b-0"
                      key={line.seq}
                    >
                      <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-baseline gap-2">
                        <time
                          className="whitespace-nowrap font-mono tabular-nums text-text-tertiary"
                          dateTime={new Date(line.timestamp).toISOString()}
                        >
                          {formatEventTime(line.timestamp)}
                        </time>
                        <div className="flex min-w-0 flex-wrap items-baseline gap-x-1.5 gap-y-0.5">
                          {readableLabel && (
                            <span className="text-xs font-medium text-foreground">
                              {readableLabel}
                            </span>
                          )}
                          <strong className="min-w-0 break-words font-mono text-[11px] font-semibold leading-4 text-text-tertiary [overflow-wrap:anywhere]">
                            {line.name}
                          </strong>
                        </div>
                      </div>
                      <p className="m-0 min-w-0 break-words leading-[1.45] text-text-secondary [overflow-wrap:anywhere]">
                        {line.detail}
                      </p>
                    </li>
                  );
                })}
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
