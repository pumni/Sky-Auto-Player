import type {
  DesktopBridge,
  LibraryNavigation,
  LibrarySource,
  SearchRequest,
  SearchResult,
  SongRow,
} from '../bridge/DesktopBridge';
import { recordStartupTelemetry } from '../bridge/startupTelemetry';
import { selectRowAtIndex, selectSongById } from './selectors';
import {
  cacheDetail,
  navigationFromDto,
  sourceKey,
  updateLoadedRows,
  updatePlaylistSummary,
} from './libraryHelpers';
import { LIBRARY_PAGE_SIZE } from './types';
import type { DetailEntry, DesktopStore } from './types';

const MAX_CATALOG_RECONCILIATION_ATTEMPTS = 3;

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface LibrarySliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  clearOperationFromEvent: () => void;
  invalidatePlaybackMembershipContext: (source: LibrarySource) => void;
  playbackAfterBrowserNavigation: (playback: DesktopStore['playback']) => DesktopStore['playback'];
  nextDetailRequestToken: () => number;
  isCurrentDetailRequest: (token: number) => boolean;
}

export type LibrarySliceActions = Pick<
  DesktopStore,
  | 'search'
  | 'selectSong'
  | 'setViewport'
  | 'selectLibrarySource'
  | 'openPlaylistAdd'
  | 'exitPlaylistAdd'
  | 'loadLibraryNavigation'
  | 'createPlaylist'
  | 'renamePlaylist'
  | 'deletePlaylist'
  | 'importLocalFilesToPlaylist'
  | 'importLocalFolderToPlaylist'
  | 'addSongToPlaylist'
  | 'removeSongFromPlaylist'
  | 'setSongLiked'
  | 'reloadLibrary'
>;

export interface LibrarySlice {
  actions: LibrarySliceActions;
  reconcileCatalog: () => Promise<void>;
  syncCatalogGeneration: (generation: number) => Promise<void>;
  loadPage: (
    query: string,
    source: LibrarySource,
    offset: number,
    generation: number,
    token: number,
    mergeIntoLibrary?: boolean,
  ) => Promise<SearchResult>;
}

