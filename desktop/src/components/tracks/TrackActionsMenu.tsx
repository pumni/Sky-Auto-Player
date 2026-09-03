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
  const [playlistIds, setPlaylistIds] = useState<string[]>([]);
  const playlistAddMode = useStore((store) => store.library.playlistAddMode);
  const onOpenChange = (nextOpen: boolean) => {
    setOpen(nextOpen);
    if (nextOpen) setPlaylistIds([...useStore.getState().libraryNavigation.playlistOrder]);
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
            if (
              key === 'remove-current' &&
              state.library.source.kind === 'playlist' &&
              state.library.playlistAddMode === null
            ) {
              void state
                .removeSongFromPlaylist(state.library.source.id, row.song_id)
                .catch(() => undefined);
              return;
            }
            if (key.startsWith('playlist:')) {
              void state
                .addSongToPlaylist(key.slice('playlist:'.length), row.song_id)
                .catch(() => undefined);
            }
          }}
        >
          {playlistIds.length === 0 ? (
            <MenuItem id="no-playlists" isDisabled className="library-menu-item">
              <ListPlus size={15} aria-hidden="true" />
              <span>No playlists yet</span>
            </MenuItem>
          ) : (
            playlistIds.map((playlistId) => {
              const playlist = useStore.getState().libraryNavigation.playlistsById.get(playlistId);
              if (!playlist) return null;
              return (
                <MenuItem
                  key={playlistId}
                  id={`playlist:${playlistId}`}
                  className="library-menu-item"
                >
                  <ListPlus size={15} aria-hidden="true" />
                  <span>Add to {playlist.name}</span>
                  <small>{playlist.song_count}</small>
                </MenuItem>
              );
            })
          )}
          {useStore.getState().library.source.kind === 'playlist' && playlistAddMode === null && (
            <MenuItem id="remove-current" className="library-menu-item is-danger">
              <MinusCircle size={15} aria-hidden="true" />
              <span>Remove from playlist</span>
            </MenuItem>
          )}
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
