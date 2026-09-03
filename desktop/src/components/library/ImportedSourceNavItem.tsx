import { FileMusic, Folder, MoreHorizontal, Trash2 } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { LibraryImportedSource } from '../../bridge/DesktopBridge';
import { LibraryNavItem } from './LibraryNavItem';

interface ImportedSourceNavItemProps {
  source: LibraryImportedSource;
  active: boolean;
  pending: boolean;
  onSelect: () => void;
  onRemove: () => void;
}

export function ImportedSourceNavItem({
  source,
  active,
  pending,
  onSelect,
  onRemove,
}: ImportedSourceNavItemProps) {
  const missing = source.availability === 'missing';
  const icon = source.kind === 'folder' ? <Folder size={17} /> : <FileMusic size={17} />;
  return (
    <div className={`library-nav-action-row${missing ? ' is-missing' : ''}`}>
      <LibraryNavItem
        label={`${source.display_name}${missing ? ' (missing)' : ''}`}
        active={active}
        count={source.song_count}
        icon={icon}
        onPress={onSelect}
      >
        {source.display_name}
      </LibraryNavItem>
      <MenuTrigger>
        <Button
          className="icon-button library-more-button"
          aria-label={`More actions for ${source.display_name}`}
          title={`More actions for ${source.display_name}`}
          isDisabled={pending}
        >
          <MoreHorizontal size={16} aria-hidden="true" />
        </Button>
        <Popover className="library-menu-popover" placement="bottom end" offset={4}>
          <Menu aria-label={`${source.display_name} actions`}>
            <MenuItem id="remove" onAction={onRemove} className="library-menu-item is-danger">
              <Trash2 size={15} aria-hidden="true" />
              <span>Remove from Library</span>
            </MenuItem>
          </Menu>
        </Popover>
      </MenuTrigger>
    </div>
  );
}
