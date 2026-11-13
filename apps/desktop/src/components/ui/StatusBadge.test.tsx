import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { StatusBadge } from './StatusBadge';

afterEach(cleanup);

describe('StatusBadge', () => {
  it('shares neutral theme styling without inline color overrides', () => {
    render(<StatusBadge aria-label="Metadata loading">Loading</StatusBadge>);
    const badge = screen.getByText('Loading');
    expect(badge).toHaveClass('status-badge');
    expect(badge).toHaveAttribute('data-tone', 'neutral');
    expect(badge).toHaveAttribute('data-appearance', 'soft');
    expect(badge).toHaveAttribute('aria-label', 'Metadata loading');
    expect(badge).not.toHaveAttribute('style');
  });

  it('retains explicit warning and danger meaning rather than relying on color alone', () => {
    render(
      <>
        <StatusBadge tone="warning" title="Conversion review required">
          Review
        </StatusBadge>
        <StatusBadge tone="danger" aria-label="Playback blocked">
          High risk
        </StatusBadge>
        <StatusBadge appearance="plain">Test · no keys</StatusBadge>
      </>,
    );
    expect(screen.getByText('Review')).toHaveAttribute('data-tone', 'warning');
    expect(screen.getByText('Review')).toHaveAttribute('title', 'Conversion review required');
    expect(screen.getByText('High risk')).toHaveAttribute('data-tone', 'danger');
    expect(screen.getByText('High risk')).toHaveAttribute('aria-label', 'Playback blocked');
    expect(screen.getByText('Test · no keys')).toHaveAttribute('data-appearance', 'plain');
  });
});
