import type {
  DesktopBridge,
  LibrarySource,
  SearchRequest,
  SearchResult,
  SongRow,
} from '../bridge/DesktopBridge';
import { selectRowAtIndex } from './selectors';
import {
  activeLibraryPageOffsets,
  retainActiveLibraryPages,
  sourceKey,
  updateLoadedRows,
} from './libraryHelpers';
import { LIBRARY_PAGE_SIZE } from './types';
import type { DesktopStore } from './types';
import type { LibraryPlaybackCoordinator } from './libraryPlaybackCoordinator';
import { createLibraryMutationActions, createLibraryMutationState } from './libraryMutationActions';
import { createLibrarySelectionActions } from './librarySelectionActions';
import { createLibraryCatalogActions } from './libraryCatalog';

// Keep a small cross-scope LRU for back-scroll and repeated source navigation.
export const MAX_CACHED_SEARCH_PAGES = 8;

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface LibrarySliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  playback: LibraryPlaybackCoordinator;
  nextDetailRequestToken: () => number;
  isCurrentDetailRequest: (token: number) => boolean;
}

export type LibrarySliceActions = Pick<
  DesktopStore,
  | 'search'
  | 'selectSong'
  | 'setViewport'
  | 'navigateBuiltinPage'
  | 'retryBuiltinPage'
  | 'showNowPlaying'
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
  const pageRequests = new Map<
    string,
    {
      promise: Promise<SearchResult>;
      likeWriteRevision: number;
      membershipRevision: number;
    }
  >();
  const viewportRequests = new Map<string, Promise<void>>();
  const mutationState = createLibraryMutationState();
  const membershipWritesBySource = new Map<string, { pending: number; revision: number }>();
  let builtinPageIntentSequence = 0;
  let builtinFocusSequence = 0;
  let pendingBuiltinFocus: { intentId: number; index: number; id: number } | null = null;

  const isBuiltinPageContext = (library: DesktopStore['library']): boolean => {
    if (library.searchSource.kind !== 'smart' || library.searchSource.id !== 'builtin') {
      return false;
    }
    if (library.source.kind === 'smart') {
      return library.source.id === 'builtin' && library.playlistAddMode === null;
    }
    return (
      library.source.kind === 'playlist' &&
      library.playlistAddMode?.playlistId === library.source.id
    );
  };

  const cacheKey = (source: LibrarySource, query: string, generation: number) =>
    `${generation}\u0000${sourceKey(source)}\u0000${query}`;
  const pageKey = (key: string, offset: number) => `${key}\u0000${offset}`;
  const membershipRevisionFor = (source: LibrarySource): number =>
    membershipWritesBySource.get(sourceKey(source))?.revision ?? 0;
  const hasPendingMembershipWrite = (source: LibrarySource): boolean =>
    (membershipWritesBySource.get(sourceKey(source))?.pending ?? 0) > 0;

  const beginMembershipMutation = (source: LibrarySource): void => {
    const key = sourceKey(source);
    const membership = membershipWritesBySource.get(key) ?? { pending: 0, revision: 0 };
    const nextMembership = { pending: membership.pending + 1, revision: membership.revision + 1 };
    membershipWritesBySource.set(key, nextMembership);
    invalidateSourceCache(source);

    const current = get().library;
    if (sourceKey(current.searchSource) !== key) return;
    set({
      library: {
        ...current,
        membershipRevision: nextMembership.revision,
        selectedSongIndex: null,
        searchRequestGeneration: current.searchRequestGeneration + 1,
      },
    });
  };

  const finishMembershipMutation = (source: LibrarySource): boolean => {
    const key = sourceKey(source);
    const membership = membershipWritesBySource.get(key) ?? { pending: 0, revision: 0 };
    const pending = Math.max(0, membership.pending - 1);
    const nextMembership = { pending, revision: membership.revision + 1 };
    membershipWritesBySource.set(key, nextMembership);
    invalidateSourceCache(source);

    const current = get().library;
    if (sourceKey(current.searchSource) === key) {
      set({
        library: {
          ...current,
          membershipRevision: nextMembership.revision,
          selectedSongIndex: null,
        },
      });
    }
    return pending === 0;
  };

  const refreshMembershipSource = async (source: LibrarySource): Promise<void> => {
    const current = get().library;
    if (sourceKey(current.searchSource) === sourceKey(source)) await get().search();
  };

  const rebasePendingLikeState = (result: SearchResult): SearchResult => {
    let items = result.items;
    for (let index = 0; index < result.items.length; index += 1) {
      const row = result.items[index]!;
      const pending = mutationState.pendingLikeStateBySongId.get(row.song_id);
      if (!pending || row.liked === pending.liked) continue;
      if (items === result.items) items = [...result.items];
      items[index] = { ...row, liked: pending.liked };
    }
    const likedTotal =
      mutationState.pendingLikeWrites.size > 0 ? get().library.likedTotal : result.liked_total;
    if (items === result.items && likedTotal === result.liked_total) return result;
    return { ...result, items, liked_total: likedTotal };
  };

  const rebasePendingLikeRows = (rows: readonly SongRow[]): SongRow[] =>
    rows.map((row) => {
      const pending = mutationState.pendingLikeStateBySongId.get(row.song_id);
      return pending && pending.liked !== row.liked ? { ...row, liked: pending.liked } : row;
    });

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

  const errorMessage = (error: unknown): string =>
    error instanceof Error ? error.message : String(error);

  const mergePage = (result: SearchResult, token: number): boolean => {
    const current = get().library;
    if (current.searchRequestGeneration !== token || current.generation !== result.generation) {
      return false;
    }
    if (
      hasPendingMembershipWrite(current.searchSource) ||
      current.membershipRevision !== membershipRevisionFor(current.searchSource)
    ) {
      return false;
    }
    let activeOffsets: Set<number>;
    try {
      activeOffsets = new Set(
        activeLibraryPageOffsets(
          current.visibleRange.first,
          current.visibleRange.last,
          current.resultTotal || result.total,
        ),
      );
    } catch {
      return false;
    }
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
        indexMembershipRevision: current.membershipRevision,
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
      const cached = getCachedPage(key, offset);
      if (cached) {
        const rebased = rebasePendingLikeState(cached);
        if (mergeIntoLibrary) mergePage(rebased, token);
        return rebased;
      }

      const inFlight = pageRequests.get(requestKey);
      if (inFlight) {
        const result = await inFlight.promise;
        if (
          pageRequests.get(requestKey) !== inFlight ||
          inFlight.likeWriteRevision !== mutationState.likeWriteRevision ||
          inFlight.membershipRevision !== membershipRevisionFor(source)
        ) {
          continue;
        }
        const rebased = rebasePendingLikeState(result);
        if (mergeIntoLibrary) mergePage(rebased, token);
        return rebased;
      }

      const requestLikeWriteRevision = mutationState.likeWriteRevision;
      const requestMembershipRevision = membershipRevisionFor(source);
      const requestPromise = bridge.searchSongs(request);
      const requestState = {
        promise: requestPromise,
        likeWriteRevision: requestLikeWriteRevision,
        membershipRevision: requestMembershipRevision,
      };
      pageRequests.set(requestKey, requestState);
      try {
        const result = await requestPromise;
        if (
          pageRequests.get(requestKey) !== requestState ||
          requestLikeWriteRevision !== mutationState.likeWriteRevision ||
          requestMembershipRevision !== membershipRevisionFor(source)
        ) {
          continue;
        }
        const rebased = rebasePendingLikeState(result);
        if (result.generation >= get().library.generation && !hasPendingMembershipWrite(source)) {
          cachePage(cacheKey(source, query, result.generation), rebased);
        }
        if (mergeIntoLibrary) mergePage(rebased, token);
        return rebased;
      } finally {
        if (pageRequests.get(requestKey) === requestState) pageRequests.delete(requestKey);
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

  const navigateBuiltinPage = async (page: number, focusIndex?: number): Promise<void> => {
    const current = get().library;
    if (
      !Number.isSafeInteger(page) ||
      page < 1 ||
      !isBuiltinPageContext(current) ||
      current.resultTotal <= 0
    ) {
      return;
    }
    const pageCount = Math.ceil(current.resultTotal / pageSize);
    if (page > pageCount) return;

    const intentId = Math.max(builtinPageIntentSequence, current.builtinPage.intentId) + 1;
    builtinPageIntentSequence = intentId;
    pendingBuiltinFocus = null;
    const focusId = focusIndex === undefined ? null : ++builtinFocusSequence;
    if (page === current.builtinPage.committedPage) {
      const latest = get().library;
      set({
        library: {
          ...latest,
          builtinPage: {
            ...latest.builtinPage,
            pendingPage: null,
            intentId,
            failedPage: null,
            ...(focusIndex !== undefined && focusId !== null
              ? { focusNowPlaying: { id: focusId, index: focusIndex } }
              : {}),
          },
        },
      });
      return;
    }

    const offset = (page - 1) * pageSize;
    const requestGeneration = current.generation;
    const requestSearchGeneration = current.searchRequestGeneration;
    const requestMembershipRevision = current.membershipRevision;
    const requestSource = current.searchSource;
    const requestQuery = current.query;
    if (focusIndex !== undefined && focusId !== null) {
      pendingBuiltinFocus = { intentId, index: focusIndex, id: focusId };
    }
    set({
      library: {
        ...current,
        builtinPage: {
          ...current.builtinPage,
          pendingPage: page,
          intentId,
          failedPage: null,
        },
      },
    });

    try {
      const result = await loadPage(
        requestQuery,
        requestSource,
        offset,
        requestGeneration,
        requestSearchGeneration,
        false,
      );
      const latest = get().library;
      if (
        latest.builtinPage.intentId !== intentId ||
        latest.builtinPage.pendingPage !== page ||
        latest.searchRequestGeneration !== requestSearchGeneration ||
        latest.generation !== requestGeneration ||
        latest.membershipRevision !== requestMembershipRevision ||
        membershipRevisionFor(latest.searchSource) !== requestMembershipRevision ||
        latest.query !== requestQuery ||
        sourceKey(latest.searchSource) !== sourceKey(requestSource) ||
        !isBuiltinPageContext(latest)
      ) {
        return;
      }
      const responseIds = new Set(result.items.map((row) => row.song_id));
      if (
        latest.resultTotal !== result.total ||
        result.generation !== requestGeneration ||
        result.offset !== offset ||
        result.limit !== pageSize ||
        result.items.length !== Math.min(pageSize, result.total - offset) ||
        responseIds.size !== result.items.length
      ) {
        invalidateSourceCache(requestSource);
        pendingBuiltinFocus = null;
        set({
          library: {
            ...latest,
            builtinPage: {
              ...latest.builtinPage,
              pendingPage: null,
              failedPage: page,
            },
          },
        });
        return;
      }

      const pages = new Map<number, readonly SongRow[]>([[offset, result.items]]);
      const indexById = new Map<string, number>();
      let selectedSongSummary = latest.selectedSongSummary;
      let selectedSongIndex = latest.selectedSongIndex;
      result.items.forEach((row, index) => {
        indexById.set(row.song_id, offset + index);
        if (row.song_id === latest.selectedSongId) {
          selectedSongSummary = row;
          selectedSongIndex = offset + index;
        }
      });
      const focus = pendingBuiltinFocus?.intentId === intentId ? pendingBuiltinFocus : null;
      pendingBuiltinFocus = null;
      set({
        library: {
          ...latest,
          pages,
          indexById,
          indexMembershipRevision: latest.membershipRevision,
          selectedSongSummary,
          selectedSongIndex,
          visibleRange: { first: offset, last: offset + result.items.length - 1 },
          builtinPage: {
            ...latest.builtinPage,
            committedPage: page,
            pendingPage: null,
            failedPage: null,
            ...(focus ? { focusNowPlaying: { id: focus.id, index: focus.index } } : {}),
          },
        },
      });
    } catch {
      const latest = get().library;
      if (
        latest.builtinPage.intentId !== intentId ||
        latest.builtinPage.pendingPage !== page ||
        latest.searchRequestGeneration !== requestSearchGeneration ||
        latest.generation !== requestGeneration ||
        latest.membershipRevision !== requestMembershipRevision ||
        membershipRevisionFor(latest.searchSource) !== requestMembershipRevision ||
        latest.query !== requestQuery ||
        sourceKey(latest.searchSource) !== sourceKey(requestSource) ||
        !isBuiltinPageContext(latest)
      ) {
        return;
      }
      pendingBuiltinFocus = null;
      set({
        library: {
          ...latest,
          builtinPage: {
            ...latest.builtinPage,
            pendingPage: null,
            failedPage: page,
          },
        },
      });
    }
  };

  const hydrateViewport = async (
    first: number,
    last: number,
    scope: {
      generation: number;
      membershipRevision: number;
      searchRequestGeneration: number;
      source: LibrarySource;
      searchSource: LibrarySource;
      query: string;
    },
  ): Promise<void> => {
    const isCurrentScope = (library: DesktopStore['library']): boolean =>
      library.generation === scope.generation &&
      library.membershipRevision === scope.membershipRevision &&
      membershipRevisionFor(library.searchSource) === scope.membershipRevision &&
      library.searchRequestGeneration === scope.searchRequestGeneration &&
      sourceKey(library.source) === sourceKey(scope.source) &&
      sourceKey(library.searchSource) === sourceKey(scope.searchSource) &&
      library.query === scope.query &&
      library.visibleRange.first === first &&
      library.visibleRange.last === last;

    for (;;) {
      const current = get().library;
      if (!isCurrentScope(current)) return;

      const songIds: string[] = [];
      for (let index = Math.max(0, first); index <= Math.max(0, last); index += 1) {
        const row = selectRowAtIndex(current, index);
        if (row) songIds.push(row.song_id);
      }
      const selectedSongId = current.selectedSongId;
      const requestLikeRevision = mutationState.likeMutationRevision;
      const requestKey = JSON.stringify([
        sourceKey(current.source),
        sourceKey(current.searchSource),
        current.query,
        current.generation,
        current.membershipRevision,
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
      const requestLikeWriteRevision = mutationState.likeWriteRevision;
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
            requestLikeRevision !== mutationState.likeMutationRevision ||
            requestLikeWriteRevision !== mutationState.likeWriteRevision ||
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
          const rebasedItems = rebasePendingLikeRows(result.items);
          for (const row of rebasedItems) {
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
          set({ library: updateLoadedRows(latest, rebasedItems) });
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

  const catalogActions = createLibraryCatalogActions({
    get,
    playback: context.playback,
    nextDetailRequestToken: context.nextDetailRequestToken,
    nextBuiltinPageIntentId: (library) => {
      const intentId = Math.max(builtinPageIntentSequence, library.builtinPage.intentId) + 1;
      builtinPageIntentSequence = intentId;
      return intentId;
    },
    clearPendingBuiltinFocus: () => {
      pendingBuiltinFocus = null;
    },
    invalidateStaleGenerations,
    membershipRevisionFor,
  });
  const { syncCatalogGeneration, reconcileCatalog } = catalogActions;
  const mutationActions = createLibraryMutationActions({
    bridge,
    get,
    set,
    playback: context.playback,
    state: mutationState,
    nextDetailRequestToken: context.nextDetailRequestToken,
    syncCatalogGeneration,
    invalidateSourceCache,
    beginMembershipMutation,
    finishMembershipMutation,
    refreshMembershipSource,
    patchCachedLike,
    patchCachedLikedTotal,
  });
  const selectionActions = createLibrarySelectionActions({
    bridge,
    get,
    set,
    playback: context.playback,
    nextDetailRequestToken: context.nextDetailRequestToken,
    isCurrentDetailRequest: context.isCurrentDetailRequest,
    navigateBuiltinPage,
    pageSize,
  });
  const actions: LibrarySliceActions = {
    ...selectionActions,
    ...mutationActions,
    async search(query, source) {
      const current = get().library;
      const nextQuery = query ?? current.query;
      const nextSource = source ?? current.source;
      const nextSearchSource: LibrarySource =
        nextSource.kind === 'playlist' && current.playlistAddMode?.playlistId === nextSource.id
          ? { kind: 'smart', id: 'builtin' }
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
      const pageIntentId = scopeChanged
        ? Math.max(builtinPageIntentSequence, current.builtinPage.intentId) + 1
        : current.builtinPage.intentId;
      if (scopeChanged) {
        builtinPageIntentSequence = pageIntentId;
        pendingBuiltinFocus = null;
      }
      set({
        library: {
          ...current,
          source: nextSource,
          searchSource: nextSearchSource,
          query: nextQuery,
          ...(scopeChanged
            ? {
                builtinPage: {
                  committedPage: 1,
                  pendingPage: null,
                  intentId: pageIntentId,
                  failedPage: null,
                  focusNowPlaying: null,
                },
              }
            : {}),
          membershipRevision: scopeChanged
            ? membershipRevisionFor(nextSearchSource)
            : current.membershipRevision,
          indexMembershipRevision: scopeChanged ? -1 : current.indexMembershipRevision,
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
            ...(generationChanged
              ? {
                  builtinPage: {
                    committedPage: 1,
                    pendingPage: null,
                    intentId: Math.max(builtinPageIntentSequence, latest.builtinPage.intentId) + 1,
                    failedPage: null,
                    focusNowPlaying: null,
                  },
                }
              : {}),
            membershipRevision: membershipRevisionFor(nextSearchSource),
            indexMembershipRevision: generationChanged ? -1 : latest.indexMembershipRevision,
            selectedSongId: generationChanged ? null : latest.selectedSongId,
            selectedSongSummary: generationChanged ? null : latest.selectedSongSummary,
            selectedSongIndex: generationChanged ? null : latest.selectedSongIndex,
            visibleRange: generationChanged ? { first: 0, last: 0 } : latest.visibleRange,
            loading: false,
            error: null,
          },
          ...(generationChanged ? { details: { bySongId: new Map() } } : {}),
        });
        if (generationChanged) {
          mergePage(result, token);
          void get().loadLibraryNavigation();
        }
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

    async setViewport(first, last) {
      const initial = get().library;
      try {
        activeLibraryPageOffsets(first, last, initial.resultTotal);
      } catch (error) {
        set({ library: { ...initial, error: errorMessage(error) } });
        return;
      }
      const ranged =
        initial.visibleRange.first !== first ||
        initial.visibleRange.last !== last ||
        initial.error?.startsWith('The visible Library range spans ') === true
          ? { ...initial, visibleRange: { first, last } }
          : initial;
      const retained = retainActiveLibraryPages(
        ranged.error?.startsWith('The visible Library range spans ')
          ? { ...ranged, error: null }
          : ranged,
        first,
        last,
      );
      if (retained !== initial) set({ library: retained });
      const library = get().library;
      if (library.generation === 0 || first > last) return;
      const scope = {
        generation: library.generation,
        membershipRevision: library.membershipRevision,
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

    async navigateBuiltinPage(page) {
      await navigateBuiltinPage(page);
    },

    async retryBuiltinPage() {
      const failedPage = get().library.builtinPage.failedPage;
      if (failedPage !== null) await navigateBuiltinPage(failedPage);
    },
  };
  return { actions, reconcileCatalog, syncCatalogGeneration, loadPage };
}
