import { ArrowLeft } from 'lucide-react';
import { useRef, useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import { BuiltinPager } from './BuiltinPager';
import { PlaylistAddSongsMenu, type PlaylistImportFeedback } from './PlaylistAddSongsMenu';
import { TrackTable, type TrackTableHandle } from './TrackTable';
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
  const trackTableRef = useRef<TrackTableHandle>(null);
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
      className="@container/track-browser flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden bg-surface"
      aria-labelledby="track-browser-title"
      data-slot="track-browser"
    >
      <header
        data-slot="track-browser-header"
        className="relative flex h-8 min-h-8 flex-none items-center gap-1.5 bg-surface px-2 @min-[680px]/track-browser:h-10 @min-[680px]/track-browser:min-h-10 @min-[680px]/track-browser:gap-2 @min-[680px]/track-browser:px-3"
      >
        <div
          data-slot="track-browser-heading-content"
          className="flex min-w-0 flex-1 items-baseline gap-1.5"
        >
          {isPlaylistAddMode && (
            <Button
              variant="outline"
              className="h-8 flex-none self-center whitespace-nowrap"
              onClick={() => void exitPlaylistAdd()}
            >
              <ArrowLeft size={14} aria-hidden="true" />
              <span>Back to playlist</span>
            </Button>
          )}
          <div className="inline-flex min-w-0 max-w-full flex-[0_1_auto] items-center rounded-control pl-1.5">
            <h1
              className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-base font-semibold leading-[22px] tracking-tight text-foreground @min-[680px]/track-browser:text-xl @min-[680px]/track-browser:leading-7"
              id="track-browser-title"
              title={heading}
            >
              {heading}
            </h1>
          </div>
          <span
            data-slot="track-browser-count"
            className="flex-none whitespace-nowrap text-[11px] tabular-nums text-text-tertiary before:mr-1.5 before:content-['·'] @min-[680px]/track-browser:text-xs"
          >
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
              <span
                data-slot="track-browser-guidance"
                className="absolute left-3 top-full mt-1 text-xs text-text-secondary"
              >
                Create a playlist to import local sheets.
              </span>
            )}
        </div>
        {isPlaylist && !isPlaylistAddMode && (
          <div className="flex flex-none items-center">
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
          className={`mx-0 flex flex-none flex-wrap items-center gap-2 border-b px-3 py-1 text-xs ${importFeedback?.state === 'error' ? 'border-danger-border bg-danger-surface text-danger' : importFeedback?.state === 'complete' ? 'border-success-border bg-success-surface text-success' : 'border-border bg-surface text-text-secondary'}`}
          data-slot="track-browser-import-feedback"
          role={importFeedback?.state === 'error' ? 'alert' : 'status'}
          aria-live={importFeedback?.state === 'error' ? 'assertive' : 'polite'}
        >
          {importFeedbackText}
        </p>
      )}
      {((error && resultTotal > 0) || (isBuiltinSearch && failedBuiltinPage !== null)) && (
        <div
          className="flex flex-none flex-wrap items-center gap-x-2 gap-y-1 border-b border-danger-border bg-danger-surface px-[18px] py-1 text-xs text-danger"
          role={isBuiltinSearch && failedBuiltinPage !== null ? 'alert' : 'status'}
        >
          {error && resultTotal > 0 && <span>{error}</span>}
          {isBuiltinSearch && failedBuiltinPage !== null && (
            <>
              <span>Couldn’t load page {failedBuiltinPage}.</span>
              <Button
                variant="outline"
                size="xs"
                className="h-7 px-2 text-xs"
                type="button"
                onClick={() => {
                  void retryBuiltinPage().finally(() => {
                    window.setTimeout(() => {
                      const pagerTarget =
                        document.querySelector<HTMLButtonElement>('[data-page-jump="true"]') ??
                        document.querySelector<HTMLButtonElement>(
                          '[aria-label="Next page"]:not(:disabled)',
                        ) ??
                        document.querySelector<HTMLButtonElement>(
                          '[aria-label="Previous page"]:not(:disabled)',
                        );
                      pagerTarget?.focus();
                    }, 0);
                  });
                }}
              >
                Retry page {failedBuiltinPage}
              </Button>
            </>
          )}
        </div>
      )}
      {loading && resultTotal === 0 ? (
        <div
          data-slot="library-empty-state"
          className="grid min-h-0 flex-1 content-center justify-items-center gap-2 p-6 text-center"
          role="status"
          aria-busy="true"
        >
          <strong className="font-semibold text-foreground">Loading library…</strong>
          <span className="text-muted-foreground">Reading your song catalog.</span>
        </div>
      ) : error && resultTotal === 0 ? (
        <div
          data-slot="library-empty-state"
          className="grid min-h-0 flex-1 content-center justify-items-center gap-2 p-6 text-center"
          role="alert"
        >
          <strong className="font-semibold text-foreground">
            {isBuiltinSearch
              ? builtinUnavailable
                ? 'Built-in Songs are unavailable'
                : 'Built-in Songs could not be loaded'
              : 'Library unavailable'}
          </strong>
          <span className="text-muted-foreground">
            {isBuiltinSearch && builtinUnavailable ? noResultsMessage : error}
          </span>
          <Button
            variant="default"
            className="h-9"
            onClick={() =>
              void (isBuiltinSearch && builtinUnavailable
                ? loadLibraryNavigation()
                : reloadLibrary())
            }
          >
            Retry
          </Button>
        </div>
      ) : !loading && resultTotal === 0 ? (
        <div
          data-slot="library-empty-state"
          className="grid min-h-0 flex-1 content-center justify-items-center gap-2 p-6 text-center"
          role={builtinCatalogMissing ? 'alert' : undefined}
        >
          <strong className="font-semibold text-foreground">{noResultsHeading}</strong>
          <span className="max-w-[42rem] text-muted-foreground">{noResultsMessage}</span>
          {isPlaylist && !isPlaylistAddMode && (
            <Button
              variant="default"
              className="mt-2 h-9"
              onClick={() => void openPlaylistAdd(source.id)}
            >
              Browse Built-in Songs…
            </Button>
          )}
          {isBuiltinSearch &&
            (builtinUnavailable || builtinStatusUnknown || builtinCatalogMissing) && (
              <Button
                variant="default"
                className="mt-2 h-9"
                onClick={() =>
                  void (builtinUnavailable || builtinStatusUnknown
                    ? loadLibraryNavigation()
                    : reloadLibrary())
                }
              >
                Retry
              </Button>
            )}
          {hasSearch && (
            <Button variant="outline" className="mt-2 h-9" onClick={() => void search('')}>
              Clear search
            </Button>
          )}
        </div>
      ) : (
        <>
          <TrackTable useStore={useStore} locateSelectedRef={trackTableRef} />
          {isBuiltinSearch && (
            <BuiltinPager
              useStore={useStore}
              onLocateSelected={() => void trackTableRef.current?.locateSelected()}
            />
          )}
        </>
      )}
    </section>
  );
}
