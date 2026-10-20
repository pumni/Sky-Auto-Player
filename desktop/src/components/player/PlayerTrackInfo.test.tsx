import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
import { DiagnosticsView } from '../utility/DiagnosticsView';
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
    expect(screen.getByText('Now Playing')).toBeInTheDocument();
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
    expect(screen.getByText('Selected')).toBeInTheDocument();
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

  it('labels the known session song without claiming playback when native status is uncertain', async () => {
    const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 1);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Mock IPC disconnected.',
        },
      }),
    );

    render(<PlayerTrackInfo useStore={store} />);

    expect(screen.getByText(row.title)).toBeInTheDocument();
    expect(screen.getByText('Session song')).toBeInTheDocument();
    expect(screen.getByText('Playback status uncertain')).toBeInTheDocument();
    expect(screen.queryByText('Now Playing')).not.toBeInTheDocument();
  });

  it.each([
    {
      name: 'a recoverable owned-session error without the status-unavailable prefix',
      error: 'Playback did not confirm the requested operation. Check the session and try again.',
      statusRetryPending: false,
      dockStatus: 'Playback status uncertain',
      runtimeStatus: 'Native playback status uncertain',
    },
    {
      name: 'a native session conflict',
      error: 'A different native playback session is active. Stop it before starting another song.',
      statusRetryPending: false,
      dockStatus: 'Native session conflict',
      runtimeStatus: 'Native playback session conflict',
    },
    {
      name: 'a pending status retry before a conflict status',
      error: 'A different native playback session is active. Stop it before starting another song.',
      statusRetryPending: true,
      dockStatus: 'Playback status uncertain',
      runtimeStatus: 'Native playback status uncertain',
    },
  ])(
    'gives the Player Dock and Runtime the same status priority for $name',
    async ({ error, statusRetryPending, dockStatus, runtimeStatus }) => {
      const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
      await act(async () => store.getState().initialize());
      const row = selectRowAtIndex(store.getState().library, 1);
      if (!row) throw new Error('mock Library is too small');
      await act(async () => store.getState().selectSong(row.song_id));
      await act(async () => store.getState().prepareSelectedPlayback());
      await act(async () => store.getState().startPreparedPlayback('proceed'));
      await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

      act(() =>
        store.setState({
          diagnostics: { ...store.getState().diagnostics, enabled: true },
          playback: {
            ...store.getState().playback,
            error,
            statusRetryPending,
          },
        }),
      );

      const { container } = render(
        <>
          <PlayerTrackInfo useStore={store} />
          <DiagnosticsView useStore={store} />
        </>,
      );
      const dockElement = container.querySelector<HTMLElement>('.player-track-info');
      if (!dockElement) throw new Error('Player Dock identity was not rendered');
      const dock = within(dockElement);
      const runtime = within(screen.getByRole('region', { name: 'Playback identity and status' }));

      expect(dock.getByText(row.title)).toBeInTheDocument();
      expect(dock.getByText('Session song')).toBeInTheDocument();
      expect(dock.getByText(dockStatus)).toBeInTheDocument();
      expect(dock.queryByText('Now Playing')).not.toBeInTheDocument();
      expect(dock.queryByText('Playing')).not.toBeInTheDocument();
      expect(runtime.getByText(`Playback status: ${runtimeStatus}`)).toBeInTheDocument();
      expect(runtime.getByText(`Last confirmed session song: ${row.title}`)).toBeInTheDocument();
    },
  );
});
