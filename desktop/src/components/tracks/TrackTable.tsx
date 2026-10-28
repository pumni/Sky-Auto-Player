import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import {
  LIBRARY_PAGE_SIZE,
  selectNowPlayingSongId,
  selectRowAtIndex,
  playbackContextMatchesLibrary,
  type DesktopStore,
  type DesktopStoreHook,
} from '../../state/store';
import { VirtualTrackRow } from './TrackRow';
import { TrackTableHeader } from './TrackTableHeader';

interface TrackTableProps {
  useStore: DesktopStoreHook;
}

function libraryScopeKey(library: DesktopStore['library']): string {
  return JSON.stringify([
    library.query,
    library.source,
    library.searchSource,
    library.playlistAddMode?.playlistId ?? null,
    library.searchSource.kind === 'smart' && library.searchSource.id === 'builtin'
      ? library.builtinPage.committedPage
      : null,
  ]);
}

function sameLibrarySource(
  left: DesktopStore['library']['source'],
  right: DesktopStore['library']['source'],
) {
  return left.kind === right.kind && left.id === right.id;
}

function sameLibraryScope(left: DesktopStore['library'], right: DesktopStore['library']): boolean {
  return (
    left.query === right.query &&
    sameLibrarySource(left.source, right.source) &&
    sameLibrarySource(left.searchSource, right.searchSource) &&
    left.playlistAddMode?.playlistId === right.playlistAddMode?.playlistId &&
    (left.searchSource.kind !== 'smart' ||
      left.searchSource.id !== 'builtin' ||
      left.builtinPage.committedPage === right.builtinPage.committedPage)
  );
}

function sameLibraryForKeyboardNavigation(
  left: DesktopStore['library'],
  right: DesktopStore['library'],
): boolean {
  return (
    sameLibraryScope(left, right) &&
    left.searchRequestGeneration === right.searchRequestGeneration &&
    left.generation === right.generation
  );
}

interface PendingKeyboardNavigation {
  id: number;
  index: number;
  page: number;
  searchRequestGeneration: number;
  scopeKey: string;
}

