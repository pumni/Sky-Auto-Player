import { Plus } from 'lucide-react';
import { Button } from 'react-aria-components';
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
    <Button
      ref={triggerRef}
      className={`icon-button library-add-button${collapsed ? ' is-collapsed' : ''}`}
      aria-label="Create playlist"
      title="Create playlist"
      onPress={onNewPlaylist}
    >
      <Plus size={18} aria-hidden="true" />
    </Button>
  );
}
