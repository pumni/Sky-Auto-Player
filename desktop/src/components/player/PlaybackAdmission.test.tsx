import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
import { PlaybackAdmission } from './PlaybackAdmission';
import { PlayerTransport } from './PlayerTransport';

async function setupPlaying(emitSnapshots = true) {
  const bridge = createMockBridge({
    playbackDurationMs: 60_000,
    startDelayMs: 5,
    emitSnapshots,
  });
  const store = createDesktopStore(bridge);
  await act(async () => store.getState().initialize());
  const row = selectRowAtIndex(store.getState().library, 0);
  if (!row) throw new Error('mock Library is empty');
  await act(async () => store.getState().selectSong(row.song_id));
  await act(async () => store.getState().prepareSelectedPlayback());
  await act(async () => store.getState().startPreparedPlayback('proceed'));
  await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  return { bridge, store };
}

describe('PlaybackAdmission recovery actions', () => {
  afterEach(cleanup);

  it('reconciles a recoverable status issue and restores the normal Pause control', async () => {
    const { store } = await setupPlaying();
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Mock playback status query unavailable.',
        },
      }),
    );

    render(
      <>
        <PlayerTransport useStore={store} />
        <PlaybackAdmission useStore={store} />
      </>,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('Playback status is unavailable');
    expect(screen.getByRole('button', { name: 'Retry status' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop playback' })).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Retry status' }));

    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Pause' })).toBeEnabled();
    expect(store.getState().playback.error).toBeNull();
  });

  it('keeps a known session owned after status query failure and offers Stop playback', async () => {
    const { bridge, store } = await setupPlaying(false);
    bridge.getPlaybackStatus = async () => {
      throw new Error('Mock IPC disconnected.');
    };
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Previous status query failed.',
        },
      }),
    );
    render(<PlaybackAdmission useStore={store} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry status' }));
    await waitFor(() => expect(store.getState().playback.statusRetryPending).toBe(false));
    expect(store.getState().playback.sessionId).not.toBeNull();
    expect(store.getState().playback.error).toContain('Mock IPC disconnected');

    fireEvent.click(screen.getByRole('button', { name: 'Stop playback' }));
    await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
  });

  it('offers status retry without Stop or Try again when native ownership is unknown', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          startRequestId: 42,
          transportOperation: null,
          error: 'Playback status is unavailable: Mock IPC disconnected.',
        },
      }),
    );

    render(<PlaybackAdmission useStore={store} />);
    expect(screen.getByRole('button', { name: 'Retry status' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: 'Stop playback' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Retry status' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled());
    expect(store.getState().playback.startRequestId).toBeNull();
    expect(store.getState().playback.error).toContain('No active playback session was created');
  });

  it('keeps a typed target failure actionable without showing a misleading Stop action', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          state: 'failed',
          error: 'target_not_found: Open Sky and make sure its window is visible, then try again.',
        },
      }),
    );

    render(<PlaybackAdmission useStore={store} />);
    expect(screen.getByRole('alert')).toHaveTextContent('Sky window was not found');
    expect(screen.getByText(/Open Sky and make sure its window is visible/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: 'Stop playback' })).not.toBeInTheDocument();
  });
});
