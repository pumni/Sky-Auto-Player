import { useVirtualizer } from '@tanstack/react-virtual';
import { useEffect, useRef } from 'react';
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
  const virtualizer = useVirtualizer({
    count: total,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 44,
    overscan: 10,
  });
  const virtualItems = virtualizer.getVirtualItems();
  const renderedItems =
    virtualItems.length > 0
      ? virtualItems
      : Array.from({ length: Math.min(total, 20) }, (_, index) => ({
          index,
          start: index * 44,
        }));
  const first = renderedItems[0]?.index ?? 0;
  const last = renderedItems.at(-1)?.index ?? -1;

  useEffect(() => {
    void setViewport(first, last);
  }, [first, last, setViewport]);

  return (
    <section className="library-panel" aria-labelledby="library-title">
      <div className="panel-heading">
        <div>
          <p className="eyebrow">COLLECTION</p>
          <h2 id="library-title">Library</h2>
        </div>
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
        <div ref={parentRef} className="virtual-list" role="listbox" aria-label="Songs">
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
                  className={`song-row${selected ? ' is-selected' : ''}`}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  onClick={() => void selectSong(row.song_id)}
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
