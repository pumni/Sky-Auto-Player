import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
import { PlayerTrackInfo } from './PlayerTrackInfo';

describe('PlayerTrackInfo', () => {
  afterEach(cleanup);

  it('shows and likes the active song while the user browses another Library row', async () => {
    const bridge = createMockBridge({ startDelayMs: 5 });
    const likedRequests: string[] = [];
    const setLiked = bridge.setSongLiked;
    bridge.setSongLiked = async (request) => {
      likedRequests.push(request.songId);
      return setLiked(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const songA = selectRowAtIndex(store.getState().library, 0);
    const songB = selectRowAtIndex(store.getState().library, 1);
    if (!songA || !songB) throw new Error('mock Library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await act(async () => store.getState().selectSong(songB.song_id));

    render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText(songA.title)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add to Liked Songs' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Add to Liked Songs' }));
    await waitFor(() => expect(store.getState().playback.currentSong?.liked).toBe(true));
    expect(likedRequests).toEqual([songA.song_id]);
    expect(store.getState().library.selectedSongId).toBe(songB.song_id);
    expect(selectRowAtIndex(store.getState().library, 1)?.liked).toBe(false);
  });

  it('falls back to the selected song when no Now Playing identity exists', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 1);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));

    render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText(row.title)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Add to Liked Songs' })).toBeInTheDocument();
  });

  it('keeps song metadata in the track line when a separate playback error is shown', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 1);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'target_not_found: Open Sky and try again.',
        },
      }),
    );

    render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText(row.title)).toBeInTheDocument();
    expect(screen.getByText(/TXT ·/)).toBeInTheDocument();
    expect(screen.queryByText('Playback error')).not.toBeInTheDocument();
  });
});
