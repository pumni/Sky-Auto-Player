import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { selectRowAtIndex, createDesktopStore } from '../../state/store';
import { SongDetailsView } from './SongDetailsView';

describe('SongDetailsView', () => {
  afterEach(() => cleanup());

  it('keeps the detail skeleton and summary stable while a new song loads', async () => {
    const bridge = createMockBridge();
    const originalDetail = bridge.getSongDetail;
    let releaseDetail: (() => void) | undefined;
    bridge.getSongDetail = async (request) => {
      if (request.songId === '1'.repeat(32)) {
        await new Promise<void>((resolve) => {
          releaseDetail = resolve;
        });
      }
      return originalDetail(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    const second = selectRowAtIndex(store.getState().library, 1);
    if (!first || !second) throw new Error('mock library is too small');
    const pendingSecond = {
      ...second,
      metadata_state: 'pending' as const,
      duration_us: null,
      note_count: null,
    };
    const pages = new Map(store.getState().library.pages);
    pages.set(0, [first, pendingSecond, ...(pages.get(0) ?? []).slice(2)]);
    store.setState({ library: { ...store.getState().library, pages } });

    await act(async () => store.getState().selectSong(first.song_id));
    const { container } = render(<SongDetailsView useStore={store} />);
    expect(screen.getByRole('heading', { name: first.title })).toBeInTheDocument();
    const sectionCount = container.querySelectorAll('.utility-section').length;

    const pending = store.getState().selectSong(second.song_id);
    await act(async () => Promise.resolve());

    expect(screen.getByRole('heading', { name: second.title })).toBeInTheDocument();
    expect(screen.queryByText('Medium timing risk')).toBeNull();
    expect(container.querySelectorAll('.utility-section')).toHaveLength(sectionCount);
    expect(container.querySelector('.song-details-heading p')).toHaveTextContent(
      'TXT · … · … notes',
    );

    releaseDetail?.();
    await act(async () => pending);
    expect(container.querySelector('.song-details-heading p')).toHaveTextContent(
      'TXT · … · … notes',
    );
  });
});
