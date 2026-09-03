import { act, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import type { SettingsPatch } from '../bridge/DesktopBridge';
import { createDesktopStore } from './store';

describe('desktop store', () => {
  it('boots, searches, selects a song, and applies a catalog event', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    expect(store.getState().bootstrapState).toBe('ready');
    expect(store.getState().library.rows.length).toBeGreaterThan(0);
    expect(store.getState().library.catalogTotal).toBe(500);

    const first = store.getState().library.rows[0];
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    expect(store.getState().detail.value?.song_id).toBe(first.song_id);

    await act(async () => store.getState().reloadLibrary());
    expect(store.getState().library.generation).toBe(2);
    expect(store.getState().library.rows).toHaveLength(500);

    await act(async () => store.getState().patchSettings({ theme: 'slate', verboseHud: true }));
    expect(store.getState().settings?.theme).toBe('slate');
    expect(store.getState().settings?.verbose_hud).toBe(true);
  });

  it('discards an older search response', async () => {
    let releaseSlow: (() => void) | undefined;
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      if (request.query === 'slow') {
        await new Promise<void>((resolve) => {
          releaseSlow = resolve;
        });
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const slow = store.getState().search('slow');
    const fast = store.getState().search('Aurora');
    await fast;
    releaseSlow?.();
    await slow;
    expect(store.getState().library.query).toBe('Aurora');
    expect(store.getState().library.rows[0]?.title).toBe('Aurora Landing');
  });

  it('loads and selects a song beyond the native page limit', async () => {
    const bridge = createMockBridge();
    const requestedOffsets: number[] = [];
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      requestedOffsets.push(request.offset);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setViewport(390, 410));
    await waitFor(() => expect(store.getState().library.rows[400]?.title).toBe('Song 401'));

    await act(async () => store.getState().selectSong(store.getState().library.rows[400]!.song_id));
    expect(store.getState().detail.value?.title).toBe('Song 401');
    expect(requestedOffsets).toContain(200);
    expect(requestedOffsets).toContain(400);
  });

  it('clears selected detail and rejects an older detail response after catalog.changed', async () => {
    let releaseDetail: (() => void) | undefined;
    const bridge = createMockBridge();
    const originalDetail = bridge.getSongDetail;
    bridge.getSongDetail = async (request) => {
      await new Promise<void>((resolve) => {
        releaseDetail = resolve;
      });
      return originalDetail(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = store.getState().library.rows[0];
    if (!first) throw new Error('mock library is empty');

    const detail = store.getState().selectSong(first.song_id);
    await act(async () => store.getState().reloadLibrary());
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(store.getState().detail.value).toBeNull();
    releaseDetail?.();
    await detail;
    expect(store.getState().detail.value).toBeNull();
  });

  it('hydrates filtered-result song IDs without confusing them with catalog indices', async () => {
    const bridge = createMockBridge();
    let viewportCalls = 0;
    let viewportSongIds: string[] = [];
    const originalViewport = bridge.setLibraryViewport;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      viewportSongIds = request.songIds;
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().search('Song'));
    await act(async () => store.getState().setViewport(200, 220));
    expect(viewportCalls).toBe(1);
    expect(viewportSongIds).toHaveLength(21);
    expect(store.getState().library.resultTotal).toBe(488);
    expect(store.getState().library.catalogTotal).toBe(500);
    expect(store.getState().library.rows[200]?.title).toBe('Song 212');
  });

  it('persists liked source state through the native bridge contract', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const first = store.getState().library.rows[0];
    if (!first) throw new Error('mock library is empty');

    await act(async () => store.getState().setSongLiked(first.song_id, true));
    expect(store.getState().library.likedTotal).toBe(1);
    expect(store.getState().library.rows[0]?.liked).toBe(true);

    await act(async () => store.getState().selectLibrarySource('liked'));
    expect(store.getState().library.source).toBe('liked');
    expect(store.getState().library.resultTotal).toBe(1);
    expect(store.getState().library.rows[0]?.song_id).toBe(first.song_id);
  });

  it('serializes settings mutations in user-intent order and preserves fields', async () => {
    const bridge = createMockBridge();
    const originalPatch = bridge.patchSettings;
    const calls: SettingsPatch[] = [];
    let releaseFirst: (() => void) | undefined;
    bridge.patchSettings = async (patch) => {
      calls.push(patch);
      if (calls.length === 1) {
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return originalPatch(patch);
    };
    const store = createDesktopStore(bridge);

    const first = store.getState().patchSettings({ theme: 'slate' });
    const second = store.getState().patchSettings({ verboseHud: true });
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual({ theme: 'slate' });

    releaseFirst?.();
    await Promise.all([first, second]);
    expect(calls).toEqual([{ theme: 'slate' }, { verboseHud: true }]);
    expect(store.getState().settings?.theme).toBe('slate');
    expect(store.getState().settings?.verbose_hud).toBe(true);
  });

  it('detaches a prepared plan when the selected song changes', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const [songA, songB] = store.getState().library.rows;
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    expect(store.getState().playback.prepared?.song.song_id).toBe(songA.song_id);

    await act(async () => store.getState().selectSong(songB.song_id));
    expect(store.getState().playback.prepared).toBeNull();
  });

  it('discards an in-flight prepare after selection changes', async () => {
    let releasePrepare: (() => void) | undefined;
    const bridge = createMockBridge();
    const originalPrepare = bridge.preparePlayback;
    bridge.preparePlayback = async (request) => {
      const result = originalPrepare(request);
      await new Promise<void>((resolve) => {
        releasePrepare = resolve;
      });
      return result;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const [songA, songB] = store.getState().library.rows;
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    const pending = store.getState().prepareSelectedPlayback();
    await act(async () => store.getState().selectSong(songB.song_id));
    releasePrepare?.();
    await pending;
    expect(store.getState().playback.prepared).toBeNull();
  });

  it('keeps the prepared/active song title while browsing another song', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const [songA, songB] = store.getState().library.rows;
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await act(async () => store.getState().selectSong(songB.song_id));
    expect(store.getState().playback.songTitle).toBe(songA.title);
  });

  it('binds a short session whose events arrive before start resolves', async () => {
    const bridge = createMockBridge();
    let listener: ((event: import('../bridge/DesktopBridge').UiEvent) => void) | undefined;
    const originalSubscribe = bridge.subscribeUiEvents;
    bridge.subscribeUiEvents = async (next) => {
      listener = next;
      return originalSubscribe(next);
    };
    const sessionId = 'e'.repeat(32);
    bridge.startPlayback = async (request) => {
      const songId = request.preparedId.replace('prepared-', '');
      const started = {
        session_id: sessionId,
        prepared_id: request.preparedId,
        song_id: songId,
        state: 'starting' as const,
        config: { hold_frames: 2, tempo_scale: 1, fps: 60, dry_run: true },
        plan_fingerprint: 'mock-plan',
      };
      listener?.({
        v: 1,
        name: 'playback.state_changed',
        payload: {
          session_id: sessionId,
          song_id: songId,
          state: 'starting',
          physical: false,
          message: null,
          outcome: null,
        },
      });
      listener?.({
        v: 1,
        name: 'playback.finished',
        payload: {
          session_id: sessionId,
          song_id: songId,
          outcome: 'finished',
          total_us: 0,
          message: 'finished',
        },
      });
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
      return started;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = store.getState().library.rows[0];
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());

    expect(store.getState().playback.sessionId).toBe(sessionId);
    expect(store.getState().playback.state).toBe('finished');
  });

  it('rejects late events from a retired session after browsing another song', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const [songA, songB] = store.getState().library.rows;
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');

    store.getState().applyEvent({
      v: 1,
      name: 'playback.finished',
      payload: {
        session_id: sessionId,
        song_id: songA.song_id,
        outcome: 'finished',
        total_us: 0,
        message: 'finished',
      },
    });
    await act(async () => store.getState().selectSong(songB.song_id));
    expect(store.getState().playback.songTitle).toBeNull();

    store.getState().applyEvent({
      v: 1,
      name: 'playback.state_changed',
      payload: {
        session_id: sessionId,
        song_id: songA.song_id,
        state: 'playing',
        physical: false,
        message: null,
        outcome: null,
      },
    });
    expect(store.getState().playback.state).toBe('finished');
    expect(store.getState().playback.songTitle).toBeNull();
  });

  it('keeps diagnostics samples, events, and logs bounded', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setDiagnosticsEnabled(true));

    for (let index = 0; index < 601; index += 1) {
      store.getState().applyEvent({
        v: 1,
        name: 'diagnostics.snapshot',
        payload: {
          seq: index,
          max_lateness_us: index,
          p50_ms: 0.1,
          p95_ms: 0.2,
          sigma_onset_ms: 0.1,
          late_2ms: 0,
          late_5ms: 0,
          late_10ms: 0,
          active_keys: 0,
          stuck_keys: 0,
          keys_dropped: 0,
          chord_split_events: 0,
          backend_status: 'healthy',
          release_max_us: null,
          release_late_2ms: null,
          session_id: null,
        },
      });
    }
    for (let index = 0; index < 501; index += 1) {
      store.getState().applyEvent({
        v: 1,
        name: 'playback.state_changed',
        payload: {
          session_id: `${index.toString(16).padStart(31, '0')}a`,
          song_id: 'b'.repeat(32),
          state: 'playing',
          physical: false,
          message: null,
          outcome: null,
        },
      });
    }

    expect(store.getState().diagnostics.samples).toHaveLength(600);
    expect(store.getState().diagnostics.events).toHaveLength(500);
    expect(store.getState().diagnostics.logs).toHaveLength(200);
  });

  it('keeps utility presentation state separate from diagnostics data', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());

    store.getState().openUtility('diagnostics');
    expect(store.getState().utility).toEqual({ open: true, activeView: 'diagnostics' });
    await waitFor(() => expect(store.getState().diagnostics.enabled).toBe(true));

    store.getState().setUtilityView('details');
    expect(store.getState().utility).toEqual({ open: true, activeView: 'details' });
    await waitFor(() => expect(store.getState().diagnostics.enabled).toBe(false));
    store.getState().closeUtility();
    expect(store.getState().utility.open).toBe(false);
  });

  it('bounds diagnostic event text by UTF-8 bytes', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const message = '🙂'.repeat(2_000);

    store.getState().applyEvent({
      v: 1,
      name: 'core.fatal',
      payload: { code: 'test', message },
    });

    const log = store.getState().diagnostics.logs.at(-1)?.message;
    const event = store.getState().diagnostics.events.at(-1)?.detail;
    expect(log).toBeDefined();
    expect(event).toBeDefined();
    expect(new TextEncoder().encode(log).length).toBeLessThanOrEqual(4096);
    expect(new TextEncoder().encode(event).length).toBeLessThanOrEqual(4096);
  });

  it('drives calibration through typed progress and terminal events', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().startCalibration('quick'));
    await waitFor(() => expect(store.getState().calibration.state).toBe('succeeded'));
    expect(store.getState().calibration.operationId).toMatch(/^[0-9a-f]{32}$/);
    expect(store.getState().calibration.result?.outcome).toBe('succeeded');
    expect(store.getState().calibration.result?.source).toBe('mock');
  });
});
