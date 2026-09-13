import { act, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import type { SearchRequest, SettingsPatch } from '../bridge/DesktopBridge';
import { createDesktopStore, selectRowAtIndex, selectSelectedDetail } from './store';

function rowAt(store: ReturnType<typeof createDesktopStore>, index: number) {
  return selectRowAtIndex(store.getState().library, index);
}

describe('desktop store', () => {
  it('boots, searches, selects a song, and applies a catalog event', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    expect(store.getState().bootstrapState).toBe('ready');
    expect(store.getState().library.pages.get(0)?.length).toBeGreaterThan(0);
    expect(store.getState().library.catalogTotal).toBe(500);

    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    expect(selectSelectedDetail(store.getState()).value?.song_id).toBe(first.song_id);

    await act(async () => store.getState().reloadLibrary());
    expect(store.getState().library.generation).toBe(2);
    expect(store.getState().library.resultTotal).toBe(500);

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
    expect(rowAt(store, 0)?.title).toBe('Aurora Landing');
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
    await waitFor(() => expect(rowAt(store, 400)?.title).toBe('Song 401'));

    await act(async () => store.getState().selectSong(rowAt(store, 400)!.song_id));
    expect(selectSelectedDetail(store.getState()).value?.title).toBe('Song 401');
    expect(requestedOffsets).toContain(200);
    expect(requestedOffsets).toContain(400);
  });

  it('reuses deep detail from the bounded song cache', async () => {
    const bridge = createMockBridge();
    const originalDetail = bridge.getSongDetail;
    let detailCalls = 0;
    bridge.getSongDetail = async (request) => {
      detailCalls += 1;
      return originalDetail(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const songA = rowAt(store, 0);
    const songB = rowAt(store, 1);
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().selectSong(songB.song_id));
    await act(async () => store.getState().selectSong(songA.song_id));

    expect(detailCalls).toBe(2);
    expect(selectSelectedDetail(store.getState()).value?.song_id).toBe(songA.song_id);
    expect(store.getState().details.bySongId.size).toBe(2);
  });

  it('does not start another detail request when the selected song is still loading', async () => {
    const bridge = createMockBridge();
    const originalDetail = bridge.getSongDetail;
    let detailCalls = 0;
    let releaseDetail: (() => void) | undefined;
    bridge.getSongDetail = async (request) => {
      detailCalls += 1;
      await new Promise<void>((resolve) => {
        releaseDetail = resolve;
      });
      return originalDetail(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');

    const firstRequest = store.getState().selectSong(first.song_id);
    await act(async () => Promise.resolve());
    await act(async () => store.getState().selectSong(first.song_id));
    expect(detailCalls).toBe(1);

    releaseDetail?.();
    await firstRequest;
    expect(selectSelectedDetail(store.getState()).value?.song_id).toBe(first.song_id);
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
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');

    const detail = store.getState().selectSong(first.song_id);
    await act(async () => store.getState().reloadLibrary());
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(selectSelectedDetail(store.getState()).value).toBeNull();
    releaseDetail?.();
    await detail;
    expect(selectSelectedDetail(store.getState()).value).toBeNull();
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
    expect(rowAt(store, 200)?.title).toBe('Song 212');
  });

  it('persists liked source state through the native bridge contract', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    const indexById = store.getState().library.indexById;

    await act(async () => store.getState().setSongLiked(first.song_id, true));
    expect(store.getState().library.likedTotal).toBe(1);
    expect(rowAt(store, 0)?.liked).toBe(true);
    expect(store.getState().library.indexById).toBe(indexById);

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'liked' });
    expect(store.getState().library.resultTotal).toBe(1);
    expect(rowAt(store, 0)?.song_id).toBe(first.song_id);
  });

  it('loads playlist navigation summaries and keeps membership out of catalog pages', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());

    expect(store.getState().libraryNavigation.loadState).toBe('ready');
    expect(store.getState().libraryNavigation.playlistOrder).toHaveLength(0);

    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)).toMatchObject({
      name: 'Practice',
      song_count: 0,
    });

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    const pagesBeforeMembership = store.getState().library.pages;
    await act(async () => store.getState().addSongToPlaylist(playlistId, first.song_id));
    expect(store.getState().library.pages).toBe(pagesBeforeMembership);
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.song_count).toBe(1);

    await act(async () =>
      store.getState().selectLibrarySource({ kind: 'playlist', id: playlistId }),
    );
    expect(store.getState().library.resultTotal).toBe(1);
    expect(rowAt(store, 0)?.song_id).toBe(first.song_id);
    await act(async () => store.getState().removeSongFromPlaylist(playlistId, first.song_id));
    expect(store.getState().library.resultTotal).toBe(0);

    await act(async () => store.getState().renamePlaylist(playlistId, 'Morning'));
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.name).toBe('Morning');
    await act(async () => store.getState().deletePlaylist(playlistId));
    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'all' });
    expect(store.getState().libraryNavigation.playlistOrder).toHaveLength(0);
  });

  it('imports local songs directly into a target playlist without exposing paths', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());

    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    await act(async () => store.getState().importLocalFolderToPlaylist(playlistId));

    const navigationJson = JSON.stringify({
      playlists: [...store.getState().libraryNavigation.playlistsById.values()],
    });
    expect(navigationJson).not.toContain('C:\\');
    expect(navigationJson).not.toContain('D:\\');
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.resultTotal).toBe(2);
    expect(rowAt(store, 0)?.title).toBe('Local Song B');

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    expect(store.getState().library.resultTotal).toBe(500);
    expect(rowAt(store, 0)?.title).not.toBe('Local Song B');
  });

  it('keeps the playlist selected while browsing All Songs to add', async () => {
    const bridge = createMockBridge();
    const requests: SearchRequest[] = [];
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      requests.push(request);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    const firstSongId = rowAt(store, 0)?.song_id;
    expect(firstSongId).toBeDefined();
    await act(async () => store.getState().setSongLiked(firstSongId!, true));
    requests.length = 0;

    await act(async () => store.getState().openPlaylistAdd(playlistId));
    expect(requests).toHaveLength(1);
    expect(requests[0]?.source).toEqual({ kind: 'smart', id: 'all' });
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.playlistAddMode).toEqual({ playlistId });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'all' });
    expect(store.getState().library.resultTotal).toBe(500);

    requests.length = 0;
    await act(async () => store.getState().search('Song 1'));
    expect(requests.at(-1)?.source).toEqual({ kind: 'smart', id: 'all' });

    await act(async () => store.getState().exitPlaylistAdd());
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.playlistAddMode).toBeNull();
    expect(store.getState().library.searchSource).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.resultTotal).toBe(0);
  });

  it('treats a playlist import cancellation as a no-op', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    bridge.importLocalFolderToPlaylist = async () => ({
      playlist: { id: playlistId, name: 'Practice', song_count: 0 },
      imported_song_count: 0,
      catalog_generation: 1,
    });
    await act(async () => store.getState().importLocalFolderToPlaylist(playlistId));

    expect(store.getState().library.generation).toBe(1);
    expect(store.getState().libraryNavigation.playlistOrder).toHaveLength(1);
    expect(store.getState().libraryNavigation.lastError).toBeNull();
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

  it('returns authoritative settings when a settings patch fails', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    bridge.patchSettings = async () => {
      throw new Error('settings IPC failed');
    };

    const authoritative = await store
      .getState()
      .patchSettings({ playbackDefaults: { timingMarginUs: 900 } });

    expect(authoritative?.playback_defaults.timing_margin_us).toBe(500);
    expect(store.getState().settings?.playback_defaults.timing_margin_us).toBe(500);
    expect(store.getState().settingsState).toBe('fatal');
  });

  it('detaches a prepared plan when the selected song changes', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const songA = rowAt(store, 0);
    const songB = rowAt(store, 1);
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
    const songA = rowAt(store, 0);
    const songB = rowAt(store, 1);
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
    const songA = rowAt(store, 0);
    const songB = rowAt(store, 1);
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await act(async () => store.getState().selectSong(songB.song_id));
    expect(store.getState().playback.currentSong?.title).toBe(songA.title);
  });

  it('keeps an explicit Now Playing identity when Library selection changes', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const songA = rowAt(store, 0);
    const songB = rowAt(store, 1);
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await act(async () => store.getState().selectSong(songB.song_id));

    const currentSong = (
      store.getState().playback as unknown as { currentSong?: { songId: string; title: string } }
    ).currentSong;
    expect(currentSong).toMatchObject({ songId: songA.song_id, title: songA.title });
    expect(store.getState().library.selectedSongId).toBe(songB.song_id);
  });

  it('coalesces concurrent prepare requests for the selected song', async () => {
    const bridge = createMockBridge();
    const originalPrepare = bridge.preparePlayback;
    let prepareCalls = 0;
    let releasePrepare: (() => void) | undefined;
    bridge.preparePlayback = async (request) => {
      prepareCalls += 1;
      await new Promise<void>((resolve) => {
        releasePrepare = resolve;
      });
      return originalPrepare(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    const firstPrepare = store.getState().prepareSelectedPlayback();
    const duplicatePrepare = store.getState().prepareSelectedPlayback();
    await act(async () => Promise.resolve());
    expect(prepareCalls).toBe(1);
    expect(
      (store.getState().playback as unknown as { transportOperation?: string }).transportOperation,
    ).toBe('preparing');

    releasePrepare?.();
    await act(async () => Promise.all([firstPrepare, duplicatePrepare]));
  });

  it('coalesces repeated starts of the same prepared playback', async () => {
    const bridge = createMockBridge();
    const originalStart = bridge.startPlayback;
    let startCalls = 0;
    let releaseStart: (() => void) | undefined;
    bridge.startPlayback = async (request) => {
      startCalls += 1;
      await new Promise<void>((resolve) => {
        releaseStart = resolve;
      });
      return originalStart(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    const firstStart = store.getState().startPreparedPlayback('proceed');
    const duplicateStart = store.getState().startPreparedPlayback('proceed');
    await act(async () => Promise.resolve());
    expect(startCalls).toBe(1);
    expect(
      (store.getState().playback as unknown as { transportOperation?: string }).transportOperation,
    ).toBe('starting');

    releaseStart?.();
    await act(async () => Promise.all([firstStart, duplicateStart]));
  });

  it('does not issue Pause while playback is still starting', async () => {
    const bridge = createMockBridge();
    const originalPause = bridge.pausePlayback;
    let pauseCalls = 0;
    bridge.pausePlayback = async (request) => {
      pauseCalls += 1;
      return originalPause(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    expect(store.getState().playback.state).toBe('starting');

    await act(async () => store.getState().pausePlayback());
    expect(pauseCalls).toBe(0);
  });

  it('clears Pause and Resume operation state only after matching lifecycle events', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 });
    const originalPause = bridge.pausePlayback;
    const originalResume = bridge.resumePlayback;
    let releasePause: (() => void) | undefined;
    let releaseResume: (() => void) | undefined;
    let pauseCalls = 0;
    let resumeCalls = 0;
    bridge.pausePlayback = async (request) => {
      pauseCalls += 1;
      await new Promise<void>((resolve) => {
        releasePause = resolve;
      });
      return originalPause(request);
    };
    bridge.resumePlayback = async (request) => {
      resumeCalls += 1;
      await new Promise<void>((resolve) => {
        releaseResume = resolve;
      });
      return originalResume(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    const pausing = store.getState().pausePlayback();
    expect(store.getState().playback.transportOperation).toBe('pausing');
    await act(async () => store.getState().resumePlayback());
    expect(resumeCalls).toBe(0);
    releasePause?.();
    await act(async () => pausing);
    expect(store.getState().playback.state).toBe('paused');
    expect(store.getState().playback.transportOperation).toBeNull();

    const resuming = store.getState().resumePlayback();
    expect(store.getState().playback.transportOperation).toBe('resuming');
    await act(async () => store.getState().pausePlayback());
    expect(pauseCalls).toBe(1);
    releaseResume?.();
    await act(async () => resuming);
    expect(store.getState().playback.state).toBe('playing');
    expect(store.getState().playback.transportOperation).toBeNull();
  });

  it('retires terminal ownership and does not treat a normal finish message as an error', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');
    store.setState({
      playback: {
        ...store.getState().playback,
        snapshot: {
          session_id: sessionId,
          song_id: first.song_id,
          title: first.title,
          state: 'playing',
          current_us: 1_000_000,
          total_us: first.duration_us ?? 1_000_000,
          physical: false,
          message: null,
        } as unknown as NonNullable<ReturnType<typeof store.getState>['playback']['snapshot']>,
      },
    });

    store.getState().applyEvent({
      v: 1,
      name: 'playback.state_changed',
      payload: {
        session_id: sessionId,
        song_id: first.song_id,
        state: 'finished',
        physical: false,
        message: 'Playback finished',
        outcome: 'finished',
      },
    });
    expect(store.getState().playback.error).toBeNull();
    store.getState().applyEvent({
      v: 1,
      name: 'playback.finished',
      payload: {
        session_id: sessionId,
        song_id: first.song_id,
        outcome: 'finished',
        total_us: first.duration_us ?? 1_000_000,
        message: 'Playback finished',
      },
    });

    expect(store.getState().playback.sessionId).toBeNull();
    expect(store.getState().playback.snapshot).toBeNull();
    expect(store.getState().playback.error).toBeNull();
  });

  it('naturally finishes at the context end into a replayable idle state', async () => {
    const store = createDesktopStore(createMockBridge({ playbackDurationMs: 80, startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setViewport(499, 499));
    await waitFor(() => expect(rowAt(store, 499)).toBeDefined());
    const last = rowAt(store, 499);
    if (!last) throw new Error('last mock-library song was not loaded');
    await act(async () => store.getState().selectSong(last.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    const firstSession = store.getState().playback.sessionId;
    if (!firstSession) throw new Error('mock session did not start');

    await waitFor(() => {
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.snapshot).toBeNull();
    });
    expect(store.getState().playback.currentSong?.songId).toBe(last.song_id);
    expect(store.getState().playback.transportOperation).toBeNull();

    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    expect(store.getState().playback.sessionId).not.toBe(firstSession);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  });

  it('automatically starts the next context song after natural retirement', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 120, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    const second = rowAt(store, 1);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    const firstSession = store.getState().playback.sessionId;
    if (!firstSession) throw new Error('mock session did not start');

    await waitFor(() => {
      expect(store.getState().playback.state).toBe('playing');
      expect(store.getState().playback.currentSong?.songId).toBe(second.song_id);
    });
    expect(store.getState().playback.sessionId).not.toBe(firstSession);
    expect(store.getState().playback.context?.currentIndex).toBe(1);
    expect(store.getState().playback.error).toBeNull();
  });

  it('Next retires the active session before starting its context neighbor', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    const nextSong = rowAt(store, 2);
    if (!first || !nextSong) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const oldSession = store.getState().playback.sessionId;
    if (!oldSession) throw new Error('mock session did not start');

    await act(async () => store.getState().nextPlayback());

    expect(store.getState().playback.currentSong?.title).toBe(nextSong.title);
    expect(store.getState().playback.sessionId).not.toBeNull();
    expect(store.getState().playback.sessionId).not.toBe(oldSession);
    expect(store.getState().playback.error).toBeNull();
    expect(store.getState().playback.context?.currentIndex).toBe(2);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  });

  it('keeps dry-run isolation when Previous and Next replace the active session', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 });
    const playbackConfigs: boolean[] = [];
    const originalPrepare = bridge.preparePlayback;
    bridge.preparePlayback = async (request) => {
      playbackConfigs.push(request.config.dry_run);
      return originalPrepare(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback({ dry_run: true }));
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    await act(async () => store.getState().nextPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await act(async () => store.getState().previousPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    expect(playbackConfigs).toEqual([true, true, true]);
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
  });

  it('resolves Next from the frozen source and query without rewriting the current Library view', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    await act(async () => store.getState().search('Aurora'));
    expect(store.getState().library.resultTotal).toBe(1);
    await act(async () => store.getState().nextPlayback());

    expect(store.getState().playback.currentSong?.title).toBe('Candle Run');
    expect(store.getState().playback.context?.query).toBe('');
    expect(store.getState().library.query).toBe('Aurora');
    expect(store.getState().library.resultTotal).toBe(1);
    expect(rowAt(store, 0)?.title).toBe('Aurora Landing');
  });

  it('lazily loads a context page for Next and keeps risk confirmation before starting it', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 });
    const searchOffsets: number[] = [];
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      searchOffsets.push(request.offset);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 199);
    if (!first) throw new Error('last row in first page was not loaded');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    await act(async () => store.getState().nextPlayback());

    expect(searchOffsets).toContain(200);
    expect(store.getState().playback.preparedIdentity?.title).toBe('Song 201');
    expect(store.getState().playback.prepared?.admission).toBe('confirmation_required');
    expect(store.getState().playback.sessionId).toBeNull();
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    expect(store.getState().playback.currentSong?.title).toBe('Song 201');
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  });

  it('keeps a next-song risk confirmation owned until confirm or cancel', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 });
    const starts: string[] = [];
    const originalStart = bridge.startPlayback;
    bridge.startPlayback = async (request) => {
      starts.push(request.preparedId);
      return originalStart(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setViewport(199, 200));
    await waitFor(() => expect(rowAt(store, 200)).toBeDefined());
    const first = rowAt(store, 199);
    const next = rowAt(store, 200);
    if (!first || !next) throw new Error('mock library page was not loaded');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await act(async () => store.getState().nextPlayback());
    expect(store.getState().playback.preparedIdentity?.songId).toBe(next.song_id);
    expect(store.getState().playback.prepared?.admission).toBe('confirmation_required');
    expect(store.getState().playback.sessionId).toBeNull();
    const preparedId = store.getState().playback.prepared?.prepared_id;

    await act(async () => {
      await store.getState().previousPlayback();
      await store.getState().nextPlayback();
      await store.getState().prepareSelectedPlayback();
      await store.getState().startPreparedPlayback();
    });

    expect(store.getState().playback.prepared?.prepared_id).toBe(preparedId);
    expect(store.getState().playback.preparedIdentity?.songId).toBe(next.song_id);
    expect(starts).toHaveLength(1);

    await act(async () => store.getState().cancelPreparedPlayback());
    expect(store.getState().playback.prepared).toBeNull();
    expect(store.getState().playback.preparedContext).toBeNull();
    expect(store.getState().playback.transportOperation).toBeNull();
  });

  it('Previous restarts after three seconds and selects the prior item near the beginning', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 20_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const third = rowAt(store, 2);
    const second = rowAt(store, 1);
    if (!third || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(third.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const oldSession = store.getState().playback.sessionId;
    if (!oldSession) throw new Error('mock session did not start');
    const snapshotAt = (sessionId: string, currentUs: number) => ({
      session_id: sessionId,
      seq: 1,
      state: 'playing' as const,
      song_id: third.song_id,
      title: third.title,
      current_us: currentUs,
      total_us: third.duration_us ?? 1_000_000,
      pre_roll_remaining_us: 0,
      focus_state: 'focused' as const,
      health: 'healthy' as const,
      input_path_degraded: false,
      message: null,
    });
    store.setState({
      playback: {
        ...store.getState().playback,
        snapshot: snapshotAt(oldSession, 3_000_001),
      },
    });

    await act(async () => store.getState().previousPlayback());
    expect(store.getState().playback.currentSong?.songId).toBe(third.song_id);
    expect(store.getState().playback.sessionId).not.toBe(oldSession);
    expect(store.getState().playback.context?.currentIndex).toBe(2);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    const restartedSession = store.getState().playback.sessionId;
    if (!restartedSession) throw new Error('restarted mock session did not start');
    store.setState({
      playback: {
        ...store.getState().playback,
        snapshot: snapshotAt(restartedSession, 3_000_000),
      },
    });
    await act(async () => store.getState().previousPlayback());
    expect(store.getState().playback.currentSong?.songId).toBe(second.song_id);
    expect(store.getState().playback.sessionId).not.toBe(restartedSession);
    expect(store.getState().playback.context?.currentIndex).toBe(1);
  });

  it('explicit Stop retires the session without advancing the frozen context', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    await act(async () => store.getState().stopPlayback());
    await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    expect(store.getState().playback.context?.currentIndex).toBe(0);
    expect(store.getState().playback.transportOperation).toBeNull();
  });

  it('Stop preempts starting after the native session is bound', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 1_000 });
    const stopped: string[] = [];
    const originalStop = bridge.stopPlayback;
    bridge.stopPlayback = async (request) => {
      stopped.push(request.sessionId);
      return originalStop(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    const startingSession = store.getState().playback.sessionId;
    expect(store.getState().playback.state).toBe('starting');
    expect(store.getState().playback.transportOperation).toBe('starting');
    expect(startingSession).not.toBeNull();

    await act(async () => store.getState().stopPlayback());
    await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
    expect(stopped).toEqual([startingSession]);
    expect(store.getState().playback.transportOperation).toBeNull();
  });

  it('binds a native session from reconciliation when its start response is delayed', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 60_000 });
    let releaseStart: (() => void) | undefined;
    const stopped: string[] = [];
    const originalSubscribe = bridge.subscribeUiEvents;
    bridge.subscribeUiEvents = async (listener) =>
      originalSubscribe((event) => {
        if (event.name === 'playback.snapshot') return;
        if (
          event.name === 'playback.state_changed' &&
          ['starting', 'playing'].includes(event.payload.state)
        )
          return;
        listener(event);
      });
    const originalStart = bridge.startPlayback;
    bridge.startPlayback = async (request) => {
      const session = await originalStart(request);
      return await new Promise((resolve) => {
        releaseStart = () => resolve(session);
      });
    };
    const originalStop = bridge.stopPlayback;
    bridge.stopPlayback = async (request) => {
      stopped.push(request.sessionId);
      return originalStop(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    vi.useFakeTimers();
    try {
      let startRequest: Promise<void> | undefined;
      await act(async () => {
        startRequest = store.getState().startPreparedPlayback('proceed');
        await Promise.resolve();
        await Promise.resolve();
      });
      expect(releaseStart).toBeDefined();
      expect(store.getState().playback.startRequestId).not.toBeNull();

      await vi.advanceTimersByTimeAsync(15_001);
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.sessionId).not.toBeNull();
      expect(store.getState().playback.startRequestId).toBeNull();
      expect(store.getState().playback.state).toBe('starting');
      expect(store.getState().playback.error).toBeNull();
      expect(stopped).toEqual([]);

      releaseStart?.();
      await act(async () => startRequest);
      await vi.advanceTimersByTimeAsync(0);

      expect(stopped).toEqual([]);
      expect(store.getState().playback.sessionId).not.toBeNull();
      const reconciledSessionId = store.getState().playback.sessionId;
      await act(async () => store.getState().stopPlayback());
      await vi.advanceTimersByTimeAsync(0);
      expect(stopped).toEqual([reconciledSessionId]);
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.startRequestId).toBeNull();
      expect(store.getState().playback.transportOperation).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('releases a never-resolving start when native status reports no session', async () => {
    const bridge = createMockBridge();
    bridge.startPlayback = () =>
      new Promise<import('../bridge/DesktopBridge').PlaybackSession>(() => undefined);
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    vi.useFakeTimers();
    try {
      void store.getState().startPreparedPlayback('proceed');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_001);
      });

      expect(store.getState().playback.startRequestId).toBeNull();
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.prepared).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.error).toContain('Press Play to try again');

      await act(async () => store.getState().prepareSelectedPlayback());
      expect(store.getState().playback.prepared).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not mistake a retired terminal for a replay of the same song', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const startedPreparedIds: string[] = [];
    const originalStart = bridge.startPlayback;
    bridge.startPlayback = async (request) => {
      startedPreparedIds.push(request.preparedId);
      return originalStart(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await act(async () => store.getState().stopPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('idle'));

    const staleTerminal = await bridge.getPlaybackStatus();
    expect(staleTerminal.active).toBeNull();
    expect(staleTerminal.last_terminal?.prepared_id).toBe(startedPreparedIds[0]);

    await act(async () => store.getState().prepareSelectedPlayback());
    const replayPreparedId = store.getState().playback.prepared?.prepared_id;
    expect(replayPreparedId).toBeDefined();
    expect(replayPreparedId).not.toBe(startedPreparedIds[0]);
    bridge.startPlayback = () =>
      new Promise<import('../bridge/DesktopBridge').PlaybackSession>(() => undefined);

    vi.useFakeTimers();
    try {
      void store.getState().startPreparedPlayback('proceed');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_001);
      });

      expect(store.getState().playback.startRequestId).toBeNull();
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.error).toContain('Press Play to try again');
      await act(async () => store.getState().prepareSelectedPlayback());
      expect(store.getState().playback.prepared).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('restores actionable native failure details when the terminal event is missing', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const prepared = store.getState().playback.prepared;
    const identity = store.getState().playback.preparedIdentity;
    const preparedId = prepared?.prepared_id;
    if (!preparedId || !identity) throw new Error('prepared playback is missing');
    const sessionId = 'c'.repeat(32);
    const failureMessage =
      'Sky window was not found. Open Sky and make sure its window is visible, then try again.';
    store.setState({
      playback: {
        ...store.getState().playback,
        sessionId,
        currentSong: identity,
        prepared: null,
        preparedIdentity: null,
        state: 'playing',
      },
    });
    bridge.stopPlayback = async (request) => ({
      accepted: true,
      session_id: request.sessionId,
      state: 'failed',
      pending_command: null,
      reason: null,
    });
    bridge.getPlaybackStatus = async () => ({
      active: null,
      last_terminal: {
        session_id: sessionId,
        prepared_id: preparedId,
        song_id: first.song_id,
        state: 'failed',
        outcome: null,
        failure_code: 'target_not_found',
        failure_message: failureMessage,
      },
    });

    await act(async () => store.getState().stopPlayback());

    expect(store.getState().playback.state).toBe('failed');
    expect(store.getState().playback.error).toBe(`target_not_found: ${failureMessage}`);
  });

  it('keeps Play retryable when both the start and status IPC calls never resolve', async () => {
    const bridge = createMockBridge();
    bridge.startPlayback = () =>
      new Promise<import('../bridge/DesktopBridge').PlaybackSession>(() => undefined);
    bridge.getPlaybackStatus = () =>
      new Promise<import('../bridge/DesktopBridge').PlaybackStatus>(() => undefined);
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    vi.useFakeTimers();
    try {
      void store.getState().startPreparedPlayback('proceed');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(17_100);
      });

      expect(store.getState().playback.startRequestId).toBeNull();
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.prepared).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.error).toContain('Native playback status query timed out');
      await act(async () => store.getState().prepareSelectedPlayback());
      expect(store.getState().playback.prepared).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('reconciles a missing terminal event after Stop from the native retirement status', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const originalSubscribe = bridge.subscribeUiEvents;
    bridge.subscribeUiEvents = async (listener) =>
      originalSubscribe((event) => {
        if (event.name === 'playback.finished' || event.name === 'playback.failed') return;
        listener(event);
      });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    vi.useFakeTimers();
    try {
      await act(async () => store.getState().stopPlayback());
      await vi.advanceTimersByTimeAsync(0);
      expect(store.getState().playback.state).toBe('finished');
      expect(store.getState().playback.sessionId).not.toBeNull();

      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_001);
      });
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.transportOperation).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('restores Pause and Resume controls when their confirmation events are missing', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    let muteStateConfirmations = false;
    const originalSubscribe = bridge.subscribeUiEvents;
    bridge.subscribeUiEvents = async (listener) =>
      originalSubscribe((event) => {
        if (
          muteStateConfirmations &&
          (event.name === 'playback.snapshot' ||
            (event.name === 'playback.state_changed' &&
              ['paused', 'playing'].includes(event.payload.state)))
        )
          return;
        listener(event);
      });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    muteStateConfirmations = true;
    vi.useFakeTimers();
    try {
      await act(async () => store.getState().pausePlayback());
      expect(store.getState().playback.transportOperation).toBe('pausing');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_001);
      });
      expect(store.getState().playback.state).toBe('paused');
      expect(store.getState().playback.transportOperation).toBeNull();

      await act(async () => store.getState().resumePlayback());
      expect(store.getState().playback.transportOperation).toBe('resuming');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(15_001);
      });
      expect(store.getState().playback.state).toBe('playing');
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.sessionId).not.toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('invalidates a Liked Songs context when the playing song is unliked', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 });
    const starts: string[] = [];
    const originalStart = bridge.startPlayback;
    bridge.startPlayback = async (request) => {
      starts.push(request.preparedId);
      return originalStart(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const allSongs = [rowAt(store, 0), rowAt(store, 1), rowAt(store, 2)];
    if (allSongs.some((song) => !song)) throw new Error('mock library is too small');
    const [first, second, third] = allSongs as [
      NonNullable<(typeof allSongs)[0]>,
      NonNullable<(typeof allSongs)[1]>,
      NonNullable<(typeof allSongs)[2]>,
    ];
    await act(async () => {
      await store.getState().setSongLiked(first.song_id, true);
      await store.getState().setSongLiked(second.song_id, true);
      await store.getState().setSongLiked(third.song_id, true);
    });
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    const likedFirst = rowAt(store, 0);
    const likedSecond = rowAt(store, 1);
    if (!likedFirst || !likedSecond) throw new Error('Liked Songs did not load');
    await act(async () => store.getState().selectSong(likedFirst.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const playingSession = store.getState().playback.sessionId;

    try {
      await act(async () => store.getState().setSongLiked(likedFirst.song_id, false));
      expect(store.getState().playback.context?.valid).toBe(false);
      await act(async () => store.getState().nextPlayback());

      expect(store.getState().playback.currentSong?.songId).toBe(likedFirst.song_id);
      expect(store.getState().playback.sessionId).toBe(playingSession);
      expect(starts).toHaveLength(1);
    } finally {
      await act(async () => store.getState().stopPlayback());
    }
  });

  it('invalidates a Playlist context when its playing song is removed', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 });
    const starts: string[] = [];
    const originalStart = bridge.startPlayback;
    bridge.startPlayback = async (request) => {
      starts.push(request.preparedId);
      return originalStart(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    const second = rowAt(store, 1);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().createPlaylist('Transport test'));
    const playlistId = store.getState().library.source.id;
    if (store.getState().library.source.kind !== 'playlist')
      throw new Error('new playlist was not selected');
    await act(async () => {
      await store.getState().addSongToPlaylist(playlistId, first.song_id);
      await store.getState().addSongToPlaylist(playlistId, second.song_id);
    });
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const activeSession = store.getState().playback.sessionId;

    try {
      await act(async () => store.getState().removeSongFromPlaylist(playlistId, first.song_id));
      expect(store.getState().playback.context?.valid).toBe(false);
      await act(async () => store.getState().nextPlayback());

      expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
      expect(store.getState().playback.sessionId).toBe(activeSession);
      expect(starts).toHaveLength(1);
    } finally {
      await act(async () => store.getState().stopPlayback());
    }
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
      const songId = store.getState().playback.preparedIdentity?.songId;
      if (!songId) throw new Error('prepared song identity is missing');
      const started = {
        session_id: sessionId,
        prepared_id: request.preparedId,
        song_id: songId,
        state: 'starting' as const,
        config: {
          hold_frames: 2,
          timing_margin_us: 800,
          down_late_grace_us: 500,
          tempo_scale: 1,
          fps: 60,
          dry_run: true,
        },
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
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));

    expect(store.getState().playback.sessionId).toBeNull();
    expect(store.getState().playback.state).toBe('idle');
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
  });

  it('rejects late events from a retired session after browsing another song', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const songA = rowAt(store, 0);
    const songB = rowAt(store, 1);
    if (!songA || !songB) throw new Error('mock library is too small');

    await act(async () => store.getState().selectSong(songA.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');

    store.getState().applyEvent({
      v: 1,
      name: 'playback.finished',
      payload: {
        session_id: sessionId,
        song_id: songA.song_id,
        outcome: 'skipped',
        total_us: 0,
        message: 'finished',
      },
    });
    await act(async () => store.getState().selectSong(songB.song_id));
    expect(store.getState().playback.currentSong).toBeNull();

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
    expect(store.getState().playback.state).toBe('idle');
    expect(store.getState().playback.currentSong).toBeNull();
  });

  it('keeps diagnostic samples and human events bounded without duplicating snapshots', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setDiagnosticsEnabled(true));
    const sessionId = 'a'.repeat(32);
    store.setState({
      diagnostics: { ...store.getState().diagnostics, events: [] },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    for (let index = 0; index < 601; index += 1) {
      store.getState().applyEvent({
        v: 1,
        name: 'diagnostics.snapshot',
        payload: {
          seq: index,
          physical_session: true,
          player_attached: true,
          sender_sample_count: index + 1,
          max_lateness_us: index,
          p50_ms: 0.1,
          p95_ms: 0.2,
          sigma_onset_ms: 0.1,
          late_2ms: 0,
          late_5ms: 0,
          late_10ms: 0,
          max_sendinput_pre_call_lateness_us: 0,
          pre_call_late_2ms: 0,
          pre_call_late_5ms: 0,
          pre_call_late_10ms: 0,
          fps: 60,
          frame_us: 16_667,
          hold_frames: 1,
          frame_base_hold_us: 16_667,
          timing_margin_us: 800,
          min_hold_us: 17_467,
          min_release_gap_us: 17_467,
          down_late_grace_us: 500,
          timing_margin_recommendation: {
            recommended_timing_margin_us: 800,
            qualified: false,
            source: 'default_fallback',
          },
          pre_call_lt_250us: 0,
          pre_call_250_500us: 0,
          pre_call_500_750us: 0,
          pre_call_750_1000us: 0,
          pre_call_1000_1500us: 0,
          pre_call_1500_2000us: 0,
          pre_call_ge_2000us: 0,
          active_keys: 0,
          stuck_keys: 0,
          keys_dropped: 0,
          chord_split_events: 0,
          missed_down_boundaries: 0,
          missed_down_keys: 0,
          missed_backlog_boundaries: 0,
          missed_hard_late_boundaries: 0,
          final_gate_cutoff_misses: 0,
          final_gate_control_rejections: 0,
          final_gate_target_changes: 0,
          final_gate_focus_losses: 0,
          final_gate_lease_expirations: 0,
          sendinput_partial_events: 0,
          sendinput_zero_progress_failures: 0,
          backend_status: 'healthy',
          release_max_us: 0,
          release_late_2ms: 0,
          session_id: sessionId,
          last_error: null,
        },
      });
    }
    expect(store.getState().diagnostics.events).toHaveLength(0);

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
    expect(
      store.getState().diagnostics.events.every((event) => event.name !== 'diagnostics.snapshot'),
    ).toBe(true);
  });

  it('starts a new diagnostic sample history when the native session changes', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setDiagnosticsEnabled(true));
    const firstSessionId = 'a'.repeat(32);
    store.setState({
      playback: { ...store.getState().playback, sessionId: firstSessionId, state: 'playing' },
    });

    const snapshot = (sessionId: string, seq: number) => ({
      v: 1 as const,
      name: 'diagnostics.snapshot' as const,
      payload: {
        seq,
        physical_session: true,
        player_attached: true,
        sender_sample_count: 1,
        max_lateness_us: seq,
        p50_ms: 0.1,
        p95_ms: 0.2,
        sigma_onset_ms: 0.1,
        late_2ms: 0,
        late_5ms: 0,
        late_10ms: 0,
        max_sendinput_pre_call_lateness_us: 0,
        pre_call_late_2ms: 0,
        pre_call_late_5ms: 0,
        pre_call_late_10ms: 0,
        fps: 60,
        frame_us: 16_667,
        hold_frames: 1,
        frame_base_hold_us: 16_667,
        timing_margin_us: 800,
        min_hold_us: 17_467,
        min_release_gap_us: 17_467,
        down_late_grace_us: 500,
        timing_margin_recommendation: {
          recommended_timing_margin_us: 800,
          qualified: false,
          source: 'default_fallback',
        },
        pre_call_lt_250us: 0,
        pre_call_250_500us: 0,
        pre_call_500_750us: seq,
        pre_call_750_1000us: 0,
        pre_call_1000_1500us: 0,
        pre_call_1500_2000us: 0,
        pre_call_ge_2000us: 0,
        active_keys: 0,
        stuck_keys: 0,
        keys_dropped: 0,
        chord_split_events: 0,
        missed_down_boundaries: 0,
        missed_down_keys: 0,
        missed_backlog_boundaries: 0,
        missed_hard_late_boundaries: 0,
        final_gate_cutoff_misses: 0,
        final_gate_control_rejections: 0,
        final_gate_target_changes: 0,
        final_gate_focus_losses: seq,
        final_gate_lease_expirations: 0,
        sendinput_partial_events: 0,
        sendinput_zero_progress_failures: seq,
        backend_status: 'healthy' as const,
        release_max_us: 0,
        release_late_2ms: 0,
        session_id: sessionId,
        last_error: null,
      },
    });

    store.getState().applyEvent(snapshot('a'.repeat(32), 1));
    store.getState().applyEvent(snapshot('a'.repeat(32), 2));
    expect(store.getState().diagnostics.samples).toHaveLength(2);
    expect(store.getState().diagnostics.samples[1]).toMatchObject({
      down_late_grace_us: 500,
      pre_call_500_750us: 2,
      final_gate_focus_losses: 2,
      sendinput_zero_progress_failures: 2,
    });

    const secondSessionId = 'b'.repeat(32);
    store.setState({
      playback: { ...store.getState().playback, sessionId: secondSessionId, state: 'playing' },
    });
    store.getState().applyEvent(snapshot(secondSessionId, 3));
    expect(store.getState().diagnostics.samples).toHaveLength(1);
    expect(store.getState().diagnostics.samples[0]?.session_id).toBe(secondSessionId);
    expect(store.getState().diagnostics.samples[0]?.pre_call_500_750us).toBe(3);

    store.getState().applyEvent({
      ...snapshot(firstSessionId, 4),
      payload: {
        ...snapshot(firstSessionId, 4).payload,
        physical_session: false,
        player_attached: false,
        sender_sample_count: 0,
        max_sendinput_pre_call_lateness_us: null,
        backend_status: 'unavailable',
      },
    });
    expect(store.getState().diagnostics.samples).toHaveLength(1);
    expect(store.getState().diagnostics.samples[0]?.session_id).toBe(secondSessionId);
    expect(store.getState().diagnostics.samples[0]?.backend_status).toBe('healthy');
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

    const event = store.getState().diagnostics.events.at(-1)?.detail;
    expect(event).toBeDefined();
    expect(new TextEncoder().encode(event).length).toBeLessThanOrEqual(4096);
  });

  it('drives calibration through typed progress and terminal events', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().startCalibration('quick'));
    await waitFor(() => expect(store.getState().calibration.state).toBe('succeeded'));
    expect(store.getState().calibration.operationId).toMatch(/^[0-9a-f]{32}$/);
    expect(store.getState().calibration.result?.outcome).toBe('succeeded');
    expect(store.getState().calibration.result?.source).toBe('qualified_calibration');
  });
});
