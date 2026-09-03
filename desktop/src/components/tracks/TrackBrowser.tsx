import { PanelRight } from 'lucide-react';
import type { RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { TrackTable } from './TrackTable';

interface TrackBrowserProps {
  detailsTriggerRef?: RefObject<HTMLButtonElement | null> | undefined;
  useStore: DesktopStoreHook;
}

export function TrackBrowser({ detailsTriggerRef, useStore }: TrackBrowserProps) {
  const total = useStore((store: DesktopStore) => store.library.total);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);
  const openUtility = useStore((store: DesktopStore) => store.openUtility);

  return (
    <section className="track-browser" aria-labelledby="track-browser-title">
      <header className="track-browser-header">
        <div>
          <h1 id="track-browser-title">All Songs</h1>
          <span className="track-browser-count">{loading ? 'Updating…' : `${total} songs`}</span>
        </div>
        <button
          ref={detailsTriggerRef}
          className="icon-button track-details-trigger"
          type="button"
          aria-label="Open song details"
          title="Open song details"
          disabled={!selectedSongId}
          onClick={() => openUtility('details')}
        >
          <PanelRight size={16} aria-hidden="true" />
        </button>
      </header>
      {error && (
        <p className="inline-error" role="status">
          {error}
        </p>
      )}
      {!loading && total === 0 ? (
        <div className="empty-state">
          <strong>No songs found</strong>
          <span className="muted">Try another search or reload the library.</span>
        </div>
      ) : (
        <TrackTable useStore={useStore} />
      )}
    </section>
  );
}
