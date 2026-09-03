import { FolderPlus, ListPlus, Plus } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';

interface LibraryAddMenuProps {
  useStore: DesktopStoreHook;
  collapsed?: boolean;
  triggerRef?: RefObject<HTMLButtonElement | null>;
  onNewCollection: () => void;
}

export function LibraryAddMenu({
  useStore,
  collapsed = false,
  triggerRef,
  onNewCollection,
}: LibraryAddMenuProps) {
  const pending = useStore((store) => store.libraryNavigation.pendingMutations);
  const importFiles = useStore((store) => store.importLocalFiles);
  const importFolder = useStore((store) => store.importLocalFolder);

  return (
    <MenuTrigger>
      <Button
        ref={triggerRef}
        className={`icon-button library-add-button${collapsed ? ' is-collapsed' : ''}`}
        aria-label="Add to Library"
        title="Add to Library"
      >
        <Plus size={18} aria-hidden="true" />
        {!collapsed && <span>Add</span>}
      </Button>
      <Popover className="library-menu-popover" placement="bottom start" offset={6}>
        <Menu aria-label="Add to Library">
          <MenuItem
            id="new-collection"
            onAction={() => onNewCollection()}
            className="library-menu-item"
          >
            <ListPlus size={16} aria-hidden="true" />
            <span>New collection</span>
          </MenuItem>
          <MenuItem
            id="import-files"
            isDisabled={pending.has('import:file')}
            onAction={() => void importFiles().catch(() => undefined)}
            className="library-menu-item"
          >
            <FolderPlus size={16} aria-hidden="true" />
            <span>Import files…</span>
          </MenuItem>
          <MenuItem
            id="import-folder"
            isDisabled={pending.has('import:folder')}
            onAction={() => void importFolder().catch(() => undefined)}
            className="library-menu-item"
          >
            <FolderPlus size={16} aria-hidden="true" />
            <span>Import folder…</span>
          </MenuItem>
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
