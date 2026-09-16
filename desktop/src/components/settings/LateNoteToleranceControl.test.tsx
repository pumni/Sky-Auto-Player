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
  normal_down_start_tolerance_min_us: 2_000,
  normal_down_start_tolerance_max_us: 10_000,
  normal_down_start_tolerance_step_us: 500,
  normal_down_start_tolerance_default_us: 2_500,
};

describe('LateNoteToleranceControl', () => {
  afterEach(() => cleanup());

  it('renders default value 2.5 ms with both stepper buttons enabled and no reset button', () => {
    render(
      <LateNoteToleranceControl value={2_500} options={options} onChange={async (val) => val} />,
    );

    expect(screen.getByRole('group', { name: 'Late note tolerance' })).toBeInTheDocument();
    expect(screen.getByText('Late note tolerance')).toBeInTheDocument();
    expect(screen.getByText('· default 2.5 ms')).toBeInTheDocument();
    expect(screen.getByText('2.5 ms')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Decrease Late note tolerance' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Increase Late note tolerance' })).toBeEnabled();
    expect(
      screen.getByText(
        'If notes are still dropped on fast chords under load, increase this step-by-step. Rescued notes may play slightly later than their authored time.',
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /Reset Late note tolerance/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('decreases to min 2.0 ms, disables decrease button, and updates guidance copy', async () => {
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

    fireEvent.click(screen.getByRole('button', { name: 'Decrease Late note tolerance' }));
    expect(changes).toEqual([2_000]);

    rerender(
      <LateNoteToleranceControl
        value={2_000}
        options={options}
        onChange={async (val) => {
          changes.push(val);
          return val;
        }}
      />,
    );

    expect(screen.getByText('2.0 ms')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Decrease Late note tolerance' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Increase Late note tolerance' })).toBeEnabled();
    expect(
      screen.getByText(
        'Lower values keep notes closer to authored timing, but Windows wake jitter may cause more late notes to be dropped.',
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /Reset Late note tolerance/i }),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('increases to 3.0 ms and shows moderate tolerance warning without reset button', async () => {
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
    expect(
      screen.getByText(
        'If notes are still dropped on fast chords under load, increase this step-by-step. Rescued notes may play slightly later than their authored time.',
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /Reset Late note tolerance/i }),
    ).not.toBeInTheDocument();
  });

  it('shows high-tolerance warning for values > 5.0 ms and disables increase button at max 10.0 ms', () => {
    const { rerender } = render(
      <LateNoteToleranceControl value={5_500} options={options} onChange={async (val) => val} />,
    );

    expect(screen.getByText('5.5 ms')).toBeInTheDocument();
    expect(
      screen.getByText(
        'High tolerance can rescue very late notes, but in dense passages it may cause following authored notes to become stale or physically infeasible. Use only when lower values still drop notes.',
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Increase Late note tolerance' })).toBeEnabled();

    rerender(
      <LateNoteToleranceControl value={10_000} options={options} onChange={async (val) => val} />,
    );

    expect(screen.getByText('10.0 ms')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Decrease Late note tolerance' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Increase Late note tolerance' })).toBeDisabled();
    expect(
      screen.getByText(
        'High tolerance can rescue very late notes, but in dense passages it may cause following authored notes to become stale or physically infeasible. Use only when lower values still drop notes.',
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /Reset Late note tolerance/i }),
    ).not.toBeInTheDocument();
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
