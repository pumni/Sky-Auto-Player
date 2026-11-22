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
    <div className={`diagnostics-empty-state is-${status}`} role="status">
      <strong>{title}</strong>
      <span>{detail}</span>
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
    <section className="diagnostics-metric-group" aria-label={title}>
      <h4>{title}</h4>
      <div className="diagnostics-metrics">{children}</div>
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
  const valueClass = [
    isNumber ? 'is-number' : 'is-status',
    tone && tone !== 'neutral' ? `is-tone-${tone}` : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <div className="diagnostics-metric">
      <span>{label}</span>
      <strong className={valueClass}>{value}</strong>
    </div>
  );
}
