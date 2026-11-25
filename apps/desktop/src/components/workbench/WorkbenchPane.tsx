import type { CSSProperties, ReactNode, TransitionEventHandler } from 'react';
import { cn } from '../../lib/utils';

interface WorkbenchPaneProps {
  children: ReactNode;
  className?: string;
  pane: 'navigator' | 'browser' | 'inspector';
  collapsed?: boolean;
  presentationMode?: 'closed' | 'docked' | 'overlay';
  style?: CSSProperties;
  ariaHidden?: boolean;
  inert?: boolean;
  open?: boolean;
  onTransitionEnd?: TransitionEventHandler<HTMLDivElement>;
  onTransitionCancel?: TransitionEventHandler<HTMLDivElement>;
}

export function WorkbenchPane({
  children,
  className = '',
  pane,
  collapsed = false,
  presentationMode,
  style,
  ariaHidden,
  inert,
  open,
  onTransitionEnd,
  onTransitionCancel,
}: WorkbenchPaneProps) {
  return (
    <div
      data-workbench-pane
      data-pane={pane}
      data-collapsed={collapsed}
      {...(presentationMode ? { 'data-presentation-mode': presentationMode } : {})}
      {...(ariaHidden !== undefined ? { 'aria-hidden': ariaHidden } : {})}
      inert={inert}
      {...(open !== undefined ? { 'data-open': open } : {})}
      onTransitionEnd={onTransitionEnd}
      onTransitionCancel={onTransitionCancel}
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
