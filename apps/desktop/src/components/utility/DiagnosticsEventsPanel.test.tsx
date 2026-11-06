import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { Tab, TabList, Tabs } from 'react-aria-components';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { DiagnosticsEventsPanel } from './DiagnosticsEventsPanel';

afterEach(cleanup);

describe('DiagnosticsEventsPanel', () => {
  it('presents newest-first events as a labelled tab panel with machine-readable timestamps', () => {
    const store = createDesktopStore(createMockBridge());
    const olderTimestamp = Date.UTC(2026, 9, 29, 9, 0, 0);
    const newerTimestamp = Date.UTC(2026, 9, 29, 9, 1, 0);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        events: [
          { seq: 1, name: 'older', detail: 'older detail', timestamp: olderTimestamp },
          { seq: 2, name: 'newer', detail: 'newer detail', timestamp: newerTimestamp },
        ],
      },
    });

    render(
      <Tabs defaultSelectedKey="events">
        <TabList aria-label="Diagnostics views">
          <Tab id="events">Events</Tab>
        </TabList>
        <DiagnosticsEventsPanel useStore={store} />
      </Tabs>,
    );

    const panel = screen.getByRole('tabpanel');
    const rows = within(panel).getAllByRole('listitem');
    expect(rows.map((row) => row.querySelector('strong')?.textContent)).toEqual(['newer', 'older']);
    expect(within(rows[0]!).getByText('newer detail')).toBeInTheDocument();
    expect(rows[0]?.querySelector('time')).toHaveAttribute(
      'datetime',
      new Date(newerTimestamp).toISOString(),
    );
  });
});
