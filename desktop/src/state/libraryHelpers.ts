import type {
  LibraryNavigation,
  LibraryPlaylistSummary,
  LibrarySource,
  SongRow,
} from '../bridge/DesktopBridge';
import { DETAIL_CACHE_LIMIT, LIBRARY_PAGE_SIZE, MAX_ACTIVE_LIBRARY_PAGES } from './types';
import type { DetailEntry, LibraryNavigationState, LibraryState } from './types';

export const sourceKey = (source: LibrarySource): string => JSON.stringify(source);

export function activeLibraryPageOffsets(first: number, last: number, total: number): number[] {
  if (total <= 0) return [];
  const firstIndex = Math.min(Math.max(0, Math.floor(first)), total - 1);
  const lastIndex = Math.min(Math.max(firstIndex, Math.floor(last)), total - 1);
  const firstVisiblePage = Math.floor(firstIndex / LIBRARY_PAGE_SIZE);
  const lastVisiblePage = Math.floor(lastIndex / LIBRARY_PAGE_SIZE);
  const visiblePageCount = lastVisiblePage - firstVisiblePage + 1;
  if (visiblePageCount > MAX_ACTIVE_LIBRARY_PAGES) {
    throw new RangeError(
      `The visible Library range spans ${visiblePageCount} pages; at most ${MAX_ACTIVE_LIBRARY_PAGES} can be active.`,
    );
  }

  const lastCatalogPage = Math.floor((total - 1) / LIBRARY_PAGE_SIZE);
  const activePageCount = Math.min(MAX_ACTIVE_LIBRARY_PAGES, lastCatalogPage + 1);
  const unusedPages = activePageCount - visiblePageCount;
  const centeredFirst = Math.max(0, firstVisiblePage - Math.floor(unusedPages / 2));
  const activeFirstPage = Math.min(centeredFirst, lastCatalogPage - activePageCount + 1);
  return Array.from(
    { length: activePageCount },
    (_, index) => (activeFirstPage + index) * LIBRARY_PAGE_SIZE,
  );
}

export function retainActiveLibraryPages(
  library: LibraryState,
  first = library.visibleRange.first,
  last = library.visibleRange.last,
): LibraryState {
  const activeOffsets = new Set(activeLibraryPageOffsets(first, last, library.resultTotal));
  if ([...library.pages.keys()].every((offset) => activeOffsets.has(offset))) return library;

  const pages = new Map<number, readonly SongRow[]>();
  const indexById = new Map<string, number>();
  const indexIsCurrent = library.indexMembershipRevision === library.membershipRevision;
  for (const [offset, rows] of library.pages) {
    if (!activeOffsets.has(offset)) continue;
    pages.set(offset, rows);
    if (indexIsCurrent) rows.forEach((row, index) => indexById.set(row.song_id, offset + index));
  }
  return { ...library, pages, indexById: indexIsCurrent ? indexById : new Map() };
}

export function updateLoadedRows(library: LibraryState, updates: readonly SongRow[]): LibraryState {
  let pages: Map<number, readonly SongRow[]> | null = null;
  const changedPages = new Map<number, SongRow[]>();
  const indexIsCurrent = library.indexMembershipRevision === library.membershipRevision;
  let selectedSongSummary = library.selectedSongSummary;
  let selectedSongIndex = library.selectedSongIndex;

  for (const row of updates) {
    const index = library.indexById.get(row.song_id);
    if (library.selectedSongId === row.song_id) {
      selectedSongSummary = row;
      if (indexIsCurrent && index !== undefined) selectedSongIndex = index;
    }
    if (index !== undefined) {
      const offset = Math.floor(index / LIBRARY_PAGE_SIZE) * LIBRARY_PAGE_SIZE;
      let page = changedPages.get(offset);
      if (!page) {
        const currentPage = library.pages.get(offset);
        if (!currentPage) continue;
        page = [...currentPage];
        changedPages.set(offset, page);
      }
      page[index - offset] = row;
    }
  }

  if (changedPages.size > 0) {
    pages = new Map(library.pages);
    for (const [offset, page] of changedPages) pages.set(offset, page);
  }
  return {
    ...library,
    ...(pages ? { pages } : {}),
    selectedSongSummary,
    selectedSongIndex,
  };
}

export function cacheDetail(
  entries: Map<string, DetailEntry>,
  songId: string,
  entry: DetailEntry,
): Map<string, DetailEntry> {
  const next = new Map(entries);
  next.delete(songId);
  next.set(songId, entry);
  while (next.size > DETAIL_CACHE_LIMIT) {
    const oldest = next.keys().next().value;
    if (oldest === undefined) break;
    next.delete(oldest);
  }
  return next;
}

export function navigationFromDto(navigation: LibraryNavigation): LibraryNavigationState {
  const playlistsById = new Map(
    navigation.playlists.map((playlist) => [playlist.id, { ...playlist }]),
  );
  return {
    loadState: 'ready',
    playlistOrder: navigation.playlists.map((playlist) => playlist.id),
    playlistsById,
    pendingMutations: new Set(),
    lastError: null,
  };
}

export function updatePlaylistSummary(
  state: LibraryNavigationState,
  summary: LibraryPlaylistSummary,
): LibraryNavigationState {
  const playlistsById = new Map(state.playlistsById);
  playlistsById.set(summary.id, { ...summary });
  return {
    ...state,
    playlistsById,
    playlistOrder: state.playlistOrder.includes(summary.id)
      ? state.playlistOrder
      : [...state.playlistOrder, summary.id],
  };
}
