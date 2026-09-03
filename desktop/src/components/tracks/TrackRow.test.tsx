import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { Profiler } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import type { SongRow } from '../../bridge/DesktopBridge';
import { createDesktopStore } from '../../state/store';
import { TrackBrowser } from './TrackBrowser';
import { TrackRow, VirtualTrackRow, formatDuration } from './TrackRow';
import { TrackTable } from './TrackTable';
import { formatSongCount } from './trackFormatting';

const row: SongRow = {
  song_id: 'a'.repeat(32),
  title: 'Liminal Garden',
  format_label: 'JSON',
  duration_us: 125_000_000,
  note_count: null,
  risk_level: 'high',
  metadata_state: 'ready',
  liked: false,
};

describe('Track Browser primitives', () => {
  afterEach(() => cleanup());

  it('formats duration and exposes note and liked semantics', () => {
    const onSelect = vi.fn();
    const onToggleLiked = vi.fn();
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
        onToggleLiked={onToggleLiked}
      />,
    );

    const renderedRow = screen.getByRole('row', { name: /Liminal Garden/ });
    expect(renderedRow).toHaveAttribute('aria-selected', 'false');
    expect(
      [...renderedRow.querySelectorAll<HTMLElement>('[role="gridcell"]')].map(
        (cell) => cell.className,
      ),
    ).toEqual([
      'track-cell track-cell-index',
      'track-cell track-cell-title',
      'track-cell track-cell-liked',
      'track-cell track-cell-notes',
      'track-cell track-cell-duration',
    ]);
    expect(screen.getByRole('gridcell', { name: '—' })).toBeInTheDocument();
    expect(screen.getByRole('gridcell', { name: '2:05' })).toBeInTheDocument();
    const likeButton = screen.getByRole('button', { name: 'Add Liminal Garden to Liked Songs' });
    expect(likeButton).toHaveAttribute('aria-pressed', 'false');
    fireEvent.click(likeButton);
    expect(onToggleLiked).toHaveBeenCalledOnce();
    expect(onSelect).not.toHaveBeenCalled();
    fireEvent.click(renderedRow);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('formats song counts with singular grammar', () => {
    expect(formatSongCount(0)).toBe('0 songs');
    expect(formatSongCount(1)).toBe('1 song');
    expect(formatSongCount(2)).toBe('2 songs');
  });

  it('does not present pending or failed metadata as final risk data', () => {
    const pending = { ...row, song_id: 'b'.repeat(32), metadata_state: 'pending' as const };
    const failed = { ...row, song_id: 'c'.repeat(32), metadata_state: 'error' as const };

    render(
      <>
        <TrackRow
          row={pending}
          index={0}
          selected={false}
          start={0}
          onFocus={vi.fn()}
          onSelect={vi.fn()}
        />
        <TrackRow
          row={failed}
          index={1}
          selected={false}
          start={46}
          onFocus={vi.fn()}
          onSelect={vi.fn()}
        />
      </>,
    );

    expect(screen.getAllByRole('gridcell', { name: 'Metadata loading' })).toHaveLength(2);
    expect(screen.getAllByRole('gridcell', { name: 'Metadata unavailable' })).toHaveLength(2);
    expect(screen.queryByText('Unknown')).toBeNull();
    expect(document.querySelectorAll('.risk-dot')).toHaveLength(0);
  });

  it('renders loading placeholders for unloaded rows', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      library: { ...store.getState().library, pages: new Map(), resultTotal: 3, loading: true },
    });

    render(<TrackTable useStore={store} />);

    expect(screen.getAllByText('Loading song…')).toHaveLength(3);
    expect(screen.getAllByRole('row', { name: /Loading song/ })[0]).toHaveAttribute(
      'aria-busy',
      'true',
    );
  });

  it('keeps table selection updates outside the TrackTable render path', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    let tableRenders = 0;
    function RenderProbe() {
      tableRenders += 1;
      return <TrackTable useStore={store} />;
    }

    render(<RenderProbe />);
    const initialRenders = tableRenders;
    await act(async () => {
      fireEvent.click(screen.getByRole('row', { name: /Aurora Landing/ }));
    });

    expect(store.getState().library.selectedSongId).toBe('0'.repeat(32));
    expect(tableRenders).toBe(initialRenders);
  });

  it('updates only the previous and next selected virtual rows', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const updates = new Map<number, number>([
      [0, 0],
      [1, 0],
      [2, 0],
    ]);
    const onRender = (id: string, phase: 'mount' | 'update' | 'nested-update') => {
      if (phase !== 'mount') updates.set(Number(id), (updates.get(Number(id)) ?? 0) + 1);
    };

    render(
      <>
        {[0, 1, 2].map((index) => (
          <Profiler key={index} id={String(index)} onRender={onRender}>
            <VirtualTrackRow index={index} start={index * 46} useStore={store} />
          </Profiler>
        ))}
      </>,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole('row', { name: /Aurora Landing/ }));
    });
    updates.forEach((_, index) => updates.set(index, 0));
    await act(async () => {
      fireEvent.click(screen.getByRole('row', { name: /Blue Bird/ }));
    });

    expect(updates.get(0)).toBe(1);
    expect(updates.get(1)).toBe(1);
    expect(updates.get(2)).toBe(0);
  });

  it('renders an explicit no-results state', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      library: { ...store.getState().library, pages: new Map(), resultTotal: 0, loading: false },
    });

    render(<TrackBrowser useStore={store} />);

    expect(screen.getByText('No songs found')).toBeInTheDocument();
    expect(screen.queryByRole('grid', { name: 'Songs' })).toBeNull();
  });
});
