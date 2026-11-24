import type { CSSProperties, ReactNode } from 'react';
import { cn } from '../../lib/utils';

interface WorkbenchPaneProps {
  children: ReactNode;
  className?: string;
  pane: 'navigator' | 'browser' | 'inspector';
  collapsed?: boolean;
  presentationMode?: 'closed' | 'docked' | 'overlay';
  style?: CSSProperties;
}

export function WorkbenchPane({
  children,
  className = '',
  pane,
  collapsed = false,
  presentationMode,
  style,
}: WorkbenchPaneProps) {
  return (
    <div
      data-workbench-pane
      data-pane={pane}
      data-collapsed={collapsed}
      {...(presentationMode ? { 'data-presentation-mode': presentationMode } : {})}
      className={cn(
        'flex min-h-0 min-w-0 overflow-hidden rounded-pane border border-border bg-card [contain:layout_style_paint]',
        className,
      )}
      style={style}
    >
      {children}
    </div>
  );
}
