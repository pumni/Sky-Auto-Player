import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { TrackTable } from './TrackTable';

describe('TrackTable built-in page semantics', () => {
  afterEach(cleanup);

  it('uses local grid row indices and global song ordinals on a short final page', async () => {
    const rows = Array.from({ length: 238 }, (_, index) => ({
      song_id: (index + 1).toString(16).padStart(32, '0'),
      title: `Track fixture ${String(index + 1).padStart(3, '0')}`,
      format_label: 'TXT',
      duration_us: 1_000_000,
      note_count: 1,
      risk_level: 'low' as const,
      metadata_state: 'ready' as const,
      liked: false,
    }));
    const store = createDesktopStore(
      createMockBridge({
        catalogRows: rows,
        builtinSongIds: rows.map((row) => row.song_id),
      }),
    );
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    await act(async () => store.getState().navigateBuiltinPage(3));
    render(<TrackTable useStore={store} />);

    const grid = screen.getByRole('grid', { name: 'Built-in Songs, page 3' });
    expect(grid).toHaveAttribute('aria-rowcount', '39');
    const firstRow = screen.getByRole('row', { name: /Track fixture 201/ });
    expect(firstRow).toHaveAttribute('aria-rowindex', '2');
    expect(within(firstRow).getByRole('gridcell', { name: '201' })).toBeInTheDocument();

    fireEvent.keyDown(grid, { key: 'End' });
    const lastRow = await screen.findByRole('row', { name: /Track fixture 238/ });
    await waitFor(() => expect(lastRow).toHaveAttribute('aria-rowindex', '39'));
    expect(within(lastRow).getByRole('gridcell', { name: '238' })).toBeInTheDocument();
    expect(store.getState().library.selectedSongId).toBe(rows[237]?.song_id);
  });
});
