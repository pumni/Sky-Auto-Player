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

  it('shows a fingerprint-matched advisory for every Prepare variant', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    const { container } = render(<PlaybackAdmission useStore={store} />);
    expect(
      screen.getByText(/informational and does not change authored timing/),
    ).toBeInTheDocument();
    expect(screen.getAllByLabelText('Offline whole-stream timing advisory')).toHaveLength(3);
    expect(
      container.querySelector('[data-plan-fingerprint="mock-plan"]')?.getAttribute('data-status'),
    ).toBe('modeled_risk');
    expect(container.querySelector('[data-plan-fingerprint="mock-plan"]')).toHaveTextContent(
      'Maximum displacement: H 24 µs · G 8 µs · HOL 16 µs',
    );
    expect(container.querySelector('[data-plan-fingerprint="mock-plan"]')).toHaveTextContent(
      'Bounds: 50000 packets, 3000000 key-mask visits, and an estimated 64 MiB working set.',
    );
    expect(
      container
        .querySelector('[data-plan-fingerprint="mock-recommended-plan"]')
        ?.getAttribute('data-status'),
    ).toBe('no_modeled_risk');
    expect(
      container.querySelector('[data-plan-fingerprint="mock-dry-run-plan"]')?.textContent,
    ).toContain('hypothetical for this dry-run');
    expect(screen.getAllByText(/Sky does not acknowledge these estimates/)).toHaveLength(3);
  });

  it('shows ready-plan modeled pressure without disabling playback', async () => {
    const store = createDesktopStore(createMockBridge({ readyPlanAdvisoryStatus: 'modeled_risk' }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 1);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    expect(store.getState().playback.prepared?.admission).toBe('ready');
    render(
      <>
        <PlayerTransport useStore={store} />
        <PlaybackAdmission useStore={store} />
      </>,
    );
    expect(
      screen
        .getByLabelText('Prepared playback timing advisory')
        .querySelector('[data-plan-fingerprint="mock-plan"]'),
    ).toHaveAttribute('data-status', 'modeled_risk');
    expect(screen.getByRole('button', { name: 'Play' })).toBeEnabled();
  });

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

  it.each([
    [
      'target_not_found',
      'Sky window was not found',
      'Open Sky and make sure its window is visible, then try again.',
    ],
    [
      'target_integrity_mismatch',
      'Sky is running with different permissions',
      'Run both applications at the same permission level.',
    ],
    ['target_focus_failed', 'Sky could not be focused', 'Focus the Sky window and try again.'],
  ])(
    'keeps %s actionable without showing a misleading Stop action',
    async (code, title, message) => {
      const store = createDesktopStore(createMockBridge({ startFailure: { code, message } }));
      await act(async () => store.getState().initialize());
      const row = selectRowAtIndex(store.getState().library, 1);
      if (!row) throw new Error('mock Library is empty');
      await act(async () => store.getState().selectSong(row.song_id));
      await act(async () => store.getState().prepareSelectedPlayback());
      await act(async () => store.getState().startPreparedPlayback());

      render(<PlaybackAdmission useStore={store} />);
      expect(screen.getByRole('alert')).toHaveTextContent(title);
      expect(screen.getByText(message)).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
      expect(screen.queryByRole('button', { name: 'Stop playback' })).not.toBeInTheDocument();
    },
  );
});
