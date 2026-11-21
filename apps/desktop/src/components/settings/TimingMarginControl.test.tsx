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
      screen.getByText('Recommended 1300 µs'),
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

    expect(screen.getByText('Recommended 5300 µs · out of range')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Use recommended/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(changes).toEqual([900]);
    expect(changes).not.toContain(5_300);
  });

  it('does not resurrect old optimistic override when authoritative prop changes away and back', async () => {
    let confirmWrite: ((value: number) => void) | null = null;
    const { rerender } = render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={() =>
          new Promise((resolve) => {
            confirmWrite = resolve;
          })
        }
      />,
    );

    // 1. User requests 900
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(screen.getByText('900 µs')).toBeInTheDocument();

    // 2. Write confirms 900
    await act(async () => {
      confirmWrite?.(900);
    });

    // 3. Parent updates prop to 900
    rerender(
      <TimingMarginControl
        value={900}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={async (v) => v}
      />,
    );
    expect(screen.getByText('900 µs')).toBeInTheDocument();

    // 4. External settings restore prop back to 800
    rerender(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={async (v) => v}
      />,
    );

    // The display must be authoritative 800 µs, NOT stale 900 µs!
    expect(screen.getByText('800 µs')).toBeInTheDocument();
  });

  it('safely handles unmount during in-flight async update', async () => {
    let resolveWrite: ((val: number) => void) | null = null;
    const { unmount } = render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={() =>
          new Promise((resolve) => {
            resolveWrite = resolve;
          })
        }
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    unmount();

    // Resolving after unmount should not throw
    await act(async () => {
      resolveWrite?.(900);
    });
  });

  it('reverts optimistic value when onChange resolves to null', async () => {
    let resolveWrite: ((val: number | null) => void) | null = null;
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={() =>
          new Promise((resolve) => {
            resolveWrite = resolve;
          })
        }
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(screen.getByText('900 µs')).toBeInTheDocument();

    await act(async () => {
      resolveWrite?.(null);
    });

    expect(screen.getByText('800 µs')).toBeInTheDocument();
  });

  it('reverts optimistic value when onChange rejects', async () => {
    let rejectWrite: ((err: Error) => void) | null = null;
    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={() =>
          new Promise((_, reject) => {
            rejectWrite = reject;
          })
        }
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(screen.getByText('900 µs')).toBeInTheDocument();

    await act(async () => {
      rejectWrite?.(new Error('Backend error'));
    });

    expect(screen.getByText('800 µs')).toBeInTheDocument();
  });

  it('handles out-of-order promise completion where older completes after newer', async () => {
    let resolveOlder: ((val: number) => void) | null = null;
    let resolveNewer: ((val: number) => void) | null = null;
    let callCount = 0;

    render(
      <TimingMarginControl
        value={800}
        options={options}
        recommendation={recommendation}
        density="compact"
        onChange={() => {
          callCount++;
          if (callCount === 1) {
            return new Promise((res) => {
              resolveOlder = res;
            });
          }
          return new Promise((res) => {
            resolveNewer = res;
          });
        }}
      />,
    );

    // Click 1 (800 -> 900)
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(screen.getByText('900 µs')).toBeInTheDocument();

    // Click 2 (900 -> 1000)
    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
    expect(screen.getByText('1000 µs')).toBeInTheDocument();

    // Newer resolves first with 1000
    await act(async () => {
      resolveNewer?.(1000);
    });
    expect(screen.getByText('1000 µs')).toBeInTheDocument();

    // Older resolves later with 900 (should NOT overwrite 1000)
    await act(async () => {
      resolveOlder?.(900);
    });
    expect(screen.getByText('1000 µs')).toBeInTheDocument();
  });

  describe('mixed two-request completion matrix', () => {
    it('newer null first, older success second -> renders older confirmed 900', async () => {
      let resolveOlder!: (val: number | null) => void;
      let resolveNewer!: (val: number | null) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveOlder = res;
              else resolveNewer = res;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      // Newer resolves with null (fails)
      await act(async () => {
        resolveNewer(null);
      });
      // Should show the older pending request (900) while it is still in flight
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      // Older resolves with 900 (success)
      await act(async () => {
        resolveOlder(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();
    });

    it('older success first, newer null second -> renders older confirmed 900', async () => {
      let resolveOlder!: (val: number | null) => void;
      let resolveNewer!: (val: number | null) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveOlder = res;
              else resolveNewer = res;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      // Older resolves with 900
      await act(async () => {
        resolveOlder(900);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument(); // newer 1000 still in flight

      // Newer resolves with null (fails)
      await act(async () => {
        resolveNewer(null);
      });
      // Must settle to the confirmed 900, not 1000 or 800!
      expect(screen.getByText('900 µs')).toBeInTheDocument();
    });

    it('older null first, newer success second -> renders newer confirmed 1000', async () => {
      let resolveOlder!: (val: number | null) => void;
      let resolveNewer!: (val: number | null) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveOlder = res;
              else resolveNewer = res;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      // Older fails first
      await act(async () => {
        resolveOlder(null);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      // Newer succeeds
      await act(async () => {
        resolveNewer(1000);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();
    });

    it('newer success first, older null second -> renders newer confirmed 1000', async () => {
      let resolveOlder!: (val: number | null) => void;
      let resolveNewer!: (val: number | null) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveOlder = res;
              else resolveNewer = res;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      // Newer succeeds first
      await act(async () => {
        resolveNewer(1000);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      // Older fails second
      await act(async () => {
        resolveOlder(null);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();
    });

    it('both null -> settles to authoritative 800', async () => {
      let resolveOlder!: (val: number | null) => void;
      let resolveNewer!: (val: number | null) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveOlder = res;
              else resolveNewer = res;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      await act(async () => {
        resolveOlder(null);
      });
      await act(async () => {
        resolveNewer(null);
      });
      expect(screen.getByText('800 µs')).toBeInTheDocument();
    });

    it('older reject, newer success -> renders newer confirmed 1000', async () => {
      let rejectOlder!: (err: Error) => void;
      let resolveNewer!: (val: number) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            if (callCount === 1) {
              return new Promise((_, rej) => {
                rejectOlder = rej;
              });
            }
            return new Promise((res) => {
              resolveNewer = res;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      await act(async () => {
        rejectOlder(new Error('Network error'));
      });
      await act(async () => {
        resolveNewer(1000);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();
    });

    it('older success, newer reject -> renders older confirmed 900', async () => {
      let resolveOlder!: (val: number) => void;
      let rejectNewer!: (err: Error) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            if (callCount === 1) {
              return new Promise((res) => {
                resolveOlder = res;
              });
            }
            return new Promise((_, rej) => {
              rejectNewer = rej;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      await act(async () => {
        resolveOlder(900);
      });
      await act(async () => {
        rejectNewer(new Error('Network error'));
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();
    });

    it('both reject -> settles to authoritative 800', async () => {
      let rejectOlder!: (err: Error) => void;
      let rejectNewer!: (err: Error) => void;
      let callCount = 0;

      render(
        <TimingMarginControl
          value={800}
          options={options}
          recommendation={recommendation}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((_, rej) => {
              if (callCount === 1) rejectOlder = rej;
              else rejectNewer = rej;
            });
          }}
        />,
      );

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000

      await act(async () => {
        rejectOlder(new Error('error 1'));
      });
      await act(async () => {
        rejectNewer(new Error('error 2'));
      });
      expect(screen.getByText('800 µs')).toBeInTheDocument();
    });
  });
});
