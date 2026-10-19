import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { Profiler } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import type { SongRow } from '../../bridge/DesktopBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
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
    const onMenuAction = vi.fn();
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
        actions={
          <button type="button" aria-label="More actions" onClick={onMenuAction}>
            More
          </button>
        }
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
    fireEvent.click(screen.getByRole('button', { name: 'More actions' }));
    expect(onMenuAction).toHaveBeenCalledOnce();
    expect(onSelect).not.toHaveBeenCalled();
    fireEvent.click(renderedRow);
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('renders Now Playing independently alongside selection without changing row geometry', () => {
    render(<TrackRow row={row} index={6} selected playing start={276} onSelect={vi.fn()} />);

    const renderedRow = screen.getByRole('row', { name: /Liminal Garden/ });
    expect(renderedRow).toHaveAttribute('aria-selected', 'true');
    expect(renderedRow).toHaveAttribute('aria-current', 'true');
    expect(renderedRow).toHaveClass('is-selected', 'is-playing');
    expect(renderedRow.style.transform).toBe('translateY(276px)');
    expect(renderedRow.querySelector('.track-cell-index svg')).toHaveAttribute(
      'aria-hidden',
      'true',
    );
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

  it('returns focus to the grid after a row click and supports the next arrow selection', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);

    render(<TrackTable useStore={store} />);
    const grid = screen.getByRole('grid', { name: 'Songs' });
    const firstRow = screen.getByRole('row', { name: /Aurora Landing/ });
    const second = selectRowAtIndex(store.getState().library, 1);
    if (!first || !second) throw new Error('mock library has fewer than two rows');

    await act(async () => fireEvent.click(firstRow));
    expect(grid).toHaveFocus();
    await waitFor(() => expect(store.getState().library.selectedSongId).toBe(first.song_id));

    await act(async () => fireEvent.keyDown(grid, { key: 'ArrowDown' }));
    await waitFor(() => expect(store.getState().library.selectedSongId).toBe(second.song_id));
    await waitFor(() =>
      expect(grid).toHaveAttribute('aria-activedescendant', `song-row-${second.song_id}`),
    );
  });

  it('omits the active descendant while its selected virtual row is not mounted', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const selected = selectRowAtIndex(store.getState().library, 0);
    if (!selected) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(selected.song_id));
    const loadedLibrary = store.getState().library;

    render(<TrackTable useStore={store} />);
    const grid = screen.getByRole('grid', { name: 'Songs' });
    await waitFor(() =>
      expect(grid).toHaveAttribute('aria-activedescendant', `song-row-${selected.song_id}`),
    );

    await act(async () => {
      store.setState({
        library: { ...store.getState().library, pages: new Map(), indexById: new Map() },
      });
    });
    await waitFor(() => expect(grid).not.toHaveAttribute('aria-activedescendant'));

    await act(async () => store.setState({ library: loadedLibrary }));
    await waitFor(() =>
      expect(grid).toHaveAttribute('aria-activedescendant', `song-row-${selected.song_id}`),
    );
  });

  it('does not complete keyboard navigation against a newer search result set', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    let releaseOldPage!: () => void;
    let signalOldPageRequested!: () => void;
    let signalOldPageReturned!: () => void;
    const oldPageRequested = new Promise<void>((resolve) => {
      signalOldPageRequested = resolve;
    });
    const oldPageReturned = new Promise<void>((resolve) => {
      signalOldPageReturned = resolve;
    });
    const oldPageRelease = new Promise<void>((resolve) => {
      releaseOldPage = resolve;
    });
    bridge.searchSongs = async (request) => {
      if (request.query === '' && request.offset === 400) {
        signalOldPageRequested();
        await oldPageRelease;
        const result = await originalSearch(request);
        signalOldPageReturned();
        return result;
      }
      return originalSearch(request.query === 'filtered' ? { ...request, query: '' } : request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    render(<TrackTable useStore={store} />);
    const grid = screen.getByRole('grid', { name: 'Songs' });
    await act(async () => fireEvent.keyDown(grid, { key: 'End' }));
    await oldPageRequested;

    await act(async () => store.getState().search('filtered'));
    await act(async () => store.getState().setViewport(400, 499));
    expect(store.getState().library.pages.get(400)?.length).toBe(100);

    await act(async () => {
      releaseOldPage();
      await oldPageReturned;
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(store.getState().library.query).toBe('filtered');
    expect(store.getState().library.selectedSongId).toBeNull();
  });

  it('anchors rapid ArrowDown navigation to the pending virtual row', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    let releasePage!: () => void;
    let signalPageRequested!: () => void;
    const pageRelease = new Promise<void>((resolve) => {
      releasePage = resolve;
    });
    const pageRequested = new Promise<void>((resolve) => {
      signalPageRequested = resolve;
    });
    bridge.searchSongs = async (request) => {
      if (request.offset === 200) {
        signalPageRequested();
        await pageRelease;
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const selected = selectRowAtIndex(store.getState().library, 199);
    if (!selected) throw new Error('mock library page does not include index 199');
    await act(async () => store.getState().selectSong(selected.song_id));

    render(<TrackTable useStore={store} />);
    const grid = screen.getByRole('grid', { name: 'Songs' });
    await act(async () => fireEvent.keyDown(grid, { key: 'ArrowDown' }));
    await pageRequested;
    await act(async () => fireEvent.keyDown(grid, { key: 'ArrowDown' }));

    await act(async () => releasePage());
    await waitFor(() => expect(selectRowAtIndex(store.getState().library, 201)).not.toBeNull());
    const expected = selectRowAtIndex(store.getState().library, 201);
    if (!expected) throw new Error('mock library page does not include index 201');
    await waitFor(() => expect(store.getState().library.selectedSongId).toBe(expected.song_id));
    await waitFor(() =>
      expect(grid).toHaveAttribute('aria-activedescendant', `song-row-${expected.song_id}`),
    );
  });

  it('uses the pending End destination for a rapid ArrowUp before page hydration', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    let releasePage!: () => void;
    let signalPageRequested!: () => void;
    const pageRelease = new Promise<void>((resolve) => {
      releasePage = resolve;
    });
    const pageRequested = new Promise<void>((resolve) => {
      signalPageRequested = resolve;
    });
    bridge.searchSongs = async (request) => {
      if (request.offset === 400) {
        signalPageRequested();
        await pageRelease;
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    render(<TrackTable useStore={store} />);
    const grid = screen.getByRole('grid', { name: 'Songs' });
    await act(async () => fireEvent.keyDown(grid, { key: 'End' }));
    await pageRequested;
    await act(async () => fireEvent.keyDown(grid, { key: 'ArrowUp' }));

    await act(async () => releasePage());
    await waitFor(() => expect(selectRowAtIndex(store.getState().library, 498)).not.toBeNull());
    const expected = selectRowAtIndex(store.getState().library, 498);
    if (!expected) throw new Error('mock library page does not include index 498');
    await waitFor(() => expect(store.getState().library.selectedSongId).toBe(expected.song_id));
    await waitFor(() =>
      expect(grid).toHaveAttribute('aria-activedescendant', `song-row-${expected.song_id}`),
    );
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

  it('clears stale selection on search failure and recovers on a later query', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      if (request.query === 'broken') throw new Error('catalog search failed');
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<TrackBrowser useStore={store} />);
    await act(async () => store.getState().search('broken'));
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(screen.getByRole('alert')).toHaveTextContent('Library unavailable');
    expect(screen.getByRole('alert')).toHaveTextContent('catalog search failed');

    await act(async () => store.getState().search('Moonlit'));
    expect(screen.getByRole('row', { name: /Moonlit Village/ })).toBeInTheDocument();
    expect(store.getState().library.error).toBeNull();
    expect(store.getState().library.selectedSongId).toBeNull();
  });

  it('renders a real browse CTA for an empty playlist', () => {
    const store = createDesktopStore(createMockBridge());
    const playlistId = 'p'.repeat(32);
    store.setState({
      library: {
        ...store.getState().library,
        source: { kind: 'playlist', id: playlistId },
        pages: new Map(),
        resultTotal: 0,
        loading: false,
      },
      libraryNavigation: {
        ...store.getState().libraryNavigation,
        playlistsById: new Map([[playlistId, { id: playlistId, name: 'Practice', song_count: 0 }]]),
      },
    });

    render(<TrackBrowser useStore={store} />);

    expect(screen.getByRole('heading', { name: 'Practice' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Browse All Songs…' })).toBeInTheDocument();
  });
});
