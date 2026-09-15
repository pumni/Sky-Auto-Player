import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { PlaybackOptionSets } from '../../bridge/DesktopBridge';
import { LateNoteToleranceControl } from './LateNoteToleranceControl';

const options: PlaybackOptionSets = {
  hold_frames: [1, 1.25, 1.5],
  tempo_scales: [0.5, 1, 2],
  fps: [30, 60, 90, 120],
  timing_margin_min_us: 0,
  timing_margin_max_us: 3_000,
  timing_margin_step_us: 100,
  normal_down_start_tolerance_min_us: 2_500,
  normal_down_start_tolerance_max_us: 5_000,
  normal_down_start_tolerance_step_us: 500,
  normal_down_start_tolerance_default_us: 2_500,
};

describe('LateNoteToleranceControl', () => {
  afterEach(() => cleanup());

  it('renders default value and disables decrease button at min', () => {
    render(
      <LateNoteToleranceControl value={2_500} options={options} onChange={async (val) => val} />,
    );

    expect(screen.getByRole('group', { name: 'Late note tolerance' })).toBeInTheDocument();
    expect(screen.getByText('Late note tolerance')).toBeInTheDocument();
    expect(screen.getByText('· default 2.5 ms')).toBeInTheDocument();
    expect(screen.getByText('2.5 ms')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Decrease Late note tolerance' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Increase Late note tolerance' })).toBeEnabled();
    expect(
      screen.queryByRole('button', { name: 'Reset Late note tolerance to default' }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('increases by 0.5 ms step and shows warning plus reset button above 2.5 ms', async () => {
    const changes: number[] = [];
    const { rerender } = render(
      <LateNoteToleranceControl
        value={2_500}
        options={options}
        onChange={async (val) => {
          changes.push(val);
          return val;
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Late note tolerance' }));

    expect(changes).toEqual([3_000]);

    rerender(
      <LateNoteToleranceControl
        value={3_000}
        options={options}
        onChange={async (val) => {
          changes.push(val);
          return val;
        }}
      />,
    );

    expect(screen.getByText('3.0 ms')).toBeInTheDocument();
    expect(
      screen.getByText(
        'Values above 2.5 ms increase late-note continuity at the cost of authored timing accuracy under load.',
      ),
    ).toBeInTheDocument();
    const resetButton = screen.getByRole('button', {
      name: 'Reset Late note tolerance to default',
    });
    expect(resetButton).toBeInTheDocument();

    fireEvent.click(resetButton);
    expect(changes).toEqual([3_000, 2_500]);
  });

  it('disables increase button at max 5.0 ms', () => {
    render(
      <LateNoteToleranceControl value={5_000} options={options} onChange={async (val) => val} />,
    );

    expect(screen.getByText('5.0 ms')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Decrease Late note tolerance' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Increase Late note tolerance' })).toBeDisabled();
  });

  it('preserves every rapid step while earlier writes are pending', async () => {
    const changes: number[] = [];
    const confirmations: Array<(value: number) => void> = [];
    render(
      <LateNoteToleranceControl
        value={2_500}
        options={options}
        onChange={(val) =>
          new Promise((resolve) => {
            changes.push(val);
            confirmations.push(resolve);
          })
        }
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Late note tolerance' }));
    fireEvent.click(screen.getByRole('button', { name: 'Increase Late note tolerance' }));

    expect(changes).toEqual([3_000, 3_500]);
    expect(screen.getByText('3.5 ms')).toBeInTheDocument();

    await act(async () => {
      confirmations[0]?.(3_000);
    });
    expect(screen.getByText('3.5 ms')).toBeInTheDocument();

    await act(async () => {
      confirmations[1]?.(3_500);
    });

    await waitFor(() => {
      expect(screen.getByText('3.5 ms')).toBeInTheDocument();
    });
  });
});
