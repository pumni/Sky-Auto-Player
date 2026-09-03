import type { ReactNode } from 'react';

interface LibraryNavItemProps {
  active?: boolean;
  collapsed?: boolean;
  children: ReactNode;
  count?: number;
  icon: ReactNode;
  iconTone?: 'default' | 'liked';
  label: string;
  onPress?: () => void;
}

export function LibraryNavItem({
  active = false,
  collapsed = false,
  children,
  count,
  icon,
  iconTone = 'default',
  label,
  onPress,
}: LibraryNavItemProps) {
  return (
    <button
      className={`library-nav-item${active ? ' is-active' : ''}`}
      data-icon-tone={iconTone}
      type="button"
      aria-current={active ? 'page' : undefined}
      onClick={onPress}
      aria-label={collapsed ? label : undefined}
      title={collapsed ? label : undefined}
    >
      <span className="library-nav-item-icon" aria-hidden="true">
        {icon}
      </span>
      <span className="library-nav-item-label">{children}</span>
      {count !== undefined && <span className="library-nav-item-count">{count}</span>}
    </button>
  );
}
