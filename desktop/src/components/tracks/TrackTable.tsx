import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { useScrollVisibility } from '../../hooks/useScrollVisibility';
import { selectRowAtIndex, type DesktopStore, type DesktopStoreHook } from '../../state/store';
import { VirtualTrackRow } from './TrackRow';
import { TrackTableHeader } from './TrackTableHeader';

interface TrackTableProps {
  useStore: DesktopStoreHook;
}

export function TrackTable({ useStore }: TrackTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const scrollVisibilityRef = useScrollVisibility<HTMLDivElement>();
  const resultTotal = useStore((store) => store.library.resultTotal);
  const setViewport = useStore((store: DesktopStore) => store.setViewport);
  const selectSong = useStore((store: DesktopStore) => store.selectSong);
  const [keyboardIndex, setKeyboardIndex] = useState<number | null>(null);
  const pendingKeyboardIndex = useRef<number | null>(null);
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
    const grid = parentRef.current;
    if (!grid) return;
    let selectedSongId: string | null | undefined;
    const syncActiveDescendant = (nextSelectedSongId: string | null | undefined) => {
      if (nextSelectedSongId === selectedSongId) return;
      selectedSongId = nextSelectedSongId;
      const row = selectRowAtIndex(
        useStore.getState().library,
        nextSelectedSongId
          ? (useStore.getState().library.indexById.get(nextSelectedSongId) ?? -1)
          : -1,
      );
      if (row) grid.setAttribute('aria-activedescendant', `song-row-${row.song_id}`);
      else grid.removeAttribute('aria-activedescendant');
    };
    syncActiveDescendant(selectedSongId);
    return useStore.subscribe((state) => {
      syncActiveDescendant(state.library.selectedSongId);
    });
  }, [useStore]);

  const moveActive = (nextIndex: number) => {
    const next = Math.max(0, Math.min(resultTotal - 1, nextIndex));
    setKeyboardIndex(next);
    virtualizer.scrollToIndex(next, { align: 'auto' });
    const row = selectRowAtIndex(useStore.getState().library, next);
    if (row) {
      pendingKeyboardIndex.current = null;
      void selectSong(row.song_id);
    } else {
      pendingKeyboardIndex.current = next;
      void setViewport(next, next).then(() => {
        if (pendingKeyboardIndex.current !== next) return;
        const loadedRow = selectRowAtIndex(useStore.getState().library, next);
        if (!loadedRow) return;
        pendingKeyboardIndex.current = null;
        void selectSong(loadedRow.song_id);
      });
    }
  };

  const onTableKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (resultTotal === 0 || event.target !== event.currentTarget) return;
    const state = useStore.getState();
    const selectedIndex = state.library.selectedSongId
      ? (state.library.indexById.get(state.library.selectedSongId) ?? -1)
      : -1;
    const current = selectedIndex >= 0 ? selectedIndex : 0;
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      moveActive(current + 1);
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      moveActive(current - 1);
    } else if (event.key === 'Home') {
      event.preventDefault();
      moveActive(0);
    } else if (event.key === 'End') {
      event.preventDefault();
      moveActive(resultTotal - 1);
    } else if (event.key === 'Enter' || event.key === ' ') {
      const row = selectRowAtIndex(state.library, current);
      if (row) {
        event.preventDefault();
        void selectSong(row.song_id);
      }
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
  );
}
