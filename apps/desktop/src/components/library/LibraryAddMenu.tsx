import { Plus } from 'lucide-react';
import type { RefObject } from 'react';
import { Button } from '../ui/button';

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
    <Button
      data-slot="library-add-button"
      data-collapsed={collapsed ? 'true' : 'false'}
      ref={triggerRef}
      variant="ghost"
      size="icon-sm"
      className={`border-0 bg-transparent p-0 text-text-secondary hover:bg-control-hover hover:text-foreground ${collapsed ? 'size-10' : 'size-[30px]'}`}
      type="button"
      aria-label="Create playlist"
      onClick={onNewPlaylist}
    >
      <Plus size={18} aria-hidden="true" />
    </Button>
  );
}