export function TrackTable({ useStore }: TrackTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const scrollVisibilityRef = useScrollVisibility<HTMLDivElement>();
  const resultTotal = useStore((store) => store.library.resultTotal);
  const committedPage = useStore((store) => store.library.builtinPage.committedPage);
  const pendingPage = useStore((store) => store.library.builtinPage.pendingPage);
  const searchSource = useStore((store) => store.library.searchSource);
  const setViewport = useStore((store: DesktopStore) => store.setViewport);
  const selectSong = useStore((store: DesktopStore) => store.selectSong);
  const [keyboardIndex, setKeyboardIndex] = useState<number | null>(null);
  const pendingKeyboardNavigation = useRef<PendingKeyboardNavigation | null>(null);
  const keyboardNavigationId = useRef(0);
  const isBuiltin = searchSource.kind === 'smart' && searchSource.id === 'builtin';
  const pageOffset = isBuiltin ? (committedPage - 1) * LIBRARY_PAGE_SIZE : 0;
  const rowCount = isBuiltin
    ? Math.max(0, Math.min(LIBRARY_PAGE_SIZE, resultTotal - pageOffset))
    : resultTotal;
  // TanStack Virtual exposes a mutable controller by design; React Compiler
  // must not memoize this third-party hook's return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 46,
    overscan: 10,
  });
  const virtualItems = virtualizer.getVirtualItems();
  const renderedItems =
    virtualItems.length > 0
      ? virtualItems
      : [
          ...Array.from({ length: Math.min(rowCount, 20) }, (_, index) => ({
            index,
            start: index * 46,
          })),
          ...(keyboardIndex !== null && keyboardIndex >= 20 && keyboardIndex < rowCount
            ? [{ index: keyboardIndex, start: keyboardIndex * 46 }]
            : []),
        ];
  const first = renderedItems[0]?.index ?? 0;
  const last = renderedItems.at(-1)?.index ?? -1;
  useEffect(() => {
    if (last < first) return;
    void setViewport(pageOffset + first, pageOffset + last);
  }, [first, last, pageOffset, setViewport]);

  useEffect(
    () =>
      useStore.subscribe(
        (state) => state.library.builtinPage.focusNowPlaying,
        (request) => {
          if (!request || !isBuiltin) return;
          const currentPageOffset =
            (useStore.getState().library.builtinPage.committedPage - 1) * LIBRARY_PAGE_SIZE;
          if (
            request.index < currentPageOffset ||
            request.index >= currentPageOffset + LIBRARY_PAGE_SIZE
          ) {
            return;
          }
          virtualizer.scrollToIndex(request.index - currentPageOffset, { align: 'center' });
        },
      ),
    [isBuiltin, useStore, virtualizer],
  );

  useEffect(() => {
    return useStore.subscribe(
      (state) => state.library,
      (library, previousLibrary) => {
        const scopeChanged = !sameLibraryScope(library, previousLibrary);
        const generationChanged = library.generation !== previousLibrary.generation;
        const searchRequestChanged =
          library.searchRequestGeneration !== previousLibrary.searchRequestGeneration;
        if (!scopeChanged && !searchRequestChanged && !generationChanged) return;
        keyboardNavigationId.current += 1;
        pendingKeyboardNavigation.current = null;
        setKeyboardIndex(null);
        if ((scopeChanged || generationChanged) && parentRef.current)
          parentRef.current.scrollTop = 0;
      },
      { equalityFn: sameLibraryForKeyboardNavigation },
    );
  }, [useStore]);

  useEffect(() => {
    return useStore.subscribe(
      (state) => state.playback.currentSong?.songId ?? null,
      (nextSongId) => {
        if (!nextSongId) return;
        const state = useStore.getState();
        if (selectNowPlayingSongId(state) !== nextSongId) return;
        const context = state.playback.context;
        if (
          context &&
          context.currentSongId === nextSongId &&
          playbackContextMatchesLibrary(context, state.library)
        ) {
          if (
            state.library.searchSource.kind === 'smart' &&
            state.library.searchSource.id === 'builtin'
          ) {
            return;
          }
          virtualizer.scrollToIndex(context.currentIndex, { align: 'auto' });
        }
      },
    );
  }, [useStore, virtualizer]);

  useEffect(() => {
    const grid = parentRef.current;
    if (!grid) return;
    const syncActiveDescendant = () => {
      const selectedSongId = useStore.getState().library.selectedSongId;
      const selectedRow = selectedSongId
        ? document.getElementById(`song-row-${selectedSongId}`)
        : null;
      const activeDescendant =
        selectedRow && grid.contains(selectedRow) && selectedRow.getAttribute('role') === 'row'
          ? selectedRow.id
          : null;
      if (activeDescendant) grid.setAttribute('aria-activedescendant', activeDescendant);
      else grid.removeAttribute('aria-activedescendant');
    };
    const unsubscribe = useStore.subscribe(
      (state) => state.library.selectedSongId,
      syncActiveDescendant,
    );
    // Virtual rows can unmount without a store selection change when the viewport moves.
    const observer = new MutationObserver(syncActiveDescendant);
    observer.observe(grid, { childList: true, subtree: true });
    syncActiveDescendant();
    return () => {
      unsubscribe();
      observer.disconnect();
      grid.removeAttribute('aria-activedescendant');
    };
  }, [useStore]);

  const moveActive = (nextLocalIndex: number) => {
    if (rowCount === 0) return;
    const nextLocal = Math.max(0, Math.min(rowCount - 1, nextLocalIndex));
    const next = pageOffset + nextLocal;
    const library = useStore.getState().library;
    const navigation: PendingKeyboardNavigation = {
      id: ++keyboardNavigationId.current,
      index: next,
      page: library.builtinPage.committedPage,
      searchRequestGeneration: library.searchRequestGeneration,
      scopeKey: libraryScopeKey(library),
    };
    setKeyboardIndex(nextLocal);
    virtualizer.scrollToIndex(nextLocal, { align: 'auto' });
    const row = selectRowAtIndex(library, next);
    if (row) {
      pendingKeyboardNavigation.current = null;
      void selectSong(row.song_id);
    } else {
      pendingKeyboardNavigation.current = navigation;
      void setViewport(next, next)
        .then(() => {
          if (pendingKeyboardNavigation.current?.id !== navigation.id) return;
          const latestLibrary = useStore.getState().library;
          if (
            latestLibrary.searchRequestGeneration !== navigation.searchRequestGeneration ||
            libraryScopeKey(latestLibrary) !== navigation.scopeKey ||
            latestLibrary.builtinPage.committedPage !== navigation.page
          ) {
            pendingKeyboardNavigation.current = null;
            return;
          }
          const loadedRow = selectRowAtIndex(latestLibrary, navigation.index);
          pendingKeyboardNavigation.current = null;
          if (loadedRow) void selectSong(loadedRow.song_id);
        })
        .catch(() => {
          if (pendingKeyboardNavigation.current?.id === navigation.id) {
            pendingKeyboardNavigation.current = null;
          }
        });
    }
  };

  const onTableKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (rowCount === 0 || event.target !== event.currentTarget) return;
    const state = useStore.getState();
    const indexIsCurrent =
      state.library.indexMembershipRevision === state.library.membershipRevision;
    const selectedIndex =
      state.library.selectedSongId && indexIsCurrent
        ? (state.library.indexById.get(state.library.selectedSongId) ??
          state.library.selectedSongIndex ??
          -1)
        : -1;
    const pendingNavigation = pendingKeyboardNavigation.current;
    const pendingMatchesCurrentResults =
      pendingNavigation !== null &&
      pendingNavigation.searchRequestGeneration === state.library.searchRequestGeneration &&
      pendingNavigation.scopeKey === libraryScopeKey(state.library) &&
      pendingNavigation.page === state.library.builtinPage.committedPage;
    if (pendingNavigation && !pendingMatchesCurrentResults) {
      pendingKeyboardNavigation.current = null;
    }
    const activeSourceIndex = pendingMatchesCurrentResults
      ? (pendingNavigation?.index ?? -1)
      : selectedIndex;
    const activeIndex = isBuiltin ? activeSourceIndex - pageOffset : activeSourceIndex;
    const currentIndex = activeIndex >= 0 && activeIndex < rowCount ? activeIndex : 0;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      moveActive(currentIndex + 1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      moveActive(currentIndex - 1);
    } else if (event.key === 'Home') {
      event.preventDefault();
      moveActive(0);
    } else if (event.key === 'End') {
      event.preventDefault();
      moveActive(rowCount - 1);
    } else if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault();
      moveActive(activeIndex < 0 ? 0 : activeIndex);
    }
  };

  return (
    <div
      ref={(element) => {
        parentRef.current = element;
        scrollVisibilityRef.current = element;
      }}
      className="track-table scroll-surface"
      role="grid"
      aria-label={isBuiltin ? `Built-in Songs, page ${committedPage}` : 'Songs'}
      aria-rowcount={rowCount + 1}
      aria-busy={pendingPage !== null || undefined}
      aria-multiselectable="false"
      tabIndex={0}
      onKeyDown={onTableKeyDown}
    >
      <div className="track-table-frame">
        <TrackTableHeader />
        <div className="track-table-virtual-inner" style={{ height: virtualizer.getTotalSize() }}>
          {renderedItems.map((virtualRow) => {
            return (
              <VirtualTrackRow
                key={virtualRow.index}
                index={virtualRow.index}
                sourceIndex={pageOffset + virtualRow.index}
                start={virtualRow.start}
                useStore={useStore}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}
