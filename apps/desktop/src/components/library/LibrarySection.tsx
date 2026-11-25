import { Children, type ReactNode } from 'react';

interface LibrarySectionProps {
  label: string;
  children: ReactNode;
  showWhenEmpty?: boolean;
}

export function LibrarySection({ label, children, showWhenEmpty = false }: LibrarySectionProps) {
  if (!showWhenEmpty && Children.count(children) === 0) return null;
  return (
    <section className="min-w-0" aria-labelledby={`library-section-${label.toLowerCase()}`}>
      <h3
        data-slot="library-section-heading"
        id={`library-section-${label.toLowerCase()}`}
        className="mb-1.5 ml-2 mr-2 mt-0 text-xs font-semibold leading-4 text-text-tertiary"
      >
        {label}
      </h3>
      <div className="grid min-w-0 gap-1">{children}</div>
    </section>
  );
}
