import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import type { SongRow } from '../../bridge/DesktopBridge';
import { createDesktopStore } from '../../state/store';
import { TrackBrowser } from './TrackBrowser';
import { TrackRow, formatDuration } from './TrackRow';
import { TrackTable } from './TrackTable';

const row: SongRow = {
  song_id: 'a'.repeat(32),
  title: 'Liminal Garden',
  duration_us: 125_000_000,
  note_count: null,
  risk_level: 'high',
  metadata_state: 'ready',
};

describe('Track Browser primitives', () => {
  afterEach(() => cleanup());

  it('formats duration and exposes note and risk semantics', () => {
    const onSelect = vi.fn();
    expect(formatDuration(null)).toBe('—');
    expect(formatDuration(125_000_000)).toBe('2:05');

    render(
      <TrackRow
        row={row}
        index={6}
        selected={false}
        start={276}
        onFocus={vi.fn()}
        onSelect={onSelect}
      />,
    );

    expect(screen.getByRole('row', { name: /Liminal Garden/ })).toHaveAttribute(
      'aria-selected',
      'false',
    );
    expect(screen.getByRole('gridcell', { name: '—' })).toBeInTheDocument();
    expect(screen.getByRole('gridcell', { name: 'High' })).toBeInTheDocument();
    expect(screen.getByRole('gridcell', { name: '2:05' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('row', { name: /Liminal Garden/ }));
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('renders loading placeholders for unloaded rows', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      library: { ...store.getState().library, rows: [], total: 3, loading: true },
    });

    render(<TrackTable useStore={store} />);

    expect(screen.getAllByText('Loading song…')).toHaveLength(3);
    expect(screen.getAllByRole('row', { name: /Loading song/ })[0]).toHaveAttribute(
      'aria-busy',
      'true',
    );
  });

  it('renders an explicit no-results state', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      library: { ...store.getState().library, rows: [], total: 0, loading: false },
    });

    render(<TrackBrowser useStore={store} />);

    expect(screen.getByText('No songs found')).toBeInTheDocument();
    expect(screen.queryByRole('grid', { name: 'Songs' })).toBeNull();
  });
});
