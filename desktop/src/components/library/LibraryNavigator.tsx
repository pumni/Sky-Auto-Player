import { ChevronLeft, ChevronRight, Folder, Heart, Library, Music2 } from 'lucide-react';
import { useRef, useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { CollectionCreateDialog } from './CollectionCreateDialog';
import { CollectionNavItem } from './CollectionNavItem';
import { CollectionRenameDialog } from './CollectionRenameDialog';
import { ImportedSourceNavItem } from './ImportedSourceNavItem';
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
  const createCollection = useStore((store: DesktopStore) => store.createCollection);
  const renameCollection = useStore((store: DesktopStore) => store.renameCollection);
  const deleteCollection = useStore((store: DesktopStore) => store.deleteCollection);
  const removeImport = useStore((store: DesktopStore) => store.removeImport);
  const [createOpen, setCreateOpen] = useState(false);
  const [renameId, setRenameId] = useState<string | null>(null);
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const [removeImportId, setRemoveImportId] = useState<string | null>(null);
  const addTriggerRef = useRef<HTMLButtonElement>(null);

  const renameTarget = renameId ? navigation.collectionsById.get(renameId) : undefined;
  const deleteTarget = deleteId ? navigation.collectionsById.get(deleteId) : undefined;
  const removeTarget = removeImportId ? navigation.importsById.get(removeImportId) : undefined;

  const runAndClose = async (operation: () => Promise<void>, close: () => void) => {
    await operation();
    close();
  };

  return (
    <nav className="library-navigator" aria-label="Library">
      <div className="library-navigator-heading">
        {!collapsed && <h2>Your Library</h2>}
        {!collapsed && (
          <LibraryAddMenu
            useStore={useStore}
            triggerRef={addTriggerRef}
            onNewCollection={() => setCreateOpen(true)}
          />
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
          {navigation.importOrder.length > 0 && (
            <LibraryNavItem
              collapsed
              label="Local sources"
              icon={<Folder size={18} />}
              onPress={onToggleCollapsed}
            >
              Local sources
            </LibraryNavItem>
          )}
          {navigation.collectionOrder.length > 0 && (
            <LibraryNavItem
              collapsed
              label="Collections"
              icon={<Music2 size={18} />}
              onPress={onToggleCollapsed}
            >
              Collections
            </LibraryNavItem>
          )}
        </div>
      ) : (
        <div className="library-secondary-sections">
          {navigation.importOrder.length > 0 && (
            <LibrarySection label="Local">
              {navigation.importOrder.map((id) => {
                const imported = navigation.importsById.get(id);
                if (!imported) return null;
                return (
                  <ImportedSourceNavItem
                    key={id}
                    source={imported}
                    active={source.kind === 'imported' && source.id === id}
                    pending={navigation.pendingMutations.has(`import:${id}:remove`)}
                    onSelect={() => void selectLibrarySource({ kind: 'imported', id })}
                    onRemove={() => setRemoveImportId(id)}
                  />
                );
              })}
            </LibrarySection>
          )}
          <LibrarySection label="Collections" showWhenEmpty>
            {navigation.collectionOrder.map((id) => {
              const collection = navigation.collectionsById.get(id);
              if (!collection) return null;
              return (
                <CollectionNavItem
                  key={id}
                  collection={collection}
                  active={source.kind === 'collection' && source.id === id}
                  pending={
                    navigation.pendingMutations.has(`collection:${id}:rename`) ||
                    navigation.pendingMutations.has(`collection:${id}:delete`)
                  }
                  onSelect={() => void selectLibrarySource({ kind: 'collection', id })}
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
            useStore={useStore}
            collapsed
            triggerRef={addTriggerRef}
            onNewCollection={() => setCreateOpen(true)}
          />
        )}
        {!collapsed && navigation.loadState === 'loading' && (
          <span className="library-loading muted">Updating library…</span>
        )}
      </div>

      <CollectionCreateDialog
        key={createOpen ? 'open' : 'closed'}
        open={createOpen}
        pending={navigation.pendingMutations.has('collection:create')}
        error={navigation.lastError}
        onOpenChange={(open) => {
          setCreateOpen(open);
          if (!open) addTriggerRef.current?.focus();
        }}
        onSubmit={createCollection}
      />
      {renameTarget && (
        <CollectionRenameDialog
          key={renameTarget.id}
          open={renameId !== null}
          initialName={renameTarget.name}
          pending={navigation.pendingMutations.has(`collection:${renameTarget.id}:rename`)}
          error={navigation.lastError}
          onOpenChange={(open) => {
            if (!open) setRenameId(null);
          }}
          onSubmit={(name) =>
            runAndClose(
              () => renameCollection(renameTarget.id, name),
              () => setRenameId(null),
            )
          }
        />
      )}
      {deleteTarget && (
        <LibraryConfirmDialog
          open={deleteId !== null}
          title={`Delete “${deleteTarget.name}”?`}
          message="The collection will be removed. Songs and local files will not be deleted."
          confirmLabel="Delete collection"
          pending={navigation.pendingMutations.has(`collection:${deleteTarget.id}:delete`)}
          onOpenChange={(open) => {
            if (!open) setDeleteId(null);
          }}
          onConfirm={() =>
            runAndClose(
              () => deleteCollection(deleteTarget.id),
              () => setDeleteId(null),
            )
          }
        />
      )}
      {removeTarget && (
        <LibraryConfirmDialog
          open={removeImportId !== null}
          title={`Remove “${removeTarget.display_name}” from the Library?`}
          message="Sky Auto Player will stop indexing this source. Files on disk will not be deleted."
          confirmLabel="Remove from Library"
          pending={navigation.pendingMutations.has(`import:${removeTarget.source_id}:remove`)}
          onOpenChange={(open) => {
            if (!open) setRemoveImportId(null);
          }}
          onConfirm={() =>
            runAndClose(
              () => removeImport(removeTarget.source_id),
              () => setRemoveImportId(null),
            )
          }
        />
      )}
    </nav>
  );
}
