import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex, type DesktopStore } from '../../state/store';
import { PlayerTransport } from './PlayerTransport';

async function setupTransport() {
  const store = createDesktopStore(createMockBridge());
  await act(async () => store.getState().initialize());
  const row = selectRowAtIndex(store.getState().library, 0);
  if (!row) throw new Error('mock Library is empty');
  const library = store.getState().library;
  const currentSong = {
    songId: row.song_id,
    title: row.title,
    liked: row.liked,
    durationUs: row.duration_us,
    formatLabel: row.format_label,
    noteCount: row.note_count,
    riskLevel: row.risk_level,
    generation: library.generation,
  };
  const context = {
    source: library.searchSource,
    query: library.query,
    generation: library.generation,
    total: 1,
    currentIndex: 0,
    currentSongId: row.song_id,
    shuffleTraversal: null,
    dryRun: false,
    membershipRevision: 0,
    valid: true,
  };
  const sessionId = 'c'.repeat(32);
  const setPlayback = (patch: Partial<ReturnType<typeof store.getState>['playback']>) => {
    store.setState({ playback: { ...store.getState().playback, ...patch } });
  };
  const view = render(<PlayerTransport useStore={store} />);
  return { store, currentSong, context, sessionId, setPlayback, ...view };
}

function expectFiveTransportButtons(primaryName: string) {
  expect(screen.getByRole('button', { name: 'Shuffle' })).toBeVisible();
  expect(screen.getByRole('button', { name: 'Previous' })).toBeVisible();
  expect(screen.getByRole('button', { name: primaryName })).toBeVisible();
  expect(screen.getByRole('button', { name: 'Next' })).toBeVisible();
  expect(screen.getByRole('button', { name: 'Stop' })).toBeVisible();
  expect(document.querySelectorAll('.player-controls-row button')).toHaveLength(5);
}

