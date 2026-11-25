import type { HTMLAttributes } from 'react';
import { cn } from 'cn';

export type StatusBadgeTone = 'neutral' | 'success' | 'warning' | 'danger';
type StatusBadgeAppearance = 'soft' | 'plain';

interface StatusBadgeProps extends HTMLAttributes<HTMLSpanElement> {
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
      data-slot="status-badge"
      className={cn(
        'inline-flex min-h-5 items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs font-medium leading-4',
        'border-border bg-secondary text-secondary-foreground',
        'data-[tone=success]:border-success-border data-[tone=success]:bg-success-surface data-[tone=success]:text-success',
        'data-[tone=warning]:border-warning-border data-[tone=warning]:bg-warning-surface data-[tone=warning]:text-warning',
        'data-[tone=danger]:border-danger-border data-[tone=danger]:bg-danger-surface data-[tone=danger]:text-danger',
        'data-[appearance=plain]:border-transparent data-[appearance=plain]:bg-transparent',
        className,
      )}
      data-tone={tone}
      data-appearance={appearance}
    >
      <span className="size-1.5 shrink-0 rounded-full bg-current" aria-hidden="true" />
      {children}
    </span>
  );
}
