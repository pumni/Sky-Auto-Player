import { Children, type ReactNode } from 'react';

interface LibrarySectionProps {
  label: string;
  children: ReactNode;
  showWhenEmpty?: boolean;
}

export function LibrarySection({ label, children, showWhenEmpty = false }: LibrarySectionProps) {
  if (!showWhenEmpty && Children.count(children) === 0) return null;
  return (
    <section className="library-section" aria-labelledby={`library-section-${label.toLowerCase()}`}>
      <h3 id={`library-section-${label.toLowerCase()}`} className="library-section-heading">
        {label}
      </h3>
      <div className="library-section-items">{children}</div>
    </section>
  );
}
