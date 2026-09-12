import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { PlaybackOptionSets } from '../../bridge/DesktopBridge';
import { LateDownToleranceControl } from './LateDownToleranceControl';

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

describe('LateDownToleranceControl', () => {
  afterEach(() => cleanup());

  it('preserves rapid 100-microsecond steps while earlier settings writes are pending', async () => {
    const changes: number[] = [];
    const confirmations: Array<(value: number) => void> = [];
    render(
      <LateDownToleranceControl
        value={500}
        options={options}
        onChange={(value) => {
          changes.push(value);
          return new Promise<number>((resolve) => confirmations.push(resolve));
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Late Down tolerance' }));
    fireEvent.click(screen.getByRole('button', { name: 'Increase Late Down tolerance' }));
    fireEvent.click(screen.getByRole('button', { name: 'Increase Late Down tolerance' }));

    expect(changes).toEqual([600, 700, 800]);
    expect(screen.getByText('800 µs')).toBeInTheDocument();

    await act(async () => {
      confirmations[0]?.(600);
      confirmations[1]?.(700);
      confirmations[2]?.(800);
    });
    await waitFor(() => expect(screen.getByText('800 µs')).toBeInTheDocument());
  });

  it('resynchronizes to the authoritative setting after a failed write', async () => {
    render(
      <LateDownToleranceControl
        value={500}
        options={options}
        onChange={async () => {
          throw new Error('settings IPC failed');
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Late Down tolerance' }));

    await waitFor(() => expect(screen.getByText('500 µs')).toBeInTheDocument());
  });

  it('disables the configured endpoints and explains zero tolerance', () => {
    const { rerender } = render(
      <LateDownToleranceControl value={0} options={options} onChange={async () => 0} />,
    );

    expect(screen.getByRole('button', { name: 'Decrease Late Down tolerance' })).toBeDisabled();
    expect(screen.getByText(/any measured lateness beyond the exact target may drop a Down/))
      .toBeInTheDocument();

    rerender(
      <LateDownToleranceControl value={5_000} options={options} onChange={async () => 5_000} />,
    );
    expect(screen.getByRole('button', { name: 'Increase Late Down tolerance' })).toBeDisabled();
  });
});
