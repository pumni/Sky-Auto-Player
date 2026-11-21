import { act, cleanup, render, renderHook, screen } from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import * as runtime from '../platform/runtime';
import * as systemAppearance from '../platform/systemAppearance';
import type { ColorMode } from './resolveAppearance';
import { useSystemAppearance } from './useSystemAppearance';

describe('useSystemAppearance', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it('shares a single active subscription across multiple components and unregisters on teardown', () => {
    vi.spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
    let subscribeCount = 0;
    const unsubscribeFn = vi.fn();

    vi.spyOn(systemAppearance, 'subscribeSystemAppearance').mockImplementation(() => {
      subscribeCount += 1;
      return {
        ready: new Promise(() => {}),
        unsubscribe: unsubscribeFn,
      };
    });

    const hook1 = renderHook(() => useSystemAppearance());
    expect(subscribeCount).toBe(1);

    const hook2 = renderHook(() => useSystemAppearance());
    expect(subscribeCount).toBe(1);

    hook1.unmount();
    expect(unsubscribeFn).not.toHaveBeenCalled();

    hook2.unmount();
    expect(unsubscribeFn).toHaveBeenCalledOnce();
  });

  it('drops stale ready resolution from previous generation when remounted', async () => {
    vi.spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
    let gen1ResolveReady!: () => void;
    let gen2ResolveReady!: () => void;
    let subCalls = 0;

    vi.spyOn(systemAppearance, 'subscribeSystemAppearance').mockImplementation(() => {
      subCalls += 1;
      const currentCall = subCalls;
      return {
        ready: new Promise<void>((resolve) => {
          if (currentCall === 1) gen1ResolveReady = resolve;
          else gen2ResolveReady = resolve;
        }),
        unsubscribe: vi.fn(),
      };
    });

    // Mount gen 1
    const hook1 = renderHook(() => useSystemAppearance());
    expect(hook1.result.current.ready).toBe(false);

    // Unmount gen 1 -> all subscribers gone, generation increments
    hook1.unmount();

    // Mount gen 2
    const hook2 = renderHook(() => useSystemAppearance());
    expect(hook2.result.current.ready).toBe(false);

    // Now resolve gen 1's ready promise
    await act(async () => {
      gen1ResolveReady();
    });

    // hook2 must still be false because gen 1's ready was dropped!
    expect(hook2.result.current.ready).toBe(false);

    // Now resolve gen 2's ready promise
    await act(async () => {
      gen2ResolveReady();
    });

    expect(hook2.result.current.ready).toBe(true);
    hook2.unmount();
  });

  it('handles ready rejection safely without throwing unhandled errors', async () => {
    vi.spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
    let rejectReady!: (err: any) => void;

    vi.spyOn(systemAppearance, 'subscribeSystemAppearance').mockImplementation(() => ({
      ready: new Promise<void>((_, reject) => {
        rejectReady = reject;
      }),
      unsubscribe: vi.fn(),
    }));

    const hook = renderHook(() => useSystemAppearance());
    expect(hook.result.current.ready).toBe(false);

    await act(async () => {
      rejectReady(new Error('native theme read failed'));
    });

    // Even on error, ready becomes true so UI does not hang forever
    expect(hook.result.current.ready).toBe(true);
    hook.unmount();
  });

  it('handles setup -> cleanup -> setup lifecycle without subscription leaks and receives theme updates', async () => {
    vi.spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
    let subscribeCount = 0;
    const unsubscribeFn = vi.fn();
    let themeCallback: ((mode: ColorMode) => void) | undefined;
    let resolveReady!: () => void;

    vi.spyOn(systemAppearance, 'subscribeSystemAppearance').mockImplementation((cb) => {
      subscribeCount += 1;
      themeCallback = cb;
      return {
        ready: new Promise<void>((resolve) => {
          resolveReady = resolve;
        }),
        unsubscribe: unsubscribeFn,
      };
    });

    // 1. Initial setup
    const hook1 = renderHook(() => useSystemAppearance());
    expect(subscribeCount).toBe(1);
    expect(hook1.result.current.ready).toBe(false);

    // 2. Cleanup (simulating StrictMode unmount)
    hook1.unmount();
    expect(unsubscribeFn).toHaveBeenCalledTimes(1);

    // 3. Remount (setup again)
    const hook2 = renderHook(() => useSystemAppearance());
    expect(subscribeCount).toBe(2);
    expect(hook2.result.current.ready).toBe(false);

    // Resolve ready for active subscription
    await act(async () => {
      resolveReady();
    });
    expect(hook2.result.current.ready).toBe(true);

    // Native theme changes update subscribers
    await act(async () => {
      themeCallback?.('light');
    });
    expect(hook2.result.current.mode).toBe('light');

    // Final unmount cleans up without leaks
    hook2.unmount();
    expect(unsubscribeFn).toHaveBeenCalledTimes(2);
  });

  it('operates correctly inside React StrictMode wrapper', async () => {
    vi.spyOn(runtime, 'isTauriRuntime').mockReturnValue(true);
    let subscribeCount = 0;
    const unsubscribeFn = vi.fn();
    let resolveReady!: () => void;

    vi.spyOn(systemAppearance, 'subscribeSystemAppearance').mockImplementation(() => {
      subscribeCount += 1;
      return {
        ready: new Promise<void>((resolve) => {
          resolveReady = resolve;
        }),
        unsubscribe: unsubscribeFn,
      };
    });

    function Consumer() {
      const { ready, mode } = useSystemAppearance();
      return <div data-testid="mode">{ready ? mode : 'loading'}</div>;
    }

    const { unmount } = render(
      <StrictMode>
        <Consumer />
      </StrictMode>,
    );

    expect(subscribeCount).toBeGreaterThanOrEqual(1);

    await act(async () => {
      resolveReady();
    });
    expect(screen.getByTestId('mode')).not.toHaveTextContent('loading');

    unmount();
    expect(unsubscribeFn).toHaveBeenCalled();
  });
});
