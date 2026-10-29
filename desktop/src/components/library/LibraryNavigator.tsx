import { Check, ChevronLeft, ChevronRight, Heart, Library, Music2, Plus } from 'lucide-react';
import { useRef, useState } from 'react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { PlaylistCreateDialog } from './PlaylistCreateDialog';
import { PlaylistNavItem } from './PlaylistNavItem';
import { PlaylistRenameDialog } from './PlaylistRenameDialog';
import { LibraryAddMenu } from './LibraryAddMenu';
import { LibraryConfirmDialog } from './LibraryConfirmDialog';
import { LibraryNavItem } from './LibraryNavItem';
import { LibrarySection } from './LibrarySection';

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

  const runAndClose = async (operation: () => Promise<void>, close: () => void) => {
    await operation();
    close();
  };

  return (
    <nav className="library-navigator" aria-label="Library">
      <div className="library-navigator-heading">
        {!collapsed && <h2>Your Library</h2>}
        <div className="library-heading-actions">
          {!collapsed && (
            <LibraryAddMenu triggerRef={addTriggerRef} onNewPlaylist={() => setCreateOpen(true)} />
          )}
          <button
            className="icon-button icon-button-boxed library-collapse-button"
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
          </button>
        </div>
      </div>

      <div className="library-primary-items">
        <LibraryNavItem
          collapsed={collapsed}
          label={builtinLabel}
          active={source.kind === 'smart' && source.id === 'builtin'}
          {...(navigationReady && navigation.builtinAvailable
            ? { count: navigation.builtinSongCount }
            : {})}
          disabled={!navigationReady || !navigation.builtinAvailable}
          icon={<Library size={18} />}
          onPress={() => void selectLibrarySource({ kind: 'smart', id: 'builtin' })}
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
          onPress={() => void selectLibrarySource({ kind: 'smart', id: 'liked' })}
        >
          Liked Songs
        </LibraryNavItem>
      </div>

      {builtinChecking && (
        <p className="library-loading muted" role="status">
          Checking Built-in Songs…
        </p>
      )}
      {navigation.loadState === 'error' && (
        <div className="library-empty-playlists" aria-label="Library navigation recovery">
          <p role="alert">
            Couldn’t check Built-in Songs or playlists.
            {navigation.lastError ? ` ${navigation.lastError}` : ''}
          </p>
          <button
            className="button library-empty-playlists-create"
            type="button"
            aria-label="Retry library navigation"
            onClick={() => void useStore.getState().loadLibraryNavigation()}
          >
            Retry
          </button>
        </div>
      )}
      {navigation.loadState === 'ready' && !navigation.builtinAvailable && (
        <p className="library-loading muted" role="status">
          Built-in Songs aren’t available in this installation. Liked Songs and playlists remain
          available.
        </p>
      )}

      {collapsed ? (
        <div className="library-collapsed-launchers">
          <MenuTrigger>
            <Button
              className={`library-nav-item${source.kind === 'playlist' ? ' is-active' : ''}`}
              aria-label="Playlists"
              title="Playlists"
              aria-current={source.kind === 'playlist' ? 'page' : undefined}
            >
              <span className="library-nav-item-icon" aria-hidden="true">
                <Music2 size={18} />
              </span>
              <span className="library-nav-item-label">Playlists</span>
            </Button>
            <Popover
              className="library-menu-popover collapsed-playlist-menu-popover"
              placement="right top"
              offset={8}
            >
              <Menu aria-label="Playlists" className="collapsed-playlist-menu">
                {collapsedPlaylists.length > 0 ? (
                  <>
                    {collapsedPlaylists.map((playlist) => {
                      const active = source.kind === 'playlist' && source.id === playlist.id;
                      return (
                        <MenuItem
                          key={playlist.id}
                          id={playlist.id}
                          aria-label={active ? `${playlist.name}, current playlist` : playlist.name}
                          className={`library-menu-item collapsed-playlist-menu-item${active ? ' is-current' : ''}`}
                          onAction={() =>
                            void selectLibrarySource({ kind: 'playlist', id: playlist.id })
                          }
                        >
                          <span className="collapsed-playlist-menu-name" title={playlist.name}>
                            {playlist.name}
                          </span>
                          {active && (
                            <>
                              <Check size={15} aria-hidden="true" />
                              <span className="visually-hidden">, current playlist</span>
                            </>
                          )}
                        </MenuItem>
                      );
                    })}
                    {navigation.loadState === 'error' && (
                      <MenuItem
                        id="retry-playlists"
                        className="library-menu-item"
                        onAction={() => void useStore.getState().loadLibraryNavigation()}
                      >
                        Retry loading playlists
                      </MenuItem>
                    )}
                  </>
                ) : navigation.loadState === 'error' ? (
                  <>
                    <MenuItem id="playlists-unavailable" isDisabled className="library-menu-item">
                      Playlists unavailable
                    </MenuItem>
                    <MenuItem
                      id="retry-playlists"
                      className="library-menu-item"
                      onAction={() => void useStore.getState().loadLibraryNavigation()}
                    >
                      Retry loading playlists
                    </MenuItem>
                  </>
                ) : navigation.loadState !== 'ready' ? (
                  <MenuItem id="loading-playlists" isDisabled className="library-menu-item">
                    Loading playlists…
                  </MenuItem>
                ) : (
                  <>
                    <MenuItem id="empty-playlists" isDisabled className="library-menu-item">
                      No playlists yet
                    </MenuItem>
                    <MenuItem
                      id="create-playlist"
                      className="library-menu-item"
                      onAction={() => setCreateOpen(true)}
                    >
                      <Plus size={15} aria-hidden="true" />
                      <span>Create playlist</span>
                    </MenuItem>
                  </>
                )}
              </Menu>
            </Popover>
          </MenuTrigger>
        </div>
      ) : (
        <div className="library-secondary-sections">
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
              <div className="library-empty-playlists">
                {(!navigation.builtinAvailable || navigation.builtinSongCount === 0) && (
                  <p>Create a playlist to collect songs or import local sheet files and folders.</p>
                )}
                <button
                  className="button library-empty-playlists-create"
                  type="button"
                  onClick={() => setCreateOpen(true)}
                >
                  Create playlist
                </button>
              </div>
            )}
          </LibrarySection>
        </div>
      )}

      {navigation.lastError && navigation.loadState !== 'error' && (
        <p className="library-inline-error" role="status">
          {navigation.lastError}
        </p>
      )}

      <div className="library-navigator-footer">
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
