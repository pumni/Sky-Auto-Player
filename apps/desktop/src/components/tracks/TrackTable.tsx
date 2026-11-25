import { useVirtualizer } from '@tanstack/react-virtual';
import {
  type KeyboardEvent,
  type Ref,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import {
  type DesktopStore,
  type DesktopStoreHook,
  LIBRARY_PAGE_SIZE,
  playbackContextMatchesLibrary,
  selectNowPlayingSongId,
  selectRowAtIndex,
} from '../../state/store';
import { TRACK_ROW_HEIGHT_PX, VirtualTrackRow } from './TrackRow';
import { TrackTableHeader } from './TrackTableHeader';
import {
  ScrollAreaContent,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';

export interface TrackTableHandle {
  locateSelected: () => Promise<void>;
}

interface TrackTableProps {
  useStore: DesktopStoreHook;
  locateSelectedRef?: Ref<TrackTableHandle>;
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

function observeDimensions(
  instance: { scrollElement: HTMLDivElement | null },
  cb: (rect: { width: number; height: number }) => void,
) {
  const element = instance.scrollElement;
  if (!element) return;
  let lastHeight = Math.round(element.getBoundingClientRect().height);
  let lastWidth = Math.round(element.getBoundingClientRect().width);
  cb({ width: lastWidth, height: lastHeight });
  if (typeof ResizeObserver === 'undefined') return;
  const observer = new ResizeObserver((entries) => {
    const entry = entries[0];
    const nextHeight = Math.round(
      entry?.borderBoxSize?.[0]?.blockSize ??
        entry?.contentRect.height ??
        element.getBoundingClientRect().height,
    );
    const nextWidth = Math.round(
      entry?.borderBoxSize?.[0]?.inlineSize ??
        entry?.contentRect.width ??
        element.getBoundingClientRect().width,
    );
    if (nextHeight === lastHeight && nextWidth === lastWidth) return;
    lastHeight = nextHeight;
    lastWidth = nextWidth;
    cb({ width: nextWidth, height: nextHeight });
  });
  observer.observe(element, { box: 'border-box' });
  return () => observer.disconnect();
}

export function TrackTable({ useStore, locateSelectedRef }: TrackTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const setScrollElement = useCallback((element: HTMLDivElement | null) => {
    parentRef.current = element;
  }, []);
  const resultTotal = useStore((store) => store.library.resultTotal);
  const committedPage = useStore((store) => store.library.builtinPage.committedPage);
  const focusNowPlaying = useStore((store) => store.library.builtinPage.focusNowPlaying);
  const pendingPage = useStore((store) => store.library.builtinPage.pendingPage);
  const searchSource = useStore((store) => store.library.searchSource);
  const setViewport = useStore((store: DesktopStore) => store.setViewport);
  const selectSong = useStore((store: DesktopStore) => store.selectSong);
  const [keyboardIndex, setKeyboardIndex] = useState<number | null>(null);
  const pendingKeyboardNavigation = useRef<PendingKeyboardNavigation | null>(null);
  const pendingLocate = useRef<{
    page: number;
    index: number;
    songId: string;
    generation: number;
  } | null>(null);
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
    estimateSize: () => TRACK_ROW_HEIGHT_PX,
    overscan: 10,
    observeElementRect: observeDimensions,
  });
  useImperativeHandle(
    locateSelectedRef,
    () => ({
      locateSelected: async () => {
        const library = useStore.getState().library;
        const index = library.selectedSongIndex;
        if (
          index === null ||
          !library.selectedSongId ||
          index < 0 ||
          index >= library.resultTotal ||
          library.membershipRevision !== library.indexMembershipRevision
        ) {
          return;
        }
        const targetPage = isBuiltin ? Math.floor(index / LIBRARY_PAGE_SIZE) + 1 : 1;
        if (isBuiltin && targetPage !== library.builtinPage.committedPage) {
          pendingLocate.current = {
            page: targetPage,
            index,
            songId: library.selectedSongId,
            generation: library.generation,
          };
          await useStore.getState().navigateBuiltinPage(targetPage);
          if (useStore.getState().library.builtinPage.committedPage !== targetPage) {
            pendingLocate.current = null;
          }
          return;
        }
        virtualizer.scrollToIndex(index - pageOffset, { align: 'center' });
        parentRef.current?.focus({ preventScroll: true });
      },
    }),
    [isBuiltin, pageOffset, useStore, virtualizer],
  );

  useLayoutEffect(() => {
    const pending = pendingLocate.current;
    if (!pending || !isBuiltin || committedPage !== pending.page) return;
    pendingLocate.current = null;
    const library = useStore.getState().library;
    if (library.selectedSongId !== pending.songId || library.generation !== pending.generation) {
      return;
    }
    virtualizer.scrollToIndex(pending.index - pageOffset, { align: 'center' });
    parentRef.current?.focus({ preventScroll: true });
  }, [committedPage, isBuiltin, pageOffset, useStore, virtualizer]);

  const virtualItems = virtualizer.getVirtualItems();
  const renderedItems = useMemo(
    () =>
      virtualItems.length > 0
        ? virtualItems
        : [
            ...Array.from({ length: Math.min(rowCount, 20) }, (_, index) => ({
              index,
              start: index * TRACK_ROW_HEIGHT_PX,
            })),
            ...(keyboardIndex !== null && keyboardIndex >= 20 && keyboardIndex < rowCount
              ? [{ index: keyboardIndex, start: keyboardIndex * TRACK_ROW_HEIGHT_PX }]
              : []),
          ],
    [keyboardIndex, rowCount, virtualItems],
  );
  const first = renderedItems[0]?.index ?? 0;
  const last = renderedItems.at(-1)?.index ?? -1;
  useEffect(() => {
    if (last < first) return;
    void setViewport(pageOffset + first, pageOffset + last);
  }, [first, last, pageOffset, setViewport]);

  useLayoutEffect(() => {
    if (!isBuiltin || !focusNowPlaying) return;
    const currentPageOffset = (committedPage - 1) * LIBRARY_PAGE_SIZE;
    const localIndex = focusNowPlaying.index - currentPageOffset;
    if (
      focusNowPlaying.index < 0 ||
      focusNowPlaying.index >= resultTotal ||
      localIndex < 0 ||
      localIndex >= rowCount
    ) {
      return;
    }

    virtualizer.scrollToIndex(localIndex, { align: 'center' });
    const latest = useStore.getState().library;
    if (latest.builtinPage.focusNowPlaying?.id === focusNowPlaying.id) {
      useStore.setState({
        library: {
          ...latest,
          builtinPage: { ...latest.builtinPage, focusNowPlaying: null },
        },
      });
    }
  }, [committedPage, focusNowPlaying, isBuiltin, resultTotal, rowCount, useStore, virtualizer]);

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

  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const pages = useStore((store) => store.library.pages);
  const activeDescendantId = useMemo(() => {
    if (!selectedSongId) return undefined;
    for (const item of renderedItems) {
      if (selectRowAtIndex({ pages }, pageOffset + item.index)?.song_id === selectedSongId) {
        return `song-row-${selectedSongId}`;
      }
    }
    return undefined;
  }, [selectedSongId, pages, renderedItems, pageOffset]);

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
      moveActive(Math.max(activeIndex, 0));
    }
  };

  return (
    <ScrollAreaRoot className="min-h-0 min-w-0 flex-1 bg-surface">
      <ScrollAreaViewport
        ref={setScrollElement}
        className="size-full focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring"
        role="grid"
        aria-label={isBuiltin ? `Built-in Songs, page ${committedPage}` : 'Songs'}
        aria-activedescendant={activeDescendantId}
        aria-rowcount={rowCount + 1}
        aria-busy={pendingPage !== null || undefined}
        aria-multiselectable="false"
        tabIndex={0}
        onKeyDown={onTableKeyDown}
      >
        <ScrollAreaContent className="relative min-h-full w-full bg-surface">
          <div data-slot="track-table-frame" className="relative min-h-full min-w-0">
            <TrackTableHeader />
            <div
              className="relative min-h-full w-full"
              style={{ height: virtualizer.getTotalSize() }}
            >
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
        </ScrollAreaContent>
      </ScrollAreaViewport>
      <ScrollAreaScrollbar orientation="vertical">
        <ScrollAreaThumb />
      </ScrollAreaScrollbar>
    </ScrollAreaRoot>
  );
}
