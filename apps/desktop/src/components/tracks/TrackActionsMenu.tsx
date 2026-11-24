import { ListPlus, MoreHorizontal, MinusCircle } from 'lucide-react';
import { useRef, useState } from 'react';
import type { SongRow } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';

interface TrackActionsMenuProps {
  row: SongRow;
  useStore: DesktopStoreHook;
}

export function TrackActionsMenu({ row, useStore }: TrackActionsMenuProps) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [playlistIds, setPlaylistIds] = useState<string[]>([]);
  const playlistAddMode = useStore((store) => store.library.playlistAddMode);
  const source = useStore((store) => store.library.source);
  const onOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (nextOpen) setPlaylistIds([...useStore.getState().libraryNavigation.playlistOrder]);
  };
  const restoreGridFocusIfTriggerWasRemoved = () => {
    window.requestAnimationFrame(() => {
      if (triggerRef.current?.isConnected) return;
      const activeElement = document.activeElement;
      if (
        activeElement instanceof HTMLElement &&
        activeElement !== document.body &&
        activeElement.isConnected
      ) {
        return;
      }
      document
        .querySelector<HTMLElement>('[role="grid"][aria-label="Songs"]')
        ?.focus({ preventScroll: true });
    });
  };

  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger
        render={
          <Button
            ref={triggerRef}
            variant="ghost"
            size="icon-sm"
            className="size-7 border-0 bg-transparent p-0 text-text-tertiary opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 focus-visible:opacity-100 hover:bg-control-hover hover:text-foreground"
            type="button"
            aria-label={`More actions for ${row.title}`}
          />
        }
      >
        <MoreHorizontal size={16} aria-hidden="true" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" aria-label={`Actions for ${row.title}`}>
        {playlistIds.length === 0 ? (
          <DropdownMenuItem disabled className="h-8 gap-2 text-[13px]">
            <ListPlus size={15} aria-hidden="true" />
            <span>No playlists yet</span>
          </DropdownMenuItem>
        ) : (
          playlistIds.map((playlistId) => {
            const playlist = useStore.getState().libraryNavigation.playlistsById.get(playlistId);
            if (!playlist) return null;
            return (
              <DropdownMenuItem
                key={playlistId}
                className="h-8 gap-2 text-[13px]"
                onClick={() => {
                  void useStore
                    .getState()
                    .addSongToPlaylist(playlistId, row.song_id)
                    .catch(() => undefined);
                }}
              >
                <ListPlus size={15} aria-hidden="true" />
                <span>Add to {playlist.name}</span>
                <small className="ml-auto text-xs tabular-nums text-text-tertiary">
                  {playlist.song_count}
                </small>
              </DropdownMenuItem>
            );
          })
        )}
        {source.kind === 'playlist' && playlistAddMode === null && (
          <DropdownMenuItem
            variant="destructive"
            className="h-8 gap-2 text-[13px]"
            onClick={() => {
              void useStore
                .getState()
                .removeSongFromPlaylist(source.id, row.song_id)
                .then(restoreGridFocusIfTriggerWasRemoved)
                .catch(() => undefined);
            }}
          >
            <MinusCircle size={15} aria-hidden="true" />
            <span>Remove from playlist</span>
          </DropdownMenuItem>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
