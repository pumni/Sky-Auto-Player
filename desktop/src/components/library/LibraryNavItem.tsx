import type { ReactNode } from 'react';

interface LibraryNavItemProps {
  active?: boolean;
  children: ReactNode;
  count?: number;
  icon: ReactNode;
  onPress?: () => void;
}

export function LibraryNavItem({
  active = false,
  children,
  count,
  icon,
  onPress,
}: LibraryNavItemProps) {
  return (
    <button
      className={`library-nav-item${active ? ' is-active' : ''}`}
      type="button"
      aria-current={active ? 'page' : undefined}
      onClick={onPress}
    >
      <span className="library-nav-item-icon" aria-hidden="true">
        {icon}
      </span>
      <span className="library-nav-item-label">{children}</span>
      {count !== undefined && <span className="library-nav-item-count">{count}</span>}
    </button>
  );
}
