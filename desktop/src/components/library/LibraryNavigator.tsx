import { ListMusic } from 'lucide-react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LibraryNavItem } from './LibraryNavItem';

interface LibraryNavigatorProps {
  useStore: DesktopStoreHook;
}

export function LibraryNavigator({ useStore }: LibraryNavigatorProps) {
  const catalogTotal = useStore((store: DesktopStore) => store.library.catalogTotal);
  const search = useStore((store: DesktopStore) => store.search);
  return (
    <nav className="library-navigator" aria-label="Library">
      <h2>Your Library</h2>
      <LibraryNavItem
        active
        count={catalogTotal}
        icon={<ListMusic size={18} />}
        onPress={() => void search('')}
      >
        All Songs
      </LibraryNavItem>
    </nav>
  );
}
