import { ChevronDown, FilePlus2, ListMusic, Plus } from 'lucide-react';
import { Button, Menu, MenuItem, MenuTrigger, Popover } from 'react-aria-components';
import type { DesktopStoreHook } from '../../state/store';

interface PlaylistAddSongsMenuProps {
  playlistId: string;
  useStore: DesktopStoreHook;
  onImportFeedback: (feedback: PlaylistImportFeedback | null) => void;
}

export type PlaylistImportKind = 'files' | 'folder';

export type PlaylistImportFeedback =
  | { state: 'pending'; kind: PlaylistImportKind }
  | { state: 'complete'; kind: PlaylistImportKind; importedSongCount: number }
  | { state: 'empty'; kind: PlaylistImportKind }
  | { state: 'error'; kind: PlaylistImportKind };

export function PlaylistAddSongsMenu({
  playlistId,
  useStore,
  onImportFeedback,
}: PlaylistAddSongsMenuProps) {
  const openPlaylistAdd = useStore((store) => store.openPlaylistAdd);
  const importFiles = useStore((store) => store.importLocalFilesToPlaylist);
  const importFolder = useStore((store) => store.importLocalFolderToPlaylist);
  const pending = useStore((store) => store.libraryNavigation.pendingMutations);

  const runImport = async (
    kind: PlaylistImportKind,
    action: (targetPlaylistId: string) => Promise<number | null>,
  ) => {
    onImportFeedback({ state: 'pending', kind });
    try {
      const importedSongCount = await action(playlistId);
      if (importedSongCount === null) {
        onImportFeedback(null);
      } else if (importedSongCount === 0) {
        onImportFeedback({ state: 'empty', kind });
      } else {
        onImportFeedback({ state: 'complete', kind, importedSongCount });
      }
    } catch {
      onImportFeedback({ state: 'error', kind });
    }
  };

  return (
    <MenuTrigger>
      <Button className="button playlist-add-songs-button">
        <Plus size={16} aria-hidden="true" />
        <span>Add songs</span>
        <ChevronDown size={14} aria-hidden="true" />
      </Button>
      <Popover className="track-menu-popover" placement="bottom end" offset={6}>
        <Menu aria-label="Add songs">
          <MenuItem
            id="browse-all-songs"
            onAction={() => void openPlaylistAdd(playlistId)}
            className="library-menu-item"
          >
            <ListMusic size={15} aria-hidden="true" />
            <span>Browse All Songs…</span>
          </MenuItem>
          <MenuItem
            id="import-files-to-playlist"
            isDisabled={pending.has(`playlist:${playlistId}:import:file`)}
            onAction={() => void runImport('files', importFiles)}
            className="library-menu-item"
          >
            <FilePlus2 size={15} aria-hidden="true" />
            <span>Import files…</span>
          </MenuItem>
          <MenuItem
            id="import-folder-to-playlist"
            isDisabled={pending.has(`playlist:${playlistId}:import:folder`)}
            onAction={() => void runImport('folder', importFolder)}
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
