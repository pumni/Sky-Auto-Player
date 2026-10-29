import { ArrowLeft } from 'lucide-react';
import { Button } from 'react-aria-components';
import { useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { PlaylistAddSongsMenu, type PlaylistImportFeedback } from './PlaylistAddSongsMenu';
import { TrackTable } from './TrackTable';
import { BuiltinPager } from './BuiltinPager';
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
  const searchSource = useStore((store: DesktopStore) => store.library.searchSource);
  const navigationState = useStore((store: DesktopStore) => store.libraryNavigation.loadState);
  const builtinAvailable = useStore(
    (store: DesktopStore) => store.libraryNavigation.builtinAvailable,
  );
  const builtinSongCount = useStore(
    (store: DesktopStore) => store.libraryNavigation.builtinSongCount,
  );
  const query = useStore((store: DesktopStore) => store.library.query);
  const resultTotal = useStore((store: DesktopStore) => store.library.resultTotal);
  const loading = useStore((store: DesktopStore) => store.library.loading);
  const error = useStore((store: DesktopStore) => store.library.error);
  const failedBuiltinPage = useStore((store: DesktopStore) => store.library.builtinPage.failedPage);
  const reloadLibrary = useStore((store: DesktopStore) => store.reloadLibrary);
  const retryBuiltinPage = useStore((store: DesktopStore) => store.retryBuiltinPage);
  const loadLibraryNavigation = useStore((store: DesktopStore) => store.loadLibraryNavigation);
  const playlistAddMode = useStore((store: DesktopStore) => store.library.playlistAddMode);
  const openPlaylistAdd = useStore((store: DesktopStore) => store.openPlaylistAdd);
  const exitPlaylistAdd = useStore((store: DesktopStore) => store.exitPlaylistAdd);
  const search = useStore((store: DesktopStore) => store.search);
  const [importFeedback, setImportFeedback] = useState<PlaylistImportFeedback | null>(null);
  const sourceName = useStore((store: DesktopStore) => {
    if (store.library.source.kind === 'playlist') {
      return store.libraryNavigation.playlistsById.get(store.library.source.id)?.name ?? 'Playlist';
    }
    if (store.library.source.id === 'liked') return 'Liked Songs';
    return store.library.source.id === 'builtin' ? 'Built-in Songs' : 'Library';
  });
  const isPlaylist = source.kind === 'playlist';
  const isBuiltin = source.kind === 'smart' && source.id === 'builtin';
  const isPlaylistAddMode = isPlaylist && playlistAddMode?.playlistId === source.id;
  const isBuiltinSearch =
    searchSource.kind === 'smart' &&
    searchSource.id === 'builtin' &&
    (isBuiltin || isPlaylistAddMode);
  const hasSearch = query.trim().length > 0;
  const builtinUnavailable = navigationState === 'ready' && !builtinAvailable;
  const builtinStatusUnknown = navigationState === 'error';
  const builtinStatusPending = navigationState === 'idle' || navigationState === 'loading';
  const builtinCatalogMissing =
    isBuiltinSearch &&
    navigationState === 'ready' &&
    builtinAvailable &&
    builtinSongCount > 0 &&
    resultTotal === 0 &&
    !hasSearch;
  const heading = isPlaylistAddMode ? `Add songs to ${sourceName}` : sourceName;

  const noResultsHeading = isBuiltinSearch
    ? builtinStatusPending
      ? 'Checking Built-in Songs'
      : builtinStatusUnknown
        ? 'Built-in Songs status unknown'
        : builtinUnavailable
          ? 'Built-in Songs are unavailable'
          : builtinCatalogMissing
            ? 'Built-in Songs could not be loaded'
            : hasSearch
              ? 'No Built-in Songs match your search'
              : 'No Built-in Songs are available'
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
          : 'Your library is empty';

  const noResultsMessage = isBuiltinSearch
    ? builtinStatusPending
      ? 'Checking the native catalog availability.'
      : builtinStatusUnknown
        ? 'The built-in catalog availability could not be checked. Retry to check again.'
        : builtinUnavailable
          ? isPlaylistAddMode
            ? 'Built-in songs are unavailable in this installation. Go back to the playlist to import local sheets.'
            : 'Built-in songs are unavailable in this installation. Create a playlist to import local sheets.'
          : builtinCatalogMissing
            ? `The native catalog reports ${formatSongCount(builtinSongCount)}, but no songs were returned. Retry to load the catalog.`
            : hasSearch
              ? 'Try a different search, or go back to the playlist.'
              : 'The built-in song catalog is empty.'
    : source.kind === 'playlist' && !hasSearch
      ? 'Browse Built-in Songs to add catalog songs, or use Add songs to import local files or a folder.'
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
          <div className="track-browser-source-label">
            <h1 id="track-browser-title" title={heading}>
              {heading}
            </h1>
          </div>
          <span className="track-browser-count">
            {loading
              ? 'Updating…'
              : isPlaylistAddMode
                ? `${formatSongCount(resultTotal)} in Built-in Songs`
                : formatSongCount(resultTotal)}
          </span>
          {isBuiltin &&
            !loading &&
            !error &&
            resultTotal === 0 &&
            !hasSearch &&
            navigationState === 'ready' &&
            builtinAvailable &&
            builtinSongCount === 0 && (
              <span className="track-browser-guidance">
                Create a playlist to import local sheets.
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
      {((error && resultTotal > 0) || (isBuiltinSearch && failedBuiltinPage !== null)) && (
        <div
          className="inline-error builtin-page-feedback"
          role={isBuiltinSearch && failedBuiltinPage !== null ? 'alert' : 'status'}
        >
          {error && resultTotal > 0 && <span>{error}</span>}
          {isBuiltinSearch && failedBuiltinPage !== null && (
            <>
              <span>Couldn’t load page {failedBuiltinPage}.</span>
              <button
                className="button"
                type="button"
                onClick={() => {
                  void retryBuiltinPage().finally(() => {
                    window.setTimeout(
                      () =>
                        document
                          .querySelector<HTMLButtonElement>('.builtin-pager-jump-trigger')
                          ?.focus(),
                      0,
                    );
                  });
                }}
              >
                Retry page {failedBuiltinPage}
              </button>
            </>
          )}
        </div>
      )}
      {loading && resultTotal === 0 ? (
        <div className="empty-state" role="status" aria-busy="true">
          <strong>Loading library…</strong>
          <span className="muted">Reading your song catalog.</span>
        </div>
      ) : error && resultTotal === 0 ? (
        <div className="empty-state" role="alert">
          <strong>
            {isBuiltinSearch
              ? builtinUnavailable
                ? 'Built-in Songs are unavailable'
                : 'Built-in Songs could not be loaded'
              : 'Library unavailable'}
          </strong>
          <span className="muted">
            {isBuiltinSearch && builtinUnavailable ? noResultsMessage : error}
          </span>
          <Button
            className="button button-primary"
            onPress={() =>
              void (isBuiltinSearch && builtinUnavailable
                ? loadLibraryNavigation()
                : reloadLibrary())
            }
          >
            Retry
          </Button>
        </div>
      ) : !loading && resultTotal === 0 ? (
        <div className="empty-state" role={builtinCatalogMissing ? 'alert' : undefined}>
          <strong>{noResultsHeading}</strong>
          <span className="muted">{noResultsMessage}</span>
          {isPlaylist && !isPlaylistAddMode && (
            <Button
              className="button button-primary empty-state-add-songs-button"
              onPress={() => void openPlaylistAdd(source.id)}
            >
              Browse Built-in Songs…
            </Button>
          )}
          {isBuiltinSearch &&
            (builtinUnavailable || builtinStatusUnknown || builtinCatalogMissing) && (
              <Button
                className="button button-primary"
                onPress={() =>
                  void (builtinUnavailable || builtinStatusUnknown
                    ? loadLibraryNavigation()
                    : reloadLibrary())
                }
              >
                Retry
              </Button>
            )}
          {hasSearch && (
            <Button className="button" onPress={() => void search('')}>
              Clear search
            </Button>
          )}
        </div>
      ) : (
        <>
          <TrackTable useStore={useStore} />
          {isBuiltinSearch && <BuiltinPager useStore={useStore} />}
        </>
      )}
    </section>
  );
}
