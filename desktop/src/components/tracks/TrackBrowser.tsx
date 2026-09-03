import { ArrowLeft } from 'lucide-react';
import { Button } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { PlaylistAddSongsMenu } from './PlaylistAddSongsMenu';
import { TrackTable } from './TrackTable';
import { formatSongCount } from './trackFormatting';

interface TrackBrowserProps {
  useStore: DesktopStoreHook;
}

export function TrackBrowser({ useStore }: TrackBrowserProps) {
  const source = useStore((store: DesktopStore) => store.library.source);
  const resultTotal = useStore((store: DesktopStore) => store.library.resultTotal);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);
  const playlistAddMode = useStore((store: DesktopStore) => store.library.playlistAddMode);
  const exitPlaylistAdd = useStore((store: DesktopStore) => store.exitPlaylistAdd);
  const sourceName = useStore((store: DesktopStore) => {
    if (store.library.source.kind === 'playlist') {
      return store.libraryNavigation.playlistsById.get(store.library.source.id)?.name ?? 'Playlist';
    }
    return store.library.source.id === 'liked' ? 'Liked Songs' : 'All Songs';
  });
  const isPlaylist = source.kind === 'playlist';
  const isPlaylistAddMode = isPlaylist && playlistAddMode?.playlistId === source.id;

  return (
    <section
      className={`track-browser${isPlaylist ? ' is-playlist' : ''}${
        isPlaylistAddMode ? ' is-playlist-add-mode' : ''
      }`}
      aria-labelledby="track-browser-title"
    >
      <header className="track-browser-header">
        <div className="track-browser-heading">
          {isPlaylistAddMode && (
            <Button
              className="button track-browser-back-button"
              onPress={() => void exitPlaylistAdd()}
            >
              <ArrowLeft size={14} aria-hidden="true" />
              <span>Back to playlist</span>
            </Button>
          )}
          <h1 id="track-browser-title">{sourceName}</h1>
          <span className="track-browser-count">
            {loading
              ? 'Updating…'
              : isPlaylistAddMode
                ? 'Adding songs'
                : formatSongCount(resultTotal)}
          </span>
        </div>
        {isPlaylist && !isPlaylistAddMode && (
          <div className="playlist-actions">
            <PlaylistAddSongsMenu playlistId={source.id} useStore={useStore} />
          </div>
        )}
      </header>
      {error && (
        <p className="inline-error" role="status">
          {error}
        </p>
      )}
      {!loading && resultTotal === 0 ? (
        <div className="empty-state">
          <strong>
            {isPlaylist && !isPlaylistAddMode ? 'Add songs to this playlist' : 'No songs found'}
          </strong>
          <span className="muted">
            {isPlaylist && !isPlaylistAddMode
              ? 'Choose from All Songs or import local files.'
              : 'Try another search or reload the library.'}
          </span>
        </div>
      ) : (
        <TrackTable useStore={useStore} />
      )}
    </section>
  );
}
