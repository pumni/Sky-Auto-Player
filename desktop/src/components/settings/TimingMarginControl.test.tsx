import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { PlaybackOptionSets, TimingMarginRecommendation } from '../../bridge/DesktopBridge';
import { TimingMarginControl } from './TimingMarginControl';

const options: PlaybackOptionSets = {
  hold_frames: [1, 1.25, 1.5],
  tempo_scales: [0.5, 1, 2],
  fps: [30, 60, 90, 120],
  timing_margin_min_us: 0,
  timing_margin_max_us: 3_000,
  timing_margin_step_us: 100,
};

const recommendation: TimingMarginRecommendation = {
  recommended_timing_margin_us: 1_300,
  qualified: true,
  source: 'qualified_calibration',
};

describe('TimingMarginControl', () => {
  afterEach(() => cleanup());

  it('steps by the backend increment and applies recommendations explicitly', () => {
    const changes: number[] = [];
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        onChange={(value) => changes.push(value)}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    fireEvent.click(screen.getByRole('button', { name: 'Use recommended (1300 µs)' }));

    expect(changes).toEqual([900, 1_300]);
  });

  it('disables steps at the configured endpoints', () => {
    const { rerender } = render(
      <TimingMarginControl
        value={0}
        options={options}
        recommendation={recommendation}
        onChange={() => undefined}
      />,
    );

    expect(screen.getByRole('button', { name: 'Decrease Timing Margin' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Increase Timing Margin' })).toBeEnabled();

    rerender(
      <TimingMarginControl
        value={3_000}
        options={options}
        recommendation={recommendation}
        onChange={() => undefined}
      />,
    );

    expect(screen.getByRole('button', { name: 'Decrease Timing Margin' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Increase Timing Margin' })).toBeDisabled();
  });
});
