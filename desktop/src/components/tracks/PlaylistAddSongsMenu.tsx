import { FilePlus2, ListMusic, Plus } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { DesktopStoreHook } from '../../state/store';

interface PlaylistAddSongsMenuProps {
  playlistId: string;
  useStore: DesktopStoreHook;
}

export function PlaylistAddSongsMenu({ playlistId, useStore }: PlaylistAddSongsMenuProps) {
  const beginPlaylistAdd = useStore((store) => store.beginPlaylistAdd);
  const importFiles = useStore((store) => store.importLocalFilesToPlaylist);
  const importFolder = useStore((store) => store.importLocalFolderToPlaylist);
  const pending = useStore((store) => store.libraryNavigation.pendingMutations);

  return (
    <MenuTrigger>
      <Button className="button playlist-add-songs-button">
        <Plus size={16} aria-hidden="true" />
        <span>Add songs</span>
      </Button>
      <Popover className="track-menu-popover" placement="bottom end" offset={6}>
        <Menu aria-label="Add songs">
          <MenuItem
            id="browse-all-songs"
            onAction={() => void beginPlaylistAdd(playlistId)}
            className="library-menu-item"
          >
            <ListMusic size={15} aria-hidden="true" />
            <span>Browse All Songs…</span>
          </MenuItem>
          <MenuItem
            id="import-files-to-playlist"
            isDisabled={pending.has(`playlist:${playlistId}:import:file`)}
            onAction={() => void importFiles(playlistId).catch(() => undefined)}
            className="library-menu-item"
          >
            <FilePlus2 size={15} aria-hidden="true" />
            <span>Import files…</span>
          </MenuItem>
          <MenuItem
            id="import-folder-to-playlist"
            isDisabled={pending.has(`playlist:${playlistId}:import:folder`)}
            onAction={() => void importFolder(playlistId).catch(() => undefined)}
            className="library-menu-item"
          >
            <FilePlus2 size={15} aria-hidden="true" />
            <span>Import folder…</span>
          </MenuItem>
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
