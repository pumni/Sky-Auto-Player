import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { TrackRow } from './TrackRow';
import { TrackTableHeader } from './TrackTableHeader';

interface TrackTableProps {
  useStore: DesktopStoreHook;
}

export function TrackTable({ useStore }: TrackTableProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const rows = useStore((store: DesktopStore) => store.library.rows);
  const total = useStore((store: DesktopStore) => store.library.total);
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);
  const setViewport = useStore((store: DesktopStore) => store.setViewport);
  const selectSong = useStore((store: DesktopStore) => store.selectSong);
  const [activeIndex, setActiveIndex] = useState(0);
  const pendingKeyboardIndex = useRef<number | null>(null);
  const virtualizer = useVirtualizer({
    count: total,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 46,
    overscan: 10,
  });
  const virtualItems = virtualizer.getVirtualItems();
  const renderedItems =
    virtualItems.length > 0
      ? virtualItems
      : [
          ...Array.from({ length: Math.min(total, 20) }, (_, index) => ({
            index,
            start: index * 46,
          })),
          ...(activeIndex >= 20 && activeIndex < total
            ? [{ index: activeIndex, start: activeIndex * 46 }]
            : []),
        ];
  const first = renderedItems[0]?.index ?? 0;
  const last = renderedItems.at(-1)?.index ?? -1;

  useEffect(() => {
    void setViewport(first, last);
  }, [first, last, setViewport]);

  useEffect(() => {
    setActiveIndex((current) => (total === 0 ? 0 : Math.min(current, total - 1)));
  }, [total]);

  useEffect(() => {
    if (pendingKeyboardIndex.current !== activeIndex) return;
    const row = rows[activeIndex];
    if (!row) return;
    pendingKeyboardIndex.current = null;
    void selectSong(row.song_id);
  }, [activeIndex, rows, selectSong]);

  const activeRow = rows[activeIndex];
  const moveActive = (nextIndex: number) => {
    const next = Math.max(0, Math.min(total - 1, nextIndex));
    setActiveIndex(next);
    virtualizer.scrollToIndex(next, { align: 'auto' });
    const row = rows[next];
    if (row) {
      pendingKeyboardIndex.current = null;
      void selectSong(row.song_id);
    } else {
      pendingKeyboardIndex.current = next;
      void setViewport(next, next);
    }
  };

  const onTableKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (total === 0) return;
    const selectedIndex = selectedSongId
      ? rows.findIndex((row) => row?.song_id === selectedSongId)
      : -1;
    const current = selectedIndex >= 0 ? selectedIndex : activeIndex;
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
      moveActive(total - 1);
    } else if (event.key === 'Enter' || event.key === ' ') {
      const row = rows[current];
      if (row) {
        event.preventDefault();
        setActiveIndex(current);
        void selectSong(row.song_id);
      }
    }
  };

  return (
    <div
      ref={parentRef}
      className="track-table"
      role="grid"
      aria-label="Songs"
      aria-rowcount={total + 1}
      aria-activedescendant={activeRow ? `song-row-${activeRow.song_id}` : undefined}
      aria-multiselectable="false"
      tabIndex={0}
      onKeyDown={onTableKeyDown}
    >
      <TrackTableHeader />
      <div className="track-table-virtual-inner" style={{ height: virtualizer.getTotalSize() }}>
        {renderedItems.map((virtualRow) => {
          const row = rows[virtualRow.index];
          if (!row) {
            return (
              <div
                key={`loading-${virtualRow.index}`}
                className="track-table-row track-row track-row-placeholder"
                role="row"
                aria-busy="true"
                aria-rowindex={virtualRow.index + 2}
                style={{ transform: `translateY(${virtualRow.start}px)` }}
              >
                <span className="track-cell track-cell-index" role="gridcell">
                  {virtualRow.index + 1}
                </span>
                <span className="track-cell track-cell-title" role="gridcell">
                  Loading song…
                </span>
              </div>
            );
          }
          return (
            <TrackRow
              key={row.song_id}
              row={row}
              index={virtualRow.index}
              selected={row.song_id === selectedSongId}
              start={virtualRow.start}
              onFocus={() => setActiveIndex(virtualRow.index)}
              onSelect={() => {
                setActiveIndex(virtualRow.index);
                void selectSong(row.song_id);
              }}
            />
          );
        })}
      </div>
    </div>
  );
}
