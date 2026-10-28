import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import {
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
    left.playlistAddMode?.playlistId === right.playlistAddMode?.playlistId
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
  searchRequestGeneration: number;
  scopeKey: string;
}

export function TrackTable({ useStore }: TrackTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const scrollVisibilityRef = useScrollVisibility<HTMLDivElement>();
  const resultTotal = useStore((store) => store.library.resultTotal);
  const setViewport = useStore((store: DesktopStore) => store.setViewport);
  const selectSong = useStore((store: DesktopStore) => store.selectSong);
  const [keyboardIndex, setKeyboardIndex] = useState<number | null>(null);
  const pendingKeyboardNavigation = useRef<PendingKeyboardNavigation | null>(null);
  const keyboardNavigationId = useRef(0);
  // TanStack Virtual exposes a mutable controller by design; React Compiler
  // must not memoize this third-party hook's return value.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: resultTotal,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 46,
    overscan: 10,
  });
  const virtualItems = virtualizer.getVirtualItems();
  const renderedItems =
    virtualItems.length > 0
      ? virtualItems
      : [
          ...Array.from({ length: Math.min(resultTotal, 20) }, (_, index) => ({
            index,
            start: index * 46,
          })),
          ...(keyboardIndex !== null && keyboardIndex >= 20 && keyboardIndex < resultTotal
            ? [{ index: keyboardIndex, start: keyboardIndex * 46 }]
            : []),
        ];
  const first = renderedItems[0]?.index ?? 0;
  const last = renderedItems.at(-1)?.index ?? -1;
  useEffect(() => {
    void setViewport(first, last);
  }, [first, last, setViewport]);

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

  const moveActive = (nextIndex: number) => {
    if (resultTotal === 0) return;
    const next = Math.max(0, Math.min(resultTotal - 1, nextIndex));
    const library = useStore.getState().library;
    const navigation: PendingKeyboardNavigation = {
      id: ++keyboardNavigationId.current,
      index: next,
      searchRequestGeneration: library.searchRequestGeneration,
      scopeKey: libraryScopeKey(library),
    };
    setKeyboardIndex(next);
    virtualizer.scrollToIndex(next, { align: 'auto' });
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
            libraryScopeKey(latestLibrary) !== navigation.scopeKey
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
    if (resultTotal === 0 || event.target !== event.currentTarget) return;
    const state = useStore.getState();
    const selectedIndex = state.library.selectedSongId
      ? (state.library.indexById.get(state.library.selectedSongId) ??
        state.library.selectedSongIndex ??
        -1)
      : -1;
    const pendingNavigation = pendingKeyboardNavigation.current;
    const pendingMatchesCurrentResults =
      pendingNavigation !== null &&
      pendingNavigation.searchRequestGeneration === state.library.searchRequestGeneration &&
      pendingNavigation.scopeKey === libraryScopeKey(state.library);
    if (pendingNavigation && !pendingMatchesCurrentResults) {
      pendingKeyboardNavigation.current = null;
    }
    const activeIndex = pendingMatchesCurrentResults
      ? (pendingNavigation?.index ?? -1)
      : selectedIndex;
    const currentIndex = activeIndex >= 0 ? activeIndex : 0;
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
      moveActive(resultTotal - 1);
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
      aria-label="Songs"
      aria-rowcount={resultTotal + 1}
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
