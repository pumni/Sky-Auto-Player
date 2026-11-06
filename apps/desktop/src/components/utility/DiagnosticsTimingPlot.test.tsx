import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { DiagnosticsTimingPlot } from './DiagnosticsTimingPlot';

afterEach(cleanup);

describe('DiagnosticsTimingPlot', () => {
  it('reports dry-run timing as not measured instead of fabricating sender samples', () => {
    render(<DiagnosticsTimingPlot samples={[]} hasActiveSession dryRun />);

    expect(screen.getByRole('status')).toHaveTextContent('No physical timing samples in dry run');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Dry-run playback sends no physical input, so sender timing is not measured.',
    );
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('keeps the timing plot unavailable until native session data is available', () => {
    render(<DiagnosticsTimingPlot samples={[]} hasActiveSession={false} dryRun={false} />);

    expect(screen.getByRole('status')).toHaveTextContent('No active playback session');
    expect(screen.queryByRole('img')).toBeNull();
  });
});
