import { ArrowLeft } from 'lucide-react';
import { Button } from 'react-aria-components';
import { useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { PlaylistAddSongsMenu, type PlaylistImportFeedback } from './PlaylistAddSongsMenu';
import { TrackTable } from './TrackTable';
import { formatSongCount } from './trackFormatting';

interface TrackBrowserProps {
  useStore: DesktopStoreHook;
}

export function TrackBrowser({ useStore }: TrackBrowserProps) {
  const source = useStore((store: DesktopStore) => store.library.source);
  const playlistAddMode = useStore((store: DesktopStore) => store.library.playlistAddMode);
  const modeKey = playlistAddMode?.playlistId === source.id ? 'add' : 'browse';
  return <TrackBrowserContent key={`${source.kind}:${source.id}:${modeKey}`} useStore={useStore} />;
}

function TrackBrowserContent({ useStore }: TrackBrowserProps) {
  const source = useStore((store: DesktopStore) => store.library.source);
  const query = useStore((store: DesktopStore) => store.library.query);
  const resultTotal = useStore((store: DesktopStore) => store.library.resultTotal);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);
  const reloadLibrary = useStore((store: DesktopStore) => store.reloadLibrary);
  const playlistAddMode = useStore((store: DesktopStore) => store.library.playlistAddMode);
  const openPlaylistAdd = useStore((store: DesktopStore) => store.openPlaylistAdd);
  const exitPlaylistAdd = useStore((store: DesktopStore) => store.exitPlaylistAdd);
  const search = useStore((store: DesktopStore) => store.search);
  const [importFeedback, setImportFeedback] = useState<PlaylistImportFeedback | null>(null);
  const sourceName = useStore((store: DesktopStore) => {
    if (store.library.source.kind === 'playlist') {
      return store.libraryNavigation.playlistsById.get(store.library.source.id)?.name ?? 'Playlist';
    }
    return store.library.source.id === 'liked' ? 'Liked Songs' : 'All Songs';
  });
  const isPlaylist = source.kind === 'playlist';
  const isPlaylistAddMode = isPlaylist && playlistAddMode?.playlistId === source.id;
  const heading = isPlaylistAddMode ? `Add songs to ${sourceName}` : sourceName;
  const hasSearch = query.trim().length > 0;

  const noResultsHeading = isPlaylistAddMode
    ? 'No All Songs match your search'
    : source.kind === 'playlist'
      ? hasSearch
        ? `No songs in ${sourceName} match your search`
        : 'This playlist is empty'
      : source.id === 'liked'
        ? hasSearch
          ? 'No Liked Songs match your search'
          : 'No liked songs yet'
        : hasSearch
          ? 'No songs match your search'
          : 'Your catalog is empty';

  const noResultsMessage = isPlaylistAddMode
    ? 'Try a different search, or go back to the playlist.'
    : source.kind === 'playlist' && !hasSearch
      ? 'Browse All Songs to add catalog songs, or use Add songs to import local files or a folder.'
      : source.kind === 'smart' && source.id === 'liked' && !hasSearch
        ? 'Like a song anywhere in the catalog to save it here.'
        : source.kind === 'smart' && source.id === 'all' && !hasSearch
          ? 'Create or open a playlist to collect songs or import local sheet files and folders.'
          : 'Try another search or clear the current search.';

  const importFeedbackText = importFeedback
    ? importFeedback.state === 'pending'
      ? `Opening the local ${importFeedback.kind === 'files' ? 'file' : 'folder'} picker…`
      : importFeedback.state === 'complete'
        ? `Import complete: ${formatSongCount(importFeedback.importedSongCount)} found in ${sourceName}.`
        : importFeedback.state === 'empty'
          ? 'No songs were imported. The picker may have been canceled, or the selection contained no playable sheets.'
          : 'Import failed. Try again with supported Sky sheet files or a folder that contains them.'
    : null;

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
          <h1 id="track-browser-title">{heading}</h1>
          <span className="track-browser-count">
            {loading
              ? 'Updating…'
              : isPlaylistAddMode
                ? `${formatSongCount(resultTotal)} in All Songs`
                : formatSongCount(resultTotal)}
          </span>
          {source.kind === 'smart' && source.id === 'all' && !isPlaylistAddMode && (
            <span className="track-browser-guidance">
              Create a playlist to collect songs or import local sheets.
            </span>
          )}
        </div>
        {isPlaylist && !isPlaylistAddMode && (
          <div className="playlist-actions">
            <PlaylistAddSongsMenu
              playlistId={source.id}
              useStore={useStore}
              onImportFeedback={setImportFeedback}
            />
          </div>
        )}
      </header>
      {importFeedbackText && (
        <p
          className={`playlist-import-feedback is-${importFeedback?.state}`}
          role={importFeedback?.state === 'error' ? 'alert' : 'status'}
          aria-live={importFeedback?.state === 'error' ? 'assertive' : 'polite'}
        >
          {importFeedbackText}
        </p>
      )}
      {error && resultTotal > 0 && (
        <p className="inline-error" role="status">
          {error}
        </p>
      )}
      {loading && resultTotal === 0 ? (
        <div className="empty-state" role="status" aria-busy="true">
          <strong>Loading library…</strong>
          <span className="muted">Reading your song catalog.</span>
        </div>
      ) : error && resultTotal === 0 ? (
        <div className="empty-state" role="alert">
          <strong>Library unavailable</strong>
          <span className="muted">{error}</span>
          <Button className="button button-primary" onPress={() => void reloadLibrary()}>
            Retry
          </Button>
        </div>
      ) : !loading && resultTotal === 0 ? (
        <div className="empty-state">
          <strong>{noResultsHeading}</strong>
          <span className="muted">{noResultsMessage}</span>
          {isPlaylist && !isPlaylistAddMode && (
            <Button
              className="button button-primary empty-state-add-songs-button"
              onPress={() => void openPlaylistAdd(source.id)}
            >
              Browse All Songs…
            </Button>
          )}
          {hasSearch && (
            <Button className="button" onPress={() => void search('')}>
              Clear search
            </Button>
          )}
        </div>
      ) : (
        <TrackTable useStore={useStore} />
      )}
    </section>
  );
}
