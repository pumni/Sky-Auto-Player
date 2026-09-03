import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { TrackTable } from './TrackTable';

interface TrackBrowserProps {
  useStore: DesktopStoreHook;
}

export function TrackBrowser({ useStore }: TrackBrowserProps) {
  const resultTotal = useStore((store: DesktopStore) => store.library.resultTotal);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);

  return (
    <section className="track-browser" aria-labelledby="track-browser-title">
      <header className="track-browser-header">
        <div>
          <h1 id="track-browser-title">All Songs</h1>
          <span className="track-browser-count">
            {loading ? 'Updating…' : `${resultTotal} songs`}
          </span>
        </div>
      </header>
      {error && (
        <p className="inline-error" role="status">
          {error}
        </p>
      )}
      {!loading && resultTotal === 0 ? (
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
