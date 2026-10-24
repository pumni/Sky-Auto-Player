import {
  act,
  cleanup,
  createEvent,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { StrictMode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { CalibrationStartAck, DesktopBridge, UiEvent } from '../bridge/DesktopBridge';
import { createMockBridge } from '../bridge/mockBridge';
import { createDesktopStore, type DesktopStoreHook } from '../state/store';
import { App } from './App';
import { usePackagedSmokeTest } from './usePackagedSmokeTest';
import { STARTUP_WINDOW_REVEAL_FALLBACK_MS } from '../platform/startupWindowReveal';

const { isTauriRuntimeMock, nativeWindow, acknowledgeStartupReveal } = vi.hoisted(() => {
  const nativeWindow = {
    show: vi.fn(async () => {}),
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {}),
    close: vi.fn(async () => {}),
    isMaximized: vi.fn(async () => false),
    onResized: vi.fn(async () => () => {}),
  };
  return {
    isTauriRuntimeMock: vi.fn(() => false),
    nativeWindow,
    acknowledgeStartupReveal: vi.fn(async () => undefined),
  };
});

vi.mock('../platform/runtime', () => ({ isTauriRuntime: isTauriRuntimeMock }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: acknowledgeStartupReveal }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => nativeWindow }));

function PackagedSmokeHarness({
  bridge,
  useStore,
}: {
  bridge: DesktopBridge;
  useStore: DesktopStoreHook;
}) {
  usePackagedSmokeTest({ bridge, useStore });
  return <div className="app-shell" />;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function createCalibrationFinished(
  operationId: string,
  outcome: 'succeeded' | 'failed' | 'cancelled',
): UiEvent {
  return {
    v: 1,
    name: 'calibration.finished',
    payload: {
      operation_id: operationId,
      outcome,
      status: outcome === 'succeeded' ? 'ready' : outcome,
      recommended_timing_margin_us: outcome === 'succeeded' ? 400 : null,
      recommendation_qualified: outcome === 'succeeded',
      sample_count: outcome === 'succeeded' ? 24 : 0,
      source: outcome === 'succeeded' ? 'qualified_calibration' : 'unavailable',
      message: outcome === 'succeeded' ? 'Calibration completed.' : 'Calibration failed.',
    },
  };
}

function installStrictSingletonEvents(bridge: DesktopBridge) {
  let listener: ((event: UiEvent) => void) | null = null;
  let subscriptionCount = 0;
  bridge.subscribeUiEvents = vi.fn(async (nextListener: (event: UiEvent) => void) => {
    subscriptionCount += 1;
    if (subscriptionCount > 1) {
      throw new Error('duplicate native UI event subscription');
    }
    listener = nextListener;
    return () => {
      listener = null;
    };
  });
  return {
    get subscriptionCount() {
      return subscriptionCount;
    },
    emit(event: UiEvent) {
      if (!listener) throw new Error('native UI event subscriber is not installed');
      listener(event);
    },
  };
}

async function createPackagedSmokeFixture() {
  const bridge = createMockBridge();
  const nativeEvents = installStrictSingletonEvents(bridge);
  const useStore = createDesktopStore(bridge);
  const shutdown = vi.spyOn(bridge, 'shutdown');
  await act(async () => {
    await useStore.getState().initialize();
  });

  const originalSubscribe = useStore.subscribe.bind(useStore);
  const observerUnsubscribe = vi.fn();
  vi.spyOn(useStore, 'subscribe').mockImplementation((listener) => {
    const unsubscribe = originalSubscribe(listener);
    return () => {
      observerUnsubscribe();
      unsubscribe();
    };
  });

  render(<PackagedSmokeHarness bridge={bridge} useStore={useStore} />);
  return { bridge, nativeEvents, useStore, shutdown, observerUnsubscribe };
}

describe('desktop application shell', () => {
  afterEach(() => {
    delete (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__;
    Reflect.deleteProperty(window, '__SKY_STARTUP_WINDOW_REVEAL_COORDINATOR__');
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
    isTauriRuntimeMock.mockReturnValue(false);
    nativeWindow.show.mockReset().mockResolvedValue(undefined);
    acknowledgeStartupReveal.mockReset().mockResolvedValue(undefined);
    nativeWindow.minimize.mockReset().mockResolvedValue(undefined);
    nativeWindow.toggleMaximize.mockReset().mockResolvedValue(undefined);
    nativeWindow.close.mockReset().mockResolvedValue(undefined);
    nativeWindow.isMaximized.mockReset().mockResolvedValue(false);
    nativeWindow.onResized.mockReset().mockResolvedValue(() => {});
    window.localStorage.clear();
  });

  it('renders the navigator, track browser, utility details, and settings', async () => {
    render(<App bridge={createMockBridge()} />);
    expect(screen.getByText('Opening your library…')).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole('row', { name: /Aurora Landing/ })).toBeInTheDocument(),
    );
    expect(screen.getByRole('navigation', { name: 'Library' })).toHaveTextContent('All Songs');
    expect(screen.getByRole('grid', { name: 'Songs' })).toBeInTheDocument();

    const firstSong = screen.getByRole('row', { name: /Aurora Landing/ });
    fireEvent.click(firstSong);
    await waitFor(() => expect(firstSong).toHaveAttribute('aria-selected', 'true'));
    expect(screen.queryByText('Medium timing risk')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open utility panel' }));
    await waitFor(() =>
      expect(screen.getByRole('region', { name: 'Utility: Song Details' })).toBeInTheDocument(),
    );
    expect(screen.getByText('Medium timing risk')).toBeInTheDocument();

    const settingsButton = screen.getByRole('button', { name: 'Open settings' });
    await act(async () => {
      fireEvent.click(settingsButton);
      await vi.dynamicImportSettled();
    });
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
    expect(screen.getByLabelText('Color palette')).toHaveValue('graphite');
    fireEvent.click(screen.getByRole('button', { name: 'About' }));
    expect(screen.getByRole('dialog', { name: 'Settings' })).toHaveTextContent('3.5.0-mock');
    expect(screen.getByRole('dialog', { name: 'Settings' })).toHaveTextContent('Native ABI');
    expect(screen.getByRole('dialog', { name: 'Settings' })).toContainElement(
      document.activeElement as HTMLElement,
    );
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Settings' }), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull());
    expect(document.activeElement).toBe(settingsButton);

    fireEvent.click(settingsButton);
    const reopenedSettings = await screen.findByRole('dialog', { name: 'Settings' });
    expect(reopenedSettings).toHaveTextContent('About Sky Auto Player');
    expect(screen.getByRole('button', { name: 'About' })).toHaveAttribute('aria-current', 'page');
    fireEvent.keyDown(reopenedSettings, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull());
    expect(document.activeElement).toBe(settingsButton);
  });

  it('supports keyboard navigation in the virtualized track table', async () => {
    render(<App bridge={createMockBridge()} />);
    const grid = await screen.findByRole('grid', { name: 'Songs' });
    grid.focus();
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Blue Bird/ })
          .some((row) => row.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
    fireEvent.keyDown(grid, { key: 'Home' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Aurora Landing/ })
          .some((row) => row.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
  });

  it('focuses the global search input from the keyboard shortcut', async () => {
    render(<App bridge={createMockBridge()} />);
    const search = await screen.findByRole('searchbox');

    fireEvent.keyDown(window, { key: '/' });

    expect(document.activeElement).toBe(search);
  });

  it('does not prevent slash input in an editable control', async () => {
    render(<App bridge={createMockBridge()} />);
    const search = await screen.findByRole('searchbox');
    search.focus();

    const event = createEvent.keyDown(search, { key: '/' });
    search.dispatchEvent(event);

    expect(event.defaultPrevented).toBe(false);
    expect(document.activeElement).toBe(search);
  });

  it('filters library rows from global search input', async () => {
    render(<App bridge={createMockBridge()} />);
    const search = await screen.findByRole('searchbox');

    fireEvent.change(search, { target: { value: 'Moonlit' } });

    await waitFor(() => {
      expect(screen.getByRole('row', { name: /Moonlit Village/ })).toBeInTheDocument();
      expect(screen.queryByRole('row', { name: /Aurora Landing/ })).toBeNull();
    });
  });

  it('loads and selects a keyboard destination on an unloaded virtualized page', async () => {
    render(<App bridge={createMockBridge()} />);
    const grid = await screen.findByRole('grid', { name: 'Songs' });
    grid.focus();
    fireEvent.keyDown(grid, { key: 'End' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Song 500/ })
          .some((row) => row.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
  });

  it('keeps the window available when native startup fails', async () => {
    const bridge = createMockBridge();
    bridge.subscribeUiEvents = async () => {
      throw new Error('Native application is unavailable');
    };
    render(<App bridge={bridge} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Native application is unavailable');
    expect(screen.queryByRole('button', { name: /Try again/i })).toBeNull();
  });

  it('applies the restored palette before revealing the native window', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    const originalBootstrap = bridge.bootstrap.bind(bridge);
    bridge.bootstrap = vi.fn(async () => {
      const bootstrap = await originalBootstrap();
      return {
        ...bootstrap,
        settings: { ...bootstrap.settings, palette: 'slate' as const },
      };
    });
    const palettesWhenShown: string[] = [];
    nativeWindow.show.mockImplementation(async () => {
      palettesWhenShown.push(document.documentElement.dataset.palette ?? '');
    });

    render(<App bridge={bridge} />);
    await screen.findByRole('grid', { name: 'Songs' });

    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
    expect(acknowledgeStartupReveal).toHaveBeenCalledTimes(1);
    expect(palettesWhenShown).toEqual(['slate']);
  });

  it('reveals the committed fatal bootstrap screen', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    bridge.subscribeUiEvents = vi.fn(async () => {
      throw new Error('Native application is unavailable');
    });
    const fatalUiVisibleWhenShown: boolean[] = [];
    nativeWindow.show.mockImplementation(async () => {
      fatalUiVisibleWhenShown.push(Boolean(document.querySelector('.fatal-screen')));
    });

    render(<App bridge={bridge} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Native application is unavailable');

    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
    expect(fatalUiVisibleWhenShown).toEqual([true]);
  });

  it('reveals the branded startup screen after the bounded bootstrap delay', async () => {
    vi.useFakeTimers();
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    const pendingBootstrap = deferred<Awaited<ReturnType<DesktopBridge['bootstrap']>>>();
    bridge.bootstrap = vi.fn(() => pendingBootstrap.promise);

    render(<App bridge={bridge} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(screen.getByText('Opening your library…')).toBeInTheDocument();
    expect(bridge.bootstrap).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(STARTUP_WINDOW_REVEAL_FALLBACK_MS);
      await Promise.resolve();
    });
    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
    expect(screen.getByText('Opening your library…')).toBeInTheDocument();

    const completedBootstrap = await createMockBridge().bootstrap();
    await act(async () => {
      pendingBootstrap.resolve(completedBootstrap);
      await pendingBootstrap.promise;
    });
    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
  });

  it('keeps StrictMode startup to one native subscription and one window reveal', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    const nativeEvents = installStrictSingletonEvents(bridge);
    const { rerender } = render(
      <StrictMode>
        <App bridge={bridge} />
      </StrictMode>,
    );
    await screen.findByRole('grid', { name: 'Songs' });
    rerender(
      <StrictMode>
        <App bridge={bridge} />
      </StrictMode>,
    );

    expect(nativeEvents.subscriptionCount).toBe(1);
    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
  });

  it.each(['before', 'after'] as const)(
    'observes calibration completion through the store when it arrives %s the start ACK',
    async (timing) => {
      const fixture = await createPackagedSmokeFixture();
      const operationId = `early-late-${timing}`;
      const startCalled = deferred<void>();
      const startAck = deferred<CalibrationStartAck>();
      fixture.bridge.startCalibration = vi.fn(() => {
        startCalled.resolve();
        if (timing === 'before') {
          fixture.nativeEvents.emit(createCalibrationFinished(operationId, 'succeeded'));
        }
        return startAck.promise;
      });

      await act(async () => {
        window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
        await startCalled.promise;
      });
      expect(fixture.nativeEvents.subscriptionCount).toBe(1);

      await act(async () => {
        startAck.resolve({ operation_id: operationId, state: 'running' });
        await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
      });
      if (timing === 'after') {
        await act(async () => {
          fixture.nativeEvents.emit(createCalibrationFinished(operationId, 'succeeded'));
        });
      }

      await waitFor(() => expect(fixture.shutdown).toHaveBeenCalledTimes(1));
      expect(fixture.shutdown).toHaveBeenCalledWith();
      expect(fixture.bridge.startCalibration).toHaveBeenCalledTimes(1);
      expect(fixture.useStore.getState().calibration.result?.operation_id).toBe(operationId);
      expect(fixture.useStore.getState().calibration.result?.outcome).toBe('succeeded');
      expect(fixture.observerUnsubscribe).toHaveBeenCalledTimes(1);
      expect(fixture.nativeEvents.subscriptionCount).toBe(1);
    },
  );

  it('fails closed on a failed calibration result and releases the store observer', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const fixture = await createPackagedSmokeFixture();
    const operationId = 'failed-calibration';
    const startCalled = deferred<void>();
    const startAck = deferred<CalibrationStartAck>();
    fixture.bridge.startCalibration = vi.fn(() => {
      startCalled.resolve();
      return startAck.promise;
    });

    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      await startCalled.promise;
    });
    await act(async () => {
      startAck.resolve({ operation_id: operationId, state: 'running' });
      await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
    });
    await act(async () => {
      fixture.nativeEvents.emit(createCalibrationFinished(operationId, 'failed'));
    });

    await waitFor(() => expect(fixture.shutdown).toHaveBeenCalledWith(true));
    expect(fixture.observerUnsubscribe).toHaveBeenCalledTimes(1);
    expect(fixture.nativeEvents.subscriptionCount).toBe(1);
  });

  it('fails closed when the terminal result has a different operation ID', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const fixture = await createPackagedSmokeFixture();
    const startCalled = deferred<void>();
    const startAck = deferred<CalibrationStartAck>();
    fixture.bridge.startCalibration = vi.fn(() => {
      startCalled.resolve();
      return startAck.promise;
    });

    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      await startCalled.promise;
    });
    await act(async () => {
      startAck.resolve({ operation_id: 'expected-operation', state: 'running' });
      await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
    });
    await act(async () => {
      fixture.nativeEvents.emit(createCalibrationFinished('unrelated-operation', 'succeeded'));
    });

    await waitFor(() => expect(fixture.shutdown).toHaveBeenCalledWith(true));
    expect(fixture.observerUnsubscribe).toHaveBeenCalledTimes(1);
    expect(fixture.nativeEvents.subscriptionCount).toBe(1);
  });

  it('fails closed after the 10-second missing-terminal timeout and clears the timer', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {});
    const fixture = await createPackagedSmokeFixture();
    const startCalled = deferred<void>();
    fixture.bridge.startCalibration = vi.fn(async () => {
      startCalled.resolve();
      return { operation_id: 'missing-terminal', state: 'running' as const };
    });
    vi.useFakeTimers();
    const clearTimeoutSpy = vi.spyOn(window, 'clearTimeout');

    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      await startCalled.promise;
      await Promise.resolve();
      await Promise.resolve();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });

    expect(fixture.shutdown).toHaveBeenCalledWith(true);
    expect(fixture.observerUnsubscribe).toHaveBeenCalledTimes(1);
    expect(clearTimeoutSpy).toHaveBeenCalled();
    expect(fixture.nativeEvents.subscriptionCount).toBe(1);
  });

  it('keeps the smoke one-shot when the host repeats its trigger', async () => {
    const fixture = await createPackagedSmokeFixture();
    const operationId = 'one-shot-calibration';
    const startCalled = deferred<void>();
    const startAck = deferred<CalibrationStartAck>();
    fixture.bridge.startCalibration = vi.fn(() => {
      startCalled.resolve();
      return startAck.promise;
    });

    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      await startCalled.promise;
    });
    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    });
    expect(fixture.bridge.startCalibration).toHaveBeenCalledTimes(1);
    await act(async () => {
      startAck.resolve({ operation_id: operationId, state: 'running' });
      await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
      fixture.nativeEvents.emit(createCalibrationFinished(operationId, 'succeeded'));
    });

    await waitFor(() => expect(fixture.shutdown).toHaveBeenCalledTimes(1));
    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    });
    expect(fixture.bridge.startCalibration).toHaveBeenCalledTimes(1);
    expect(fixture.shutdown).toHaveBeenCalledWith();
    expect(fixture.observerUnsubscribe).toHaveBeenCalledTimes(1);
    expect(fixture.nativeEvents.subscriptionCount).toBe(1);
  });

  it('opens the docked utility diagnostics and closes with Escape', async () => {
    render(<App bridge={createMockBridge()} />);
    await screen.findByRole('row', { name: /Aurora Landing/ });

    const trigger = screen.getByRole('button', { name: 'Open utility panel' });
    fireEvent.click(trigger);
    const utility = await screen.findByRole('region', { name: 'Utility: Song Details' });
    fireEvent.click(screen.getByRole('tab', { name: 'Runtime' }));
    const diagnostics = await screen.findByRole('region', { name: 'Utility: Diagnostics' });
    expect(utility).toBeInTheDocument();
    expect(await screen.findByRole('tab', { name: 'Performance' })).toBeInTheDocument();
    fireEvent.keyDown(diagnostics, { key: 'Escape' });
    await waitFor(() =>
      expect(screen.queryByRole('region', { name: 'Utility: Diagnostics' })).toBeNull(),
    );
    expect(document.activeElement).toBe(trigger);

    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open calibration' }));
    expect(await screen.findByRole('dialog', { name: 'Timing calibration' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Start quick calibration' }));
    expect(await screen.findByText('Calibration complete')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: 'Timing calibration' })).toBeNull(),
    );
    expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Open settings' }));
  });
});
