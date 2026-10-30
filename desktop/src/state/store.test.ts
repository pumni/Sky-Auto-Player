import { act, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import type {
  LibraryNavigation,
  LibrarySource,
  SearchRequest,
  SearchResult,
  Settings,
  SettingsPatch,
  UiEvent,
} from '../bridge/DesktopBridge';
import {
  AUTO_PLAY_HANDOFF_MS,
  createShuffleTraversal,
  createDesktopStore,
  playbackContextIndexAt,
  playbackContextMatchesLibrary,
  playbackIssuePresentation,
  selectNowPlayingSongId,
  selectRowAtIndex,
  selectSongById,
  type PlaybackContext,
  selectSelectedDetail,
} from './store';
import { activeLibraryPageOffsets } from './libraryHelpers';
import { MAX_CACHED_SEARCH_PAGES } from './librarySlice';
import { LIBRARY_PAGE_SIZE } from './types';
import { WORKBENCH_STORAGE_KEY } from './workbenchPreferences';

afterEach(() => window.localStorage.clear());

function rowAt(store: ReturnType<typeof createDesktopStore>, index: number) {
  return selectRowAtIndex(store.getState().library, index);
}

function syntheticRows(count: number): SearchResult['items'] {
  return Array.from({ length: count }, (_, index) => ({
    song_id: (100_000 + index).toString(16).padStart(32, '0'),
    title: `Fixture Song ${String(index + 1).padStart(5, '0')}`,
    format_label: 'TXT',
    duration_us: 90_000_000 + index,
    note_count: 100 + index,
    risk_level: 'low' as const,
    metadata_state: 'ready' as const,
    liked: false,
  }));
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function successfulCalibrationEvent(): UiEvent {
  return {
    v: 1,
    name: 'calibration.finished',
    payload: {
      operation_id: 'calibration-refresh-test',
      outcome: 'succeeded',
      status: 'complete',
      recommended_timing_margin_us: 900,
      recommendation_qualified: true,
      sample_count: 10,
      source: 'qualified_calibration',
      message: 'Calibration finished.',
    },
  };
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
    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'builtin' });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'builtin' });
    expect(store.getState().playback.shuffleEnabled).toBe(false);
    expect(store.getState().settings?.auto_play).toBe(true);
  });

  it('loads authoritative navigation while the first catalog page is still pending', async () => {
    const builtinSongIds = Array.from({ length: 96 }, (_, index) =>
      index.toString(16).padStart(32, '0'),
    );
    const bridge = createMockBridge({ builtinSongIds });
    const originalSearch = bridge.searchSongs.bind(bridge);
    const originalListNavigation = bridge.listLibraryNavigation.bind(bridge);
    const pendingSearch = deferred<SearchResult>();
    const searchStarted = deferred<SearchRequest>();
    bridge.searchSongs = async (request) => {
      searchStarted.resolve(request);
      return pendingSearch.promise;
    };
    const listNavigation = vi.spyOn(bridge, 'listLibraryNavigation');
    const store = createDesktopStore(bridge);

    const nativeNavigation = await originalListNavigation();
    expect(nativeNavigation).toMatchObject({ builtin_available: true, builtin_song_count: 96 });

    const initialization = store.getState().initialize();
    const request = await searchStarted.promise;
    try {
      await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('ready'));
      expect(listNavigation).toHaveBeenCalledTimes(1);
      expect(store.getState().libraryNavigation).toMatchObject({
        builtinAvailable: true,
        builtinSongCount: 96,
      });
    } finally {
      pendingSearch.resolve(await originalSearch(request));
    }
    await initialization;
  });

  it('loads native navigation when bootstrap reports that catalog loading is still in progress', async () => {
    const builtinSongIds = Array.from({ length: 96 }, (_, index) =>
      index.toString(16).padStart(32, '0'),
    );
    const bridge = createMockBridge({ builtinSongIds });
    const originalBootstrap = bridge.bootstrap.bind(bridge);
    bridge.bootstrap = async () => ({
      ...(await originalBootstrap()),
      catalog_state: 'loading',
      catalog_generation: null,
    });
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('ready'));

    expect(store.getState().libraryNavigation).toMatchObject({
      builtinAvailable: true,
      builtinSongCount: 96,
    });
  });

  it('ignores older navigation after a catalog generation change and repeated event', async () => {
    const bridge = createMockBridge({ builtinSongIds: ['00000000000000000000000000000001'] });
    const originalSearch = bridge.searchSongs.bind(bridge);
    bridge.searchSongs = async (request) => {
      const result = await originalSearch(request);
      return request.generation === undefined
        ? result
        : { ...result, generation: request.generation };
    };
    const firstNavigation = deferred<LibraryNavigation>();
    const secondNavigation = deferred<LibraryNavigation>();
    let navigationCalls = 0;
    bridge.listLibraryNavigation = vi.fn(() => {
      navigationCalls += 1;
      return navigationCalls === 1 ? firstNavigation.promise : secondNavigation.promise;
    });
    const store = createDesktopStore(bridge);

    const initialization = store.getState().initialize();
    await waitFor(() => expect(navigationCalls).toBe(1));
    await initialization;
    const changed = {
      v: 1 as const,
      name: 'catalog.changed' as const,
      payload: { generation: 2, total: 500 },
    };
    store.getState().applyEvent(changed);
    store.getState().applyEvent(changed);
    await waitFor(() => expect(navigationCalls).toBe(2));
    secondNavigation.resolve({ builtin_available: true, builtin_song_count: 96, playlists: [] });
    await waitFor(() => expect(store.getState().libraryNavigation.builtinSongCount).toBe(96));
    firstNavigation.resolve({ builtin_available: false, builtin_song_count: 0, playlists: [] });

    await waitFor(() => {
      expect(store.getState().libraryNavigation.loadState).toBe('ready');
      expect(store.getState().libraryNavigation.builtinAvailable).toBe(true);
      expect(store.getState().libraryNavigation.builtinSongCount).toBe(96);
    });
  });

  it('refreshes navigation after an older response overlaps a playlist rename', async () => {
    const bridge = createMockBridge();
    const originalListNavigation = bridge.listLibraryNavigation.bind(bridge);
    const originalRenamePlaylist = bridge.renamePlaylist.bind(bridge);
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await store.getState().createPlaylist('Morning');
    const playlistId = store.getState().libraryNavigation.playlistOrder[0];
    if (!playlistId) throw new Error('mock playlist creation failed');
    const staleNavigation = await originalListNavigation();
    const pendingNavigation = deferred<LibraryNavigation>();
    const renameGate = deferred<void>();
    let navigationCalls = 0;
    bridge.listLibraryNavigation = vi.fn(() => {
      navigationCalls += 1;
      return navigationCalls === 1 ? pendingNavigation.promise : originalListNavigation();
    });
    bridge.renamePlaylist = async (playlistId, name) => {
      await renameGate.promise;
      return originalRenamePlaylist(playlistId, name);
    };

    const navigationRequest = store.getState().loadLibraryNavigation();
    const rename = store.getState().renamePlaylist(playlistId, 'Evening');
    expect(
      store.getState().libraryNavigation.pendingMutations.has(`playlist:${playlistId}:rename`),
    ).toBe(true);
    renameGate.resolve();
    await rename;
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.name).toBe('Evening');

    pendingNavigation.resolve(staleNavigation);
    await navigationRequest;
    await waitFor(() => {
      expect(navigationCalls).toBe(2);
      expect(store.getState().libraryNavigation.loadState).toBe('ready');
      expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.name).toBe(
        'Evening',
      );
      expect(
        store.getState().libraryNavigation.pendingMutations.has(`playlist:${playlistId}:rename`),
      ).toBe(false);
    });
  });

  it('keeps pending playlist mutation state and refreshes after stale navigation settles', async () => {
    const bridge = createMockBridge();
    const originalListNavigation = bridge.listLibraryNavigation.bind(bridge);
    const originalRenamePlaylist = bridge.renamePlaylist.bind(bridge);
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await store.getState().createPlaylist('Morning');
    const playlistId = store.getState().libraryNavigation.playlistOrder[0];
    if (!playlistId) throw new Error('mock playlist creation failed');
    const staleNavigation = await originalListNavigation();
    const pendingNavigation = deferred<LibraryNavigation>();
    const renameGate = deferred<void>();
    let navigationCalls = 0;
    bridge.listLibraryNavigation = vi.fn(() => {
      navigationCalls += 1;
      return navigationCalls === 1 ? pendingNavigation.promise : originalListNavigation();
    });
    bridge.renamePlaylist = async (id, name) => {
      await renameGate.promise;
      return originalRenamePlaylist(id, name);
    };

    const navigationRequest = store.getState().loadLibraryNavigation();
    const rename = store.getState().renamePlaylist(playlistId, 'Evening');
    pendingNavigation.resolve(staleNavigation);
    await navigationRequest;
    expect(
      store.getState().libraryNavigation.pendingMutations.has(`playlist:${playlistId}:rename`),
    ).toBe(true);

    renameGate.resolve();
    await rename;
    await waitFor(() => {
      expect(navigationCalls).toBe(2);
      expect(store.getState().libraryNavigation.loadState).toBe('ready');
      expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.name).toBe(
        'Evening',
      );
      expect(
        store.getState().libraryNavigation.pendingMutations.has(`playlist:${playlistId}:rename`),
      ).toBe(false);
    });
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
      palette: 'graphite',
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

  it('builds Built-in Shuffle traversal from the full source rather than the visible page', () => {
    const total = 500;
    const originIndex = 99;
    const traversal = createShuffleTraversal(total, originIndex, 'package-d-builtin-shuffle');
    if (!traversal) throw new Error('Built-in shuffle traversal was not created');
    const context: PlaybackContext = {
      source: { kind: 'smart', id: 'builtin' },
      query: '',
      generation: 1,
      total,
      currentIndex: originIndex,
      currentSongId: 'built-in-song-100',
      shuffleTraversal: traversal,
      dryRun: false,
      membershipRevision: 0,
      valid: true,
    };

    const order = Array.from({ length: total }, (_, position) =>
      playbackContextIndexAt(context, position),
    );
    expect(order[0]).toBe(originIndex);
    expect(order).toHaveLength(500);
    expect(new Set(order).size).toBe(500);
    expect(order.some((index) => index !== null && index >= 100)).toBe(true);
  });

  it('boots, searches, selects a song, and applies a catalog event', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    await act(async () => store.getState().search('', { kind: 'smart', id: 'all' }));
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

    await act(async () => store.getState().patchSettings({ palette: 'slate', verboseHud: true }));
    expect(store.getState().settings?.palette).toBe('slate');
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

  it('merges a joined in-flight page for the newest A-B-A search token', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    const firstAResponse = deferred<SearchResult>();
    const staleBResponse = deferred<SearchResult>();
    const requests: SearchRequest[] = [];
    bridge.searchSongs = (request) => {
      if (request.query === 'Aurora') {
        requests.push(request);
        return firstAResponse.promise;
      }
      if (request.query === 'Moonlit') {
        requests.push(request);
        return staleBResponse.promise;
      }
      return originalSearch(request);
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const currentLibrary = store.getState().library;
    const playing = rowAt(store, 0);
    if (!playing) throw new Error('mock library is empty');
    store.setState({
      playback: {
        ...store.getState().playback,
        sessionId: 'native-session',
        state: 'playing',
        currentSong: {
          songId: playing.song_id,
          title: playing.title,
          liked: playing.liked,
          durationUs: playing.duration_us,
          formatLabel: playing.format_label,
          noteCount: playing.note_count,
          riskLevel: playing.risk_level,
          generation: currentLibrary.generation,
        },
      },
    });

    const firstA = store.getState().search('Aurora');
    const searchB = store.getState().search('Moonlit');
    const latestA = store.getState().search('Aurora');
    expect(requests.map((request) => request.query)).toEqual(['Aurora', 'Moonlit']);
    expect(requests.every((request) => request.generation === currentLibrary.generation)).toBe(
      true,
    );

    const requestA = requests[0];
    const requestB = requests[1];
    if (!requestA || !requestB) throw new Error('expected both deferred search requests');
    const firstAResult = await originalSearch(requestA);
    firstAResponse.resolve(firstAResult);
    await Promise.all([firstA, latestA]);

    expect(store.getState().library.query).toBe('Aurora');
    expect(store.getState().library.loading).toBe(false);
    expect(store.getState().library.generation).toBe(currentLibrary.generation);
    expect(store.getState().library.pages.get(0)).toEqual(firstAResult.items);
    expect(rowAt(store, 0)?.title).toBe('Aurora Landing');
    expect(requests).toHaveLength(2);

    const staleBResult = await originalSearch(requestB);
    staleBResponse.resolve(staleBResult);
    await searchB;

    expect(store.getState().library.query).toBe('Aurora');
    expect(store.getState().library.pages.get(0)).toEqual(firstAResult.items);
    expect(store.getState().playback.sessionId).toBe('native-session');
    expect(store.getState().playback.currentSong?.songId).toBe(playing.song_id);
    expect(selectNowPlayingSongId(store.getState())).toBe(playing.song_id);
  });

  it('scopes browser selection to its search while preserving native Now Playing', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const selected = rowAt(store, 0);
    if (!selected) throw new Error('mock library is empty');

    await act(async () => store.getState().selectSong(selected.song_id));
    const selectedDetail = selectSelectedDetail(store.getState());
    expect(selectedDetail.value?.song_id).toBe(selected.song_id);
    const requestGeneration = store.getState().library.searchRequestGeneration;

    await act(async () => store.getState().search(''));
    expect(store.getState().library.searchRequestGeneration).toBe(requestGeneration);
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);
    expect(selectSelectedDetail(store.getState())).toBe(selectedDetail);

    const library = store.getState().library;
    store.setState({
      playback: {
        ...store.getState().playback,
        sessionId: 'native-session',
        state: 'playing',
        currentSong: {
          songId: selected.song_id,
          title: selected.title,
          liked: selected.liked,
          durationUs: selected.duration_us,
          formatLabel: selected.format_label,
          noteCount: selected.note_count,
          riskLevel: selected.risk_level,
          generation: library.generation,
        },
      },
    });

    await act(async () => store.getState().search('Moonlit'));

    expect(store.getState().library.selectedSongId).toBeNull();
    expect(selectSelectedDetail(store.getState()).value).toBeNull();
    expect(store.getState().details.bySongId.size).toBe(0);
    expect(store.getState().playback.sessionId).toBe('native-session');
    expect(store.getState().playback.currentSong?.songId).toBe(selected.song_id);
    expect(selectNowPlayingSongId(store.getState())).toBe(selected.song_id);
    expect(rowAt(store, 0)?.title).toBe('Moonlit Village');
  });

  it('reuses same-generation search cache and refetches during reload reconciliation', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    const searchRequests: SearchRequest[] = [];
    bridge.searchSongs = async (request) => {
      searchRequests.push(request);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const selected = rowAt(store, 0);
    if (!selected) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(selected.song_id));
    const initialRequestCount = searchRequests.length;

    await act(async () => store.getState().search(''));

    expect(searchRequests).toHaveLength(initialRequestCount);
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);
    expect(selectSelectedDetail(store.getState()).value?.song_id).toBe(selected.song_id);

    await act(async () => store.getState().reloadLibrary());
    await waitFor(() => {
      expect(store.getState().library.generation).toBe(2);
      expect(store.getState().library.loading).toBe(false);
      expect(store.getState().library.pages.get(0)?.length).toBeGreaterThan(0);
    });

    expect(searchRequests).toHaveLength(initialRequestCount + 1);
    expect(searchRequests.at(-1)?.generation).toBe(2);
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(selectSelectedDetail(store.getState()).value).toBeNull();
  });

  it('bounds resolved page history across playlist switches and catalog generations', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    const requests: SearchRequest[] = [];
    bridge.searchSongs = (request) => {
      requests.push(request);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const initialGeneration = store.getState().library.generation;

    await store.getState().search('Aurora');
    await store.getState().search('Aurora', { kind: 'smart', id: 'all' });
    const afterAllSongs = requests.length;
    const playlistSource = { kind: 'playlist' as const, id: 'cache-playlist' };
    await store.getState().search('Aurora', playlistSource);
    expect(requests).toHaveLength(afterAllSongs + 1);
    await store.getState().search('Aurora', { kind: 'smart', id: 'all' });
    expect(requests).toHaveLength(afterAllSongs + 1);

    await act(async () => store.getState().reloadLibrary());
    const currentGeneration = store.getState().library.generation;
    expect(currentGeneration).toBeGreaterThan(initialGeneration);
    expect(requests.at(-1)?.generation).toBe(currentGeneration);

    const sourceForCacheQuery = (index: number): LibrarySource =>
      index % 2 === 0
        ? { kind: 'playlist' as const, id: `cache-playlist-${index}` }
        : { kind: 'smart' as const, id: 'all' };
    for (let index = 0; index < MAX_CACHED_SEARCH_PAGES; index += 1) {
      await store.getState().search(`cache-${index}`, sourceForCacheQuery(index));
    }
    const afterFillingCache = requests.length;

    await store
      .getState()
      .search(
        `cache-${MAX_CACHED_SEARCH_PAGES - 2}`,
        sourceForCacheQuery(MAX_CACHED_SEARCH_PAGES - 2),
      );
    await store
      .getState()
      .search(
        `cache-${MAX_CACHED_SEARCH_PAGES - 1}`,
        sourceForCacheQuery(MAX_CACHED_SEARCH_PAGES - 1),
      );
    expect(requests).toHaveLength(afterFillingCache);

    await store.getState().search('Aurora', { kind: 'smart', id: 'all' });
    expect(requests).toHaveLength(afterFillingCache + 1);
    expect(requests.at(-1)?.query).toBe('Aurora');
    expect(requests.at(-1)?.generation).toBe(currentGeneration);
  });

  it('invalidates the Liked Songs projection while preserving unrelated search pages', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    const requests: SearchRequest[] = [];
    bridge.searchSongs = async (request) => {
      requests.push(request);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().search('Aurora'));
    expect(rowAt(store, 0)?.song_id).toBe(first.song_id);

    await act(async () => store.getState().setSongLiked(first.song_id, true));
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    expect(store.getState().library.resultTotal).toBe(1);
    expect(rowAt(store, 0)?.song_id).toBe(first.song_id);

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    await act(async () => store.getState().search('Aurora'));

    expect(rowAt(store, 0)?.liked).toBe(true);
    expect(
      requests.filter(
        (request) =>
          request.query === 'Aurora' &&
          request.source.kind === 'smart' &&
          request.source.id === 'all',
      ),
    ).toHaveLength(1);
  });

  it('invalidates playlist membership pages without clearing All Songs pages', async () => {
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    const requests: SearchRequest[] = [];
    bridge.searchSongs = async (request) => {
      requests.push(request);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().search('Aurora'));

    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    await act(async () => store.getState().addSongToPlaylist(playlistId, first.song_id));
    expect(store.getState().library.resultTotal).toBe(1);
    expect(rowAt(store, 0)?.song_id).toBe(first.song_id);

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    await act(async () => store.getState().search('Aurora'));

    expect(rowAt(store, 0)?.song_id).toBe(first.song_id);
    expect(
      requests.filter(
        (request) =>
          request.query === 'Aurora' &&
          request.source.kind === 'smart' &&
          request.source.id === 'all',
      ),
    ).toHaveLength(1);
  });

  it('reloads the active Liked page after a membership change at a deep viewport', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());

    for (const start of [0, 200, 400]) {
      const end = Math.min(start + 199, 449);
      await store.getState().setViewport(start, end);
      for (let index = start; index <= end; index += 1) {
        const row = rowAt(store, index);
        if (!row) throw new Error(`catalog row ${index} was not loaded`);
        await store.getState().setSongLiked(row.song_id, true);
      }
    }

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    expect(store.getState().library.resultTotal).toBe(450);
    await act(async () => store.getState().setViewport(400, 410));
    const removed = rowAt(store, 400);
    const next = rowAt(store, 401);
    if (!removed || !next) throw new Error('deep Liked page is incomplete');

    await act(async () => store.getState().setSongLiked(removed.song_id, false));

    expect(store.getState().library.resultTotal).toBe(449);
    expect(store.getState().library.pages.has(400)).toBe(true);
    expect(rowAt(store, 400)?.song_id).toBe(next.song_id);
    expect(store.getState().library.pages.size).toBeLessThanOrEqual(3);
    expect(store.getState().library.indexById.size).toBeLessThanOrEqual(3 * LIBRARY_PAGE_SIZE);
  });

  it('does not reuse an evicted selected ordinal after an earlier Liked member is removed', async () => {
    const rows = syntheticRows(3000);
    const bridge = createMockBridge({
      catalogRows: rows,
      playbackDurationMs: 60_000,
      startDelayMs: 1,
    });
    for (const row of rows) await bridge.setSongLiked({ songId: row.song_id, liked: true });

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    const earlierMember = rowAt(store, 5);
    if (!earlierMember) throw new Error('early Liked member was not loaded');
    await act(async () => store.getState().selectSong(earlierMember.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    await act(async () => store.getState().setViewport(1490, 1510));
    const selected = rowAt(store, 1500);
    if (!selected) throw new Error('deep Liked member was not loaded');
    await act(async () => store.getState().selectSong(selected.song_id));
    await act(async () => store.getState().setViewport(2500, 2510));
    expect(store.getState().library.pages.has(1400)).toBe(false);
    expect(store.getState().library.selectedSongIndex).toBe(1500);

    try {
      await act(async () => store.getState().setSongLiked(earlierMember.song_id, false));
      expect(store.getState().library.selectedSongId).toBe(selected.song_id);
      expect(store.getState().library.selectedSongSummary?.song_id).toBe(selected.song_id);
      expect(store.getState().library.selectedSongIndex).toBeNull();
      expect(store.getState().library.indexMembershipRevision).toBe(
        store.getState().library.membershipRevision,
      );

      await act(async () => store.getState().stopPlayback());
      await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
      await act(async () => store.getState().prepareSelectedPlayback());
      expect(store.getState().playback.preparedContext).toBeNull();
      expect(store.getState().playback.error).toMatch(/Select a loaded Library row/);

      await act(async () => store.getState().setViewport(1490, 1510));
      const newSelectedIndex = 1499;
      expect(rowAt(store, newSelectedIndex)?.song_id).toBe(selected.song_id);
      expect(store.getState().library.selectedSongIndex).toBe(newSelectedIndex);
      await act(async () => store.getState().prepareSelectedPlayback());
      expect(store.getState().playback.preparedContext?.currentIndex).toBe(newSelectedIndex);
      await act(async () => store.getState().startPreparedPlayback('proceed'));
      await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

      await act(async () => store.getState().nextPlayback());
      expect(store.getState().playback.currentSong?.songId).toBe(rows[1501]!.song_id);
      expect(store.getState().playback.context?.currentIndex).toBe(1500);
    } finally {
      await act(async () => store.getState().stopPlayback());
    }
  });

  it('clears browser selection when its source or playlist-add scope changes', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const firstCatalogRow = rowAt(store, 0);
    if (!firstCatalogRow) throw new Error('mock library is empty');
    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    if (store.getState().library.source.kind !== 'playlist') {
      throw new Error('new playlist was not selected');
    }

    await act(async () => store.getState().addSongToPlaylist(playlistId, firstCatalogRow.song_id));
    await act(async () => store.getState().selectSong(firstCatalogRow.song_id));
    expect(store.getState().library.selectedSongId).toBe(firstCatalogRow.song_id);

    await act(async () => store.getState().openPlaylistAdd(playlistId));
    expect(store.getState().library.playlistAddMode).toEqual({ playlistId });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'builtin' });
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(selectSelectedDetail(store.getState()).value).toBeNull();

    const availableRow = rowAt(store, 0);
    if (!availableRow) throw new Error('Built-in Songs were not loaded in playlist-add mode');
    await act(async () => store.getState().selectSong(availableRow.song_id));
    const playbackBeforePage = store.getState().playback;

    await act(async () => store.getState().navigateBuiltinPage(2));
    const row101 = rowAt(store, 100);
    if (!row101) throw new Error('Built-in picker could not reach song #101');
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.selectedSongId).toBe(availableRow.song_id);
    expect(selectSelectedDetail(store.getState()).value?.song_id).toBe(availableRow.song_id);
    expect(store.getState().playback).toBe(playbackBeforePage);
    await act(async () => store.getState().addSongToPlaylist(playlistId, row101.song_id));
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.song_count).toBe(2);

    await act(async () => store.getState().exitPlaylistAdd());
    expect(store.getState().library.playlistAddMode).toBeNull();
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.resultTotal).toBe(2);
    expect(rowAt(store, 1)?.song_id).toBe(row101.song_id);
    expect(store.getState().library.selectedSongId).toBeNull();

    const playlistRow = rowAt(store, 0);
    if (!playlistRow) throw new Error('playlist result was not restored');
    await act(async () => store.getState().selectSong(playlistRow.song_id));
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(selectSelectedDetail(store.getState()).value).toBeNull();
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
    expect(requestedOffsets).toContain(300);
    expect(requestedOffsets).toContain(400);
  });

  it('keeps the 96-song catalog complete with a single bounded active page', async () => {
    const store = createDesktopStore(createMockBridge({ catalogRows: syntheticRows(96) }));
    await act(async () => store.getState().initialize());

    expect(store.getState().library.resultTotal).toBe(96);
    expect(store.getState().library.pages.size).toBe(1);
    expect(store.getState().library.indexById.size).toBe(96);

    await act(async () => store.getState().setViewport(80, 95));

    expect(store.getState().library.pages.size).toBe(1);
    expect(store.getState().library.indexById.size).toBe(96);
    expect(rowAt(store, 95)?.title).toBe('Fixture Song 00096');
  });

  it('uses explicit built-in membership instead of the All Songs result set', async () => {
    const rows = syntheticRows(3);
    const builtinIds = [rows[0]!.song_id, rows[2]!.song_id];
    const store = createDesktopStore(
      createMockBridge({ catalogRows: rows, builtinSongIds: builtinIds }),
    );
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    expect(store.getState().library.catalogTotal).toBe(3);
    expect(store.getState().libraryNavigation.builtinSongCount).toBe(2);

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    expect(store.getState().library.resultTotal).toBe(2);
    expect(rowAt(store, 0)?.song_id).toBe(builtinIds[0]);
    expect(rowAt(store, 1)?.song_id).toBe(builtinIds[1]);
    expect(store.getState().library.indexById.has(rows[1]!.song_id)).toBe(false);
  });

  it.each([0, 1, 96, 99, 100, 101, 199, 200, 201, 2_438, 10_000])(
    'pages the explicit built-in membership for %i songs in 100-row transfers',
    async (count) => {
      const rows = syntheticRows(count);
      const requests: SearchRequest[] = [];
      const bridge = createMockBridge({
        catalogRows: rows,
        builtinSongIds: rows.map((row) => row.song_id),
      });
      const originalSearch = bridge.searchSongs;
      bridge.searchSongs = async (request) => {
        requests.push(request);
        return originalSearch(request);
      };
      const store = createDesktopStore(bridge);
      await act(async () => store.getState().initialize());
      await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));

      expect(store.getState().library.resultTotal).toBe(count);
      expect(store.getState().libraryNavigation.builtinAvailable).toBe(true);
      expect(store.getState().libraryNavigation.builtinSongCount).toBe(count);
      expect(
        requests
          .filter((request) => request.source.kind === 'smart' && request.source.id === 'builtin')
          .every((request) => request.limit === 100),
      ).toBe(true);
      expect(store.getState().library.builtinPage.committedPage).toBe(1);

      if (count > LIBRARY_PAGE_SIZE) {
        const pageCount = Math.ceil(count / LIBRARY_PAGE_SIZE);
        const targetPage = count === 2_438 ? 25 : pageCount;
        await act(async () => store.getState().navigateBuiltinPage(targetPage));
        const offset = (targetPage - 1) * LIBRARY_PAGE_SIZE;
        const expectedLength = Math.min(LIBRARY_PAGE_SIZE, count - offset);
        expect(store.getState().library.builtinPage.committedPage).toBe(targetPage);
        expect(store.getState().library.builtinPage.pendingPage).toBeNull();
        expect(store.getState().library.pages.get(offset)).toHaveLength(expectedLength);
        expect(rowAt(store, offset + expectedLength - 1)?.title).toBe(
          `Fixture Song ${String(offset + expectedLength).padStart(5, '0')}`,
        );
      } else {
        if (count === 0) expect(store.getState().library.pages.size).toBe(0);
        else expect(store.getState().library.pages.get(0)).toHaveLength(count);
      }
      expect(store.getState().library.indexById.size).toBeLessThanOrEqual(3 * LIBRARY_PAGE_SIZE);
    },
  );

  it.each(['library', 'playlist-add'] as const)(
    'commits only the latest built-in page intent in %s context and keeps the prior page visible',
    async (context) => {
      const rows = syntheticRows(500);
      const bridge = createMockBridge({
        catalogRows: rows,
        builtinSongIds: rows.map((row) => row.song_id),
      });
      const originalSearch = bridge.searchSongs;
      const delayed = new Map<number, ReturnType<typeof deferred<void>>>();
      bridge.searchSongs = async (request) => {
        if (
          request.source.kind === 'smart' &&
          request.source.id === 'builtin' &&
          request.offset > 0
        ) {
          const gate = deferred<void>();
          delayed.set(request.offset, gate);
          await gate.promise;
        }
        return originalSearch(request);
      };
      const store = createDesktopStore(bridge);
      await act(async () => store.getState().initialize());
      let playlistId: string | null = null;
      if (context === 'playlist-add') {
        await act(async () => store.getState().createPlaylist('Practice'));
        const source = store.getState().library.source;
        if (source.kind !== 'playlist') throw new Error('new playlist was not selected');
        playlistId = source.id;
        await act(async () => store.getState().openPlaylistAdd(source.id));
      } else {
        await act(async () =>
          store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }),
        );
      }

      let pageTwo!: Promise<void>;
      let pageFour!: Promise<void>;
      let pageThree!: Promise<void>;
      act(() => {
        pageTwo = store.getState().navigateBuiltinPage(2);
        pageFour = store.getState().navigateBuiltinPage(4);
        pageThree = store.getState().navigateBuiltinPage(3);
      });
      await waitFor(() => expect(delayed.size).toBe(3));
      expect(store.getState().library.builtinPage.committedPage).toBe(1);
      expect(store.getState().library.builtinPage.pendingPage).toBe(3);
      expect(rowAt(store, 0)?.title).toBe('Fixture Song 00001');

      await act(async () => {
        delayed.get(300)?.resolve();
        delayed.get(100)?.resolve();
        delayed.get(200)?.resolve();
        await Promise.all([pageTwo, pageFour, pageThree]);
      });
      expect(store.getState().library.builtinPage.committedPage).toBe(3);
      expect(store.getState().library.builtinPage.pendingPage).toBeNull();
      expect(rowAt(store, 200)?.title).toBe('Fixture Song 00201');
      expect(rowAt(store, 100)).toBeUndefined();
      if (playlistId) {
        expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
        expect(store.getState().library.playlistAddMode).toEqual({ playlistId });
      }
    },
  );

  it('drops an older built-in page response after the query changes', async () => {
    const rows = syntheticRows(500);
    const bridge = createMockBridge({
      catalogRows: rows,
      builtinSongIds: rows.map((row) => row.song_id),
    });
    const originalSearch = bridge.searchSongs;
    const pageTwoStarted = deferred<void>();
    const releasePageTwo = deferred<void>();
    bridge.searchSongs = async (request) => {
      if (
        request.source.kind === 'smart' &&
        request.source.id === 'builtin' &&
        request.query === '' &&
        request.offset === 100
      ) {
        pageTwoStarted.resolve();
        await releasePageTwo.promise;
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));

    let oldNavigation!: Promise<void>;
    act(() => {
      oldNavigation = store.getState().navigateBuiltinPage(2);
    });
    await pageTwoStarted.promise;
    await act(async () => store.getState().search('Fixture Song 00002'));
    expect(store.getState().library.resultTotal).toBe(1);
    expect(store.getState().library.builtinPage.committedPage).toBe(1);
    expect(rowAt(store, 0)?.title).toBe('Fixture Song 00002');

    await act(async () => {
      releasePageTwo.resolve();
      await oldNavigation;
    });
    expect(store.getState().library.query).toBe('Fixture Song 00002');
    expect(store.getState().library.builtinPage.committedPage).toBe(1);
    expect(store.getState().library.builtinPage.pendingPage).toBeNull();
    expect(store.getState().library.resultTotal).toBe(1);
    expect(rowAt(store, 0)?.title).toBe('Fixture Song 00002');
  });

  it('drops an older built-in page response after source navigation', async () => {
    const rows = syntheticRows(500);
    const bridge = createMockBridge({
      catalogRows: rows,
      builtinSongIds: rows.map((row) => row.song_id),
    });
    const originalSearch = bridge.searchSongs;
    const pageTwoStarted = deferred<void>();
    const releasePageTwo = deferred<void>();
    bridge.searchSongs = async (request) => {
      if (
        request.source.kind === 'smart' &&
        request.source.id === 'builtin' &&
        request.offset === 100
      ) {
        pageTwoStarted.resolve();
        await releasePageTwo.promise;
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));

    let oldNavigation!: Promise<void>;
    act(() => {
      oldNavigation = store.getState().navigateBuiltinPage(2);
    });
    await pageTwoStarted.promise;
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'all' }));
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'all' });
    expect(store.getState().library.builtinPage.committedPage).toBe(1);

    await act(async () => {
      releasePageTwo.resolve();
      await oldNavigation;
    });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'all' });
    expect(store.getState().library.builtinPage.committedPage).toBe(1);
    expect(store.getState().library.resultTotal).toBe(500);
    expect(rowAt(store, 0)?.title).toBe('Fixture Song 00001');
  });

  it('preserves selection and detail on page-load failure and retries that target', async () => {
    const rows = syntheticRows(250);
    const bridge = createMockBridge({
      catalogRows: rows,
      builtinSongIds: rows.map((row) => row.song_id),
    });
    const originalSearch = bridge.searchSongs;
    let failFirstPageTwo = true;
    bridge.searchSongs = async (request) => {
      if (
        request.source.kind === 'smart' &&
        request.source.id === 'builtin' &&
        request.offset === 100 &&
        failFirstPageTwo
      ) {
        failFirstPageTwo = false;
        throw new Error('temporary page failure');
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    const selected = rowAt(store, 4);
    if (!selected) throw new Error('built-in page one was not loaded');
    await act(async () => store.getState().selectSong(selected.song_id));
    const selectedDetail = selectSelectedDetail(store.getState()).value;

    await act(async () => store.getState().navigateBuiltinPage(2));
    expect(store.getState().library.builtinPage.committedPage).toBe(1);
    expect(store.getState().library.builtinPage.failedPage).toBe(2);
    expect(rowAt(store, 4)?.song_id).toBe(selected.song_id);
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);
    expect(selectSelectedDetail(store.getState()).value).toEqual(selectedDetail);

    await act(async () => store.getState().retryBuiltinPage());
    expect(store.getState().library.builtinPage.committedPage).toBe(2);
    expect(store.getState().library.builtinPage.failedPage).toBeNull();
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);
    expect(store.getState().library.selectedSongSummary?.song_id).toBe(selected.song_id);
    expect(selectSelectedDetail(store.getState()).value).toEqual(selectedDetail);
  });

  it('rejects a page response with a mismatched result count before committing it', async () => {
    const rows = syntheticRows(250);
    const bridge = createMockBridge({
      catalogRows: rows,
      builtinSongIds: rows.map((row) => row.song_id),
    });
    const originalSearch = bridge.searchSongs;
    let pageTwoRequests = 0;
    bridge.searchSongs = async (request) => {
      const result = await originalSearch(request);
      if (
        request.source.kind === 'smart' &&
        request.source.id === 'builtin' &&
        request.offset === 100
      ) {
        pageTwoRequests += 1;
        if (pageTwoRequests === 1) return { ...result, total: result.total - 1 };
      }
      return result;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));

    await act(async () => store.getState().navigateBuiltinPage(2));
    expect(store.getState().library.builtinPage.committedPage).toBe(1);
    expect(store.getState().library.builtinPage.failedPage).toBe(2);
    expect(rowAt(store, 0)?.title).toBe('Fixture Song 00001');
    expect(rowAt(store, 100)).toBeUndefined();

    await act(async () => store.getState().retryBuiltinPage());
    expect(store.getState().library.builtinPage.committedPage).toBe(2);
    expect(store.getState().library.builtinPage.failedPage).toBeNull();
    expect(pageTwoRequests).toBe(2);
    expect(rowAt(store, 100)?.title).toBe('Fixture Song 00101');
  });

  it('bounds a 10,000-song scroll sweep, pins selection/playback, and reloads evicted pages', async () => {
    const bridge = createMockBridge({ catalogRows: syntheticRows(10_000) });
    const originalSearch = bridge.searchSongs;
    const requestedOffsets: number[] = [];
    bridge.searchSongs = async (request) => {
      if (request.query === '') requestedOffsets.push(request.offset);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const selected = rowAt(store, 0);
    if (!selected) throw new Error('synthetic catalog is empty');
    await act(async () => store.getState().selectSong(selected.song_id));

    for (let offset = 0; offset < 10_000; offset += LIBRARY_PAGE_SIZE) {
      await act(async () => store.getState().setViewport(offset, offset));
      expect(store.getState().library.pages.size).toBeLessThanOrEqual(3);
      expect(store.getState().library.indexById.size).toBeLessThanOrEqual(3 * LIBRARY_PAGE_SIZE);
    }

    expect(store.getState().library.pages.has(0)).toBe(false);
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);
    expect(store.getState().library.selectedSongSummary?.title).toBe(selected.title);
    expect(selectSongById(store.getState().library, selected.song_id)?.title).toBe(selected.title);
    expect(store.getState().library.selectedSongIndex).toBe(0);

    await act(async () => store.getState().prepareSelectedPlayback());
    expect(store.getState().playback.prepared?.song.title).toBe(selected.title);
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.currentSong?.title).toBe(selected.title));

    const initialZeroPageRequests = requestedOffsets.filter((offset) => offset === 0).length;
    await act(async () => store.getState().setViewport(0, 0));

    expect(requestedOffsets.filter((offset) => offset === 0)).toHaveLength(
      initialZeroPageRequests + 1,
    );
    expect(rowAt(store, 0)?.song_id).toBe(selected.song_id);
    expect(store.getState().library.selectedSongSummary?.title).toBe(selected.title);
    expect(store.getState().playback.currentSong?.title).toBe(selected.title);
    expect(store.getState().library.pages.size).toBeLessThanOrEqual(3);
    expect(store.getState().library.indexById.size).toBeLessThanOrEqual(3 * LIBRARY_PAGE_SIZE);
  });

  it('keeps All-page reads and Next moving while a Like write is deferred', async () => {
    const rows = syntheticRows(10_000);
    const bridge = createMockBridge({
      catalogRows: rows,
      playbackDurationMs: 60_000,
      startDelayMs: 1,
    });
    const originalSearch = bridge.searchSongs;
    const requestedOffsets: number[] = [];
    bridge.searchSongs = async (request) => {
      if (request.query === '') requestedOffsets.push(request.offset);
      return originalSearch(request);
    };
    const likeStarted = deferred<void>();
    const releaseLike = deferred<Awaited<ReturnType<typeof bridge.setSongLiked>>>();
    bridge.setSongLiked = async () => {
      likeStarted.resolve();
      return releaseLike.promise;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setViewport(5190, 5200));
    const playing = rowAt(store, 5199);
    if (!playing) throw new Error('page-boundary playback row was not loaded');
    await act(async () => store.getState().selectSong(playing.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    for (let offset = 5400; offset <= 7200; offset += LIBRARY_PAGE_SIZE) {
      await act(async () => store.getState().setViewport(offset, offset));
    }
    await act(async () => store.getState().setViewport(9000, 9010));
    expect(requestedOffsets.filter((offset) => offset === 5100)).toHaveLength(1);
    expect(requestedOffsets.filter((offset) => offset === 5200)).toHaveLength(1);

    const likePromise = store.getState().setSongLiked(playing.song_id, true);
    await likeStarted.promise;
    let viewportPromise: Promise<void> | undefined;
    let nextPromise: Promise<void> | undefined;
    try {
      viewportPromise = store.getState().setViewport(5100, 5110);
      await waitFor(
        () => expect(requestedOffsets.filter((offset) => offset === 5100)).toHaveLength(2),
        { timeout: 1_000 },
      );
      await viewportPromise;
      expect(rowAt(store, 5199)?.liked).toBe(true);
      expect(store.getState().library.likedTotal).toBe(1);

      await act(async () => store.getState().setViewport(5190, 5199));
      expect(rowAt(store, 5199)?.liked).toBe(true);
      expect(store.getState().library.likedTotal).toBe(1);

      nextPromise = store.getState().nextPlayback();
      await waitFor(
        () => expect(requestedOffsets.filter((offset) => offset === 5200)).toHaveLength(2),
        { timeout: 1_000 },
      );
      await waitFor(() =>
        expect(store.getState().playback.currentSong?.songId).toBe(rows[5200]!.song_id),
      );
      await nextPromise;
      expect(store.getState().playback.context?.currentIndex).toBe(5200);
    } finally {
      releaseLike.resolve({ song_id: playing.song_id, liked: true, total: 1 });
      await Promise.allSettled([likePromise, viewportPromise, nextPromise].filter(Boolean));
      await act(async () => store.getState().stopPlayback());
    }
  });

  it('rejects a visible range spanning four pages instead of omitting its last page', async () => {
    const bridge = createMockBridge({ catalogRows: syntheticRows(800) });
    const originalSearch = bridge.searchSongs;
    const requestedOffsets: number[] = [];
    bridge.searchSongs = async (request) => {
      requestedOffsets.push(request.offset);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const before = store.getState().library;
    const requestsBefore = requestedOffsets.length;

    expect(activeLibraryPageOffsets(0, 299, 800)).toEqual([0, 100, 200]);
    expect(() => activeLibraryPageOffsets(0, 300, 800)).toThrow(/spans 4 pages/);
    await act(async () => store.getState().setViewport(0, 300));

    const after = store.getState().library;
    expect(after.error).toMatch(/spans 4 pages/);
    expect(after.visibleRange).toEqual(before.visibleRange);
    expect([...after.pages.keys()]).toEqual([...before.pages.keys()]);
    expect([...after.indexById]).toEqual([...before.indexById]);
    expect(requestedOffsets).toHaveLength(requestsBefore);

    await act(async () => store.getState().setViewport(0, 20));
    expect(store.getState().library.error).toBeNull();
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
    await act(async () => store.getState().search('', { kind: 'smart', id: 'all' }));
    await act(async () => store.getState().search('Song', { kind: 'smart', id: 'all' }));
    await act(async () => store.getState().setViewport(200, 220));
    expect(viewportCalls).toBe(1);
    expect(viewportSongIds).toHaveLength(21);
    expect(store.getState().library.resultTotal).toBe(488);
    expect(store.getState().library.catalogTotal).toBe(500);
    expect(rowAt(store, 200)?.title).toBe('Song 212');
  });

  it('coalesces only identical in-flight viewport hydration requests', async () => {
    const bridge = createMockBridge();
    const originalViewport = bridge.setLibraryViewport;
    const entered = deferred<void>();
    const release = deferred<void>();
    let viewportCalls = 0;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      if (viewportCalls === 1) {
        entered.resolve();
        await release.promise;
      }
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    let visibleRangeChanges = 0;
    store.subscribe(
      (state) => state.library.visibleRange,
      () => {
        visibleRangeChanges += 1;
      },
    );

    const first = store.getState().setViewport(0, 10);
    await entered.promise;
    const overlapping = store.getState().setViewport(0, 10);
    await Promise.resolve();
    expect(viewportCalls).toBe(1);
    release.resolve();
    await Promise.all([first, overlapping]);
    expect(viewportCalls).toBe(1);
    expect(visibleRangeChanges).toBe(1);

    await act(async () => store.getState().setViewport(0, 10));
    expect(viewportCalls).toBe(2);
  });

  it('retries a failed viewport hydration on the next identical request', async () => {
    const bridge = createMockBridge();
    const originalViewport = bridge.setLibraryViewport;
    let viewportCalls = 0;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      if (viewportCalls === 1) throw new Error('temporary viewport bridge failure');
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    await act(async () => store.getState().setViewport(0, 10));
    await act(async () => store.getState().setViewport(0, 10));

    expect(viewportCalls).toBe(2);
  });

  it('retries an incomplete viewport hydration on the next identical request', async () => {
    const bridge = createMockBridge();
    const originalViewport = bridge.setLibraryViewport;
    let viewportCalls = 0;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      if (viewportCalls === 1) {
        return {
          accepted: true,
          generation: request.generation,
          first_index: request.firstIndex,
          last_index: request.lastIndex,
          selected_song_id: request.selectedSongId,
          items: [],
        };
      }
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    await act(async () => store.getState().setViewport(0, 10));
    await act(async () => store.getState().setViewport(0, 10));

    expect(viewportCalls).toBe(2);
  });

  it('retries viewport hydration when a Like mutation overtakes its response', async () => {
    const bridge = createMockBridge();
    const originalViewport = bridge.setLibraryViewport;
    const entered = deferred<void>();
    const release = deferred<void>();
    let viewportCalls = 0;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      if (viewportCalls === 1) {
        const result = await originalViewport(request);
        entered.resolve();
        await release.promise;
        return result;
      }
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');

    let viewport!: Promise<void>;
    await act(async () => {
      viewport = store.getState().setViewport(0, 10);
      await entered.promise;
    });
    await act(async () => store.getState().setSongLiked(first.song_id, true));
    await act(async () => {
      release.resolve();
      await viewport;
    });

    expect(viewportCalls).toBe(2);
    expect(rowAt(store, 0)?.liked).toBe(true);
    expect(store.getState().library.likedTotal).toBe(1);
  });

  it('drops viewport hydration from a source that is no longer active', async () => {
    const bridge = createMockBridge();
    const originalViewport = bridge.setLibraryViewport;
    const entered = deferred<void>();
    const release = deferred<void>();
    let viewportCalls = 0;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      if (viewportCalls === 1) {
        const result = await originalViewport(request);
        entered.resolve();
        await release.promise;
        return {
          ...result,
          items: result.items.map((row) => ({ ...row, title: 'Stale source title' })),
        };
      }
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().setSongLiked(first.song_id, true));

    let staleViewport!: Promise<void>;
    await act(async () => {
      staleViewport = store.getState().setViewport(0, 10);
      await entered.promise;
    });
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    await act(async () => store.getState().setViewport(0, 10));
    release.resolve();
    await act(async () => staleViewport);

    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'liked' });
    expect(rowAt(store, 0)?.title).toBe(first.title);
    expect(rowAt(store, 0)?.liked).toBe(true);
    expect(viewportCalls).toBe(2);
  });

  it('drops an older viewport response after rapid scrolling within one source', async () => {
    const bridge = createMockBridge();
    const originalViewport = bridge.setLibraryViewport;
    const entered = deferred<void>();
    const release = deferred<void>();
    let viewportCalls = 0;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      if (viewportCalls === 1) {
        const result = await originalViewport(request);
        entered.resolve();
        await release.promise;
        return {
          ...result,
          items: result.items.map((row) => ({ ...row, title: 'Obsolete viewport title' })),
        };
      }
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    let firstViewport!: Promise<void>;
    await act(async () => {
      firstViewport = store.getState().setViewport(0, 10);
      await entered.promise;
    });
    await act(async () => store.getState().setViewport(400, 410));
    release.resolve();
    await act(async () => firstViewport);

    expect(viewportCalls).toBe(2);
    expect(rowAt(store, 400)?.title).toBe('Song 401');
    expect(store.getState().library.pages.size).toBeLessThanOrEqual(3);
    expect(store.getState().library.indexById.size).toBeLessThanOrEqual(3 * LIBRARY_PAGE_SIZE);
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

  it('serializes repeated Like changes so the latest mutation owns the result', async () => {
    const bridge = createMockBridge();
    const originalSetLiked = bridge.setSongLiked;
    const entered = deferred<void>();
    const release = deferred<void>();
    const requests: boolean[] = [];
    bridge.setSongLiked = async (request) => {
      requests.push(request.liked);
      const result = await originalSetLiked(request);
      if (requests.length === 1) {
        entered.resolve();
        await release.promise;
      }
      return result;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');

    let addLike!: Promise<void>;
    await act(async () => {
      addLike = store.getState().setSongLiked(first.song_id, true);
      await entered.promise;
    });
    const removeLike = store.getState().setSongLiked(first.song_id, false);
    expect(rowAt(store, 0)?.liked).toBe(false);
    expect(requests).toEqual([true]);

    release.resolve();
    await act(async () => Promise.all([addLike, removeLike]));

    expect(requests).toEqual([true, false]);
    expect(rowAt(store, 0)?.liked).toBe(false);
    expect(store.getState().library.likedTotal).toBe(0);
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
    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'builtin' });
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

  it('keeps the playlist selected while browsing Built-in Songs to add', async () => {
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
    expect(requests).toHaveLength(0);
    expect(store.getState().library.source).toEqual({ kind: 'playlist', id: playlistId });
    expect(store.getState().library.playlistAddMode).toEqual({ playlistId });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'builtin' });
    expect(store.getState().library.resultTotal).toBe(500);
    expect(rowAt(store, 0)?.liked).toBe(true);

    requests.length = 0;
    await act(async () => store.getState().search('Song 1'));
    expect(requests.at(-1)?.source).toEqual({ kind: 'smart', id: 'builtin' });

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

    const first = store.getState().patchSettings({ palette: 'slate' });
    const second = store.getState().patchSettings({ verboseHud: true });
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual({ palette: 'slate' });

    releaseFirst?.();
    await Promise.all([first, second]);
    expect(calls).toEqual([{ palette: 'slate' }, { verboseHud: true }]);
    expect(store.getState().settings?.palette).toBe('slate');
    expect(store.getState().settings?.verbose_hud).toBe(true);
  });

  it('orders a calibration refresh before a later patch even when the refresh is stale', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    const staleSettings = await bridge.getSettings();
    store.setState({ settings: staleSettings, settingsState: 'ready' });

    const refreshResponse = deferred<Settings>();
    const order: string[] = [];
    const originalPatch = bridge.patchSettings;
    bridge.getSettings = async () => {
      order.push('refresh-start');
      const settings = await refreshResponse.promise;
      order.push('refresh-complete');
      return settings;
    };
    bridge.patchSettings = async (patch) => {
      order.push('patch-start');
      return originalPatch(patch);
    };

    act(() => store.getState().applyEvent(successfulCalibrationEvent()));
    const mutation = store.getState().patchSettings({ palette: 'slate' });
    await Promise.resolve();
    if (order.includes('patch-start')) await mutation;

    refreshResponse.resolve(staleSettings);
    await mutation;
    await waitFor(() => expect(order).toContain('refresh-complete'));

    expect(store.getState().settings).toEqual({ ...staleSettings, palette: 'slate' });
    expect(order).toEqual(['refresh-start', 'refresh-complete', 'patch-start']);
    expect(store.getState().settingsState).toBe('ready');
  });

  it('runs a calibration refresh after a settings patch already in flight', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    const initialSettings = await bridge.getSettings();
    store.setState({ settings: initialSettings, settingsState: 'ready' });

    const releasePatch = deferred<void>();
    const patchStarted = deferred<void>();
    const order: string[] = [];
    const originalGetSettings = bridge.getSettings;
    const originalPatch = bridge.patchSettings;
    bridge.patchSettings = async (patch) => {
      order.push('patch-start');
      patchStarted.resolve();
      await releasePatch.promise;
      const settings = await originalPatch(patch);
      order.push('patch-complete');
      return settings;
    };
    bridge.getSettings = async () => {
      order.push('refresh-start');
      const settings = await originalGetSettings();
      order.push('refresh-complete');
      return settings;
    };

    const mutation = store.getState().patchSettings({ verboseHud: true });
    await patchStarted.promise;
    act(() => store.getState().applyEvent(successfulCalibrationEvent()));
    const refreshStartedBeforePatchFinished = order.includes('refresh-start');

    releasePatch.resolve();
    await mutation;
    await waitFor(() =>
      expect(order).toEqual(['patch-start', 'patch-complete', 'refresh-start', 'refresh-complete']),
    );

    expect(refreshStartedBeforePatchFinished).toBe(false);
    expect(store.getState().settings).toEqual({ ...initialSettings, verbose_hud: true });
    expect(store.getState().settingsState).toBe('ready');
  });

  it('handles a rejected calibration refresh without losing settings or poisoning the queue', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    const confirmedSettings = await bridge.getSettings();
    store.setState({ settings: confirmedSettings, settingsState: 'ready' });

    const refreshResponse = deferred<Settings>();
    const unhandledRejections: unknown[] = [];
    const onUnhandledRejection = (reason: unknown) => unhandledRejections.push(reason);
    process.on('unhandledRejection', onUnhandledRejection);
    bridge.getSettings = () => refreshResponse.promise;

    try {
      act(() => store.getState().applyEvent(successfulCalibrationEvent()));
      refreshResponse.reject(new Error('settings GET failed'));
      await new Promise<void>((resolve) => setTimeout(resolve, 0));

      expect(unhandledRejections).toEqual([]);
      expect(store.getState().settings).toEqual(confirmedSettings);
      expect(store.getState().fatal).toBeNull();

      const recovered = await store.getState().patchSettings({ palette: 'slate' });
      expect(recovered?.palette).toBe('slate');
      expect(store.getState().settings?.palette).toBe('slate');
      expect(store.getState().settingsState).toBe('ready');
      expect(store.getState().settingsError).toBeNull();
    } finally {
      process.off('unhandledRejection', onUnhandledRejection);
    }
  });

  it('returns null and preserves cached settings when a settings patch fails', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const originalPatch = bridge.patchSettings;
    const calls: SettingsPatch[] = [];
    bridge.patchSettings = async (patch) => {
      calls.push(patch);
      if (calls.length === 1) throw new Error('settings IPC failed');
      return originalPatch(patch);
    };
    const priorSettings = store.getState().settings;

    const authoritative = await store
      .getState()
      .patchSettings({ playbackDefaults: { timingMarginUs: 900 } });

    expect(authoritative).toBeNull();
    expect(store.getState().settings).toEqual(priorSettings);
    expect(store.getState().settingsState).toBe('ready');
    expect(store.getState().settingsError).toContain('settings IPC failed');
    expect(store.getState().fatal).toBeNull();

    const recovered = await store.getState().patchSettings({ palette: 'slate' });

    expect(calls).toEqual([{ playbackDefaults: { timingMarginUs: 900 } }, { palette: 'slate' }]);
    expect(recovered?.palette).toBe('slate');
    expect(store.getState().settings?.palette).toBe('slate');
    expect(store.getState().settingsError).toBeNull();
    expect(store.getState().settingsState).toBe('ready');
  });

  it('preserves a ready plan across palette and combined palette/Auto Play patches', async () => {
    const store = createDesktopStore(createMockBridge({ startDelayMs: 5 }));
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const prepared = store.getState().playback.prepared;
    const identity = store.getState().playback.preparedIdentity;
    const context = store.getState().playback.preparedContext;
    expect(prepared).not.toBeNull();

    await act(async () => store.getState().patchSettings({ palette: 'slate' }));
    expect(store.getState().playback.prepared).toBe(prepared);
    expect(store.getState().playback.preparedIdentity).toBe(identity);
    expect(store.getState().playback.preparedContext).toBe(context);

    await act(async () => store.getState().patchSettings({ palette: 'aurora', autoPlay: false }));
    expect(store.getState().playback.prepared).toBe(prepared);
    expect(store.getState().playback.preparedIdentity).toBe(identity);
    expect(store.getState().playback.preparedContext).toBe(context);

    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  });

  it('discards an in-flight prepare when the catalog generation changes', async () => {
    const bridge = createMockBridge();
    const originalPrepare = bridge.preparePlayback;
    const entered = deferred<void>();
    const release = deferred<void>();
    bridge.preparePlayback = async (request) => {
      const prepared = await originalPrepare(request);
      entered.resolve();
      await release.promise;
      return prepared;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    const preparation = store.getState().prepareSelectedPlayback();
    await entered.promise;
    expect(store.getState().playback.transportOperation).toBe('preparing');

    act(() =>
      store.getState().applyEvent({
        v: 1,
        name: 'catalog.changed',
        payload: {
          generation: store.getState().library.generation + 1,
          total: store.getState().library.catalogTotal,
        },
      }),
    );
    expect(store.getState().playback.transportOperation).toBeNull();
    expect(store.getState().playback.prepared).toBeNull();

    release.resolve();
    await act(async () => preparation);
    expect(store.getState().playback.transportOperation).toBeNull();
    expect(store.getState().playback.prepared).toBeNull();
    expect(store.getState().playback.preparedIdentity).toBeNull();
    expect(store.getState().playback.preparedContext).toBeNull();
  });

  it('keeps an in-flight prepare current through a palette patch and accepts its late response', async () => {
    const bridge = createMockBridge({ startDelayMs: 5 });
    const originalPrepare = bridge.preparePlayback;
    const entered = deferred<void>();
    const release = deferred<void>();
    bridge.preparePlayback = async (request) => {
      const prepared = await originalPrepare(request);
      entered.resolve();
      await release.promise;
      return prepared;
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    const preparation = store.getState().prepareSelectedPlayback();
    await entered.promise;
    expect(store.getState().playback.transportOperation).toBe('preparing');
    await act(async () => store.getState().patchSettings({ palette: 'slate' }));
    expect(store.getState().playback.transportOperation).toBe('preparing');
    expect(store.getState().playback.prepared).toBeNull();

    release.resolve();
    await act(async () => preparation);
    expect(store.getState().playback.transportOperation).toBeNull();
    expect(store.getState().playback.prepared?.song.song_id).toBe(first.song_id);
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  });

  it('preserves an existing plan when a settings patch is rejected', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const prepared = store.getState().playback.prepared;
    const identity = store.getState().playback.preparedIdentity;
    const context = store.getState().playback.preparedContext;
    bridge.patchSettings = async () => {
      throw new Error('settings IPC rejected');
    };

    await act(async () => store.getState().patchSettings({ palette: 'slate' }));

    expect(store.getState().settingsError).toContain('settings IPC rejected');
    expect(store.getState().playback.prepared).toBe(prepared);
    expect(store.getState().playback.preparedIdentity).toBe(identity);
    expect(store.getState().playback.preparedContext).toBe(context);
  });

  it('invalidates preparation for a queued mixed palette and timing patch', async () => {
    const bridge = createMockBridge();
    const originalPatch = bridge.patchSettings;
    const firstPatchStarted = deferred<void>();
    const releaseFirstPatch = deferred<void>();
    let patchCount = 0;
    bridge.patchSettings = async (patch) => {
      patchCount += 1;
      if (patchCount === 1) {
        firstPatchStarted.resolve();
        await releaseFirstPatch.promise;
      }
      return originalPatch(patch);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    expect(store.getState().playback.prepared).not.toBeNull();

    const palettePatch = store.getState().patchSettings({ palette: 'slate' });
    await firstPatchStarted.promise;
    const mixedPatch = store
      .getState()
      .patchSettings({ palette: 'aurora', playbackDefaults: { tempoScale: 0.95 } });
    releaseFirstPatch.resolve();
    await Promise.all([palettePatch, mixedPatch]);

    expect(patchCount).toBe(2);
    expect(store.getState().settings?.palette).toBe('aurora');
    expect(store.getState().settings?.playback_defaults.tempo_scale).toBe(0.95);
    expect(store.getState().playback.prepared).toBeNull();
    expect(store.getState().playback.preparedIdentity).toBeNull();
    expect(store.getState().playback.preparedContext).toBeNull();
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
      playbackDurationMs: 150,
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
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'), { timeout: 3000 });
    await waitFor(() => expect(store.getState().playback.currentSong?.songId).toBe(first.song_id), {
      timeout: 3000,
    });

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

  it('does not let an old session playing event clear explicit Next', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 60_000, startDelayMs: 1 });
    const originalSubscribe = bridge.subscribeUiEvents;
    const originalSkip = bridge.skipPlayback;
    let forwardEvent: ((event: UiEvent) => void) | undefined;
    let resolveSkipCalled!: () => void;
    const skipCalled = new Promise<void>((resolve) => {
      resolveSkipCalled = resolve;
    });
    let releaseSkip!: () => void;
    const skipRelease = new Promise<void>((resolve) => {
      releaseSkip = resolve;
    });
    bridge.subscribeUiEvents = async (listener) => {
      forwardEvent = listener;
      return originalSubscribe(listener);
    };
    bridge.skipPlayback = async (request) => {
      resolveSkipCalled();
      await skipRelease;
      return originalSkip(request);
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = rowAt(store, 0);
    const nextSong = rowAt(store, 1);
    if (!first || !nextSong) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().patchSettings({ autoPlay: false }));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    const oldSession = store.getState().playback.sessionId;
    if (!oldSession) throw new Error('mock session did not start');

    const nextPromise = store.getState().nextPlayback();
    await skipCalled;
    expect(store.getState().playback.transportOperation).toBe('advancing');

    forwardEvent?.({
      v: 1,
      name: 'playback.state_changed',
      payload: {
        session_id: oldSession,
        song_id: first.song_id,
        state: 'playing',
        physical: false,
        message: null,
        outcome: null,
      },
    });
    expect(store.getState().playback.transportOperation).toBe('advancing');

    forwardEvent?.({
      v: 1,
      name: 'playback.snapshot',
      payload: {
        session_id: oldSession,
        seq: 99,
        state: 'playing',
        song_id: first.song_id,
        title: first.title,
        current_us: 1_000,
        total_us: first.duration_us ?? 1_000_000,
        pre_roll_remaining_us: 0,
        focus_state: 'focused',
        health: 'healthy',
        input_path_degraded: false,
        message: null,
      },
    });
    expect(store.getState().playback.transportOperation).toBe('advancing');

    releaseSkip();
    await nextPromise;
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    expect(store.getState().playback.currentSong?.songId).toBe(nextSong.song_id);
    expect(store.getState().playback.sessionId).not.toBe(oldSession);
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
    const first = rowAt(store, 99);
    if (!first) throw new Error('last row in first page was not loaded');
    await act(async () => store.getState().selectSong(first.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback());
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));

    await act(async () => store.getState().nextPlayback());

    expect(searchOffsets).toContain(100);
    expect(store.getState().playback.preparedIdentity?.title).toBe('Song 101');
    expect(store.getState().playback.prepared?.admission).toBe('confirmation_required');
    expect(store.getState().playback.sessionId).toBeNull();
    await act(async () => store.getState().startPreparedPlayback('proceed'));
    expect(store.getState().playback.currentSong?.title).toBe('Song 101');
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  });

  it('auto-plays across a Built-in Songs page boundary using the full source context', async () => {
    const rows = syntheticRows(500).map((row) => ({ ...row, risk_level: 'low' as const }));
    const requests: SearchRequest[] = [];
    const bridge = createMockBridge({
      catalogRows: rows,
      builtinSongIds: rows.map((row) => row.song_id),
      playbackDurationMs: 120,
      startDelayMs: 5,
    });
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      requests.push(request);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    await act(async () => store.getState().setViewport(99, 99));

    const selected = rowAt(store, 99);
    if (!selected) throw new Error('last row in the first Built-in Songs page was not loaded');
    await act(async () => store.getState().selectSong(selected.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    await act(async () => store.getState().startPreparedPlayback('proceed'));

    await waitFor(() => {
      expect(store.getState().playback.state).toBe('playing');
      expect(store.getState().playback.currentSong?.songId).toBe(rows[100]?.song_id);
    });

    const context = store.getState().playback.context;
    expect(context?.source).toEqual({ kind: 'smart', id: 'builtin' });
    expect(context?.query).toBe('');
    expect(context?.total).toBe(500);
    expect(context?.currentIndex).toBe(100);
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'builtin' });
    expect(store.getState().library.resultTotal).toBe(500);
    expect(store.getState().library.selectedSongId).toBe(selected.song_id);
    expect(store.getState().library.selectedSongSummary?.song_id).toBe(selected.song_id);
    expect(
      requests.some(
        (request) =>
          request.source.kind === 'smart' &&
          request.source.id === 'builtin' &&
          request.offset === 100 &&
          request.limit === 100,
      ),
    ).toBe(true);
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
    await act(async () => store.getState().setViewport(99, 100));
    await waitFor(() => expect(rowAt(store, 100)).toBeDefined());
    const first = rowAt(store, 99);
    const next = rowAt(store, 100);
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
        terminal_release_verified: null,
        release_obligation_mask: null,
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

  it('restores utility open state on Details and persists only the open state', async () => {
    window.localStorage.setItem(
      WORKBENCH_STORAGE_KEY,
      JSON.stringify({
        version: 5,
        navigatorPreference: 'expanded',
        expandedNavigatorWidth: 300,
        utilityWidth: 360,
        utilityOpen: true,
      }),
    );
    const store = createDesktopStore(createMockBridge());
    expect(store.getState().utility).toEqual({ open: true, activeView: 'details' });
    expect(store.getState().diagnostics.enabled).toBe(false);

    const setItem = vi.spyOn(Storage.prototype, 'setItem');
    store.getState().openUtility('details');
    expect(JSON.parse(window.localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      utilityOpen: true,
    });
    store.getState().closeUtility();
    expect(JSON.parse(window.localStorage.getItem(WORKBENCH_STORAGE_KEY)!)).toMatchObject({
      utilityOpen: false,
    });
    store.getState().toggleUtility();
    expect(store.getState().utility.open).toBe(true);
    expect(setItem).toHaveBeenCalledTimes(3);

    store.getState().setUtilityView('diagnostics');
    await waitFor(() => expect(store.getState().diagnostics.enabled).toBe(true));
    expect(setItem).toHaveBeenCalledTimes(3);

    const freshStore = createDesktopStore(createMockBridge());
    expect(freshStore.getState().utility).toEqual({ open: true, activeView: 'details' });
    expect(freshStore.getState().diagnostics.enabled).toBe(false);
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

describe('desktop update UX and native authority', () => {
  it('manual check opens dialog, calls bridge with manual origin, and presents Current terminal result', async () => {
    const bridge = createMockBridge({
      updateCheckResult: {
        state: 'current',
        current_version: '4.1.0',
        available_version: null,
        channel: 'stable',
        release_notes: null,
        published_at: null,
        error: null,
      },
    });
    const checkSpy = vi.spyOn(bridge, 'checkForUpdate');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    checkSpy.mockClear();
    await act(async () => store.getState().checkForUpdate('manual'));

    expect(checkSpy).toHaveBeenCalledWith({ origin: 'manual' });
    const update = store.getState().update;
    expect(update.dialogOpen).toBe(true);
    expect(update.state).toBe('current');
    expect(update.currentVersion).toBe('4.1.0');
    expect(update.availableVersion).toBeNull();
  });

  it('native updater error emits snapshot, surfaces error code and detail, and updates native error timestamp', async () => {
    const bridge = createMockBridge({
      updateCheckResult: {
        state: 'error',
        current_version: '4.1.0',
        available_version: null,
        channel: 'stable',
        release_notes: null,
        published_at: null,
        error: 'network_timeout: failed to fetch release metadata',
      },
    });
    const prefsSpy = vi.spyOn(bridge, 'getUpdatePreferences');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    prefsSpy.mockClear();

    await act(async () => store.getState().checkForUpdate('manual'));

    const update = store.getState().update;
    expect(update.state).toBe('error');
    expect(update.dialogOpen).toBe(true);
    expect(update.errorCode).toBe('check_failed');
    expect(update.errorDetail).toBe('network_timeout: failed to fetch release metadata');
    expect(update.retryAction).toBe('check');

    // Frontend does not query getUpdatePreferences() post-check
    expect(prefsSpy).not.toHaveBeenCalled();

    // Native authority updated its preferences timestamp
    const nativePrefs = await bridge.getUpdatePreferences();
    expect(nativePrefs.last_error_ts).toBeGreaterThan(0);
    expect(nativePrefs.last_check_ts).toBe(0);
  });

  it('thrown bridge/IPC failure records transportError without inventing timestamps or corrupting native state', async () => {
    const bridge = createMockBridge({
      updateCheckTransportError: 'ipc_connection_failed: backend process terminated',
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const initialPrefs = { ...store.getState().settings?.update_preferences };

    await act(async () => store.getState().checkForUpdate('manual'));

    const update = store.getState().update;
    expect(update.checkRequestPending).toBe(false);
    expect(update.transportError).toBe('ipc_connection_failed: backend process terminated');
    expect(update.transportErrorAction).toBe('check');
    expect(update.dialogOpen).toBe(true);

    // Timestamps are not invented on IPC exception
    const currentPrefs = store.getState().settings?.update_preferences;
    expect(currentPrefs?.last_check_ts).toBe(initialPrefs.last_check_ts);
    const nativePrefs = await bridge.getUpdatePreferences();
    expect(nativePrefs.last_check_ts).toBe(0);
    expect(nativePrefs.last_error_ts).toBe(0);
  });

  it('background check sends origin background to native bridge', async () => {
    const bridge = createMockBridge();
    const checkSpy = vi.spyOn(bridge, 'checkForUpdate');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    checkSpy.mockClear();
    await act(async () => store.getState().checkForUpdate('background'));

    expect(checkSpy).toHaveBeenCalledWith({ origin: 'background' });
  });

  it('background check runs silently and surfaces passive available indicator without modal dialog', async () => {
    const bridge = createMockBridge({
      updateCheckResult: {
        state: 'available',
        current_version: '4.1.0',
        available_version: '4.2.0',
        channel: 'stable',
        release_notes: 'New release notes',
        published_at: '2026-09-01T00:00:00Z',
        error: null,
      },
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    expect(store.getState().update.dialogOpen).toBe(false);

    await act(async () => store.getState().checkForUpdate('background'));

    const update = store.getState().update;
    expect(update.dialogOpen).toBe(false); // Non-disruptive: dialog does not open
    expect(update.state).toBe('available');
    expect(update.availableVersion).toBe('4.2.0');
  });

  it('background check with transient IPC error records transportError and does not open dialog', async () => {
    const bridge = createMockBridge({
      updateCheckTransportError: 'fetch_failed: server returned 503',
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    expect(store.getState().update.dialogOpen).toBe(false);

    await act(async () => store.getState().checkForUpdate('background'));

    const update = store.getState().update;
    expect(update.dialogOpen).toBe(false);
    expect(update.transportError).toBe('fetch_failed: server returned 503');
    expect(update.transportErrorAction).toBe('check');
    expect(update.checkRequestPending).toBe(false);
  });

  it('thrown bridge/IPC failure during install handoff records transportError with transportErrorAction install', async () => {
    const bridge = createMockBridge({
      beginUpdateTransportError: 'ipc_connection_failed: install handoff timed out',
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    act(() => {
      store.setState({
        update: {
          ...store.getState().update,
          state: 'available',
          availableVersion: '4.2.0',
        },
      });
    });

    await act(async () => store.getState().beginUpdateHandoff());

    const update = store.getState().update;
    expect(update.installRequestPending).toBe(false);
    expect(update.transportError).toBe('ipc_connection_failed: install handoff timed out');
    expect(update.transportErrorAction).toBe('install');
    expect(update.state).toBe('available');
    expect(update.availableVersion).toBe('4.2.0');
  });

  it('translates physical playback rejection during install handoff to authoritative error state without transportError', async () => {
    const bridge = createMockBridge({
      beginUpdateProductError: {
        code: 'playback_active',
        detail: 'playback_active: update installation cannot run during physical playback',
        retryAction: 'install',
        preserveCandidate: true,
      },
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    act(() => {
      store.setState({
        update: {
          ...store.getState().update,
          state: 'available',
          availableVersion: '4.2.0',
        },
      });
    });

    await act(async () => store.getState().beginUpdateHandoff());

    const update = store.getState().update;
    expect(update.state).toBe('error');
    expect(update.errorCode).toBe('playback_active');
    expect(update.retryAction).toBe('install');
    expect(update.transportError).toBeNull();
    expect(update.transportErrorAction).toBeNull();
    expect(update.installRequestPending).toBe(false);
  });

  it('translates calibration rejection during install handoff to authoritative error state without transportError', async () => {
    const bridge = createMockBridge({
      beginUpdateProductError: {
        code: 'calibration_active',
        detail: 'calibration_active: update installation cannot run during calibration',
        retryAction: 'install',
        preserveCandidate: true,
      },
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    act(() => {
      store.setState({
        update: {
          ...store.getState().update,
          state: 'available',
          availableVersion: '4.2.0',
        },
      });
    });

    await act(async () => store.getState().beginUpdateHandoff());

    const update = store.getState().update;
    expect(update.state).toBe('error');
    expect(update.errorCode).toBe('calibration_active');
    expect(update.retryAction).toBe('install');
    expect(update.transportError).toBeNull();
    expect(update.transportErrorAction).toBeNull();
    expect(update.installRequestPending).toBe(false);
  });

  it('discards out-of-order update.changed events by monotonic revision', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    act(() => {
      store.getState().applyEvent({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: 2,
          state: 'available',
          current_version: '4.0.0',
          available_version: '4.2.0',
          channel: 'stable',
          release_notes: 'New release notes v2',
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: null,
          progress: null,
        },
      });
    });

    expect(store.getState().update.lastNativeRevision).toBe(2);
    expect(store.getState().update.availableVersion).toBe('4.2.0');

    // Stale event with revision: 1 should be discarded even with v: 1
    act(() => {
      store.getState().applyEvent({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: 1,
          state: 'available',
          current_version: '4.0.0',
          available_version: '4.1.0',
          channel: 'stable',
          release_notes: 'Old release notes v1',
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: null,
          progress: null,
        },
      });
    });

    expect(store.getState().update.lastNativeRevision).toBe(2);
    expect(store.getState().update.availableVersion).toBe('4.2.0');
    expect(store.getState().update.releaseNotes).toBe('New release notes v2');
  });

  it('accepts monotonic revision progression with constant protocol version v=1', () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);

    act(() => {
      store.getState().applyEvent({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: 1,
          state: 'checking',
          current_version: '4.0.0',
          available_version: null,
          channel: 'stable',
          release_notes: null,
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: null,
          progress: null,
        },
      });
    });
    expect(store.getState().update.lastNativeRevision).toBe(1);
    expect(store.getState().update.state).toBe('checking');

    act(() => {
      store.getState().applyEvent({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: 2,
          state: 'available',
          current_version: '4.0.0',
          available_version: '4.1.0',
          channel: 'stable',
          release_notes: 'Update notes',
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: null,
          progress: null,
        },
      });
    });
    expect(store.getState().update.lastNativeRevision).toBe(2);
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.1.0');
  });

  it('copies native cumulative update progress without adding chunks in the frontend', () => {
    const store = createDesktopStore(createMockBridge());
    const completed = [16_000, 32_000, 40_000];

    completed.forEach((value, index) => {
      store.getState().applyEvent({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: index + 1,
          state: 'downloading',
          current_version: '4.0.0',
          available_version: '4.1.0',
          channel: 'stable',
          release_notes: null,
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: '0123456789abcdef0123456789abcdef',
          progress: { completed: value, total: 40_000, message: 'Downloading update' },
        },
      });
      expect(store.getState().update.progress?.completed).toBe(value);
    });
  });

  it('does not emit update.changed when background check is disabled by auto_check=false', async () => {
    const bridge = createMockBridge();
    await bridge.patchUpdatePreferences({ autoCheck: false });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const initialRevision = store.getState().update.lastNativeRevision;
    const initialAvailableVersion = store.getState().update.availableVersion;

    await act(async () => store.getState().checkForUpdate('background'));

    expect(store.getState().update.lastNativeRevision).toBe(initialRevision);
    expect(store.getState().update.availableVersion).toBe(initialAvailableVersion);
    expect(store.getState().update.transportError).toBeNull();
  });

  it('update.changed atomically updates native fields while preserving local dialog state', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    act(() => {
      store.setState({
        update: {
          ...store.getState().update,
          dialogOpen: true,
          checkRequestPending: true,
          transportError: 'prior transport error',
        },
      });
    });

    act(() => {
      store.getState().applyEvent({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: 5,
          state: 'available',
          current_version: '4.0.0',
          available_version: '4.3.0',
          channel: 'stable',
          release_notes: 'Version 4.3.0',
          published_at: '2026-09-18T00:00:00Z',
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: null,
          progress: null,
        },
      });
    });

    const update = store.getState().update;
    expect(update.dialogOpen).toBe(true);
    expect(update.checkRequestPending).toBe(true);
    expect(update.transportError).toBeNull(); // Cleared by authoritative event
    expect(update.lastNativeRevision).toBe(5);
    expect(update.availableVersion).toBe('4.3.0');
    expect(update.state).toBe('available');
  });

  it('preserves newer native snapshot and suppresses transportError when checkForUpdate rejects late after update.changed', async () => {
    const bridge = createMockBridge();
    let listener: ((event: import('../bridge/DesktopBridge').UiEvent) => void) | undefined;
    const originalSubscribe = bridge.subscribeUiEvents;
    bridge.subscribeUiEvents = async (next) => {
      listener = next;
      return originalSubscribe(next);
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    bridge.checkForUpdate = async () => {
      const rev = store.getState().update.lastNativeRevision + 1;
      listener?.({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: rev,
          state: 'available',
          current_version: '4.0.0',
          available_version: '4.2.0',
          channel: 'stable',
          release_notes: 'Version 4.2.0 release notes',
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: null,
          progress: null,
        },
      });
      throw new Error('ipc_timeout: connection dropped');
    };

    await act(async () => store.getState().checkForUpdate('manual'));

    const update = store.getState().update;
    expect(update.state).toBe('available');
    expect(update.availableVersion).toBe('4.2.0');
    expect(update.releaseNotes).toBe('Version 4.2.0 release notes');
    expect(update.transportError).toBeNull();
    expect(update.transportErrorAction).toBeNull();
    expect(update.checkRequestPending).toBe(false);
  });

  it('preserves newer native snapshot and suppresses transportError when beginUpdateHandoff rejects late after update.changed', async () => {
    const bridge = createMockBridge();
    let listener: ((event: import('../bridge/DesktopBridge').UiEvent) => void) | undefined;
    const originalSubscribe = bridge.subscribeUiEvents;
    bridge.subscribeUiEvents = async (next) => {
      listener = next;
      return originalSubscribe(next);
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    act(() => {
      store.setState({
        update: {
          ...store.getState().update,
          lastNativeRevision: 2,
          state: 'available',
          availableVersion: '4.2.0',
        },
      });
    });

    bridge.beginUpdateHandoff = async () => {
      listener?.({
        v: 1,
        name: 'update.changed',
        payload: {
          revision: 3,
          state: 'downloading',
          current_version: '4.0.0',
          available_version: '4.2.0',
          channel: 'stable',
          release_notes: 'Version 4.2.0',
          published_at: null,
          error_code: null,
          error_detail: null,
          retry_action: 'none',
          operation_id: 'op-download-123',
          progress: { completed: 50, total: 100, message: 'Downloading' },
        },
      });
      throw new Error('ipc_channel_closed');
    };

    await act(async () => store.getState().beginUpdateHandoff());

    const update = store.getState().update;
    expect(update.lastNativeRevision).toBe(3);
    expect(update.state).toBe('downloading');
    expect(update.handoffId).toBe('op-download-123');
    expect(update.progress?.completed).toBe(50);
    expect(update.transportError).toBeNull();
    expect(update.transportErrorAction).toBeNull();
    expect(update.installRequestPending).toBe(false);
  });

  it('exercises product install errors via beginUpdateHandoff preserving or clearing candidate metadata according to error type', async () => {
    const testCases: Array<{
      code: import('../bridge/DesktopBridge').UpdateErrorCode;
      detail: string;
      retryAction: import('../bridge/DesktopBridge').UpdateRetryAction;
      preserveCandidate: boolean;
    }> = [
      {
        code: 'download_failed',
        detail: 'Checksum mismatch during download',
        retryAction: 'install',
        preserveCandidate: true,
      },
      {
        code: 'install_failed',
        detail: 'Installer execution exited with code 1',
        retryAction: 'install',
        preserveCandidate: true,
      },
      {
        code: 'stale_update',
        detail: 'Update candidate is no longer valid',
        retryAction: 'check',
        preserveCandidate: false,
      },
      {
        code: 'update_unavailable',
        detail: 'Release assets removed from mirror',
        retryAction: 'check',
        preserveCandidate: false,
      },
    ];

    for (const tc of testCases) {
      const bridge = createMockBridge({
        updateSnapshot: {
          state: 'available',
          current_version: '4.0.0',
          available_version: '4.2.0',
          channel: 'stable',
          release_notes: 'Release notes v4.2.0',
          published_at: '2026-09-01T00:00:00Z',
        },
        beginUpdateProductError: {
          code: tc.code,
          detail: tc.detail,
          retryAction: tc.retryAction,
          preserveCandidate: tc.preserveCandidate,
        },
      });
      const store = createDesktopStore(bridge);
      await act(async () => store.getState().initialize());

      expect(store.getState().update.state).toBe('available');
      expect(store.getState().update.availableVersion).toBe('4.2.0');

      await act(async () => store.getState().beginUpdateHandoff());

      const update = store.getState().update;
      expect(update.state).toBe('error');
      expect(update.errorCode).toBe(tc.code);
      expect(update.errorDetail).toBe(tc.detail);
      expect(update.retryAction).toBe(tc.retryAction);
      expect(update.transportError).toBeNull();
      expect(update.transportErrorAction).toBeNull();
      expect(update.installRequestPending).toBe(false);

      if (tc.preserveCandidate) {
        expect(update.availableVersion).toBe('4.2.0');
        expect(update.releaseNotes).toBe('Release notes v4.2.0');
        expect(update.publishedAt).toBe('2026-09-01T00:00:00Z');
      } else {
        expect(update.availableVersion).toBeNull();
        expect(update.releaseNotes).toBeNull();
        expect(update.publishedAt).toBeNull();
      }
    }
  });

  it('mock candidate invalidation matrix for generic patchSettings', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const setAvailable = async () => {
      await act(async () => {
        await store.getState().checkForUpdate('manual');
      });
      expect(store.getState().update.state).toBe('available');
      expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');
    };

    // 1. auto_check change => remains Available
    await setAvailable();
    await act(async () => {
      await store.getState().patchSettings({
        updatePreferences: { autoCheck: false },
      });
    });
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');

    // 2. Stable -> Stable no-op => remains Available
    await act(async () => {
      await store.getState().patchSettings({
        updatePreferences: { channel: 'stable' },
      });
    });
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');

    // 3. skip X -> X no-op => remains Available
    await act(async () => {
      await store.getState().patchSettings({
        updatePreferences: { skipVersion: '4.1.0' },
      });
    });
    expect(store.getState().update.state).toBe('idle');
    await setAvailable();
    await act(async () => {
      await store.getState().patchSettings({
        updatePreferences: { skipVersion: '4.1.0' },
      });
    });
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');

    // 4. actual skip change => Idle and candidate cleared
    await act(async () => {
      await store.getState().patchSettings({
        updatePreferences: { skipVersion: '4.2.0' },
      });
    });
    expect(store.getState().update.state).toBe('idle');
    expect(store.getState().update.availableVersion).toBeNull();
  });

  it('mock candidate invalidation matrix for dedicated patchUpdatePreferences', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const setAvailable = async () => {
      await act(async () => {
        await store.getState().checkForUpdate('manual');
      });
      expect(store.getState().update.state).toBe('available');
      expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');
    };

    // 1. auto_check change => remains Available
    await setAvailable();
    await act(async () => {
      await bridge.patchUpdatePreferences({ autoCheck: false });
    });
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');

    // 2. Stable -> Stable no-op => remains Available
    await act(async () => {
      await bridge.patchUpdatePreferences({ channel: 'stable' });
    });
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');

    // 3. skip X -> X no-op => remains Available
    await act(async () => {
      await bridge.patchUpdatePreferences({ skipVersion: '4.1.0' });
    });
    expect(store.getState().update.state).toBe('idle');
    await setAvailable();
    await act(async () => {
      await bridge.patchUpdatePreferences({ skipVersion: '4.1.0' });
    });
    expect(store.getState().update.state).toBe('available');
    expect(store.getState().update.availableVersion).toBe('4.0.0-alpha.2-mock');

    // 4. actual skip change => Idle and candidate cleared
    await act(async () => {
      await bridge.patchUpdatePreferences({ skipVersion: '4.2.0' });
    });
    expect(store.getState().update.state).toBe('idle');
    expect(store.getState().update.availableVersion).toBeNull();
  });

  it('delegates background check to bridge on startup respecting auto_check=false', async () => {
    const bridge = createMockBridge();
    await bridge.patchUpdatePreferences({ autoCheck: false });
    const checkSpy = vi.spyOn(bridge, 'checkForUpdate');
    const store = createDesktopStore(bridge);

    const initialRevision = store.getState().update.lastNativeRevision;
    await act(async () => store.getState().initialize());

    expect(checkSpy).toHaveBeenCalledTimes(1);
    expect(checkSpy).toHaveBeenCalledWith({ origin: 'background' });
    expect(store.getState().update.lastNativeRevision).toBe(initialRevision);
    expect(store.getState().update.dialogOpen).toBe(false);
    expect(store.getState().update.transportError).toBeNull();
  });

  it('tracks complete install sequence Downloading -> Ready -> Installing with v=1, monotonic revisions, matching operation ID, and candidate metadata', async () => {
    const bridge = createMockBridge();
    const emittedEvents: Array<import('../bridge/DesktopBridge').UiEvent> = [];
    await bridge.subscribeUiEvents((event) => {
      emittedEvents.push(event);
    });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    // Prepare an available update
    await act(async () => store.getState().checkForUpdate('manual'));
    expect(store.getState().update.state).toBe('available');
    const availableVersion = store.getState().update.availableVersion;
    expect(availableVersion).toBe('4.0.0-alpha.2-mock');

    emittedEvents.length = 0; // Clear check events
    await act(async () => store.getState().beginUpdateHandoff());

    const updateEvents = emittedEvents.filter(
      (e): e is Extract<import('../bridge/DesktopBridge').UiEvent, { name: 'update.changed' }> =>
        e.name === 'update.changed',
    );
    expect(updateEvents).toHaveLength(3);

    const downloading = updateEvents[0];
    const ready = updateEvents[1];
    const installing = updateEvents[2];
    if (!downloading || !ready || !installing) {
      throw new Error('Expected 3 update events');
    }

    // Protocol version v = 1 on all events
    expect(downloading.v).toBe(1);
    expect(ready.v).toBe(1);
    expect(installing.v).toBe(1);

    // States in order
    expect(downloading.payload.state).toBe('downloading');
    expect(ready.payload.state).toBe('ready');
    expect(installing.payload.state).toBe('installing');

    // Strictly monotonic revisions
    expect(downloading.payload.revision).toBeLessThan(ready.payload.revision);
    expect(ready.payload.revision).toBeLessThan(installing.payload.revision);

    // Matching operation ID
    const operationId = downloading.payload.operation_id;
    expect(operationId).toBeTruthy();
    expect(ready.payload.operation_id).toBe(operationId);
    expect(installing.payload.operation_id).toBe(operationId);

    // Preserved candidate metadata across all stages
    for (const evt of [downloading, ready, installing]) {
      expect(evt.payload.available_version).toBe(availableVersion);
      expect(evt.payload.channel).toBe('stable');
      expect(evt.payload.release_notes).toBe('A deterministic update fixture for the desktop UI.');
      expect(evt.payload.published_at).toBe('2026-08-30T00:00:00Z');
      expect(evt.payload.error_code).toBeNull();
      expect(evt.payload.retry_action).toBe('none');
    }

    // Final store state reflects installing
    const finalUpdate = store.getState().update;
    expect(finalUpdate.state).toBe('installing');
    expect(finalUpdate.handoffId).toBe(operationId);
    expect(finalUpdate.availableVersion).toBe(availableVersion);
    expect(finalUpdate.transportError).toBeNull();
    expect(finalUpdate.installRequestPending).toBe(false);
  });
});
