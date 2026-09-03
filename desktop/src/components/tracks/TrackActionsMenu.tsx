import { ListPlus, MoreHorizontal, MinusCircle } from 'lucide-react';
import { useState } from 'react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';

interface TrackActionsMenuProps {
  row: SongRow;
  useStore: DesktopStoreHook;
}

export function TrackActionsMenu({ row, useStore }: TrackActionsMenuProps) {
  const [open, setOpen] = useState(false);
  const [collectionIds, setCollectionIds] = useState<string[]>([]);
  const onOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (nextOpen) setCollectionIds([...useStore.getState().libraryNavigation.collectionOrder]);
  };

  return (
    <MenuTrigger isOpen={open} onOpenChange={onOpenChange}>
      <Button
        className="icon-button track-actions-button"
        aria-label={`More actions for ${row.title}`}
        title={`More actions for ${row.title}`}
      >
        <MoreHorizontal size={16} aria-hidden="true" />
      </Button>
      <Popover className="track-menu-popover" placement="bottom end" offset={4}>
        <Menu
          aria-label={`Actions for ${row.title}`}
          onAction={(key) => {
            const state = useStore.getState();
            if (key === 'remove-current' && state.library.source.kind === 'collection') {
              void state
                .removeSongFromCollection(state.library.source.id, row.song_id)
                .catch(() => undefined);
              return;
            }
            if (key.startsWith('collection:')) {
              void state
                .addSongToCollection(key.slice('collection:'.length), row.song_id)
                .catch(() => undefined);
            }
          }}
        >
          {collectionIds.length === 0 ? (
            <MenuItem id="no-collections" isDisabled className="library-menu-item">
              <ListPlus size={15} aria-hidden="true" />
              <span>No collections yet</span>
            </MenuItem>
          ) : (
            collectionIds.map((collectionId) => {
              const collection = useStore
                .getState()
                .libraryNavigation.collectionsById.get(collectionId);
              if (!collection) return null;
              return (
                <MenuItem
                  key={collectionId}
                  id={`collection:${collectionId}`}
                  className="library-menu-item"
                >
                  <ListPlus size={15} aria-hidden="true" />
                  <span>Add to {collection.name}</span>
                  <small>{collection.song_count}</small>
                </MenuItem>
              );
            })
          )}
          {useStore.getState().library.source.kind === 'collection' && (
            <MenuItem id="remove-current" className="library-menu-item is-danger">
              <MinusCircle size={15} aria-hidden="true" />
              <span>Remove from this collection</span>
            </MenuItem>
          )}
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
