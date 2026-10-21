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
    expect(screen.getByText('Playing', { selector: '.player-track-status' })).toBeInTheDocument();
    expect(document.querySelectorAll('.player-track-status')).toHaveLength(1);
    expect(document.querySelector('.player-identity-label')).toBeNull();
    expect(document.querySelector('.player-dry-run-label')).toBeNull();
    expect(screen.queryByText('Now Playing')).not.toBeInTheDocument();
    expect(screen.getByRole('group', { name: `Playback for ${songA.title}` })).toBeInTheDocument();
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

    const { container } = render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText(row.title)).toBeInTheDocument();
    expect(screen.getByText('Ready to play')).toBeInTheDocument();
    expect(container.querySelectorAll('.player-track-status')).toHaveLength(1);
    expect(container.querySelector('.player-identity-label')).toBeNull();
    expect(screen.getByRole('button', { name: 'Add to Liked Songs' })).toBeInTheDocument();
  });

  it('keeps one authoritative status in the track line when an error is shown separately', async () => {
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
    expect(
      screen.getByText('Ready to play', { selector: '.player-track-status' }),
    ).toBeInTheDocument();
    expect(document.querySelectorAll('.player-track-status')).toHaveLength(1);
    expect(screen.queryByText(/TXT ·/)).not.toBeInTheDocument();
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
    expect(
      screen.getByText('Playback status uncertain', { selector: '.player-track-status' }),
    ).toBeInTheDocument();
    expect(document.querySelectorAll('.player-track-status')).toHaveLength(1);
    expect(screen.queryByText('Session song')).not.toBeInTheDocument();
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
      expect(dock.getByText(dockStatus, { selector: '.player-track-status' })).toBeInTheDocument();
      expect(dockElement.querySelectorAll('.player-track-status')).toHaveLength(1);
      expect(dock.queryByText('Session song')).not.toBeInTheDocument();
      expect(dock.queryByText('Now Playing')).not.toBeInTheDocument();
      expect(dock.queryByText('Playing')).not.toBeInTheDocument();
      expect(runtime.getByText(`Playback status: ${runtimeStatus}`)).toBeInTheDocument();
      expect(runtime.getByText(`Last confirmed session song: ${row.title}`)).toBeInTheDocument();
    },
  );

  it('shows a library prompt when no song identity is known', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());

    const { container } = render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText('Select a song from your Library')).toBeInTheDocument();
    expect(screen.getByText('No song selected')).toBeInTheDocument();
    expect(container.querySelectorAll('.player-track-status')).toHaveLength(1);
  });

  it.each([
    ['starting', 'Starting…'],
    ['playing', 'Playing'],
    ['paused', 'Paused'],
    ['stopping', 'Stopping…'],
  ] as const)('shows one native %s status', async (state, label) => {
    const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    act(() =>
      store.setState({
        playback: { ...store.getState().playback, state },
      }),
    );

    const { container } = render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText(row.title)).toBeInTheDocument();
    expect(screen.getByText(label, { selector: '.player-track-status' })).toBeInTheDocument();
    expect(container.querySelectorAll('.player-track-status')).toHaveLength(1);
    expect(screen.queryByText('Now Playing')).not.toBeInTheDocument();
  });

  it('keeps dry-run playback and the no-input explanation in one accessible status', async () => {
    const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback({ dry_run: true }));
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    const { container } = render(<PlayerTrackInfo useStore={store} />);
    const status = container.querySelector<HTMLElement>('.player-track-status');
    expect(status).not.toBeNull();
    expect(status).toHaveTextContent('Dry run · Playing');
    expect(status).toHaveAttribute(
      'aria-label',
      `${row.title}: Dry run · Playing. No physical input is sent.`,
    );
    expect(container.querySelectorAll('.player-track-status')).toHaveLength(1);
  });

  it('keeps dry-run truth when native status confirmation is uncertain', async () => {
    const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback({ dry_run: true }));
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Mock IPC disconnected.',
        },
      }),
    );

    const { container } = render(<PlayerTrackInfo useStore={store} />);
    const status = container.querySelector<HTMLElement>('.player-track-status');
    expect(status).toHaveTextContent('Dry run · Playback status uncertain');
    expect(status).toHaveAttribute(
      'aria-label',
      expect.stringContaining('No physical input is sent.'),
    );
    expect(container.querySelectorAll('.player-track-status')).toHaveLength(1);
  });

  it('labels a prepared song and preserves last-played meaning after Stop', async () => {
    const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    const prepared = render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText('Prepared', { selector: '.player-track-status' })).toBeInTheDocument();
    expect(prepared.container.querySelectorAll('.player-track-status')).toHaveLength(1);
    prepared.unmount();

    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await act(async () => store.getState().stopPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('idle'));
    const { container } = render(<PlayerTrackInfo useStore={store} />);
    expect(screen.getByText(row.title)).toBeInTheDocument();
    expect(
      screen.getByText('Last played', { selector: '.player-track-status' }),
    ).toBeInTheDocument();
    expect(container.querySelectorAll('.player-track-status')).toHaveLength(1);
  });
});