describe('PlayerTransport', () => {
  afterEach(cleanup);

  it('keeps all five controls visible and allows Stop during starting with a session', async () => {
    const { setPlayback, sessionId } = await setupTransport();
    act(() =>
      setPlayback({
        state: 'starting',
        sessionId,
        transportOperation: 'starting',
      }),
    );

    expect(screen.queryByRole('button', { name: 'Pause' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
    expectFiveTransportButtons('Starting playback');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
    expect(
      screen.getByRole('button', { name: 'Stop' }).querySelector('svg.lucide-square'),
    ).toBeInTheDocument();
  });

  it('shows all five controls while starting before a session id exists', async () => {
    const { setPlayback } = await setupTransport();
    act(() => setPlayback({ startRequestId: 7, transportOperation: 'starting' }));

    expect(screen.getByRole('button', { name: 'Starting playback' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Play' })).not.toBeInTheDocument();
    expectFiveTransportButtons('Starting playback');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });

  it('locks Previous and Next while a risk confirmation owns the prepared song', async () => {
    const { setPlayback, currentSong, context } = await setupTransport();
    act(() =>
      setPlayback({
        currentSong,
        context: { ...context, total: 2 },
        prepared: {
          admission: 'confirmation_required',
          decisions: [
            { decision: 'proceed', label: 'Proceed with current settings' },
            { decision: 'use_recommended', label: 'Use recommended settings' },
            { decision: 'dry_run', label: 'Test playback (no input)' },
          ],
        } as unknown as NonNullable<DesktopStore['playback']['prepared']>,
      }),
    );

    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
    expectFiveTransportButtons('Play');
    expect(screen.getByRole('button', { name: 'Play' })).toBeDisabled();
    expect(screen.getByTestId('player-controls-row')).toBeInTheDocument();
    expect(screen.getByTestId('player-primary-slot')).toBeInTheDocument();
    expect(screen.getByRole('progressbar')).toBeInTheDocument();
  });

  it('maps playing to Pause, paused to Resume, and keeps Stop discoverable', async () => {
    const { setPlayback, sessionId } = await setupTransport();
    act(() => setPlayback({ state: 'playing', sessionId }));
    expectFiveTransportButtons('Pause');
    expect(screen.getByRole('button', { name: 'Pause' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();

    act(() => setPlayback({ state: 'paused' }));
    expectFiveTransportButtons('Resume');
    expect(screen.getByRole('button', { name: 'Resume' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });

  it('keeps Stop available while native retirement is still pending', async () => {
    const { setPlayback, sessionId } = await setupTransport();
    act(() => setPlayback({ state: 'stopping', sessionId, transportOperation: null }));

    expect(screen.getByRole('button', { name: 'Playback transition pending' })).toBeDisabled();
    expectFiveTransportButtons('Playback transition pending');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });

  it('keeps the five-button layout in idle, failed, and stopping-operation states', async () => {
    const { setPlayback, sessionId } = await setupTransport();
    expectFiveTransportButtons('Play');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();

    act(() => setPlayback({ state: 'failed', sessionId: null, error: 'native failure' }));
    expectFiveTransportButtons('Play');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();

    act(() =>
      setPlayback({
        prepared: { admission: 'blocked' } as NonNullable<DesktopStore['playback']['prepared']>,
      }),
    );
    expectFiveTransportButtons('Play');
    expect(screen.getByRole('button', { name: 'Play' })).toBeDisabled();

    act(() => setPlayback({ state: 'stopping', sessionId, transportOperation: 'stopping' }));
    expectFiveTransportButtons('Stopping playback');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });

  it('keeps a disabled primary pending control while playback ownership is uncertain', async () => {
    const { setPlayback } = await setupTransport();
    act(() =>
      setPlayback({ error: 'Playback status is unavailable: bridge timed out', state: 'idle' }),
    );

    expectFiveTransportButtons('Playback transition pending');
    expect(screen.getByRole('button', { name: 'Playback transition pending' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });

  it('locks navigation while an operation is pending but keeps Stop as a safety control', async () => {
    const { setPlayback, sessionId, currentSong, context } = await setupTransport();
    act(() =>
      setPlayback({
        state: 'playing',
        sessionId,
        currentSong,
        context,
        transportOperation: 'advancing',
      }),
    );

    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });

  it('exposes the familiar Previous and Next names and disables Next at the context end', async () => {
    const { store, setPlayback, currentSong, context } = await setupTransport();
    act(() => setPlayback({ currentSong, context, state: 'idle' }));

    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Play' })).toBeEnabled();

    act(() =>
      setPlayback({
        context: { ...context, total: 2 },
      }),
    );
    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Next' })).toBeEnabled();
    expect(store.getState().playback.context?.currentIndex).toBe(0);
  });

  it('enables Previous only when restart or a previous traversal item applies', async () => {
    const { setPlayback, currentSong, context, sessionId } = await setupTransport();
    const snapshot = (currentUs: number) => ({
      session_id: sessionId,
      seq: 1,
      state: 'playing' as const,
      song_id: currentSong.songId,
      title: currentSong.title,
      current_us: currentUs,
      total_us: 10_000_000,
      pre_roll_remaining_us: 0,
      focus_state: 'focused' as const,
      health: 'healthy' as const,
      input_path_degraded: false,
      message: null,
    });
    act(() =>
      setPlayback({
        currentSong,
        context: { ...context, total: 2 },
        state: 'playing',
        sessionId,
        snapshot: snapshot(3_000_000),
      }),
    );
    expect(screen.getByRole('button', { name: 'Previous' })).toBeDisabled();

    act(() => setPlayback({ snapshot: snapshot(3_000_001) }));
    expect(screen.getByRole('button', { name: 'Previous' })).toBeEnabled();

    act(() =>
      setPlayback({
        context: { ...context, currentIndex: 1, total: 2 },
        snapshot: snapshot(3_000_000),
      }),
    );
    expect(screen.getByRole('button', { name: 'Previous' })).toBeEnabled();
  });

  it('toggles Shuffle accessibly without changing the current song', async () => {
    const { store, setPlayback, currentSong, context } = await setupTransport();
    act(() => setPlayback({ currentSong, context, state: 'playing' }));

    const shuffle = screen.getByRole('button', { name: 'Shuffle' });
    expect(shuffle).toHaveAttribute('aria-pressed', 'false');
    act(() => shuffle.click());

    expect(screen.getByRole('button', { name: 'Shuffle' })).toHaveAttribute('aria-pressed', 'true');
    expect(store.getState().playback.shuffleEnabled).toBe(true);
    expect(store.getState().playback.currentSong?.songId).toBe(currentSong.songId);
    expect(store.getState().playback.context?.shuffleTraversal?.originIndex).toBe(0);
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(0);
  });
});
