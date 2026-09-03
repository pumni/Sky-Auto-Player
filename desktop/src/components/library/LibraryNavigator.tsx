import { ChevronLeft, ChevronRight, Heart, ListMusic } from 'lucide-react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { LibraryNavItem } from './LibraryNavItem';

interface LibraryNavigatorProps {
  collapsed: boolean;
  onToggleCollapsed: () => void;
  useStore: DesktopStoreHook;
}

export function LibraryNavigator({
  collapsed,
  onToggleCollapsed,
  useStore,
}: LibraryNavigatorProps) {
  const catalogTotal = useStore((store: DesktopStore) => store.library.catalogTotal);
  const likedTotal = useStore((store: DesktopStore) => store.library.likedTotal);
  const source = useStore((store: DesktopStore) => store.library.source);
  const selectLibrarySource = useStore((store: DesktopStore) => store.selectLibrarySource);
  return (
    <nav className="library-navigator" aria-label="Library">
      <div className="library-navigator-heading">
        <h2>Your Library</h2>
        <button
          className="icon-button library-collapse-button"
          type="button"
          aria-label={collapsed ? 'Expand library navigator' : 'Collapse library navigator'}
          title={collapsed ? 'Expand library navigator' : 'Collapse library navigator'}
          aria-pressed={collapsed}
          onClick={onToggleCollapsed}
        >
          {collapsed ? (
            <ChevronRight size={16} aria-hidden="true" />
          ) : (
            <ChevronLeft size={16} aria-hidden="true" />
          )}
        </button>
      </div>
      <LibraryNavItem
        collapsed={collapsed}
        label="Liked Songs"
        active={source === 'liked'}
        count={likedTotal}
        icon={<Heart size={18} />}
        onPress={() => void selectLibrarySource('liked')}
      >
        Liked Songs
      </LibraryNavItem>
      <LibraryNavItem
        collapsed={collapsed}
        label="All Songs"
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
