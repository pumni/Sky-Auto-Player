import { ListMusic, ListPlus, MoreHorizontal, Pencil, Trash2 } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { LibraryPlaylistSummary } from '../../bridge/DesktopBridge';
import { LibraryNavItem } from './LibraryNavItem';

interface PlaylistNavItemProps {
  playlist: LibraryPlaylistSummary;
  active: boolean;
  pending: boolean;
  onSelect: () => void;
  onAddSongs: () => void;
  onRename: () => void;
  onDelete: () => void;
}

export function PlaylistNavItem({
  playlist,
  active,
  pending,
  onSelect,
  onAddSongs,
  onRename,
  onDelete,
}: PlaylistNavItemProps) {
  return (
    <div className={`library-nav-action-row${active ? ' is-active' : ''}`}>
      <LibraryNavItem
        label={playlist.name}
        active={active}
        icon={<ListMusic size={18} />}
        onPress={onSelect}
      >
        {playlist.name}
      </LibraryNavItem>
      <MenuTrigger>
        <Button
          className="icon-button library-more-button"
          aria-label={`More actions for ${playlist.name}`}
          title={`More actions for ${playlist.name}`}
          isDisabled={pending}
        >
          <MoreHorizontal size={16} aria-hidden="true" />
        </Button>
        <Popover className="library-menu-popover" placement="bottom end" offset={4}>
          <Menu aria-label={`${playlist.name} actions`}>
            <MenuItem id="add-songs" onAction={onAddSongs} className="library-menu-item">
              <ListPlus size={15} aria-hidden="true" />
              <span>Add songs</span>
            </MenuItem>
            <MenuItem id="rename" onAction={onRename} className="library-menu-item">
              <Pencil size={15} aria-hidden="true" />
              <span>Rename</span>
            </MenuItem>
            <MenuItem id="delete" onAction={onDelete} className="library-menu-item is-danger">
              <Trash2 size={15} aria-hidden="true" />
              <span>Delete playlist</span>
            </MenuItem>
          </Menu>
        </Popover>
      </MenuTrigger>
    </div>
  );
}
