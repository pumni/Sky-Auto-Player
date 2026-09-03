import { ListMusic } from 'lucide-react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LibraryNavItem } from './LibraryNavItem';

interface LibraryNavigatorProps {
  useStore: DesktopStoreHook;
}

export function LibraryNavigator({ useStore }: LibraryNavigatorProps) {
  const total = useStore((store: DesktopStore) => store.library.total);
  return (
    <nav className="library-navigator" aria-label="Library">
      <h2>Your Library</h2>
      <LibraryNavItem active count={total} icon={<ListMusic size={18} />}>
        All Songs
      </LibraryNavItem>
    </nav>
  );
}
