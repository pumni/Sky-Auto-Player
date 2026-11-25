import { ChevronLeft, ChevronRight, Disc3, Heart, ListMusic, RefreshCw } from 'lucide-react';
import { useRef, useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/tooltip';
import { LibraryAddMenu } from './LibraryAddMenu';
import { LibraryConfirmDialog } from './LibraryConfirmDialog';
import { LibraryNavItem } from './LibraryNavItem';
import { LibrarySection } from './LibrarySection';
import { PlaylistCreateDialog } from './PlaylistCreateDialog';
import { PlaylistNavItem } from './PlaylistNavItem';
import { PlaylistRenameDialog } from './PlaylistRenameDialog';

const playlistNameSegmenter =
  typeof Intl.Segmenter === 'undefined'
    ? null
    : new Intl.Segmenter(undefined, { granularity: 'grapheme' });

function playlistMonogram(name: string) {
  const trimmed = name.trim();
  if (!trimmed || !playlistNameSegmenter) return null;
  return playlistNameSegmenter.segment(trimmed)[Symbol.iterator]().next().value?.segment ?? null;
}

interface CollapsedPlaylistTileProps {
  id: string;
  displayName: string;
  accessibleName: string;
  monogram: string | null;
  active: boolean;
  onSelect: () => void;
}

function CollapsedPlaylistTile({
  id,
  displayName,
  accessibleName,
  monogram,
  active,
  onSelect,
}: CollapsedPlaylistTileProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="relative size-10 rounded-control border-0 bg-transparent p-0 after:absolute after:inset-y-2 after:left-0 after:w-[3px] after:rounded-r-full after:bg-accent-ink after:opacity-0 hover:bg-control-hover data-[active=true]:bg-selection data-[active=true]:hover:bg-selection data-[active=true]:after:opacity-100"
            type="button"
            aria-label={accessibleName}
            {...(active ? { 'aria-current': 'page' as const } : {})}
            data-playlist-id={id}
            data-slot="collapsed-playlist-tile"
            data-active={active}
            onClick={onSelect}
          />
        }
      >
        <span
          className="grid size-[30px] place-items-center rounded-control bg-surface text-sm font-semibold uppercase text-text-secondary"
          aria-hidden="true"
        >
          {monogram ? <span>{monogram}</span> : <ListMusic size={18} />}
        </span>
      </TooltipTrigger>
      <TooltipContent
        side="right"
        className="max-w-[min(320px,calc(100vw-64px))] break-words rounded-control border border-border bg-popover px-2 py-1.5 text-xs leading-[1.4] text-popover-foreground shadow-lg"
      >
        {displayName}
      </TooltipContent>
    </Tooltip>
  );
}

interface LibraryNavigatorProps {
  collapsed: boolean;
  onToggleCollapsed: () => void;
  useStore: DesktopStoreHook;
}

