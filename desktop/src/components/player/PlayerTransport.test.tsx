import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
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
    dryRun: false,
  };
  const sessionId = 'c'.repeat(32);
  const setPlayback = (patch: Partial<ReturnType<typeof store.getState>['playback']>) => {
    store.setState({ playback: { ...store.getState().playback, ...patch } });
  };
  const view = render(<PlayerTransport useStore={store} />);
  return { store, currentSong, context, sessionId, setPlayback, ...view };
}

describe('PlayerTransport', () => {
  afterEach(cleanup);

  it('does not expose Pause during starting and keeps Stop tied to a real session', async () => {
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
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });

  it('maps playing to Pause, paused to Resume, and keeps Stop discoverable', async () => {
    const { setPlayback, sessionId } = await setupTransport();
    act(() => setPlayback({ state: 'playing', sessionId }));
    expect(screen.getByRole('button', { name: 'Pause' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();

    act(() => setPlayback({ state: 'paused' }));
    expect(screen.getByRole('button', { name: 'Resume' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });

  it('disables Previous, Next, and Stop while another transport operation owns the controls', async () => {
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
    expect(screen.getByRole('button', { name: 'Stop' })).toBeDisabled();
  });

  it('exposes the familiar Previous and Next names and disables Next at the context end', async () => {
    const { store, setPlayback, currentSong, context } = await setupTransport();
    act(() => setPlayback({ currentSong, context, state: 'idle' }));

    expect(screen.getByRole('button', { name: 'Previous' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Next' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Play' })).toBeEnabled();

    act(() =>
      setPlayback({
        context: { ...context, total: 2 },
      }),
    );
    expect(screen.getByRole('button', { name: 'Next' })).toBeEnabled();
    expect(store.getState().playback.context?.currentIndex).toBe(0);
  });
});
