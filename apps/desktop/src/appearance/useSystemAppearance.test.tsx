import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import * as runtime from '../platform/runtime';
import * as systemAppearance from '../platform/systemAppearance';
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
});
