import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import type { DesktopStoreHook, DesktopStore } from '../../state/store';

interface LibraryPanelProps {
  useStore: DesktopStoreHook;
}

function durationLabel(durationUs: number | null): string {
  if (durationUs === null) return '—';
  const seconds = Math.max(0, Math.round(durationUs / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

export function LibraryPanel({ useStore }: LibraryPanelProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const rows = useStore((store: DesktopStore) => store.library.rows);
  const total = useStore((store: DesktopStore) => store.library.total);
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);
  const selectSong = useStore((store: DesktopStore) => store.selectSong);
  const setViewport = useStore((store: DesktopStore) => store.setViewport);
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
      // Keep the active index while the corresponding page is fetched. The
      // effect above commits selection only after the row arrives, so End and
      // Home retain their keyboard semantics across unloaded pages.
      pendingKeyboardIndex.current = next;
      void setViewport(next, next);
    }
  };

  const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
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
    <section className="library-panel" aria-labelledby="library-title">
      <div className="panel-heading">
        <h2 id="library-title">Library</h2>
        <span className="count-label">{loading ? 'Updating…' : `${total} songs`}</span>
      </div>
      {error && (
        <p className="inline-error" role="status">
          {error}
        </p>
      )}
      {!loading && total === 0 ? (
        <div className="empty-state">
          <span className="empty-glyph" aria-hidden="true">
            ∿
          </span>
          <strong>{total === 0 ? 'No songs found' : 'No matches'}</strong>
          <span className="muted">Try another search or reload the library.</span>
        </div>
      ) : (
        <div
          ref={parentRef}
          className="virtual-list"
          role="listbox"
          aria-label="Songs"
          aria-activedescendant={activeRow ? `song-option-${activeRow.song_id}` : undefined}
          aria-multiselectable="false"
          tabIndex={0}
          onKeyDown={onListKeyDown}
        >
          <div className="virtual-list-inner" style={{ height: virtualizer.getTotalSize() }}>
            {renderedItems.map((virtualRow) => {
              const row = rows[virtualRow.index];
              if (!row) {
                return (
                  <div
                    key={`loading-${virtualRow.index}`}
                    className="song-row song-row-placeholder"
                    aria-busy="true"
                    style={{ transform: `translateY(${virtualRow.start}px)` }}
                  >
                    <span className="song-row-title">Loading song…</span>
                  </div>
                );
              }
              const selected = row.song_id === selectedSongId;
              return (
                <button
                  key={row.song_id}
                  id={`song-option-${row.song_id}`}
                  className={`song-row${selected ? ' is-selected' : ''}`}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  aria-setsize={total}
                  aria-posinset={virtualRow.index + 1}
                  tabIndex={-1}
                  onFocus={() => setActiveIndex(virtualRow.index)}
                  onClick={() => {
                    setActiveIndex(virtualRow.index);
                    void selectSong(row.song_id);
                  }}
                  style={{ transform: `translateY(${virtualRow.start}px)` }}
                >
                  <span className="song-row-title">{row.title}</span>
                  <span className="song-row-meta">
                    <span>{durationLabel(row.duration_us)}</span>
                    <span
                      className={`risk-dot risk-${row.risk_level}`}
                      aria-label={`${row.risk_level} risk`}
                    />
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      )}
    </section>
  );
}