export function createLibrarySlice(context: LibrarySliceContext): LibrarySlice {
  const { bridge, get, set } = context;
  const pageSize = LIBRARY_PAGE_SIZE;
  const pageCache = new Map<string, Map<number, SearchResult>>();
  const pageRequests = new Map<string, Promise<SearchResult>>();
  let catalogReconciliationTail: Promise<void> = Promise.resolve();
  let catalogReadyRecorded = false;

  const cacheKey = (source: LibrarySource, query: string, generation: number) =>
    `${generation}\u0000${sourceKey(source)}\u0000${query}`;

  const invalidateSourceCache = (source: LibrarySource): void => {
    const marker = `\u0000${sourceKey(source)}\u0000`;
    for (const key of pageCache.keys()) {
      if (key.includes(marker)) pageCache.delete(key);
    }
    for (const key of pageRequests.keys()) {
      if (key.includes(marker)) pageRequests.delete(key);
    }
  };

  const setLibraryMutationPending = (key: string, pending: boolean): boolean => {
    const current = get().libraryNavigation;
    if (pending && current.pendingMutations.has(key)) return false;
    const nextPending = new Set(current.pendingMutations);
    if (pending) nextPending.add(key);
    else nextPending.delete(key);
    set({
      libraryNavigation: {
        ...current,
        pendingMutations: nextPending,
        lastError: current.lastError,
      },
    });
    return true;
  };

  const setLibraryError = (message: string | null) => {
    set({
      libraryNavigation: { ...get().libraryNavigation, lastError: message },
    });
  };

  const errorMessage = (error: unknown): string =>
    error instanceof Error ? error.message : String(error);

  const updateNavigation = (navigation: LibraryNavigation) => {
    const pendingMutations = get().libraryNavigation.pendingMutations;
    set({
      libraryNavigation: {
        ...navigationFromDto(navigation),
        pendingMutations: new Set(pendingMutations),
      },
    });
  };

  const mergePage = (result: SearchResult, token: number): boolean => {
    const current = get().library;
    if (current.searchRequestGeneration !== token || current.generation !== result.generation) {
      return false;
    }
    const pages = new Map(current.pages);
    const indexById = new Map(current.indexById);
    pages.set(result.offset, result.items);
    result.items.forEach((row, index) => {
      indexById.set(row.song_id, result.offset + index);
    });
    set({
      library: {
        ...current,
        pages,
        indexById,
        catalogTotal:
          current.searchSource.kind === 'smart' &&
          current.searchSource.id === 'all' &&
          current.query.trim() === ''
            ? result.total
            : current.catalogTotal,
        likedTotal: result.liked_total,
        resultTotal: result.total,
        generation: result.generation,
      },
    });
    return true;
  };

  const loadPage = async (
    query: string,
    source: LibrarySource,
    offset: number,
    generation: number,
    token: number,
    mergeIntoLibrary = true,
  ): Promise<SearchResult> => {
    const key = cacheKey(source, query, generation);
    const cached = pageCache.get(key)?.get(offset);
    if (cached) {
      if (mergeIntoLibrary) mergePage(cached, token);
      return cached;
    }
    const inFlight = pageRequests.get(`${key}\u0000${offset}`);
    if (inFlight) return inFlight;

    const request: SearchRequest = {
      query,
      offset,
      limit: pageSize,
      source,
      ...(generation > 0 ? { generation } : {}),
    };
    const requestPromise = bridge.searchSongs(request);
    const requestKey = `${key}\u0000${offset}`;
    pageRequests.set(requestKey, requestPromise);
    try {
      const result = await requestPromise;
      const generationPages =
        pageCache.get(cacheKey(source, query, result.generation)) ?? new Map();
      generationPages.set(result.offset, result);
      pageCache.set(cacheKey(source, query, result.generation), generationPages);
      if (mergeIntoLibrary) mergePage(result, token);
      return result;
    } finally {
      if (pageRequests.get(requestKey) === requestPromise) pageRequests.delete(requestKey);
    }
  };

  const ensureRange = async (first: number, last: number, token: number): Promise<void> => {
    if (last < first) return;
    const current = get().library;
    if (current.resultTotal === 0) return;
    const offsets: number[] = [];
    const firstPage = Math.floor(Math.max(0, first) / pageSize) * pageSize;
    const lastPage = Math.floor(Math.max(0, last) / pageSize) * pageSize;
    for (let offset = firstPage; offset <= lastPage; offset += pageSize) offsets.push(offset);
    try {
      await Promise.all(
        offsets.map((offset) =>
          loadPage(current.query, current.searchSource, offset, current.generation, token),
        ),
      );
    } catch (error) {
      if (get().library.searchRequestGeneration !== token) return;
      set({ library: { ...get().library, loading: false, error: errorMessage(error) } });
    }
  };

  const syncCatalogGeneration = async (generation: number): Promise<void> => {
    const current = get().library;
    if (generation <= current.generation) return;
    context.nextDetailRequestToken();
    const playback = get().playback;
    const cancelPreparing = playback.transportOperation === 'preparing';
    if (cancelPreparing) context.clearOperationFromEvent();
    set({
      library: {
        ...current,
        generation,
        pages: new Map(),
        indexById: new Map(),
        selectedSongId: null,
        resultTotal: 0,
        loading: true,
        error: null,
      },
      details: { bySongId: new Map() },
      playback: {
        ...playback,
        prepared: null,
        preparedIdentity: null,
        preparedContext: null,
        transportOperation: cancelPreparing ? null : playback.transportOperation,
      },
    });
    await get().search();
  };

  const reconcileCatalog = (): Promise<void> => {
    catalogReconciliationTail = catalogReconciliationTail.then(async () => {
      if (get().bootstrapState !== 'ready' || get().library.error !== null) return;
      for (let attempt = 0; attempt < MAX_CATALOG_RECONCILIATION_ATTEMPTS; attempt += 1) {
        const requestedGeneration = get().library.generation;
        await get().search();
        const afterSearch = get().library;
        if (afterSearch.error !== null) return;
        if (afterSearch.loading || afterSearch.generation !== requestedGeneration) continue;

        await get().loadLibraryNavigation();
        const settled = get().library;
        if (settled.error !== null || settled.loading || settled.generation !== requestedGeneration)
          continue;
        if (!catalogReadyRecorded) {
          catalogReadyRecorded = true;
          recordStartupTelemetry('react.catalog_ready');
        }
        return;
      }
      // A catalog.changed event queues another reconciliation after this
      // bounded attempt window. Do not spin on a moving generation here.
    });
    return catalogReconciliationTail;
  };

  const actions: LibrarySliceActions = {
    async search(query, source) {
      const current = get().library;
      const nextQuery = query ?? current.query;
      const nextSource = source ?? current.source;
      const nextSearchSource: LibrarySource =
        nextSource.kind === 'playlist' && current.playlistAddMode?.playlistId === nextSource.id
          ? { kind: 'smart', id: 'all' }
          : nextSource;
      const token = current.searchRequestGeneration + 1;
      set({
        library: {
          ...current,
          source: nextSource,
          searchSource: nextSearchSource,
          query: nextQuery,
          pages: new Map(),
          indexById: new Map(),
          resultTotal: 0,
          loading: true,
          error: null,
          searchRequestGeneration: token,
        },
      });
      try {
        const result = await loadPage(nextQuery, nextSearchSource, 0, current.generation, token);
        if (get().library.searchRequestGeneration !== token) return;
        set({
          library: {
            ...get().library,
            catalogTotal:
              nextSearchSource.kind === 'smart' &&
              nextSearchSource.id === 'all' &&
              nextQuery.trim() === ''
                ? result.total
                : get().library.catalogTotal,
            likedTotal: result.liked_total,
            resultTotal: result.total,
            generation: result.generation,
            loading: false,
            error: null,
          },
        });
      } catch (error) {
        if (get().library.searchRequestGeneration !== token) return;
        if (get().library.generation !== current.generation) return;
        set({ library: { ...get().library, loading: false, error: errorMessage(error) } });
      }
    },

    async selectSong(songId) {
      const current = get();
      const cached = current.details.bySongId.get(songId);
      const selectingDifferentSongAfterRetirement =
        current.playback.sessionId === null &&
        current.playback.transportOperation === null &&
        current.playback.currentSong !== null &&
        current.playback.currentSong?.songId !== songId;
      if (
        current.library.selectedSongId === songId &&
        !selectingDifferentSongAfterRetirement &&
        cached &&
        (cached.state === 'ready' || cached.state === 'loading')
      ) {
        return;
      }
      const token = context.nextDetailRequestToken();
      const requestGeneration = current.library.generation;
      const currentPlayback = get().playback;
      const cancelPreparing = currentPlayback.transportOperation === 'preparing';
      if (cancelPreparing) context.clearOperationFromEvent();
      const preservePlaybackIdentity =
        currentPlayback.sessionId !== null ||
        (!cancelPreparing && currentPlayback.transportOperation !== null);
      const detail =
        cached ??
        ({
          state: 'loading',
          value: null,
          error: null,
        } satisfies DetailEntry);
      set({
        library: { ...current.library, selectedSongId: songId },
        details: {
          bySongId: cached
            ? cacheDetail(current.details.bySongId, songId, cached)
            : cacheDetail(current.details.bySongId, songId, detail),
        },
        playback: {
          ...currentPlayback,
          prepared: null,
          preparedIdentity: null,
          preparedContext: null,
          currentSong: preservePlaybackIdentity ? currentPlayback.currentSong : null,
          context: preservePlaybackIdentity ? currentPlayback.context : null,
          state: preservePlaybackIdentity ? currentPlayback.state : 'idle',
          sessionId: preservePlaybackIdentity ? currentPlayback.sessionId : null,
          snapshot: preservePlaybackIdentity ? currentPlayback.snapshot : null,
          transportOperation: cancelPreparing ? null : currentPlayback.transportOperation,
          error: null,
        },
      });
      if (cached && cached.state !== 'loading') return;
      try {
        const request = { songId } as { songId: string; generation?: number };
        if (requestGeneration > 0) request.generation = requestGeneration;
        const value = await bridge.getSongDetail(request);
        if (!context.isCurrentDetailRequest(token)) return;
        if (get().library.generation !== requestGeneration) return;
        const nextDetail = { state: 'ready' as const, value, error: null };
        set({
          details: {
            bySongId: cacheDetail(get().details.bySongId, songId, nextDetail),
          },
        });
      } catch (error) {
        if (!context.isCurrentDetailRequest(token)) return;
        const nextDetail = {
          state: 'fatal' as const,
          value: null,
          error: errorMessage(error),
        };
        set({
          details: {
            bySongId: cacheDetail(get().details.bySongId, songId, nextDetail),
          },
        });
      }
    },

    async setViewport(first, last) {
      const library = get().library;
      set({ library: { ...library, visibleRange: { first, last } } });
      if (library.generation === 0 || first > last) return;
      await ensureRange(first, last, library.searchRequestGeneration);
      const current = get().library;
      if (
        current.searchRequestGeneration !== library.searchRequestGeneration ||
        current.generation !== library.generation
      ) {
        return;
      }
      const songIds: string[] = [];
      for (let index = Math.max(0, first); index <= Math.max(0, last); index += 1) {
        const row = selectRowAtIndex(current, index);
        if (row) songIds.push(row.song_id);
      }
      try {
        const result = await bridge.setLibraryViewport({
          generation: current.generation,
          firstIndex: first,
          lastIndex: last,
          selectedSongId: current.selectedSongId,
          songIds,
        });
        if (result.generation !== get().library.generation) return;
        if (result.items.length === 0) return;
        const latest = get().library;
        const cachedPages = pageCache.get(
          cacheKey(latest.searchSource, latest.query, latest.generation),
        );
        if (cachedPages) {
          const updatesByPage = new Map<number, SongRow[]>();
          for (const row of result.items) {
            const index = latest.indexById.get(row.song_id);
            if (index === undefined) continue;
            const offset = Math.floor(index / pageSize) * pageSize;
            let items = updatesByPage.get(offset);
            if (!items) {
              const cached = cachedPages.get(offset);
              if (!cached) continue;
              items = [...cached.items];
              updatesByPage.set(offset, items);
            }
            items[index - offset] = row;
          }
          for (const [offset, items] of updatesByPage) {
            const cached = cachedPages.get(offset);
            if (cached) cached.items = items;
          }
        }
        set({
          library: updateLoadedRows(latest, result.items),
        });
      } catch {
        // Metadata hydration is best effort for a moving viewport. A later
        // viewport event retries the bounded request without blocking rows.
      }
    },

    async loadLibraryNavigation() {
      const current = get().libraryNavigation;
      set({
        libraryNavigation: { ...current, loadState: 'loading', lastError: null },
      });
      try {
        const navigation = await bridge.listLibraryNavigation();
        updateNavigation(navigation);
      } catch (error) {
        set({
          libraryNavigation: {
            ...get().libraryNavigation,
            loadState: 'error',
            lastError: errorMessage(error),
          },
        });
      }
    },

    async createPlaylist(name) {
      const key = 'playlist:create';
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      try {
        const summary = await bridge.createPlaylist(name.trim());
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary),
        });
        await get().selectLibrarySource({ kind: 'playlist', id: summary.id });
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async renamePlaylist(playlistId, name) {
      const key = `playlist:${playlistId}:rename`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      try {
        const summary = await bridge.renamePlaylist(playlistId, name.trim());
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary),
        });
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async deletePlaylist(playlistId) {
      const key = `playlist:${playlistId}:delete`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      const active = get().library.source;
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const removed = await bridge.deletePlaylist(playlistId);
        if (removed) {
          const current = get().libraryNavigation;
          const playlistsById = new Map(current.playlistsById);
          playlistsById.delete(playlistId);
          set({
            libraryNavigation: {
              ...current,
              playlistsById,
              playlistOrder: current.playlistOrder.filter((id) => id !== playlistId),
            },
          });
          if (active.kind === 'playlist' && active.id === playlistId) {
            await get().selectLibrarySource({ kind: 'smart', id: 'all' });
          }
        }
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async importLocalFilesToPlaylist(playlistId) {
      const key = `playlist:${playlistId}:import:file`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const result = await bridge.importLocalFilesToPlaylist(playlistId);
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, result.playlist),
        });
        if (result.imported_song_count > 0) await syncCatalogGeneration(result.catalog_generation);
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async importLocalFolderToPlaylist(playlistId) {
      const key = `playlist:${playlistId}:import:folder`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const result = await bridge.importLocalFolderToPlaylist(playlistId);
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, result.playlist),
        });
        if (result.imported_song_count > 0) await syncCatalogGeneration(result.catalog_generation);
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async addSongToPlaylist(playlistId, songId) {
      const key = `playlist:${playlistId}:song:${songId}:add`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const summary = await bridge.addSongsToPlaylist(playlistId, [songId]);
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary),
        });
        invalidateSourceCache({ kind: 'playlist', id: playlistId });
        const active = get().library.source;
        if (active.kind === 'playlist' && active.id === playlistId) await get().search();
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async removeSongFromPlaylist(playlistId, songId) {
      const key = `playlist:${playlistId}:song:${songId}:remove`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const summary = await bridge.removeSongsFromPlaylist(playlistId, [songId]);
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary),
        });
        invalidateSourceCache({ kind: 'playlist', id: playlistId });
        const active = get().library.source;
        if (active.kind === 'playlist' && active.id === playlistId) {
          await get().search();
        }
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async selectLibrarySource(source) {
      const current = get().library;
      if (sourceKey(current.source) !== sourceKey(source) || current.playlistAddMode !== null) {
        context.nextDetailRequestToken();
        set({
          library: { ...get().library, selectedSongId: null, playlistAddMode: null },
          details: { bySongId: new Map() },
          playback: context.playbackAfterBrowserNavigation(get().playback),
        });
      }
      await get().search('', source);
    },

    async openPlaylistAdd(playlistId) {
      const playlistSource: LibrarySource = { kind: 'playlist', id: playlistId };
      context.nextDetailRequestToken();
      const current = get().library;
      set({
        library: {
          ...current,
          source: playlistSource,
          playlistAddMode: { playlistId },
          selectedSongId: null,
        },
        details: { bySongId: new Map() },
        playback: context.playbackAfterBrowserNavigation(get().playback),
      });
      await get().search('', playlistSource);
    },

    async exitPlaylistAdd() {
      const current = get().library;
      const playlistId = current.playlistAddMode?.playlistId;
      if (!playlistId) return;
      context.nextDetailRequestToken();
      const playlistSource: LibrarySource = { kind: 'playlist', id: playlistId };
      set({
        library: {
          ...current,
          source: playlistSource,
          playlistAddMode: null,
          selectedSongId: null,
        },
        details: { bySongId: new Map() },
        playback: context.playbackAfterBrowserNavigation(get().playback),
      });
      await get().search('', playlistSource);
    },

    async setSongLiked(songId, liked) {
      const current = get().library;
      const playback = get().playback;
      const previous = selectSongById(current, songId);
      const currentPlaybackSong =
        playback.currentSong?.songId === songId ? playback.currentSong : null;
      if (!previous && !currentPlaybackSong) return;
      const previousLiked = previous?.liked ?? currentPlaybackSong?.liked ?? false;
      if (previousLiked === liked) return;
      context.invalidatePlaybackMembershipContext({ kind: 'smart', id: 'liked' });
      const optimistic = previous ? { ...previous, liked } : null;
      const likedDelta = liked ? 1 : -1;
      pageCache.clear();
      set({
        library: {
          ...(optimistic ? updateLoadedRows(current, [optimistic]) : current),
          likedTotal: Math.max(0, current.likedTotal + likedDelta),
        },
        playback: currentPlaybackSong
          ? { ...get().playback, currentSong: { ...currentPlaybackSong, liked } }
          : get().playback,
      });
      const targetGeneration = currentPlaybackSong?.generation ?? current.generation;
      const generation = targetGeneration > 0 ? targetGeneration : undefined;
      try {
        const result = await bridge.setSongLiked({
          songId,
          liked,
          ...(generation === undefined ? {} : { generation }),
        });
        const library = get().library;
        set({
          library: {
            ...library,
            likedTotal: result.total,
          },
        });
        if (library.source.kind === 'smart' && library.source.id === 'liked') {
          await get().search();
        }
      } catch (error) {
        const message = errorMessage(error);
        const latest = get().library;
        const row = selectSongById(latest, songId);
        const shouldRollback = row?.liked === liked;
        const rollback = shouldRollback
          ? updateLoadedRows(latest, [{ ...row!, liked: previousLiked }])
          : latest;
        const latestPlayback = get().playback;
        set({
          library: {
            ...rollback,
            likedTotal: shouldRollback
              ? Math.max(0, latest.likedTotal - likedDelta)
              : rollback.likedTotal,
          },
          playback:
            latestPlayback.currentSong?.songId === songId
              ? {
                  ...latestPlayback,
                  currentSong: { ...latestPlayback.currentSong, liked: previousLiked },
                }
              : latestPlayback,
        });
        set({ library: { ...get().library, error: message } });
      }
    },

    async reloadLibrary() {
      set({ library: { ...get().library, loading: true, error: null } });
      try {
        await bridge.reloadLibrary();
        await get().loadLibraryNavigation();
      } catch (error) {
        set({ library: { ...get().library, loading: false, error: errorMessage(error) } });
      }
    },
  };

  return { actions, reconcileCatalog, syncCatalogGeneration, loadPage };
}
