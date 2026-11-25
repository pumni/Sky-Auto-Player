import type { ReactNode } from 'react';

export function DiagnosticsEmptyState({
  title,
  detail,
  status = 'empty',
}: {
  title: string;
  detail: string;
  status?: 'empty' | 'error';
}) {
  return (
    <div
      data-slot="diagnostics-empty-state"
      className="grid justify-items-center gap-1.5 px-3 py-5 text-center text-text-tertiary"
      role="status"
    >
      <strong
        className={`text-sm font-semibold ${status === 'error' ? 'text-danger' : 'text-foreground'}`}
      >
        {title}
      </strong>
      <span className="max-w-[34em] text-xs text-text-secondary">{detail}</span>
    </div>
  );
}

export function DiagnosticsMetricGroup({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <section data-slot="diagnostics-metric-group" className="mb-3 grid gap-2" aria-label={title}>
      <h4 className="m-0 text-xs font-semibold uppercase tracking-wide text-text-secondary">
        {title}
      </h4>
      <div className="grid grid-cols-2 gap-x-3 @max-[360px]/utility:grid-cols-1">{children}</div>
    </section>
  );
}

export type DiagnosticsTone = 'healthy' | 'warning' | 'danger' | 'neutral';

export function DiagnosticsMetric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: DiagnosticsTone | undefined;
}) {
  const isNumber = /^[-+]?(?:\d|\.\d)/.test(value);
  const valueTone =
    tone === 'healthy'
      ? 'text-success'
      : tone === 'warning'
        ? 'text-warning'
        : tone === 'danger'
          ? 'text-danger'
          : 'text-foreground';
  return (
    <div
      data-slot="diagnostics-metric"
      className="flex min-w-0 items-baseline justify-between gap-2 border-b border-border py-2"
    >
      <span className="text-xs text-text-tertiary">{label}</span>
      <strong
        className={`text-right text-xs tabular-nums ${isNumber ? 'font-mono' : 'font-medium text-text-secondary'} ${valueTone}`}
      >
        {value}
      </strong>
    </div>
  );
}
