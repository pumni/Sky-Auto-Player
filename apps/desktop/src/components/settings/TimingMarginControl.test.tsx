import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { PlaybackOptionSets } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { TimingMarginControl } from './TimingMarginControl';

const options: PlaybackOptionSets = {
  hold_frames: [1, 1.25, 1.5],
  tempo_scales: [0.5, 1, 2],
  fps: [30, 60, 90, 120],
  timing_margin_min_us: 0,
  timing_margin_max_us: 3_000,
  timing_margin_step_us: 100,
};

describe('TimingMarginControl', () => {
  afterEach(() => cleanup());

  it('shows the manual value and steps by the backend increment', () => {
    const changes: number[] = [];
    render(
      <TimingMarginControl
        value={800}
        options={options}
        density="full"
        onChange={async (value) => {
          changes.push(value);
          return value;
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));

    expect(changes).toEqual([900]);
    expect(screen.getByText('900 µs')).toBeInTheDocument();
    expect(screen.getByText('Applies to the minimum key hold')).toBeInTheDocument();
  });

  it('keeps the compact variant focused on the manual value', () => {
    render(
      <TimingMarginControl
        value={800}
        options={options}
        density="compact"
        onChange={async (value) => value}
      />,
    );

    expect(screen.getByText('800 µs')).toBeInTheDocument();
    expect(screen.queryByText(/Source:/)).not.toBeInTheDocument();
    expect(screen.queryByText('Applies to the minimum key hold')).not.toBeInTheDocument();
  });

  it('disables steps at the configured endpoints', () => {
    const { rerender } = render(
      <TimingMarginControl value={0} options={options} density="full" onChange={async () => 0} />,
    );

    expect(screen.getByRole('button', { name: 'Decrease Timing Margin' })).toBeDisabled();
    expect(screen.getByRole('button', { name: 'Increase Timing Margin' })).toBeEnabled();

    rerender(
      <TimingMarginControl
        value={3_000}
        options={options}
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
        density="full"
        onChange={async () => {
          throw new Error('settings IPC failed');
        }}
      />,
    );

    fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));

    await waitFor(() => expect(screen.getByText('800 µs')).toBeInTheDocument());
  });

  it('does not resurrect old optimistic override when authoritative prop changes away and back', async () => {
    let confirmWrite: ((value: number) => void) | null = null;
    const { rerender } = render(
      <TimingMarginControl
        value={800}
        options={options}
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

    it('case 8: older success -> newer success -> renders newer 1000', async () => {
      let resolveOlder!: (val: number) => void;
      let resolveNewer!: (val: number) => void;
      let callCount = 0;
      render(
        <TimingMarginControl
          value={800}
          options={options}
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
        resolveOlder(900);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();
      await act(async () => {
        resolveNewer(1000);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();
    });

    it('case 15: both succeed with different authoritative returned values (older then newer, and newer then older)', async () => {
      // Test A: older 950 first, newer 1050 second
      let resolveOlderA!: (val: number) => void;
      let resolveNewerA!: (val: number) => void;
      let callCountA = 0;
      const { unmount } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() => {
            callCountA++;
            return new Promise((res) => {
              if (callCountA === 1) resolveOlderA = res;
              else resolveNewerA = res;
            });
          }}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // req 1
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // req 2
      await act(async () => {
        resolveOlderA(950);
      });
      await act(async () => {
        resolveNewerA(1050);
      });
      expect(screen.getByText('1050 µs')).toBeInTheDocument();
      unmount();

      // Test B: newer 1050 first, older 950 second (out-of-order)
      let resolveOlderB!: (val: number) => void;
      let resolveNewerB!: (val: number) => void;
      let callCountB = 0;
      render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() => {
            callCountB++;
            return new Promise((res) => {
              if (callCountB === 1) resolveOlderB = res;
              else resolveNewerB = res;
            });
          }}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      await act(async () => {
        resolveNewerB(1050);
      });
      expect(screen.getByText('1050 µs')).toBeInTheDocument();
      await act(async () => {
        resolveOlderB(950);
      });
      expect(screen.getByText('1050 µs')).toBeInTheDocument();
    });
  });

  describe('authoritative prop synchronization', () => {
    it('case 16: initial prop 800 -> confirmation 900 -> prop 900', async () => {
      let resolveWrite!: (val: number) => void;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      await act(async () => {
        resolveWrite(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      rerender(
        <TimingMarginControl
          value={900}
          options={options}
          density="compact"
          onChange={async (v) => v}
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();
    });

    it('case 17: prop 800 -> confirmation 900 -> prop 900 -> prop 800', async () => {
      let resolveWrite!: (val: number) => void;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      await act(async () => {
        resolveWrite(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      rerender(
        <TimingMarginControl
          value={900}
          options={options}
          density="compact"
          onChange={async (v) => v}
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      rerender(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={async (v) => v}
        />,
      );
      expect(screen.getByText('800 µs')).toBeInTheDocument();
    });

    it('case 18: prop changes while one mutation is pending (preserves pending intent, settles correctly on fail or success)', async () => {
      // 18a: pending request fails -> settles to new prop
      let resolveWriteA!: (val: number | null) => void;
      const { rerender, unmount } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWriteA = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      rerender(
        <TimingMarginControl
          value={1200}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWriteA = res;
            })
          }
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      await act(async () => {
        resolveWriteA(null);
      });
      expect(screen.getByText('1200 µs')).toBeInTheDocument();
      unmount();

      // 18b: pending request succeeds -> settles to confirmed value
      let resolveWriteB!: (val: number | null) => void;
      const viewB = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWriteB = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      viewB.rerender(
        <TimingMarginControl
          value={1200}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWriteB = res;
            })
          }
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();
      await act(async () => {
        resolveWriteB(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();
    });

    it('case 19: prop changes while two mutations are pending -> retains latest pending, settles to prop on both fail', async () => {
      let resolveFirst!: (val: number | null) => void;
      let resolveSecond!: (val: number | null) => void;
      let callCount = 0;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveFirst = res;
              else resolveSecond = res;
            });
          }}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={1200}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveFirst = res;
              else resolveSecond = res;
            });
          }}
        />,
      );
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      await act(async () => {
        resolveFirst(null);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();
      await act(async () => {
        resolveSecond(null);
      });
      expect(screen.getByText('1200 µs')).toBeInTheDocument();
    });

    it('case 20: prop changes while an older confirmation is retained -> clears stale confirmation', async () => {
      let resolveWrite!: (val: number | null) => void;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      await act(async () => {
        resolveWrite(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={1300}
          options={options}
          density="compact"
          onChange={async (v) => v}
        />,
      );
      expect(screen.getByText('1300 µs')).toBeInTheDocument();
    });

    it('case 21: prop changes to 1100 while a 1000 request is pending -> 1000 fails -> UI must display 1100', async () => {
      let resolveFirst!: (val: number | null) => void;
      let resolveSecond!: (val: number | null) => void;
      let callCount = 0;

      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveFirst = res;
              else resolveSecond = res;
            });
          }}
        />,
      );

      // 1. User requests 900
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      // 2. Request 1 succeeds with 900
      await act(async () => {
        resolveFirst(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      // 3. User requests 1000 (pending)
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      // 4. Parent externally updates authoritative prop to 1100
      rerender(
        <TimingMarginControl
          value={1100}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              resolveSecond = res;
            });
          }}
        />,
      );
      // While request 2 is pending, user intent 1000 is still visible
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      // 5. Request 2 resolves to null (fails)
      await act(async () => {
        resolveSecond(null);
      });

      // 6. Expected: 1100 µs. With bug: displays stale 900 µs!
      expect(screen.getByText('1100 µs')).toBeInTheDocument();
    });

    it('case 22: same as case 21, but pending request rejects instead of resolving null -> UI displays 1100', async () => {
      let resolveFirst!: (val: number | null) => void;
      let rejectSecond!: (err: Error) => void;
      let callCount = 0;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            if (callCount === 1) {
              return new Promise((res) => {
                resolveFirst = res;
              });
            }
            return new Promise((_, rej) => {
              rejectSecond = rej;
            });
          }}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      await act(async () => {
        resolveFirst(900);
      });
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 1000
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={1100}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((_, rej) => {
              rejectSecond = rej;
            });
          }}
        />,
      );
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      await act(async () => {
        rejectSecond(new Error('Backend error'));
      });
      expect(screen.getByText('1100 µs')).toBeInTheDocument();
    });

    it('case 23: prop changes while pending request succeeds with a different returned value -> displays returned value', async () => {
      let resolveWrite!: (val: number | null) => void;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // requests 900
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={1100}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      await act(async () => {
        resolveWrite(950);
      });
      expect(screen.getByText('950 µs')).toBeInTheDocument();
    });

    it('case 24: parent prop A -> B -> A during asynchronous operations -> renders authoritative A', async () => {
      let resolveWrite!: (val: number | null) => void;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' })); // 900
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={1000}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() =>
            new Promise((res) => {
              resolveWrite = res;
            })
          }
        />,
      );
      expect(screen.getByText('900 µs')).toBeInTheDocument();

      await act(async () => {
        resolveWrite(null);
      });
      expect(screen.getByText('800 µs')).toBeInTheDocument();
    });

    it('case 25: no stale optimistic or confirmed state survives after all operations settle', async () => {
      let resolveFirst!: (val: number | null) => void;
      let resolveSecond!: (val: number | null) => void;
      let callCount = 0;
      const { rerender } = render(
        <TimingMarginControl
          value={800}
          options={options}
          density="compact"
          onChange={() => {
            callCount++;
            return new Promise((res) => {
              if (callCount === 1) resolveFirst = res;
              else resolveSecond = res;
            });
          }}
        />,
      );
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      await act(async () => {
        resolveFirst(900);
      });
      await act(async () => {
        resolveSecond(1000);
      });
      expect(screen.getByText('1000 µs')).toBeInTheDocument();

      rerender(
        <TimingMarginControl
          value={1500}
          options={options}
          density="compact"
          onChange={async (v) => v}
        />,
      );
      expect(screen.getByText('1500 µs')).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Decrease Timing Margin' })).toBeEnabled();
      expect(screen.getByRole('button', { name: 'Increase Timing Margin' })).toBeEnabled();
    });
  });

  describe('real store serialization integration', () => {
    it('serializes sequential timing margin writes through createSettingsSlice', async () => {
      const bridge = createMockBridge();
      const store = createDesktopStore(bridge);
      await act(async () => store.getState().initialize());
      expect(store.getState().settings?.playback_defaults.timing_margin_us).toBe(500);

      function ConnectedControl() {
        const timingMargin = store((s) => s.settings?.playback_defaults.timing_margin_us ?? 500);
        const patchSettings = store((s) => s.patchSettings);
        return (
          <TimingMarginControl
            value={timingMargin}
            options={options}
            density="compact"
            onChange={(val) =>
              patchSettings({ playbackDefaults: { timingMarginUs: val } }).then(
                (res) => res?.playback_defaults.timing_margin_us ?? null,
              )
            }
          />
        );
      }

      render(<ConnectedControl />);
      expect(screen.getByText('500 µs')).toBeInTheDocument();

      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      fireEvent.click(screen.getByRole('button', { name: 'Increase Timing Margin' }));
      expect(screen.getByText('700 µs')).toBeInTheDocument();

      await waitFor(() => {
        expect(store.getState().settings?.playback_defaults.timing_margin_us).toBe(700);
      });
      expect(screen.getByText('700 µs')).toBeInTheDocument();
    });
  });
});