export function LibraryNavigator({
  collapsed,
  onToggleCollapsed,
  useStore,
}: LibraryNavigatorProps) {
  const likedTotal = useStore((store: DesktopStore) => store.library.likedTotal);
  const source = useStore((store: DesktopStore) => store.library.source);
  const navigation = useStore((store: DesktopStore) => store.libraryNavigation);
  const navigationReady = navigation.loadState === 'ready';
  const builtinChecking = navigation.loadState === 'idle' || navigation.loadState === 'loading';
  const builtinLabel = builtinChecking
    ? 'Checking Built-in Songs'
    : navigation.loadState === 'error'
      ? 'Built-in Songs status unknown'
      : navigation.builtinAvailable
        ? 'Built-in Songs'
        : 'Built-in Songs unavailable';
  const selectLibrarySource = useStore((store: DesktopStore) => store.selectLibrarySource);
  const openPlaylistAdd = useStore((store: DesktopStore) => store.openPlaylistAdd);
  const createPlaylist = useStore((store: DesktopStore) => store.createPlaylist);
  const renamePlaylist = useStore((store: DesktopStore) => store.renamePlaylist);
  const deletePlaylist = useStore((store: DesktopStore) => store.deletePlaylist);
  const [createOpen, setCreateOpen] = useState(false);
  const [renameId, setRenameId] = useState<string | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const addTriggerRef = useRef<HTMLButtonElement>(null);

  const renameTarget = renameId ? navigation.playlistsById.get(renameId) : undefined;
  const deleteTarget = deleteId ? navigation.playlistsById.get(deleteId) : undefined;
  const collapsedPlaylists = navigation.playlistOrder.flatMap((id) => {
    const playlist = navigation.playlistsById.get(id);
    return playlist ? [playlist] : [];
  });
  const playlistNameCounts = new Map<string, number>();
  for (const playlist of collapsedPlaylists) {
    playlistNameCounts.set(playlist.name, (playlistNameCounts.get(playlist.name) ?? 0) + 1);
  }

  const runAndClose = async (operation: () => Promise<void>, close: () => void) => {
    await operation();
    close();
  };

  return (
    <nav
      data-slot="library-navigator"
      className={`flex min-h-0 min-w-0 flex-1 flex-col ${collapsed ? 'items-center px-[3px] py-2' : 'px-2 pb-3'}`}
      aria-label="Library"
    >
      <div
        data-slot="library-navigator-heading"
        className={`flex min-h-10 min-w-0 items-center justify-between ${collapsed ? 'mb-1 justify-center' : 'mb-1.5 pl-2'}`}
      >
        {!collapsed && (
          <h2 className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-sm font-semibold leading-5 text-foreground">
            Your Library
          </h2>
        )}
        <div data-slot="library-heading-actions" className="flex flex-none items-center gap-1">
          {!collapsed && (
            <LibraryAddMenu triggerRef={addTriggerRef} onNewPlaylist={() => setCreateOpen(true)} />
          )}
          <Button
            variant="ghost"
            size={collapsed ? 'icon-lg' : 'icon-sm'}
            className={`${collapsed ? '' : 'size-[30px]'} border-0 bg-transparent p-0 !text-text-secondary hover:bg-control-hover hover:!text-foreground`}
            type="button"
            aria-label={collapsed ? 'Expand library navigator' : 'Collapse library navigator'}
            title={collapsed ? 'Expand library navigator' : 'Collapse library navigator'}
            onClick={onToggleCollapsed}
          >
            {collapsed ? (
              <ChevronRight size={17} aria-hidden="true" />
            ) : (
              <ChevronLeft size={17} aria-hidden="true" />
            )}
          </Button>
        </div>
      </div>

      <div data-slot="library-primary-items" className="grid min-w-0 gap-1">
        <LibraryNavItem
          collapsed={collapsed}
          label={builtinLabel}
          active={source.kind === 'smart' && source.id === 'builtin'}
          {...(navigationReady && navigation.builtinAvailable
            ? { count: navigation.builtinSongCount }
            : {})}
          disabled={!navigationReady || !navigation.builtinAvailable}
          icon={<Disc3 size={18} />}
          onSelect={() => void selectLibrarySource({ kind: 'smart', id: 'builtin' })}
        >
          {builtinChecking ? 'Checking Built-in Songs…' : 'Built-in Songs'}
        </LibraryNavItem>
        <LibraryNavItem
          collapsed={collapsed}
          label="Liked Songs"
          active={source.kind === 'smart' && source.id === 'liked'}
          count={likedTotal}
          icon={<Heart size={18} />}
          iconTone="liked"
          onSelect={() => void selectLibrarySource({ kind: 'smart', id: 'liked' })}
        >
          Liked Songs
        </LibraryNavItem>
      </div>

      {!collapsed && builtinChecking && (
        <p className="min-w-0 overflow-hidden px-2 text-xs text-muted-foreground" role="status">
          Checking Built-in Songs…
        </p>
      )}
      {!collapsed && navigation.loadState === 'error' && (
        <div className="grid min-w-0 gap-2 px-2 pt-2" aria-label="Library navigation recovery">
          <p className="m-0 text-xs leading-relaxed text-danger" role="alert">
            Couldn’t check Built-in Songs or playlists.
            {navigation.lastError ? ` ${navigation.lastError}` : ''}
          </p>
          <Button
            variant="outline"
            className="h-9 justify-center border-border text-[13px] text-foreground"
            type="button"
            aria-label="Retry library navigation"
            onClick={() => void useStore.getState().loadLibraryNavigation()}
          >
            Retry
          </Button>
        </div>
      )}
      {!collapsed && navigation.loadState === 'ready' && !navigation.builtinAvailable && (
        <p className="min-w-0 overflow-hidden px-2 text-xs text-muted-foreground" role="status">
          Built-in Songs aren’t available in this installation. Liked Songs and playlists remain
          available.
        </p>
      )}

      {collapsed ? (
        <div
          data-slot="library-collapsed-playlists"
          className="mt-2 flex min-h-0 w-11 flex-1 flex-col border-t border-border pt-[6px]"
        >
          <div
            data-slot="library-collapsed-playlist-scroller"
            className="flex min-h-0 w-full flex-1 flex-col items-center gap-1 overflow-y-auto overscroll-contain"
            role="group"
            aria-label="Playlists"
          >
            {collapsedPlaylists.map((playlist, index) => {
              const active = source.kind === 'playlist' && source.id === playlist.id;
              const duplicateName = (playlistNameCounts.get(playlist.name) ?? 0) > 1;
              const displayName = playlist.name.trim() ? playlist.name : `Playlist ${index + 1}`;
              const accessibleName = duplicateName
                ? `${displayName}, playlist ${index + 1}`
                : displayName;
              const monogram = playlistMonogram(playlist.name);

              return (
                <CollapsedPlaylistTile
                  key={playlist.id}
                  id={playlist.id}
                  displayName={displayName}
                  accessibleName={accessibleName}
                  monogram={monogram}
                  active={active}
                  onSelect={() => void selectLibrarySource({ kind: 'playlist', id: playlist.id })}
                />
              );
            })}
            {navigation.loadState === 'error' && (
              <>
                <span className="sr-only" role="alert">
                  Couldn’t load playlists. {navigation.lastError ?? ''}
                </span>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="grid size-10 place-items-center rounded-control border-0 bg-transparent text-text-secondary hover:bg-control-hover hover:text-foreground"
                  type="button"
                  aria-label="Retry library navigation"
                  onClick={() => void useStore.getState().loadLibraryNavigation()}
                >
                  <RefreshCw size={16} aria-hidden="true" />
                </Button>
              </>
            )}
            {!navigationReady && navigation.loadState !== 'error' && (
              <span className="sr-only" role="status">
                Loading playlists.
              </span>
            )}
          </div>
        </div>
      ) : (
        <div
          data-slot="library-secondary-sections"
          className="mt-3 grid min-h-0 min-w-0 flex-1 content-start gap-3 overflow-auto border-t border-border pt-2"
        >
          <LibrarySection label="Playlists" showWhenEmpty>
            {navigation.playlistOrder.map((id) => {
              const playlist = navigation.playlistsById.get(id);
              if (!playlist) return null;
              return (
                <PlaylistNavItem
                  key={id}
                  playlist={playlist}
                  active={source.kind === 'playlist' && source.id === id}
                  pending={
                    navigation.pendingMutations.has(`playlist:${id}:rename`) ||
                    navigation.pendingMutations.has(`playlist:${id}:delete`)
                  }
                  onSelect={() => void selectLibrarySource({ kind: 'playlist', id })}
                  onAddSongs={() => void openPlaylistAdd(id)}
                  onRename={() => setRenameId(id)}
                  onDelete={() => setDeleteId(id)}
                />
              );
            })}
            {navigation.loadState === 'ready' && navigation.playlistOrder.length === 0 && (
              <div data-slot="library-empty-playlists" className="mx-2 mt-2 grid min-w-0 gap-2">
                <p
                  data-slot="library-empty-playlists-guidance"
                  className="m-0 text-xs leading-relaxed text-text-secondary"
                >
                  Create a playlist to collect songs or import local sheets.
                </p>
                <Button
                  variant={
                    (navigation.builtinAvailable && navigation.builtinSongCount > 0) ||
                    likedTotal > 0
                      ? 'outline'
                      : 'default'
                  }
                  data-slot="library-empty-playlists-create"
                  className="h-9 justify-center text-[13px]"
                  type="button"
                  onClick={() => setCreateOpen(true)}
                >
                  Create playlist
                </Button>
              </div>
            )}
          </LibrarySection>
        </div>
      )}

      {!collapsed && navigation.lastError && navigation.loadState !== 'error' && (
        <p className="mt-2 min-w-0 overflow-hidden px-2 text-xs text-danger" role="status">
          {navigation.lastError}
        </p>
      )}

      <div
        data-slot="library-navigator-footer"
        className="mt-auto flex min-w-0 items-center justify-center pt-2"
      >
        {collapsed && (
          <LibraryAddMenu
            collapsed
            triggerRef={addTriggerRef}
            onNewPlaylist={() => setCreateOpen(true)}
          />
        )}
      </div>

      <PlaylistCreateDialog
        key={createOpen ? 'open' : 'closed'}
        open={createOpen}
        pending={navigation.pendingMutations.has('playlist:create')}
        error={navigation.lastError}
        onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) addTriggerRef.current?.focus();
        }}
        onSubmit={createPlaylist}
      />
      {renameTarget && (
        <PlaylistRenameDialog
          key={renameTarget.id}
          open={renameId !== null}
          initialName={renameTarget.name}
          pending={navigation.pendingMutations.has(`playlist:${renameTarget.id}:rename`)}
          error={navigation.lastError}
          onOpenChange={(open) => {
            if (!open) setRenameId(null);
          }}
          onSubmit={(name) =>
            runAndClose(
              () => renamePlaylist(renameTarget.id, name),
              () => setRenameId(null),
            )
          }
        />
      )}
      {deleteTarget && (
        <LibraryConfirmDialog
          open={deleteId !== null}
          title={`Delete “${deleteTarget.name}”?`}
          message="The playlist will be removed. Songs and local files will not be deleted."
          confirmLabel="Delete playlist"
          pending={navigation.pendingMutations.has(`playlist:${deleteTarget.id}:delete`)}
          onOpenChange={(open) => {
            if (!open) setDeleteId(null);
          }}
          onConfirm={() =>
            runAndClose(
              () => deletePlaylist(deleteTarget.id),
              () => setDeleteId(null),
            )
          }
        />
      )}
    </nav>
  );
}
