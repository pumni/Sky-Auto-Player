import type { HTMLAttributes } from 'react';
import { cn } from 'cn';

export type StatusBadgeTone = 'neutral' | 'success' | 'warning' | 'danger';
type StatusBadgeAppearance = 'soft' | 'plain';

interface StatusBadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: StatusBadgeTone;
  appearance?: StatusBadgeAppearance;
  indicator?: boolean;
}

/**
 * Quiet, theme-aware status label. Severity is expressed with a small marker
 * and explicit text, never with a full warning/danger-colored fill.
 */
export function StatusBadge({
  tone = 'neutral',
  appearance = 'soft',
  indicator = true,
  className,
  children,
  ...props
}: StatusBadgeProps) {
  const toneText = {
    neutral: 'text-secondary-foreground',
    success: 'text-success',
    warning: 'text-warning',
    danger: 'text-danger',
  }[tone];
  const surface =
    appearance === 'plain'
      ? 'border-0 bg-transparent px-0 py-0'
      : {
          neutral: 'border-border bg-secondary px-1.5 py-0.5',
          success: 'border-success-border bg-success-surface px-1.5 py-0.5',
          warning: 'border-warning-border bg-warning-surface px-1.5 py-0.5',
          danger: 'border-danger-border bg-danger-surface px-1.5 py-0.5',
        }[tone];

  return (
    <span
      {...props}
      data-slot="status-badge"
      className={cn(
        'inline-flex min-h-5 items-center justify-center gap-1 rounded-full border text-xs font-medium leading-4',
        toneText,
        surface,
        className,
      )}
      data-tone={tone}
      data-appearance={appearance}
    >
      {indicator && (
        <span className="size-1.5 shrink-0 rounded-full bg-current" aria-hidden="true" />
      )}
      {children}
    </span>
  );
}
