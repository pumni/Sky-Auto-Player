import { ChevronLeft, ChevronRight, Heart, Music2 } from 'lucide-react';
import { useRef, useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { PlaylistCreateDialog } from './CollectionCreateDialog';
import { PlaylistNavItem } from './CollectionNavItem';
import { PlaylistRenameDialog } from './CollectionRenameDialog';
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
  const catalogTotal = useStore((store: DesktopStore) => store.library.catalogTotal);
  const likedTotal = useStore((store: DesktopStore) => store.library.likedTotal);
  const source = useStore((store: DesktopStore) => store.library.source);
  const navigation = useStore((store: DesktopStore) => store.libraryNavigation);
  const selectLibrarySource = useStore((store: DesktopStore) => store.selectLibrarySource);
  const createPlaylist = useStore((store: DesktopStore) => store.createPlaylist);
  const renamePlaylist = useStore((store: DesktopStore) => store.renamePlaylist);
  const deletePlaylist = useStore((store: DesktopStore) => store.deletePlaylist);
  const [createOpen, setCreateOpen] = useState(false);
  const [renameId, setRenameId] = useState<string | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const addTriggerRef = useRef<HTMLButtonElement>(null);

  const renameTarget = renameId ? navigation.playlistsById.get(renameId) : undefined;
  const deleteTarget = deleteId ? navigation.playlistsById.get(deleteId) : undefined;

  const runAndClose = async (operation: () => Promise<void>, close: () => void) => {
    await operation();
    close();
  };

  return (
    <nav className="library-navigator" aria-label="Library">
      <div className="library-navigator-heading">
        {!collapsed && <h2>Your Library</h2>}
        {!collapsed && (
          <LibraryAddMenu triggerRef={addTriggerRef} onNewPlaylist={() => setCreateOpen(true)} />
        )}
        <button
          className="icon-button library-collapse-button"
          type="button"
          aria-label={collapsed ? 'Expand library navigator' : 'Collapse library navigator'}
          title={collapsed ? 'Expand library navigator' : 'Collapse library navigator'}
          aria-pressed={collapsed}
          onClick={onToggleCollapsed}
        >
          {collapsed ? (
            <ChevronRight size={17} aria-hidden="true" />
          ) : (
            <ChevronLeft size={17} aria-hidden="true" />
          )}
        </button>
      </div>

      <div className="library-primary-items">
        <LibraryNavItem
          collapsed={collapsed}
          label="Liked Songs"
          active={source.kind === 'smart' && source.id === 'liked'}
          count={likedTotal}
          icon={<Heart size={18} />}
          onPress={() => void selectLibrarySource({ kind: 'smart', id: 'liked' })}
        >
          Liked Songs
        </LibraryNavItem>
        <LibraryNavItem
          collapsed={collapsed}
          label="All Songs"
          active={source.kind === 'smart' && source.id === 'all'}
          count={catalogTotal}
          icon={<Music2 size={18} />}
          onPress={() => void selectLibrarySource({ kind: 'smart', id: 'all' })}
        >
          All Songs
        </LibraryNavItem>
      </div>

      {collapsed ? (
        <div className="library-collapsed-launchers">
          {navigation.playlistOrder.length > 0 && (
            <LibraryNavItem
              collapsed
              label="Playlists"
              icon={<Music2 size={18} />}
              onPress={onToggleCollapsed}
            >
              Playlists
            </LibraryNavItem>
          )}
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
                  onRename={() => setRenameId(id)}
                  onDelete={() => setDeleteId(id)}
                />
              );
            })}
          </LibrarySection>
        </div>
      )}

      {navigation.lastError && (
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
        {!collapsed && navigation.loadState === 'loading' && (
          <span className="library-loading muted">Updating library…</span>
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
