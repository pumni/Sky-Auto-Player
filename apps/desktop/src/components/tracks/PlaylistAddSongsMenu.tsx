import { ChevronDown, FilePlus2, ListMusic, Plus } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';

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
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            data-slot="playlist-add-songs-button"
            variant="outline"
            className="h-[30px] gap-1.5 border-border bg-surface px-2.5 text-xs text-text-secondary hover:bg-control-hover"
            type="button"
          />
        }
      >
        <Plus size={16} aria-hidden="true" />
        <span>Add songs</span>
        <ChevronDown size={14} aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" aria-label="Add songs">
        <DropdownMenuItem
          className="h-8 gap-2 text-[13px]"
          onClick={() => void openPlaylistAdd(playlistId)}
        >
          <ListMusic size={15} aria-hidden="true" />
          <span>Browse Built-in Songs…</span>
        </DropdownMenuItem>
        <DropdownMenuItem
          className="h-8 gap-2 text-[13px]"
          disabled={pending.has(`playlist:${playlistId}:import:file`)}
          onClick={() => void runImport('files', importFiles)}
        >
          <FilePlus2 size={15} aria-hidden="true" />
          <span>Import files…</span>
        </DropdownMenuItem>
        <DropdownMenuItem
          className="h-8 gap-2 text-[13px]"
          disabled={pending.has(`playlist:${playlistId}:import:folder`)}
          onClick={() => void runImport('folder', importFolder)}
        >
          <FilePlus2 size={15} aria-hidden="true" />
          <span>Import folder…</span>
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
