import { Heart, ListMusic } from 'lucide-react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LibraryNavItem } from './LibraryNavItem';

interface LibraryNavigatorProps {
  useStore: DesktopStoreHook;
}

export function LibraryNavigator({ useStore }: LibraryNavigatorProps) {
  const catalogTotal = useStore((store: DesktopStore) => store.library.catalogTotal);
  const likedTotal = useStore((store: DesktopStore) => store.library.likedTotal);
  const source = useStore((store: DesktopStore) => store.library.source);
  const selectLibrarySource = useStore((store: DesktopStore) => store.selectLibrarySource);
  return (
    <nav className="library-navigator" aria-label="Library">
      <h2>Your Library</h2>
      <LibraryNavItem
        active={source === 'liked'}
        count={likedTotal}
        icon={<Heart size={18} />}
        onPress={() => void selectLibrarySource('liked')}
      >
        Liked Songs
      </LibraryNavItem>
      <LibraryNavItem
        active={source === 'all'}
        count={catalogTotal}
        icon={<ListMusic size={18} />}
        onPress={() => void selectLibrarySource('all')}
      >
        All Songs
      </LibraryNavItem>
    </nav>
  );
}
