import { ListMusic, ListPlus, MoreHorizontal, Pencil, Trash2 } from 'lucide-react';
import type { LibraryPlaylistSummary } from '../../bridge/DesktopBridge';
import { Button } from '../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
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
    <div
      data-slot="playlist-nav-item"
      className={`group relative grid min-h-[42px] min-w-0 grid-cols-[minmax(0,1fr)_30px] items-center rounded-control ${active ? 'bg-selection' : 'hover:bg-control-hover'}`}
      data-active={active}
    >
      <LibraryNavItem
        label={playlist.name}
        active={active}
        icon={<ListMusic size={18} />}
        onSelect={onSelect}
      >
        {playlist.name}
      </LibraryNavItem>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-[30px] border-0 bg-transparent p-0 text-text-tertiary opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 group-data-[active=true]:opacity-100 aria-expanded:opacity-100 hover:bg-control-hover hover:text-foreground focus-visible:opacity-100"
              type="button"
              aria-label={`More actions for ${playlist.name}`}
              disabled={pending}
            />
          }
        >
          <MoreHorizontal size={16} aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" aria-label={`${playlist.name} actions`}>
          <DropdownMenuItem className="h-8 gap-2 text-[13px]" onClick={onAddSongs}>
            <ListPlus size={15} aria-hidden="true" />
            <span>Add songs</span>
          </DropdownMenuItem>
          <DropdownMenuItem className="h-8 gap-2 text-[13px]" onClick={onRename}>
            <Pencil size={15} aria-hidden="true" />
            <span>Rename</span>
          </DropdownMenuItem>
          <DropdownMenuItem
            variant="destructive"
            className="h-8 gap-2 text-[13px]"
            onClick={onDelete}
          >
            <Trash2 size={15} aria-hidden="true" />
            <span>Delete playlist</span>
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
