import { TabPanel } from 'react-aria-components';
import { useMemo } from 'react';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import type { DesktopStoreHook } from '../../state/store';
import { formatEventTime } from './diagnosticsPresentation';

export function DiagnosticsEventsPanel({ useStore }: { useStore: DesktopStoreHook }) {
  const events = useStore((store) => store.diagnostics.events);
  const eventsScrollRef = useScrollVisibility<HTMLDivElement>();
  const reversedEvents = useMemo(() => events.slice().reverse(), [events]);

  return (
    <TabPanel
      ref={eventsScrollRef}
      id="events"
      className="diagnostics-panel diagnostics-scroll scroll-surface"
    >
      {events.length === 0 ? (
        <p className="muted">No events recorded.</p>
      ) : (
        <ol className="diagnostics-lines">
          {reversedEvents.map((line) => (
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
  );
}
