import type { CSSProperties, ReactNode } from 'react';

interface WorkbenchPaneProps {
  children: ReactNode;
  className?: string;
  style?: CSSProperties;
}

export function WorkbenchPane({ children, className = '', style }: WorkbenchPaneProps) {
  return (
    <div className={`workbench-pane ${className}`.trim()} style={style}>
      {children}
    </div>
  );
}
