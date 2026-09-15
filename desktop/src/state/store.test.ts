import { act, waitFor } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import type { SearchRequest, SettingsPatch, UiEvent } from '../bridge/DesktopBridge';
import {
  AUTO_PLAY_HANDOFF_MS,
  createShuffleTraversal,
  createDesktopStore,
  playbackContextIndexAt,
  playbackContextMatchesLibrary,
  playbackIssuePresentation,
  selectNowPlayingSongId,
  selectRowAtIndex,
  type PlaybackContext,
  selectSelectedDetail,
} from './store';

function rowAt(store: ReturnType<typeof createDesktopStore>, index: number) {
  return selectRowAtIndex(store.getState().library, index);
}

async function startFirstSong(store: ReturnType<typeof createDesktopStore>) {
  const first = rowAt(store, 0);
  if (!first) throw new Error('mock library is empty');
  await act(async () => store.getState().selectSong(first.song_id));
  await act(async () => store.getState().prepareSelectedPlayback());
  await act(async () => store.getState().startPreparedPlayback('proceed'));
  await waitFor(() => expect(store.getState().playback.sessionId).not.toBeNull());
  return first;
}

describe('desktop store', () => {
  it('defaults Shuffle off and persists Auto Play on', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    expect(store.getState().playback.shuffleEnabled).toBe(false);
    expect(store.getState().settings?.auto_play).toBe(true);
  });

  it('keeps catalog loading and failure states separate from shell readiness', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      library: { ...store.getState().library, loading: true, error: null },
    });

    store.getState().applyEvent({
      v: 1,
      name: 'catalog.load_failed',
      payload: { message: 'catalog source is unavailable' },
    });

    expect(store.getState().bootstrapState).toBe('idle');
    expect(store.getState().library.loading).toBe(false);
    expect(store.getState().library.error).toBe('catalog source is unavailable');
  });

  it('does not overwrite a catalog failure received while bootstrap is loading', async () => {
    const bridge = createMockBridge();
    const originalBootstrap = bridge.bootstrap;
    let signalBootstrapStarted!: () => void;
    const bootstrapStarted = new Promise<void>((resolve) => {
      signalBootstrapStarted = resolve;
    });
    let releaseBootstrap!: () => void;
    const bootstrapRelease = new Promise<void>((resolve) => {
      releaseBootstrap = resolve;
    });
    bridge.bootstrap = async () => {
      signalBootstrapStarted();
      await bootstrapRelease;
      return {
        ...(await originalBootstrap()),
        catalog_state: 'loading',
        catalog_generation: null,
      };
    };
    const store = createDesktopStore(bridge);
    const initialization = store.getState().initialize();

    await bootstrapStarted;
    store.getState().applyEvent({
      v: 1,
      name: 'catalog.load_failed',
      payload: { message: 'catalog source is unavailable' },
    });
    releaseBootstrap();
    await act(async () => initialization);

    expect(store.getState().bootstrapState).toBe('ready');
    expect(store.getState().library.loading).toBe(false);
    expect(store.getState().library.error).toBe('catalog source is unavailable');
  });

  it('uses the authoritative settings snapshot from bootstrap', async () => {
    const bridge = createMockBridge();
    const getSettings = vi.spyOn(bridge, 'getSettings');
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());

    expect(getSettings).not.toHaveBeenCalled();
    expect(store.getState().settings).toMatchObject({
      theme: 'aurora',
      auto_play: true,
      verbose_hud: false,
    });
  });

  it('derives Now Playing from playback identity without changing the selected song', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const selected = rowAt(store, 0);
    const playing = rowAt(store, 1);
    if (!selected || !playing) throw new Error('mock library rows were not loaded');
    const library = store.getState().library;
    store.setState({
      library: { ...library, selectedSongId: selected.song_id },
      playback: {
        ...store.getState().playback,
        currentSong: {
          songId: playing.song_id,
          title: playing.title,
          liked: playing.liked,
          durationUs: playing.duration_us,
          formatLabel: playing.format_label,
          noteCount: playing.note_count,
          riskLevel: playing.risk_level,
          generation: library.generation,
        },
        context: {
          source: library.searchSource,
          query: library.query,
          generation: library.generation,
          total: library.resultTotal,
          currentIndex: 1,
          currentSongId: playing.song_id,
          shuffleTraversal: null,
          dryRun: false,
          membershipRevision: 0,
          valid: true,
        },
      },
    });

    expect(selectNowPlayingSongId(store.getState())).toBe(playing.song_id);
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);

    const otherQuery = { ...store.getState().library, query: 'Moonlit' };
    expect(playbackContextMatchesLibrary(store.getState().playback.context, otherQuery)).toBe(
      false,
    );
    store.setState({ library: { ...otherQuery, generation: library.generation + 1 } });
    expect(selectNowPlayingSongId(store.getState())).toBeNull();
  });

  it('creates a deterministic shuffle permutation that visits every context index once', () => {
    for (const total of [2, 3, 4, 5, 10, 12, 37]) {
      const originIndex = total - 1;
      const traversal = createShuffleTraversal(total, originIndex, `stable-seed-${total}`);
      expect(traversal).not.toBeNull();
      if (!traversal) throw new Error('shuffle traversal was not created');
      const context: PlaybackContext = {
        source: { kind: 'smart', id: 'all' },
        query: '',
        generation: 1,
        total,
        currentIndex: originIndex,
        currentSongId: `song-${originIndex}`,
        shuffleTraversal: traversal,
        dryRun: false,
        membershipRevision: 0,
        valid: true,
      };
      const order = Array.from({ length: total }, (_, position) =>
        playbackContextIndexAt(context, position),
      );
      expect(order).toHaveLength(total);
      expect(order[0]).toBe(originIndex);
      expect(order[1]).not.toBe(originIndex);
      expect(new Set(order).size).toBe(total);
      expect(order).toEqual(
        Array.from({ length: total }, (_, position) =>
          playbackContextIndexAt(
            {
              ...context,
              shuffleTraversal: createShuffleTraversal(total, originIndex, `stable-seed-${total}`),
            },
            position,
          ),
        ),
      );
    }
  });

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

  it('retries cached catalog hydration when reconciliation changes the generation mid-search', async () => {
    const bridge = createMockBridge();
    const originalSubscribe = bridge.subscribeUiEvents;
    const originalSearch = bridge.searchSongs;
    let listener: ((event: import('../bridge/DesktopBridge').UiEvent) => void) | undefined;
    let firstSearch = true;
    bridge.subscribeUiEvents = async (next) => {
      listener = next;
      return originalSubscribe(next);
    };
    bridge.searchSongs = async (request) => {
      if (firstSearch && request.generation === 1) {
        firstSearch = false;
        listener?.({
          v: 1,
          name: 'catalog.changed',
          payload: { generation: 2, total: 500 },
        });
        throw new Error('catalog generation is stale');
      }
      const result = await originalSearch(request);
      return request.generation === 2 ? { ...result, generation: 2 } : result;
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    expect(store.getState().library.generation).toBe(2);
    expect(store.getState().library.loading).toBe(false);
    expect(store.getState().library.error).toBeNull();
    expect(store.getState().library.pages.get(0)?.length).toBeGreaterThan(0);
  });

  it('bounds generation retries and converges through queued catalog changes', async () => {
    const bridge = createMockBridge();
    const originalSubscribe = bridge.subscribeUiEvents;
    const originalSearch = bridge.searchSongs;
    let listener: ((event: import('../bridge/DesktopBridge').UiEvent) => void) | undefined;
    const pendingGenerations = [2, 3, 4];
    let signalStableSearch!: () => void;
    const stableSearchStarted = new Promise<void>((resolve) => {
      signalStableSearch = resolve;
    });
    let releaseStableSearch!: () => void;
    const stableSearchRelease = new Promise<void>((resolve) => {
      releaseStableSearch = resolve;
    });
    let searchCalls = 0;
    bridge.subscribeUiEvents = async (next) => {
      listener = next;
      return originalSubscribe(next);
    };
    bridge.searchSongs = async (request) => {
      searchCalls += 1;
      const nextGeneration = pendingGenerations.shift();
      if (nextGeneration !== undefined) {
        listener?.({
          v: 1,
          name: 'catalog.changed',
          payload: { generation: nextGeneration, total: 500 },
        });
        throw new Error('catalog generation is stale');
      }
      signalStableSearch();
      await stableSearchRelease;
      const result = await originalSearch(request);
      return { ...result, generation: 4 };
    };

    const store = createDesktopStore(bridge);
    const initialization = store.getState().initialize();

    await stableSearchStarted;
    await initialization;
    expect(searchCalls).toBe(4);
    releaseStableSearch();
    await waitFor(() => {
      expect(store.getState().library.generation).toBe(4);
      expect(store.getState().library.loading).toBe(false);
      expect(store.getState().library.error).toBeNull();
    });
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

  it('keeps auto-advance ordered when successor playing arrives before start resolves', async () => {
    const bridge = createMockBridge({
      playbackDurationMs: 60,
      startDelayMs: 1,
      emitSnapshots: false,
    });
    const originalSubscribe = bridge.subscribeUiEvents;
    const originalStart = bridge.startPlayback;
    let forwardEvent: ((event: UiEvent) => void) | undefined;
    let holdSuccessorPlaying = false;
    let heldPlaying: UiEvent | null = null;
    let releaseSuccessorStart!: () => void;
    let resolveSuccessorStart!: () => void;
    const successorStartReleased = new Promise<void>((resolve) => {
      releaseSuccessorStart = resolve;
    });
    const successorStarted = new Promise<void>((resolve) => {
      resolveSuccessorStart = resolve;
    });
    let resolveSuccessorPlaying!: () => void;
    const successorPlayingCaptured = new Promise<void>((resolve) => {
      resolveSuccessorPlaying = resolve;
    });

    bridge.subscribeUiEvents = async (listener) => {
      forwardEvent = listener;
      return originalSubscribe((event) => {
        if (
          holdSuccessorPlaying &&
          event.name === 'playback.state_changed' &&
          event.payload.state === 'playing'
        ) {
          heldPlaying = event;
          resolveSuccessorPlaying();
          return;
        }
        listener(event);
      });
    };
    let startCount = 0;
    bridge.startPlayback = async (request) => {
      const successor = startCount++ === 1;
      if (successor) {
        holdSuccessorPlaying = true;
        resolveSuccessorStart();
      }
      const session = await originalStart(request);
      if (successor) await successorStartReleased;
      return session;
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    const second = rowAt(store, 1);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await waitFor(() => expect(store.getState().playback.currentSong?.songId).toBe(first.song_id));

    await successorStarted;
    await successorPlayingCaptured;
    expect(store.getState().playback.currentSong?.songId).toBe(second.song_id);
    expect(store.getState().playback.transportOperation).toBe('advancing');
    expect(heldPlaying).not.toBeNull();

    forwardEvent?.(heldPlaying!);
    expect(store.getState().playback.currentSong?.songId).toBe(second.song_id);
    expect(store.getState().playback.state).toBe('playing');
    expect(store.getState().playback.transportOperation).toBeNull();

    releaseSuccessorStart();
    await waitFor(() => expect(store.getState().playback.sessionId).not.toBeNull());
    expect(store.getState().playback.currentSong?.songId).toBe(second.song_id);
    expect(store.getState().playback.state).toBe('playing');
    expect(store.getState().playback.transportOperation).toBeNull();
  });

  it('waits for the Auto Play handoff after natural retirement', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const preparePlayback = vi.spyOn(bridge, 'preparePlayback');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = await startFirstSong(store);
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');
    const prepareCount = preparePlayback.mock.calls.length;

    vi.useFakeTimers();
    try {
      act(() => {
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
      });
      expect(store.getState().playback.transportOperation).toBe('advancing');
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount);

      await act(async () => vi.advanceTimersByTimeAsync(AUTO_PLAY_HANDOFF_MS - 1));
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount);

      await act(async () => vi.advanceTimersByTimeAsync(1));
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount + 1);
      expect(preparePlayback.mock.calls.at(-1)?.[0].songId).toBe(rowAt(store, 1)?.song_id);
    } finally {
      vi.useRealTimers();
    }
  });

  it('starts explicit Next without the natural Auto Play handoff', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const preparePlayback = vi.spyOn(bridge, 'preparePlayback');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await startFirstSong(store);
    await waitFor(() => {
      expect(store.getState().playback.state).toBe('playing');
      expect(store.getState().playback.transportOperation).toBeNull();
    });
    const prepareCount = preparePlayback.mock.calls.length;

    vi.useFakeTimers();
    try {
      let nextPlayback: Promise<void> | undefined;
      await act(async () => {
        nextPlayback = store.getState().nextPlayback();
        await vi.advanceTimersByTimeAsync(10);
      });
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount + 1);
      expect(preparePlayback.mock.calls.at(-1)?.[0].songId).toBe(rowAt(store, 1)?.song_id);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(50);
        await nextPlayback;
      });
      expect(store.getState().playback.currentSong?.songId).toBe(rowAt(store, 1)?.song_id);

      let previousPlayback: Promise<void> | undefined;
      await act(async () => {
        previousPlayback = store.getState().previousPlayback();
        await vi.advanceTimersByTimeAsync(10);
      });
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount + 2);
      expect(preparePlayback.mock.calls.at(-1)?.[0].songId).toBe(rowAt(store, 0)?.song_id);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5);
        await previousPlayback;
      });
    } finally {
      vi.useRealTimers();
    }
  });

  it('cancels a pending natural advance when Auto Play is disabled', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const preparePlayback = vi.spyOn(bridge, 'preparePlayback');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = await startFirstSong(store);
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');
    const prepareCount = preparePlayback.mock.calls.length;

    vi.useFakeTimers();
    try {
      act(() => {
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
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(180);
        await store.getState().patchSettings({ autoPlay: false });
      });
      await act(async () => vi.advanceTimersByTimeAsync(AUTO_PLAY_HANDOFF_MS));

      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount);
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    } finally {
      vi.useRealTimers();
    }
  });

  it('continues natural Auto Play from its frozen context when Library search changes before finish', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const preparePlayback = vi.spyOn(bridge, 'preparePlayback');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = await startFirstSong(store);
    const second = rowAt(store, 1);
    if (!second) throw new Error('mock library is too small');
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');
    await act(async () => store.getState().search('Moonlit'));
    const prepareCount = preparePlayback.mock.calls.length;

    vi.useFakeTimers();
    try {
      act(() => {
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
      });
      await act(async () => vi.advanceTimersByTimeAsync(AUTO_PLAY_HANDOFF_MS + 100));

      expect(store.getState().library.query).toBe('Moonlit');
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount + 1);
      expect(preparePlayback.mock.calls.at(-1)?.[0].songId).toBe(second.song_id);
      expect(store.getState().playback.context?.query).toBe('');
      expect(store.getState().playback.transportOperation).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('continues natural Auto Play when Library search changes during handoff', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const preparePlayback = vi.spyOn(bridge, 'preparePlayback');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = await startFirstSong(store);
    const second = rowAt(store, 1);
    if (!second) throw new Error('mock library is too small');
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');
    const prepareCount = preparePlayback.mock.calls.length;

    vi.useFakeTimers();
    try {
      act(() => {
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
      });
      expect(store.getState().playback.transportOperation).toBe('advancing');
      await act(async () => {
        await vi.advanceTimersByTimeAsync(120);
        await store.getState().search('Moonlit');
      });
      await act(async () => vi.advanceTimersByTimeAsync(AUTO_PLAY_HANDOFF_MS - 120 + 100));

      expect(store.getState().library.query).toBe('Moonlit');
      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount + 1);
      expect(preparePlayback.mock.calls.at(-1)?.[0].songId).toBe(second.song_id);
      expect(store.getState().playback.context?.query).toBe('');
      expect(store.getState().playback.transportOperation).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('cancels pending natural Auto Play when playback membership is invalidated', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 5 });
    const preparePlayback = vi.spyOn(bridge, 'preparePlayback');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    const second = rowAt(store, 1);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().createPlaylist('Auto Play invalidation'));
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
    const sessionId = store.getState().playback.sessionId;
    if (!sessionId) throw new Error('mock session did not start');
    const prepareCount = preparePlayback.mock.calls.length;

    vi.useFakeTimers();
    try {
      act(() => {
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
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(120);
        await store.getState().removeSongFromPlaylist(playlistId, first.song_id);
      });
      expect(store.getState().playback.context?.valid).toBe(false);
      await act(async () => vi.advanceTimersByTimeAsync(AUTO_PLAY_HANDOFF_MS));

      expect(preparePlayback).toHaveBeenCalledTimes(prepareCount);
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    } finally {
      vi.useRealTimers();
    }
  });

  it('uses the shuffled successor for natural Auto Play', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 140, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is too small');
    await act(async () => store.getState().setShuffleEnabled(true));
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    const initialContext = store.getState().playback.context;
    if (!initialContext?.shuffleTraversal) throw new Error('shuffle traversal was not captured');
    const nextIndex = playbackContextIndexAt(initialContext, 1);
    if (nextIndex === null) throw new Error('shuffle next index was not resolved');
    await act(async () => store.getState().setViewport(nextIndex, nextIndex));
    await waitFor(() => expect(rowAt(store, nextIndex)).toBeDefined());
    const expectedNext = rowAt(store, nextIndex);
    if (!expectedNext) throw new Error('expected shuffled row was not loaded');

    await waitFor(() => {
      expect(store.getState().playback.state).toBe('playing');
      expect(store.getState().playback.currentSong?.songId).toBe(expectedNext.song_id);
    });
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(1);
  });

  it('keeps dry-run Test playback one-shot when Auto Play is enabled', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 120, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback({ dry_run: true }));
    await act(async () => store.getState().startPreparedPlayback('proceed'));

    await waitFor(() => {
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
    });
    expect(store.getState().settings?.auto_play).toBe(true);
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    expect(store.getState().playback.context?.dryRun).toBe(true);
  });

  it('uses the same shuffled history for Next and Previous, then resumes sequentially when disabled', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 5_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    act(() => store.getState().setShuffleEnabled(true));
    const shuffledContext = store.getState().playback.context;
    if (!shuffledContext?.shuffleTraversal) throw new Error('shuffle context was not initialized');
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    const shuffledIndex = playbackContextIndexAt(shuffledContext, 1);
    if (shuffledIndex === null) throw new Error('shuffle next index is unavailable');
    await act(async () => store.getState().nextPlayback());
    await waitFor(() =>
      expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(1),
    );
    await waitFor(() => expect(store.getState().playback.transportOperation).toBeNull());
    expect(store.getState().playback.context?.currentIndex).toBe(shuffledIndex);
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(1);

    act(() => store.setState({ playback: { ...store.getState().playback, snapshot: null } }));
    await act(async () => store.getState().previousPlayback());
    await waitFor(() =>
      expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(0),
    );
    await waitFor(() => expect(store.getState().playback.transportOperation).toBeNull());
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(0);

    act(() => store.getState().setShuffleEnabled(false));
    await act(async () => store.getState().nextPlayback());
    await waitFor(() => expect(store.getState().playback.context?.currentIndex).toBe(2));
    await waitFor(() => expect(store.getState().playback.transportOperation).toBeNull());
    expect(store.getState().playback.context?.currentIndex).toBe(2);
    expect(store.getState().playback.context?.shuffleTraversal).toBeNull();
  });

  it('Shuffle Previous restarts the current shuffled item after three seconds', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 20_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    act(() => store.getState().setShuffleEnabled(true));
    const initialContext = store.getState().playback.context;
    if (!initialContext?.shuffleTraversal) throw new Error('shuffle traversal was not initialized');
    const nextIndex = playbackContextIndexAt(initialContext, 1);
    if (nextIndex === null) throw new Error('shuffle next index was not resolved');
    await act(async () => store.getState().nextPlayback());
    await waitFor(() =>
      expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(1),
    );
    if (store.getState().playback.prepared?.admission === 'confirmation_required') {
      await act(async () => store.getState().startPreparedPlayback('proceed'));
    }
    await waitFor(() => expect(store.getState().playback.transportOperation).toBeNull());
    await act(async () => store.getState().setViewport(nextIndex, nextIndex));
    await waitFor(() => expect(rowAt(store, nextIndex)).toBeDefined());
    const currentSongId = rowAt(store, nextIndex)?.song_id;
    expect(store.getState().playback.currentSong?.songId).toBe(currentSongId);
    const currentSession = store.getState().playback.sessionId;
    if (!currentSession) throw new Error('shuffled session did not start');
    const song = rowAt(store, nextIndex);
    if (!song) throw new Error('shuffled song was not loaded');
    store.setState({
      playback: {
        ...store.getState().playback,
        snapshot: {
          session_id: currentSession,
          seq: 1,
          state: 'playing',
          song_id: song.song_id,
          title: song.title,
          current_us: 3_000_001,
          total_us: song.duration_us ?? 10_000_000,
          pre_roll_remaining_us: 0,
          focus_state: 'focused',
          health: 'healthy',
          input_path_degraded: false,
          message: null,
        },
      },
    });

    await act(async () => store.getState().previousPlayback());
    if (store.getState().playback.prepared?.admission === 'confirmation_required') {
      await act(async () => store.getState().startPreparedPlayback('proceed'));
    }
    await waitFor(() => expect(store.getState().playback.sessionId).not.toBe(currentSession));
    await waitFor(() => expect(store.getState().playback.transportOperation).toBeNull());
    expect(store.getState().playback.currentSong?.songId).toBe(song.song_id);
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(1);
  });

  it('turning Auto Play off during a song keeps natural finish replayable on that song', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 350, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    const first = await startFirstSong(store);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    await act(async () => store.getState().patchSettings({ autoPlay: false }));

    await waitFor(() => {
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
    });
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    expect(store.getState().playback.context?.currentIndex).toBe(0);
    expect(store.getState().settings?.auto_play).toBe(false);
  });

  it('keeps a prepared plan when Auto Play alone is patched', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const prepared = store.getState().playback.prepared;
    expect(prepared?.prepared_id).toBeTruthy();

    await act(async () => store.getState().patchSettings({ autoPlay: false }));

    expect(store.getState().playback.prepared?.prepared_id).toBe(prepared?.prepared_id);
    expect(store.getState().playback.preparedContext).not.toBeNull();
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
    await act(async () => store.getState().patchSettings({ autoPlay: false }));
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

  it('does nothing at the traversal origin before the restart threshold, then restarts after it', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 20_000, startDelayMs: 5 }),
    );
    await act(async () => store.getState().initialize());
    act(() => store.getState().setShuffleEnabled(true));
    const first = rowAt(store, 1);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(0);
    const originalSession = store.getState().playback.sessionId;
    if (!originalSession) throw new Error('mock session did not start');
    const snapshotAt = (sessionId: string, currentUs: number) => ({
      session_id: sessionId,
      seq: 1,
      state: 'playing' as const,
      song_id: first.song_id,
      title: first.title,
      current_us: currentUs,
      total_us: first.duration_us ?? 10_000_000,
      pre_roll_remaining_us: 0,
      focus_state: 'focused' as const,
      health: 'healthy' as const,
      input_path_degraded: false,
      message: null,
    });

    store.setState({
      playback: {
        ...store.getState().playback,
        snapshot: snapshotAt(originalSession, 3_000_000),
      },
    });
    await act(async () => store.getState().previousPlayback());
    expect(store.getState().playback.sessionId).toBe(originalSession);
    expect(store.getState().playback.transportOperation).toBeNull();
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(0);

    store.setState({
      playback: {
        ...store.getState().playback,
        snapshot: snapshotAt(originalSession, 3_000_001),
      },
    });
    await act(async () => store.getState().previousPlayback());
    expect(store.getState().playback.currentSong?.songId).toBe(first.song_id);
    expect(store.getState().playback.sessionId).not.toBe(originalSession);
    expect(store.getState().playback.context?.shuffleTraversal?.position).toBe(0);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
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
      expect(playbackIssuePresentation(store.getState().playback)?.kind).toBe('recoverable_status');
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

  it('keeps ownership while status is unknown, then enables Play after authoritative no-session status', async () => {
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

      expect(store.getState().playback.startRequestId).not.toBeNull();
      expect(store.getState().playback.sessionId).toBeNull();
      expect(store.getState().playback.transportOperation).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(playbackIssuePresentation(store.getState().playback)?.kind).toBe('recoverable_status');

      bridge.getPlaybackStatus = async () => ({ active: null, last_terminal: null });
      await act(async () => store.getState().retryPlaybackStatus());
      expect(store.getState().playback.startRequestId).toBeNull();
      expect(store.getState().playback.prepared).toBeNull();
      expect(store.getState().playback.state).toBe('idle');
      expect(store.getState().playback.error).toContain('No active playback session was created');
      expect(playbackIssuePresentation(store.getState().playback)?.kind).toBe('playback_failure');
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
    act(() => store.getState().setShuffleEnabled(true));
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
          missed_unobserved_backlog_boundaries: 0,
          missed_physical_window_boundaries: 0,
          final_sender_window_expirations: 0,
          hold_floor_delay_boundaries: 0,
          max_hold_floor_delay_us: 0,
          release_floor_delay_boundaries: 0,
          release_floor_infeasible_boundaries: 0,
          max_release_floor_delay_us: 0,
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
          power_request_created: true,
          power_request_active: true,
          power_request_create_failures: 0,
          power_request_set_failures: 0,
          power_request_clear_failures: 0,
          power_request_close_failures: 0,
          suspend_resume_registered: true,
          suspend_resume_registration_failures: 0,
          suspend_resume_unregistration_failures: 0,
          system_suspend_active: false,
          system_suspend_notifications: 0,
          system_resume_notifications: 0,
          duplicate_system_power_notifications: 0,
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
        missed_unobserved_backlog_boundaries: 0,
        missed_physical_window_boundaries: 0,
        final_sender_window_expirations: 0,
        hold_floor_delay_boundaries: 0,
        max_hold_floor_delay_us: 0,
        release_floor_delay_boundaries: 0,
        release_floor_infeasible_boundaries: 0,
        max_release_floor_delay_us: 0,
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
        power_request_created: true,
        power_request_active: true,
        power_request_create_failures: 0,
        power_request_set_failures: 0,
        power_request_clear_failures: 0,
        power_request_close_failures: 0,
        suspend_resume_registered: true,
        suspend_resume_registration_failures: 0,
        suspend_resume_unregistration_failures: 0,
        system_suspend_active: false,
        system_suspend_notifications: 0,
        system_resume_notifications: 0,
        duplicate_system_power_notifications: 0,
      },
    });

    store.getState().applyEvent(snapshot('a'.repeat(32), 1));
    store.getState().applyEvent(snapshot('a'.repeat(32), 2));
    expect(store.getState().diagnostics.samples).toHaveLength(2);
    expect(store.getState().diagnostics.samples[1]).toMatchObject({
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

  it('restores Playing and Paused from authoritative Retry status without an error', async () => {
    for (const targetState of ['playing', 'paused'] as const) {
      const store = createDesktopStore(createMockBridge({ playbackDurationMs: 60_000 }));
      await act(async () => store.getState().initialize());
      await startFirstSong(store);
      await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
      if (targetState === 'paused') {
        await act(async () => store.getState().pausePlayback());
        await waitFor(() => expect(store.getState().playback.state).toBe('paused'));
      }
      const sessionId = store.getState().playback.sessionId;
      act(() =>
        store.setState({
          playback: {
            ...store.getState().playback,
            error: 'Playback status is unavailable: Mock event delivery was interrupted.',
          },
        }),
      );

      await act(async () => store.getState().retryPlaybackStatus());

      expect(store.getState().playback.sessionId).toBe(sessionId);
      expect(store.getState().playback.state).toBe(targetState);
      expect(store.getState().playback.error).toBeNull();
      expect(playbackIssuePresentation(store.getState().playback)).toBeNull();
      await act(async () => store.getState().stopPlayback());
    }
  });

  it('keeps Starting recoverable with Retry status and Stop after reconciliation', async () => {
    const store = createDesktopStore(
      createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 60_000 }),
    );
    await act(async () => store.getState().initialize());
    await startFirstSong(store);
    expect(store.getState().playback.state).toBe('starting');
    const sessionId = store.getState().playback.sessionId;

    await act(async () => store.getState().retryPlaybackStatus());

    expect(store.getState().playback.sessionId).toBe(sessionId);
    expect(store.getState().playback.state).toBe('starting');
    expect(playbackIssuePresentation(store.getState().playback)?.kind).toBe('recoverable_status');
    await act(async () => store.getState().stopPlayback());
    await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
  });

  it('keeps the owned session and recovery actions when Retry status also fails', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, emitSnapshots: false });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await startFirstSong(store);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const sessionId = store.getState().playback.sessionId;
    bridge.getPlaybackStatus = async () => {
      throw new Error('Mock IPC disconnected.');
    };

    await act(async () => store.getState().retryPlaybackStatus());

    expect(store.getState().playback.sessionId).toBe(sessionId);
    expect(store.getState().playback.error).toContain('Mock IPC disconnected');
    expect(playbackIssuePresentation(store.getState().playback)?.kind).toBe('recoverable_status');
    await act(async () => store.getState().stopPlayback());
    await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
  });

  it('deduplicates Retry status calls and rejects a stale response after a newer session binds', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000 });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await startFirstSong(store);
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const originalStatus = await bridge.getPlaybackStatus();
    const oldSessionId = store.getState().playback.sessionId;
    let releaseStatus: ((status: typeof originalStatus) => void) | undefined;
    let queryCount = 0;
    bridge.getPlaybackStatus = () => {
      queryCount += 1;
      return new Promise((resolve) => {
        releaseStatus = resolve;
      });
    };
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Mock event delivery was interrupted.',
        },
      }),
    );

    let firstRetry: Promise<void> | undefined;
    let secondRetry: Promise<void> | undefined;
    act(() => {
      firstRetry = store.getState().retryPlaybackStatus();
      secondRetry = store.getState().retryPlaybackStatus();
    });
    expect(firstRetry).toBe(secondRetry);
    expect(queryCount).toBe(1);
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          sessionId: 'd'.repeat(32),
          state: 'playing',
          error: null,
        },
      }),
    );
    releaseStatus?.(originalStatus);
    await act(async () => firstRetry);

    expect(store.getState().playback.sessionId).toBe('d'.repeat(32));
    expect(store.getState().playback.state).toBe('playing');
    expect(store.getState().playback.error).toBeNull();
    expect(store.getState().playback.statusRetryPending).toBe(false);
    expect(oldSessionId).not.toBe(store.getState().playback.sessionId);
  });
});
