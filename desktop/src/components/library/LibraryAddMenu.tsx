import { ListPlus, Plus } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { RefObject } from 'react';

interface LibraryAddMenuProps {
  collapsed?: boolean;
  triggerRef?: RefObject<HTMLButtonElement | null>;
  onNewPlaylist: () => void;
}

export function LibraryAddMenu({
  collapsed = false,
  triggerRef,
  onNewPlaylist,
}: LibraryAddMenuProps) {
  return (
    <MenuTrigger>
      <Button
        ref={triggerRef}
        className={`icon-button library-add-button${collapsed ? ' is-collapsed' : ''}`}
        aria-label="Add to Your Library"
        title="Add to Your Library"
      >
        <Plus size={18} aria-hidden="true" />
        {!collapsed && <span>Add</span>}
      </Button>
      <Popover className="library-menu-popover" placement="bottom start" offset={6}>
        <Menu aria-label="Your Library">
          <MenuItem
            id="create-playlist"
            onAction={() => onNewPlaylist()}
            className="library-menu-item"
          >
            <ListPlus size={16} aria-hidden="true" />
            <span>Create playlist</span>
          </MenuItem>
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
