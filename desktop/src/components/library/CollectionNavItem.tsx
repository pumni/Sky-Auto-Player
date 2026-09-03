import { ListMusic, MoreHorizontal, Pencil, Trash2 } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { LibraryCollectionSummary } from '../../bridge/DesktopBridge';
import { LibraryNavItem } from './LibraryNavItem';

interface CollectionNavItemProps {
  collection: LibraryCollectionSummary;
  active: boolean;
  pending: boolean;
  onSelect: () => void;
  onRename: () => void;
  onDelete: () => void;
}

export function CollectionNavItem({
  collection,
  active,
  pending,
  onSelect,
  onRename,
  onDelete,
}: CollectionNavItemProps) {
  return (
    <div className="library-nav-action-row">
      <LibraryNavItem
        label={collection.name}
        active={active}
        count={collection.song_count}
        icon={<ListMusic size={18} />}
        onPress={onSelect}
      >
        {collection.name}
      </LibraryNavItem>
      <MenuTrigger>
        <Button
          className="icon-button library-more-button"
          aria-label={`More actions for ${collection.name}`}
          title={`More actions for ${collection.name}`}
          isDisabled={pending}
        >
          <MoreHorizontal size={16} aria-hidden="true" />
        </Button>
        <Popover className="library-menu-popover" placement="bottom end" offset={4}>
          <Menu aria-label={`${collection.name} actions`}>
            <MenuItem id="rename" onAction={onRename} className="library-menu-item">
              <Pencil size={15} aria-hidden="true" />
              <span>Rename</span>
            </MenuItem>
            <MenuItem id="delete" onAction={onDelete} className="library-menu-item is-danger">
              <Trash2 size={15} aria-hidden="true" />
              <span>Delete collection</span>
            </MenuItem>
          </Menu>
        </Popover>
      </MenuTrigger>
    </div>
  );
}
