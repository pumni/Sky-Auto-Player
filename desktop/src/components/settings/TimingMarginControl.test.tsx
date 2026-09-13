import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
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
  down_late_grace_min_us: 0,
  down_late_grace_max_us: 5_000,
  down_late_grace_step_us: 100,
};

const recommendation: TimingMarginRecommendation = {
  recommended_timing_margin_us: 1_300,
  qualified: true,
  source: 'qualified_calibration',
};

describe('TimingMarginControl', () => {
  afterEach(() => cleanup());

  it('shows recommendation as text and steps by the backend increment', () => {
    const changes: number[] = [];
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="full"
        onChange={async (value) => {
          changes.push(value);
          return value;
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));

    expect(changes).toEqual([900]);
    expect(screen.getByText('· rec. 1300 µs')).toBeInTheDocument();
    expect(screen.getByText('Source: Qualified calibration')).toBeInTheDocument();
    expect(screen.getByText('Applies to Hold and Release Gap')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Use recommended/ })).not.toBeInTheDocument();
  });

  it('keeps the compact variant informational and omits source details', () => {
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={async (value) => value}
      />,
    );

    expect(screen.getByRole('group', { name: 'Timing Margin' })).toContainElement(
      screen.getByText('· rec. 1300 µs'),
    );
    expect(screen.queryByText(/Source:/)).not.toBeInTheDocument();
    expect(screen.queryByText('Applies to Hold and Release Gap')).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Use recommended/ })).not.toBeInTheDocument();
  });

  it('disables steps at the configured endpoints', () => {
    const { rerender } = render(
      <TimingMarginControl
        value={0}
        options={options}
        recommendation={recommendation}
        density="full"
        onChange={async () => 0}
      />,
    );

    expect(screen.getByRole('button', { name: 'Decrease Timing Margin' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Increase Timing Margin' })).toBeEnabled();

    rerender(
      <TimingMarginControl
        value={3_000}
        options={options}
        recommendation={recommendation}
        density="full"
        onChange={async () => 3_000}
      />,
    );

    expect(screen.getByRole('button', { name: 'Decrease Timing Margin' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Increase Timing Margin' })).toBeDisabled();
  });

  it('preserves every rapid step while earlier settings writes are pending', async () => {
    const changes: number[] = [];
    const confirmations: Array<(value: number) => void> = [];
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={(value) => {
          changes.push(value);
          return new Promise<number>((resolve) => confirmations.push(resolve));
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));

    expect(changes).toEqual([900, 1_000, 1_100]);
    expect(screen.getByText('1100 µs')).toBeInTheDocument();

    await act(async () => {
      confirmations[0]?.(900);
      confirmations[1]?.(1_000);
      confirmations[2]?.(1_100);
    });
    await waitFor(() => expect(screen.getByText('1100 µs')).toBeInTheDocument());
  });

  it('resynchronizes to the authoritative value when a settings write fails', async () => {
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="full"
        onChange={async () => {
          throw new Error('settings IPC failed');
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));

    await waitFor(() => expect(screen.getByText('800 µs')).toBeInTheDocument());
  });

  it('shows an over-range recommendation as information and never writes it', () => {
    const changes: number[] = [];
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={{ ...recommendation, recommended_timing_margin_us: 5_300 }}
        density="compact"
        onChange={async (value) => {
          changes.push(value);
          return value;
        }}
      />,
    );

    expect(screen.getByText('· rec. 5300 µs · out of range')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Use recommended/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(changes).toEqual([900]);
    expect(changes).not.toContain(5_300);
  });
});
