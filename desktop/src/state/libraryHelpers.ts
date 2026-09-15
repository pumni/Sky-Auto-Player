import type {
  LibraryNavigation,
  LibraryPlaylistSummary,
  LibrarySource,
  SongRow,
} from '../bridge/DesktopBridge';
import { DETAIL_CACHE_LIMIT, LIBRARY_PAGE_SIZE } from './types';
import type { DetailEntry, LibraryNavigationState, LibraryState } from './types';

export const sourceKey = (source: LibrarySource): string => JSON.stringify(source);

export function updateLoadedRows(library: LibraryState, updates: readonly SongRow[]): LibraryState {
  const pages = new Map(library.pages);
  const changedPages = new Map<number, SongRow[]>();

  for (const row of updates) {
    const index = library.indexById.get(row.song_id);
    if (index === undefined) continue;
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

  for (const [offset, page] of changedPages) pages.set(offset, page);
  return { ...library, pages };
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
