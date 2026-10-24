import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { isTauriRuntimeMock, showWindow, acknowledgeStartupReveal } = vi.hoisted(() => ({
  isTauriRuntimeMock: vi.fn(() => true),
  showWindow: vi.fn(async () => {}),
  acknowledgeStartupReveal: vi.fn(async () => undefined),
}));

vi.mock('./runtime', () => ({ isTauriRuntime: isTauriRuntimeMock }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: acknowledgeStartupReveal }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ show: showWindow }),
}));

import {
  createStartupWindowRevealCoordinator,
  revealStartupWindow,
  scheduleStartupWindowRevealFallback,
} from './startupWindowReveal';
import { STARTUP_WINDOW_REVEALED_COMMAND } from '../bridge/generated/command_names';

async function flushPromiseJobs(): Promise<void> {
  for (let index = 0; index < 6; index += 1) await Promise.resolve();
}

describe('startup window reveal coordinator', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
    acknowledgeStartupReveal.mockReset().mockResolvedValue(undefined);
    Reflect.deleteProperty(window, '__SKY_STARTUP_WINDOW_REVEAL_COORDINATOR__');
  });

  beforeEach(() => {
    vi.useFakeTimers();
    isTauriRuntimeMock.mockReturnValue(true);
    showWindow.mockReset().mockResolvedValue(undefined);
  });

  it('coalesces StrictMode-ready and fatal reveal requests into one show', async () => {
    const onShown = vi.fn();
    const onFailure = vi.fn();
    const coordinator = createStartupWindowRevealCoordinator(
      showWindow,
      onShown,
      onFailure,
      vi.fn(),
      100,
    );
    coordinator.scheduleFallback();
    coordinator.scheduleFallback();
    coordinator.reveal('bootstrap-ready');
    coordinator.reveal('bootstrap-failed');
    await Promise.resolve();

    expect(showWindow).toHaveBeenCalledTimes(1);
    expect(onShown).toHaveBeenCalledTimes(1);
    expect(onFailure).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('reveals once after the slow-bootstrap timeout and cancels later triggers', async () => {
    const onSlowBootstrap = vi.fn();
    const coordinator = createStartupWindowRevealCoordinator(
      showWindow,
      vi.fn(),
      vi.fn(),
      onSlowBootstrap,
      100,
    );
    coordinator.scheduleFallback();
    coordinator.scheduleFallback();
    await vi.advanceTimersByTimeAsync(100);
    coordinator.reveal('bootstrap-ready');

    expect(onSlowBootstrap).toHaveBeenCalledTimes(1);
    expect(showWindow).toHaveBeenCalledTimes(1);
  });

  it('reports an actionable error when the native show request fails', async () => {
    const error = new Error('window permission denied');
    showWindow.mockRejectedValue(error);
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    revealStartupWindow('bootstrap-ready');
    await flushPromiseJobs();

    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining('native startup fallback may retry'),
      error,
    );
  });

  it('preserves the coordinator across HMR module replacement', async () => {
    revealStartupWindow('bootstrap-ready');
    await Promise.resolve();
    vi.resetModules();
    const refreshed = await import('./startupWindowReveal');
    refreshed.revealStartupWindow('bootstrap-ready');
    await Promise.resolve();

    expect(showWindow).toHaveBeenCalledTimes(1);
  });

  it('acknowledges the native startup reveal after the window show succeeds', async () => {
    revealStartupWindow('bootstrap-ready');
    await Promise.resolve();
    await Promise.resolve();

    expect(showWindow).toHaveBeenCalledTimes(1);
    expect(acknowledgeStartupReveal).toHaveBeenCalledWith(STARTUP_WINDOW_REVEALED_COMMAND);
  });

  it('keeps native fallback eligible when the completion acknowledgment fails', async () => {
    const error = new Error('window is still hidden');
    acknowledgeStartupReveal.mockRejectedValue(error);
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});

    revealStartupWindow('bootstrap-ready');
    await flushPromiseJobs();

    expect(showWindow).toHaveBeenCalledTimes(1);
    expect(acknowledgeStartupReveal).toHaveBeenCalledWith(STARTUP_WINDOW_REVEALED_COMMAND);
    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining('native startup fallback may retry'),
      error,
    );
  });

  it('does not call native window APIs or create timers in browser environments', () => {
    isTauriRuntimeMock.mockReturnValue(false);

    scheduleStartupWindowRevealFallback();
    revealStartupWindow('bootstrap-ready');

    expect(showWindow).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });
});
