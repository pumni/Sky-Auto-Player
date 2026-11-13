import type { HTMLAttributes } from 'react';

export type StatusBadgeTone = 'neutral' | 'warning' | 'danger';
export type StatusBadgeAppearance = 'soft' | 'plain';

export interface StatusBadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: StatusBadgeTone;
  appearance?: StatusBadgeAppearance;
}

/**
 * Quiet, theme-aware status label. Severity is expressed with a small marker
 * and explicit text, never with a full warning/danger-colored fill.
 */
export function StatusBadge({
  tone = 'neutral',
  appearance = 'soft',
  className,
  children,
  ...props
}: StatusBadgeProps) {
  return (
    <span
      {...props}
      className={`status-badge${className ? ` ${className}` : ''}`}
      data-tone={tone}
      data-appearance={appearance}
    >
      {children}
    </span>
  );
}
