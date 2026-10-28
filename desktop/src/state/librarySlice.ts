import type {
  DesktopBridge,
  LibraryPlaylistImportResult,
  LibraryNavigation,
  LibrarySource,
  SearchRequest,
  SearchResult,
  SongRow,
} from '../bridge/DesktopBridge';
import { recordStartupTelemetry } from '../bridge/startupTelemetry';
import { selectRowAtIndex, selectSongById } from './selectors';
import {
  activeLibraryPageOffsets,
  cacheDetail,
  navigationFromDto,
  retainActiveLibraryPages,
  sourceKey,
  updateLoadedRows,
  updatePlaylistSummary,
} from './libraryHelpers';
import { LIBRARY_PAGE_SIZE } from './types';
import type { DetailEntry, DesktopStore } from './types';

const MAX_CATALOG_RECONCILIATION_ATTEMPTS = 3;
// Keep a small cross-scope LRU for back-scroll and repeated source navigation.
export const MAX_CACHED_SEARCH_PAGES = 8;

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
  const pageCache = new Map<string, SearchResult>();
  const pageRequests = new Map<string, Promise<SearchResult>>();
  const viewportRequests = new Map<string, Promise<void>>();
  const pendingLikeWrites = new Set<Promise<unknown>>();
  const likeWriteTails = new Map<string, Promise<unknown>>();
  const latestLikeMutationBySongId = new Map<string, number>();
  let likeMutationRevision = 0;
  let likeMutationSequence = 0;
  let catalogReconciliationTail: Promise<void> = Promise.resolve();
  let catalogReadyRecorded = false;

  const cacheKey = (source: LibrarySource, query: string, generation: number) =>
    `${generation}\u0000${sourceKey(source)}\u0000${query}`;
  const pageKey = (key: string, offset: number) => `${key}\u0000${offset}`;

  const waitForLikeWrites = async (): Promise<void> => {
    while (pendingLikeWrites.size > 0) {
      await Promise.allSettled([...pendingLikeWrites]);
    }
  };

  const getCachedPage = (key: string, offset: number): SearchResult | undefined => {
    const cacheKey = pageKey(key, offset);
    const result = pageCache.get(cacheKey);
    if (result) {
      pageCache.delete(cacheKey);
      pageCache.set(cacheKey, result);
    }
    return result;
  };

  const cachePage = (key: string, result: SearchResult): void => {
    const cacheKey = pageKey(key, result.offset);
    pageCache.delete(cacheKey);
    pageCache.set(cacheKey, result);
    while (pageCache.size > MAX_CACHED_SEARCH_PAGES) {
      const oldest = pageCache.keys().next().value;
      if (oldest === undefined) break;
      pageCache.delete(oldest);
    }
  };

  const invalidateStaleGenerations = (generation: number): void => {
    const activePrefix = `${generation}\u0000`;
    for (const key of pageCache.keys()) {
      if (!key.startsWith(activePrefix)) pageCache.delete(key);
    }
  };

  const invalidateSourceCache = (source: LibrarySource): void => {
    const marker = `\u0000${sourceKey(source)}\u0000`;
    for (const key of pageCache.keys()) {
      if (key.includes(marker)) pageCache.delete(key);
    }
    for (const key of pageRequests.keys()) {
      if (key.includes(marker)) pageRequests.delete(key);
    }
  };

  const patchCachedLike = (songId: string, liked: boolean, likedTotal: number): void => {
    for (const [key, result] of pageCache) {
      const index = result.items.findIndex((row) => row.song_id === songId);
      if (index < 0 && result.liked_total === likedTotal) continue;
      const items = index < 0 ? result.items : [...result.items];
      if (index >= 0) items[index] = { ...items[index]!, liked };
      pageCache.set(key, { ...result, items, liked_total: likedTotal });
    }
  };

  const patchCachedLikedTotal = (likedTotal: number): void => {
    for (const [key, result] of pageCache) {
      if (result.liked_total !== likedTotal) {
        pageCache.set(key, { ...result, liked_total: likedTotal });
      }
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

  const importedSongCount = (result: LibraryPlaylistImportResult | null | undefined): number => {
    const count = result?.imported_song_count;
    if (typeof count !== 'number' || !Number.isSafeInteger(count) || count < 0) {
      throw new Error('The import did not return a valid result.');
    }
    return count;
  };

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
    const activeOffsets = new Set(
      activeLibraryPageOffsets(
        current.visibleRange.first,
        current.visibleRange.last,
        current.resultTotal || result.total,
      ),
    );
    if (!activeOffsets.has(result.offset)) return false;

    const retained = retainActiveLibraryPages(current);
    const pages = new Map(retained.pages);
    pages.set(result.offset, result.items);
    for (const offset of pages.keys()) {
      if (!activeOffsets.has(offset)) pages.delete(offset);
    }
    const indexById = new Map<string, number>();
    let selectedSongSummary = current.selectedSongSummary;
    let selectedSongIndex = current.selectedSongIndex;
    result.items.forEach((row, index) => {
      indexById.set(row.song_id, result.offset + index);
      if (row.song_id === current.selectedSongId) {
        selectedSongSummary = row;
        selectedSongIndex = result.offset + index;
      }
    });
    for (const [offset, rows] of pages) {
      if (offset === result.offset) continue;
      rows.forEach((row, index) => indexById.set(row.song_id, offset + index));
    }
    set({
      library: {
        ...current,
        pages,
        indexById,
        selectedSongSummary,
        selectedSongIndex,
        catalogTotal:
          current.searchSource.kind === 'smart' &&
          current.searchSource.id === 'all' &&
          current.query.trim() === ''
            ? result.total
            : current.catalogTotal,
        likedTotal: result.liked_total,
        resultTotal: result.total,
        generation: result.generation,
        loading: result.offset === 0 ? false : current.loading,
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
    const requestKey = pageKey(key, offset);
    const request: SearchRequest = {
      query,
      offset,
      limit: pageSize,
      source,
      ...(generation > 0 ? { generation } : {}),
    };
    for (;;) {
      if (pendingLikeWrites.size > 0) await waitForLikeWrites();
      const cached = getCachedPage(key, offset);
      if (cached) {
        if (mergeIntoLibrary) mergePage(cached, token);
        return cached;
      }

      const requestLikeRevision = likeMutationRevision;
      const inFlight = pageRequests.get(requestKey);
      if (inFlight) {
        const result = await inFlight;
        if (
          pageRequests.get(requestKey) !== inFlight ||
          requestLikeRevision !== likeMutationRevision ||
          pendingLikeWrites.size > 0
        ) {
          continue;
        }
        if (mergeIntoLibrary) mergePage(result, token);
        return result;
      }

      const requestPromise = bridge.searchSongs(request);
      pageRequests.set(requestKey, requestPromise);
      try {
        const result = await requestPromise;
        if (
          pageRequests.get(requestKey) !== requestPromise ||
          requestLikeRevision !== likeMutationRevision ||
          pendingLikeWrites.size > 0
        ) {
          continue;
        }
        if (result.generation >= get().library.generation) {
          cachePage(cacheKey(source, query, result.generation), result);
        }
        if (mergeIntoLibrary) mergePage(result, token);
        return result;
      } finally {
        if (pageRequests.get(requestKey) === requestPromise) pageRequests.delete(requestKey);
      }
    }
  };

  const ensureRange = async (first: number, last: number, token: number): Promise<void> => {
    if (last < first) return;
    const current = get().library;
    if (current.resultTotal === 0) return;
    const firstPage = Math.floor(Math.max(0, first) / pageSize) * pageSize;
    const lastPage = Math.floor(Math.max(0, last) / pageSize) * pageSize;
    const offsets = activeLibraryPageOffsets(first, last, current.resultTotal).filter(
      (offset) => offset >= firstPage && offset <= lastPage && !current.pages.has(offset),
    );
    try {
      await Promise.all(
        offsets.map((offset) =>
          loadPage(current.query, current.searchSource, offset, current.generation, token),
        ),
      );
    } catch (error) {
      const latest = get().library;
      if (
        latest.searchRequestGeneration !== token ||
        latest.generation !== current.generation ||
        latest.visibleRange.first !== first ||
        latest.visibleRange.last !== last
      ) {
        return;
      }
      set({ library: { ...latest, loading: false, error: errorMessage(error) } });
    }
  };

  const hydrateViewport = async (
    first: number,
    last: number,
    scope: {
      generation: number;
      searchRequestGeneration: number;
      source: LibrarySource;
      searchSource: LibrarySource;
      query: string;
    },
  ): Promise<void> => {
    const isCurrentScope = (library: DesktopStore['library']): boolean =>
      library.generation === scope.generation &&
      library.searchRequestGeneration === scope.searchRequestGeneration &&
      sourceKey(library.source) === sourceKey(scope.source) &&
      sourceKey(library.searchSource) === sourceKey(scope.searchSource) &&
      library.query === scope.query &&
      library.visibleRange.first === first &&
      library.visibleRange.last === last;

    for (;;) {
      if (pendingLikeWrites.size > 0) await waitForLikeWrites();
      const current = get().library;
      if (!isCurrentScope(current)) return;

      const songIds: string[] = [];
      for (let index = Math.max(0, first); index <= Math.max(0, last); index += 1) {
        const row = selectRowAtIndex(current, index);
        if (row) songIds.push(row.song_id);
      }
      const selectedSongId = current.selectedSongId;
      const requestLikeRevision = likeMutationRevision;
      const requestKey = JSON.stringify([
        sourceKey(current.source),
        sourceKey(current.searchSource),
        current.query,
        current.generation,
        current.searchRequestGeneration,
        requestLikeRevision,
        first,
        last,
        selectedSongId,
        songIds,
      ]);
      const pendingRequest = viewportRequests.get(requestKey);
      if (pendingRequest) {
        await pendingRequest;
        return;
      }

      let staleLikeSnapshot = false;
      const hydration = (async () => {
        try {
          const result = await bridge.setLibraryViewport({
            generation: scope.generation,
            firstIndex: first,
            lastIndex: last,
            selectedSongId,
            songIds,
          });
          const latest = get().library;
          if (!isCurrentScope(latest)) return;
          if (
            requestLikeRevision !== likeMutationRevision ||
            pendingLikeWrites.size > 0 ||
            latest.selectedSongId !== selectedSongId
          ) {
            staleLikeSnapshot = true;
            return;
          }
          if (
            !result.accepted ||
            result.generation !== scope.generation ||
            result.first_index !== first ||
            result.last_index !== last ||
            result.selected_song_id !== selectedSongId ||
            result.items.length === 0
          ) {
            return;
          }

          const cachedSearchKey = cacheKey(latest.searchSource, latest.query, latest.generation);
          const updatesByPage = new Map<number, SongRow[]>();
          for (const row of result.items) {
            const index =
              latest.indexById.get(row.song_id) ??
              (latest.selectedSongId === row.song_id
                ? (latest.selectedSongIndex ?? undefined)
                : undefined);
            if (index === undefined) continue;
            const offset = Math.floor(index / pageSize) * pageSize;
            let items = updatesByPage.get(offset);
            if (!items) {
              const cached = getCachedPage(cachedSearchKey, offset);
              if (!cached) continue;
              items = [...cached.items];
              updatesByPage.set(offset, items);
            }
            items[index - offset] = row;
          }
          for (const [offset, items] of updatesByPage) {
            const cached = getCachedPage(cachedSearchKey, offset);
            if (cached) cachePage(cachedSearchKey, { ...cached, items });
          }
          set({ library: updateLoadedRows(latest, result.items) });
        } catch {
          // Metadata hydration is best effort for a moving viewport. A later
          // viewport event retries the bounded request without blocking rows.
        }
      })();
      viewportRequests.set(requestKey, hydration);
      try {
        await hydration;
      } finally {
        if (viewportRequests.get(requestKey) === hydration) {
          viewportRequests.delete(requestKey);
        }
      }
      if (!staleLikeSnapshot) return;
    }
  };

  const syncCatalogGeneration = async (generation: number): Promise<void> => {
    const current = get().library;
    if (generation <= current.generation) return;
    invalidateStaleGenerations(generation);
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
        selectedSongSummary: null,
        selectedSongIndex: null,
        visibleRange: { first: 0, last: 0 },
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
      const scopeChanged =
        current.query !== nextQuery ||
        sourceKey(current.source) !== sourceKey(nextSource) ||
        sourceKey(current.searchSource) !== sourceKey(nextSearchSource);
      const nextSearchKey = cacheKey(nextSearchSource, nextQuery, current.generation);
      const cachedFirstPage = getCachedPage(nextSearchKey, 0);
      const firstPageRequestKey = pageKey(nextSearchKey, 0);
      // Same-generation reconciliation may reuse the authoritative first page. A
      // catalog reload publishes a newer generation, so its cache key necessarily misses.
      if (
        !scopeChanged &&
        !current.error &&
        (cachedFirstPage || pageRequests.has(firstPageRequestKey))
      ) {
        return;
      }

      const clearBrowserSelection = scopeChanged;
      if (clearBrowserSelection) context.nextDetailRequestToken();
      const token = current.searchRequestGeneration + 1;
      set({
        library: {
          ...current,
          source: nextSource,
          searchSource: nextSearchSource,
          query: nextQuery,
          selectedSongId: clearBrowserSelection ? null : current.selectedSongId,
          selectedSongSummary: clearBrowserSelection ? null : current.selectedSongSummary,
          selectedSongIndex: clearBrowserSelection ? null : current.selectedSongIndex,
          visibleRange: clearBrowserSelection ? { first: 0, last: 0 } : current.visibleRange,
          pages: new Map(),
          indexById: new Map(),
          resultTotal: 0,
          loading: true,
          error: null,
          searchRequestGeneration: token,
        },
        ...(clearBrowserSelection ? { details: { bySongId: new Map() } } : {}),
      });
      try {
        const result = await loadPage(nextQuery, nextSearchSource, 0, current.generation, token);
        const latest = get().library;
        if (latest.searchRequestGeneration !== token) return;
        const generationChanged = latest.generation !== result.generation;
        if (generationChanged) {
          context.nextDetailRequestToken();
          invalidateStaleGenerations(result.generation);
        }
        set({
          library: {
            ...latest,
            catalogTotal:
              nextSearchSource.kind === 'smart' &&
              nextSearchSource.id === 'all' &&
              nextQuery.trim() === ''
                ? result.total
                : get().library.catalogTotal,
            likedTotal: result.liked_total,
            resultTotal: result.total,
            generation: result.generation,
            selectedSongId: generationChanged ? null : latest.selectedSongId,
            selectedSongSummary: generationChanged ? null : latest.selectedSongSummary,
            selectedSongIndex: generationChanged ? null : latest.selectedSongIndex,
            visibleRange: generationChanged ? { first: 0, last: 0 } : latest.visibleRange,
            loading: false,
            error: null,
          },
          ...(generationChanged ? { details: { bySongId: new Map() } } : {}),
        });
        if (generationChanged) mergePage(result, token);
        if (!generationChanged && result.total > 0) {
          const afterSearch = get().library;
          const lastIndex = result.total - 1;
          const first = Math.min(Math.max(0, afterSearch.visibleRange.first), lastIndex);
          const last = Math.min(Math.max(first, afterSearch.visibleRange.last), lastIndex);
          if (first !== afterSearch.visibleRange.first || last !== afterSearch.visibleRange.last) {
            const ranged = { ...afterSearch, visibleRange: { first, last } };
            set({ library: retainActiveLibraryPages(ranged, first, last) });
          }
          await ensureRange(first, last, token);
        }
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
      const selectedSummary = selectSongById(current.library, songId);
      const selectedIndex =
        current.library.indexById.get(songId) ??
        (current.library.selectedSongId === songId ? current.library.selectedSongIndex : null);
      set({
        library: {
          ...current.library,
          selectedSongId: songId,
          selectedSongSummary: selectedSummary ?? null,
          selectedSongIndex: selectedIndex,
        },
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
      const initial = get().library;
      const ranged =
        initial.visibleRange.first !== first || initial.visibleRange.last !== last
          ? { ...initial, visibleRange: { first, last } }
          : initial;
      const retained = retainActiveLibraryPages(ranged, first, last);
      if (retained !== initial) set({ library: retained });
      const library = get().library;
      if (library.generation === 0 || first > last) return;
      const scope = {
        generation: library.generation,
        searchRequestGeneration: library.searchRequestGeneration,
        source: library.source,
        searchSource: library.searchSource,
        query: library.query,
      };
      await ensureRange(first, last, scope.searchRequestGeneration);
      const current = get().library;
      if (
        current.searchRequestGeneration !== scope.searchRequestGeneration ||
        current.generation !== scope.generation ||
        sourceKey(current.source) !== sourceKey(scope.source) ||
        sourceKey(current.searchSource) !== sourceKey(scope.searchSource) ||
        current.query !== scope.query ||
        current.visibleRange.first !== first ||
        current.visibleRange.last !== last
      ) {
        return;
      }
      await hydrateViewport(first, last, scope);
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
          invalidateSourceCache({ kind: 'playlist', id: playlistId });
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
      if (!setLibraryMutationPending(key, true)) return null;
      setLibraryError(null);
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const result = await bridge.importLocalFilesToPlaylist(playlistId);
        const count = importedSongCount(result);
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, result.playlist),
        });
        if (count > 0) await syncCatalogGeneration(result.catalog_generation);
        return count;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async importLocalFolderToPlaylist(playlistId) {
      const key = `playlist:${playlistId}:import:folder`;
      if (!setLibraryMutationPending(key, true)) return null;
      setLibraryError(null);
      context.invalidatePlaybackMembershipContext({ kind: 'playlist', id: playlistId });
      try {
        const result = await bridge.importLocalFolderToPlaylist(playlistId);
        const count = importedSongCount(result);
        set({
          libraryNavigation: updatePlaylistSummary(get().libraryNavigation, result.playlist),
        });
        if (count > 0) await syncCatalogGeneration(result.catalog_generation);
        return count;
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
          if (get().library.selectedSongId === songId) {
            context.nextDetailRequestToken();
            set({
              library: {
                ...get().library,
                selectedSongId: null,
                selectedSongSummary: null,
                selectedSongIndex: null,
              },
              details: { bySongId: new Map() },
            });
          }
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
          library: {
            ...get().library,
            selectedSongId: null,
            selectedSongSummary: null,
            selectedSongIndex: null,
            playlistAddMode: null,
          },
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
          selectedSongSummary: null,
          selectedSongIndex: null,
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
          selectedSongSummary: null,
          selectedSongIndex: null,
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
      const mutationSequence = ++likeMutationSequence;
      const mutationRevision = ++likeMutationRevision;
      latestLikeMutationBySongId.set(songId, mutationSequence);
      context.invalidatePlaybackMembershipContext({ kind: 'smart', id: 'liked' });
      const optimistic = previous ? { ...previous, liked } : null;
      const likedDelta = liked ? 1 : -1;
      invalidateSourceCache({ kind: 'smart', id: 'liked' });
      const optimisticLikedTotal = Math.max(0, current.likedTotal + likedDelta);
      patchCachedLike(songId, liked, optimisticLikedTotal);
      set({
        library: {
          ...(optimistic ? updateLoadedRows(current, [optimistic]) : current),
          likedTotal: optimisticLikedTotal,
        },
        playback: currentPlaybackSong
          ? { ...get().playback, currentSong: { ...currentPlaybackSong, liked } }
          : get().playback,
      });
      const targetGeneration = currentPlaybackSong?.generation ?? current.generation;
      const generation = targetGeneration > 0 ? targetGeneration : undefined;
      const request = {
        songId,
        liked,
        ...(generation === undefined ? {} : { generation }),
      };
      const previousWrite = likeWriteTails.get(songId);
      const write = (previousWrite ?? Promise.resolve())
        .catch(() => undefined)
        .then(() => bridge.setSongLiked(request));
      likeWriteTails.set(songId, write);
      pendingLikeWrites.add(write);

      const clearWrite = () => {
        pendingLikeWrites.delete(write);
        if (likeWriteTails.get(songId) === write) likeWriteTails.delete(songId);
      };

      try {
        const result = await write;
        clearWrite();
        if (latestLikeMutationBySongId.get(songId) !== mutationSequence) return;
        latestLikeMutationBySongId.delete(songId);
        if (mutationRevision === likeMutationRevision && pendingLikeWrites.size === 0) {
          set({ library: { ...get().library, likedTotal: result.total } });
          patchCachedLikedTotal(result.total);
        }
        const library = get().library;
        if (library.source.kind === 'smart' && library.source.id === 'liked') {
          if (!liked && get().library.selectedSongId === songId) {
            context.nextDetailRequestToken();
            set({
              library: {
                ...get().library,
                selectedSongId: null,
                selectedSongSummary: null,
                selectedSongIndex: null,
              },
              details: { bySongId: new Map() },
            });
          }
          await get().search();
        }
      } catch (error) {
        clearWrite();
        if (latestLikeMutationBySongId.get(songId) !== mutationSequence) return;
        latestLikeMutationBySongId.delete(songId);
        const message = errorMessage(error);
        const latest = get().library;
        const row = selectSongById(latest, songId);
        const latestPlayback = get().playback;
        const playbackSong =
          latestPlayback.currentSong?.songId === songId ? latestPlayback.currentSong : null;
        const shouldRollback = (row?.liked ?? playbackSong?.liked) === liked;
        const rollbackTotal = shouldRollback
          ? Math.max(0, latest.likedTotal - likedDelta)
          : latest.likedTotal;
        const rollback = shouldRollback
          ? row
            ? updateLoadedRows(latest, [{ ...row, liked: previousLiked }])
            : latest
          : latest;
        if (shouldRollback) patchCachedLike(songId, previousLiked, rollbackTotal);
        set({
          library: {
            ...rollback,
            likedTotal: rollbackTotal,
            error: message,
          },
          playback:
            playbackSong && shouldRollback
              ? {
                  ...latestPlayback,
                  currentSong: { ...playbackSong, liked: previousLiked },
                }
              : latestPlayback,
        });
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
