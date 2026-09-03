import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { TrackTable } from './TrackTable';

interface TrackBrowserProps {
  useStore: DesktopStoreHook;
}

export function TrackBrowser({ useStore }: TrackBrowserProps) {
  const source = useStore((store: DesktopStore) => store.library.source);
  const resultTotal = useStore((store: DesktopStore) => store.library.resultTotal);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);
  const sourceName = useStore((store: DesktopStore) => {
    if (store.library.source.kind === 'collection') {
      return (
        store.libraryNavigation.collectionsById.get(store.library.source.id)?.name ?? 'Collection'
      );
    }
    if (store.library.source.kind === 'imported') {
      return (
        store.libraryNavigation.importsById.get(store.library.source.id)?.display_name ??
        'Local source'
      );
    }
    return store.library.source.id === 'liked' ? 'Liked Songs' : 'All Songs';
  });

  return (
    <section className="track-browser" aria-labelledby="track-browser-title">
      <header className="track-browser-header">
        <div>
          <h1 id="track-browser-title">{sourceName}</h1>
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
          <strong>
            {source.kind === 'collection' ? 'This collection is empty' : 'No songs found'}
          </strong>
          <span className="muted">
            {source.kind === 'collection'
              ? 'Add songs from All Songs or Liked Songs.'
              : 'Try another search or reload the library.'}
          </span>
        </div>
      ) : (
        <TrackTable useStore={useStore} />
      )}
    </section>
  );
}
