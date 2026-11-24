import type { ReactNode } from 'react';

interface LibraryNavItemProps {
  active?: boolean;
  collapsed?: boolean;
  children: ReactNode;
  count?: number;
  disabled?: boolean;
  icon: ReactNode;
  iconTone?: 'default' | 'liked';
  label: string;
  onSelect?: () => void;
}

export function LibraryNavItem({
  active = false,
  collapsed = false,
  children,
  count,
  disabled = false,
  icon,
  iconTone = 'default',
  label,
  onSelect,
}: LibraryNavItemProps) {
  return (
    <button
      className={`relative grid grid-cols-[24px_minmax(0,1fr)_auto] items-center gap-2 rounded-control border-0 bg-transparent px-2 text-left text-[13px] font-medium text-text-secondary outline-none transition-colors duration-fast hover:bg-control-hover hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:cursor-not-allowed disabled:opacity-60 data-[active=true]:bg-selection data-[active=true]:text-foreground ${collapsed ? 'size-10 min-h-10 grid-cols-1 justify-items-center px-0' : 'min-h-[42px]'}`}
      data-active={active}
      data-icon-tone={iconTone}
      data-slot="library-nav-item"
      type="button"
      disabled={disabled}
      aria-current={active ? 'page' : undefined}
      onClick={onSelect}
      aria-label={collapsed ? label : undefined}
      title={collapsed || disabled ? label : undefined}
    >
      <span
        data-slot="library-nav-icon"
        className={`grid place-items-center ${iconTone === 'liked' || active ? 'text-accent-ink' : 'text-text-secondary'}`}
        aria-hidden="true"
      >
        {icon}
      </span>
      {!collapsed && (
        <span
          data-slot="library-nav-label"
          className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap"
        >
          {children}
        </span>
      )}
      {!collapsed && count !== undefined && (
        <span className="tabular-nums text-xs text-text-tertiary">{count}</span>
      )}
    </button>
  );
}
