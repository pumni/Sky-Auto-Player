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
import type { DesktopBridge, UiEvent } from '../bridge/DesktopBridge';
import { createMockBridge } from '../bridge/mockBridge';
import { STARTUP_WINDOW_REVEAL_FALLBACK_MS } from '../platform/startupWindowReveal';
import { createDesktopStore, type DesktopStoreHook } from '../state/store';
import { App } from './App';
import { usePackagedSmokeTest } from './usePackagedSmokeTest';

const { isTauriRuntimeMock, nativeWindow, acknowledgeStartupReveal, telemetryChannel } = vi.hoisted(
  () => {
    const themeListeners = new Set<(event: { payload: 'light' | 'dark' }) => void>();
    const subscribeTheme = (listener: (event: { payload: 'light' | 'dark' }) => void) => {
      themeListeners.add(listener);
      return () => themeListeners.delete(listener);
    };
    const nativeWindow = {
      show: vi.fn(async () => {}),
      minimize: vi.fn(async () => {}),
      toggleMaximize: vi.fn(async () => {}),
      close: vi.fn(async () => {}),
      isMaximized: vi.fn(async () => false),
      onResized: vi.fn(async () => () => {}),
      theme: vi.fn(async (): Promise<'light' | 'dark'> => 'dark'),
      onThemeChanged: vi.fn(async (listener: (event: { payload: 'light' | 'dark' }) => void) =>
        subscribeTheme(listener),
      ),
      emitTheme(payload: 'light' | 'dark') {
        for (const listener of themeListeners) listener({ payload });
      },
      clearThemeListeners() {
        themeListeners.clear();
      },
      activeThemeListenerCount() {
        return themeListeners.size;
      },
      subscribeTheme,
    };
    class TelemetryChannel {}
    return {
      isTauriRuntimeMock: vi.fn(() => false),
      nativeWindow,
      acknowledgeStartupReveal: vi.fn(async (..._args: unknown[]) => undefined),
      telemetryChannel: TelemetryChannel,
    };
  },
);

vi.mock('../platform/runtime', () => ({ isTauriRuntime: isTauriRuntimeMock }));
vi.mock('@tauri-apps/api/core', () => ({
  Channel: telemetryChannel,
  invoke: acknowledgeStartupReveal,
}));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => nativeWindow }));

let matchMediaDescriptor: PropertyDescriptor | undefined;
let matchMediaOverridden = false;

function PackagedSmokeHarness({
  bridge,
  useStore,
  authorized = true,
}: {
  bridge: DesktopBridge;
  useStore: DesktopStoreHook;
  authorized?: boolean;
}) {
  usePackagedSmokeTest({ bridge, useStore, authorized });
  return <div data-app-shell />;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
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

async function createPackagedSmokeFixture(authorized = true) {
  const bridge = createMockBridge();
  const nativeEvents = installStrictSingletonEvents(bridge);
  const useStore = createDesktopStore(bridge);
  const shutdown = vi.spyOn(bridge, 'shutdown');
  const patchSettings = vi.spyOn(bridge, 'patchSettings');
  const setDiagnosticsEnabled = vi.spyOn(bridge, 'setDiagnosticsEnabled');
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

  render(<PackagedSmokeHarness bridge={bridge} useStore={useStore} authorized={authorized} />);
  return {
    bridge,
    nativeEvents,
    useStore,
    shutdown,
    patchSettings,
    setDiagnosticsEnabled,
    observerUnsubscribe,
  };
}

async function renderOrdinaryAppWithSmokeSpies() {
  const bridge = createMockBridge();
  const patchSettings = vi.spyOn(bridge, 'patchSettings');
  const setDiagnosticsEnabled = vi.spyOn(bridge, 'setDiagnosticsEnabled');
  const shutdown = vi.spyOn(bridge, 'shutdown');
  render(<App bridge={bridge} />);
  await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
  return { bridge, patchSettings, setDiagnosticsEnabled, shutdown };
}

function authorizeGuiSmoke(bridge: DesktopBridge) {
  const originalBootstrap = bridge.bootstrap.bind(bridge);
  bridge.bootstrap = vi.fn(async () => {
    const bootstrap = await originalBootstrap();
    return { ...bootstrap, gui_smoke: true };
  });
}

describe('desktop application shell', () => {
  afterEach(() => {
    delete (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__;
    delete (window as Window & { __SKY_STARTUP_TELEMETRY_ENABLED__?: boolean })
      .__SKY_STARTUP_TELEMETRY_ENABLED__;
    Reflect.deleteProperty(window, '__SKY_STARTUP_WINDOW_REVEAL_COORDINATOR__');
    cleanup();
    vi.useRealTimers();
    vi.restoreAllMocks();
    if (matchMediaOverridden) {
      if (matchMediaDescriptor) {
        Object.defineProperty(window, 'matchMedia', matchMediaDescriptor);
      } else {
        Reflect.deleteProperty(window, 'matchMedia');
      }
      matchMediaDescriptor = undefined;
      matchMediaOverridden = false;
    }
    isTauriRuntimeMock.mockReturnValue(false);
    nativeWindow.show.mockReset().mockResolvedValue(undefined);
    acknowledgeStartupReveal.mockReset().mockResolvedValue(undefined);
    nativeWindow.minimize.mockReset().mockResolvedValue(undefined);
    nativeWindow.toggleMaximize.mockReset().mockResolvedValue(undefined);
    nativeWindow.close.mockReset().mockResolvedValue(undefined);
    nativeWindow.isMaximized.mockReset().mockResolvedValue(false);
    nativeWindow.onResized.mockReset().mockResolvedValue(() => {});
    nativeWindow.theme.mockReset().mockResolvedValue('dark');
    nativeWindow.onThemeChanged
      .mockReset()
      .mockImplementation(async (listener) => nativeWindow.subscribeTheme(listener));
    nativeWindow.clearThemeListeners();
    window.localStorage.clear();
  });

  it('renders the navigator, track browser, utility details, and settings', async () => {
    render(<App bridge={createMockBridge()} />);
    expect(screen.getByText('Opening your library…')).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole('row', { name: /Aurora Landing/ })).toBeInTheDocument(),
    );
    const navigator = screen.getByRole('navigation', { name: 'Library' });
    expect(navigator).toHaveTextContent('Built-in Songs');
    expect(navigator).not.toHaveTextContent('All Songs');
    expect(navigator).not.toHaveTextContent('My Songs');
    expect(screen.getByRole('grid', { name: 'Built-in Songs, page 1' })).toBeInTheDocument();

    const firstSong = screen.getByRole('row', { name: /Aurora Landing/ });
    fireEvent.click(firstSong);
    await waitFor(() => expect(firstSong).toHaveAttribute('aria-selected', 'true'));
    expect(screen.queryByText('Medium timing risk')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open utility panel' }));
    await waitFor(() =>
      expect(screen.getByRole('region', { name: 'Utility: Song Details' })).toBeInTheDocument(),
    );
    expect(screen.getByText('Medium timing risk')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Readiness' })).toHaveAttribute(
      'data-readiness',
      'review',
    );
    expect(screen.getByRole('region', { name: 'Readiness' })).toHaveTextContent(
      'Check the timing risk guidance below before starting playback.',
    );

    const settingsButton = screen.getByRole('button', { name: 'Open settings' });
    settingsButton.focus();
    await act(async () => {
      fireEvent.click(settingsButton);
      await vi.dynamicImportSettled();
    });
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
    expect(screen.getByLabelText('Dark palette')).toHaveTextContent('Graphite (Default)');
    const aboutButton = screen.getByRole('button', { name: 'About' });
    aboutButton.focus();
    fireEvent.click(aboutButton);
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
    const grid = await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
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

  it('ignores a malformed keydown event without a string key', async () => {
    render(<App bridge={createMockBridge()} />);
    await screen.findByRole('searchbox');

    const malformed = new Event('keydown') as KeyboardEvent;
    Object.defineProperty(malformed, 'key', { value: undefined });

    expect(() => window.dispatchEvent(malformed)).not.toThrow();
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

  it('opens Settings and toggles the Inspector from documented shortcuts', async () => {
    render(<App bridge={createMockBridge()} />);
    await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });

    fireEvent.keyDown(window, { key: ',', ctrlKey: true });
    expect(await screen.findByRole('dialog', { name: 'Settings' })).toBeVisible();
    fireEvent.keyDown(window, { key: 'i', ctrlKey: true });
    expect(document.querySelector('[aria-label="Open utility panel"]')).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    fireEvent.click(screen.getByRole('button', { name: 'Close settings' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull());

    fireEvent.keyDown(window, { key: 'i', ctrlKey: true });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Close utility panel' })).toHaveAttribute(
        'aria-expanded',
        'true',
      ),
    );
  });

  it('keeps Ctrl+I and Ctrl+, from hijacking an editable field or modal', async () => {
    render(<App bridge={createMockBridge()} />);
    const search = await screen.findByRole('searchbox');
    search.focus();
    const inspectorEvent = createEvent.keyDown(search, { key: 'i', ctrlKey: true });
    search.dispatchEvent(inspectorEvent);
    expect(inspectorEvent.defaultPrevented).toBe(false);
    expect(document.querySelector('[aria-label="Open utility panel"]')).toHaveAttribute(
      'aria-expanded',
      'false',
    );

    search.blur();
    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await screen.findByRole('dialog', { name: 'Settings' });
    fireEvent.keyDown(window, { key: 'i', ctrlKey: true });
    fireEvent.keyDown(window, { key: ',', ctrlKey: true });
    expect(document.querySelector('[aria-label="Open utility panel"]')).toHaveAttribute(
      'aria-expanded',
      'false',
    );
    expect(screen.getAllByRole('dialog', { name: 'Settings' })).toHaveLength(1);
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

  it('selects the last row on the active Built-in page with End', async () => {
    render(<App bridge={createMockBridge()} />);
    const grid = await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
    grid.focus();
    fireEvent.keyDown(grid, { key: 'End' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Song 100/ })
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
    nativeWindow.theme.mockResolvedValue('light');
    const originalBootstrap = bridge.bootstrap.bind(bridge);
    bridge.bootstrap = vi.fn(async () => {
      const bootstrap = await originalBootstrap();
      return {
        ...bootstrap,
        settings: { ...bootstrap.settings, palette: 'slate' as const },
      };
    });
    const appearanceWhenShown: Array<{ palette: string; mode: string }> = [];
    nativeWindow.show.mockImplementation(async () => {
      appearanceWhenShown.push({
        palette: document.documentElement.dataset.palette ?? '',
        mode: document.documentElement.dataset.colorMode ?? '',
      });
    });

    render(<App bridge={bridge} />);
    await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });

    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
    expect(acknowledgeStartupReveal).toHaveBeenCalledTimes(1);
    expect(appearanceWhenShown).toEqual([{ palette: 'slate', mode: 'light' }]);
  });

  it('waits for the native System theme before first reveal and follows later OS changes', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    const pendingTheme = deferred<'light' | 'dark'>();
    nativeWindow.theme.mockReturnValue(pendingTheme.promise);
    const appearanceWhenShown: string[] = [];
    nativeWindow.show.mockImplementation(async () => {
      appearanceWhenShown.push(document.documentElement.dataset.colorMode ?? '');
    });

    render(<App bridge={bridge} />);
    await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
    expect(nativeWindow.show).not.toHaveBeenCalled();

    await act(async () => pendingTheme.resolve('light'));
    await waitFor(() => expect(nativeWindow.show).toHaveBeenCalledTimes(1));
    expect(appearanceWhenShown).toEqual(['light']);

    nativeWindow.emitTheme('dark');
    await waitFor(() => expect(document.documentElement.dataset.colorMode).toBe('dark'));

    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await screen.findByRole('dialog', { name: 'Settings' });
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
    const lightRadio = await screen.findByRole('radio', { name: 'Light' });
    fireEvent.click(lightRadio);
    await waitFor(() => expect(document.documentElement.dataset.colorMode).toBe('light'));

    nativeWindow.emitTheme('dark');
    await waitFor(() => expect(document.documentElement.dataset.colorMode).toBe('light'));

    fireEvent.click(screen.getByRole('radio', { name: 'System' }));
    await waitFor(() => expect(document.documentElement.dataset.colorMode).toBe('dark'));
  });

  it('reveals the committed fatal bootstrap screen', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    bridge.subscribeUiEvents = vi.fn(async () => {
      throw new Error('Native application is unavailable');
    });
    const fatalUiVisibleWhenShown: boolean[] = [];
    nativeWindow.show.mockImplementation(async () => {
      fatalUiVisibleWhenShown.push(Boolean(screen.queryByRole('alert')));
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

  it('keeps the 2.5-second fail-open reveal while bootstrap and the native theme getter are pending', async () => {
    vi.useFakeTimers();
    isTauriRuntimeMock.mockReturnValue(true);
    matchMediaDescriptor = Object.getOwnPropertyDescriptor(window, 'matchMedia');
    matchMediaOverridden = true;
    const lightSystemQuery = {
      matches: false,
      media: '(prefers-color-scheme: dark)',
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList;
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => lightSystemQuery),
    });

    const pendingBootstrap = deferred<Awaited<ReturnType<DesktopBridge['bootstrap']>>>();
    const pendingTheme = deferred<'light' | 'dark'>();
    const bridge = createMockBridge();
    bridge.bootstrap = vi.fn(() => pendingBootstrap.promise);
    nativeWindow.theme.mockReturnValue(pendingTheme.promise);
    const appearanceWhenShown: string[] = [];
    nativeWindow.show.mockImplementation(async () => {
      appearanceWhenShown.push(document.documentElement.dataset.colorMode ?? '');
    });

    render(<App bridge={bridge} />);
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(bridge.bootstrap).toHaveBeenCalledTimes(1);
    expect(nativeWindow.show).not.toHaveBeenCalled();
    expect(document.documentElement.dataset.colorMode).toBe('light');

    await act(async () => {
      await vi.advanceTimersByTimeAsync(STARTUP_WINDOW_REVEAL_FALLBACK_MS);
      await Promise.resolve();
    });
    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
    expect(appearanceWhenShown).toEqual(['light']);

    const legacyBootstrap = await createMockBridge().bootstrap();
    const darkLegacyBootstrap = {
      ...legacyBootstrap,
      settings: { ...legacyBootstrap.settings, appearance_mode: 'dark' as const },
    };
    await act(async () => {
      pendingBootstrap.resolve(darkLegacyBootstrap);
      await pendingBootstrap.promise;
    });
    expect(document.documentElement.dataset.colorMode).toBe('dark');

    await act(async () => {
      pendingTheme.resolve('light');
      await pendingTheme.promise;
    });
    expect(document.documentElement.dataset.colorMode).toBe('dark');
    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
  });

  it('keeps StrictMode startup to one native subscription and one window reveal', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const bridge = createMockBridge();
    const listNavigation = vi.spyOn(bridge, 'listLibraryNavigation');
    const nativeEvents = installStrictSingletonEvents(bridge);
    const { rerender, unmount } = render(
      <StrictMode>
        <App bridge={bridge} />
      </StrictMode>,
    );
    await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
    rerender(
      <StrictMode>
        <App bridge={bridge} />
      </StrictMode>,
    );

    expect(nativeEvents.subscriptionCount).toBe(1);
    expect(listNavigation).toHaveBeenCalledTimes(1);
    expect(nativeWindow.show).toHaveBeenCalledTimes(1);
    expect(nativeWindow.activeThemeListenerCount()).toBe(1);

    unmount();
    await waitFor(() => expect(nativeWindow.activeThemeListenerCount()).toBe(0));
  });

  it.each(['event', 'global', 'event-and-global'] as const)(
    'ordinary App ignores a forged %s smoke trigger',
    async (mode) => {
      const addEventListener = vi.spyOn(window, 'addEventListener');
      if (mode !== 'event') {
        (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__ =
          true;
      }
      const { patchSettings, setDiagnosticsEnabled, shutdown } =
        await renderOrdinaryAppWithSmokeSpies();

      await act(async () => {
        if (mode !== 'global') {
          window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
          window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
        }
      });

      expect(addEventListener).not.toHaveBeenCalledWith(
        'sky-desktop-gui-smoke',
        expect.any(Function),
      );
      expect(patchSettings).not.toHaveBeenCalled();
      expect(setDiagnosticsEnabled).not.toHaveBeenCalled();
      expect(shutdown).not.toHaveBeenCalled();
    },
  );

  it('ordinary App ignores a smoke event dispatched before React mounts', async () => {
    const addEventListener = vi.spyOn(window, 'addEventListener');
    window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    const { patchSettings, setDiagnosticsEnabled, shutdown } =
      await renderOrdinaryAppWithSmokeSpies();

    expect(addEventListener).not.toHaveBeenCalledWith(
      'sky-desktop-gui-smoke',
      expect.any(Function),
    );
    expect(patchSettings).not.toHaveBeenCalled();
    expect(setDiagnosticsEnabled).not.toHaveBeenCalled();
    expect(shutdown).not.toHaveBeenCalled();
  });

  it('ordinary App fails closed when bootstrap omits gui_smoke', async () => {
    const addEventListener = vi.spyOn(window, 'addEventListener');
    const bridge = createMockBridge();
    const originalBootstrap = bridge.bootstrap.bind(bridge);
    bridge.bootstrap = vi.fn(async () => {
      const bootstrap = await originalBootstrap();
      const { gui_smoke: _guiSmoke, ...legacyBootstrap } = bootstrap;
      return legacyBootstrap as typeof bootstrap;
    });
    const patchSettings = vi.spyOn(bridge, 'patchSettings');
    const setDiagnosticsEnabled = vi.spyOn(bridge, 'setDiagnosticsEnabled');
    const shutdown = vi.spyOn(bridge, 'shutdown');
    (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__ = true;

    render(<App bridge={bridge} />);
    await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    });

    expect(addEventListener).not.toHaveBeenCalledWith(
      'sky-desktop-gui-smoke',
      expect.any(Function),
    );
    expect(patchSettings).not.toHaveBeenCalled();
    expect(setDiagnosticsEnabled).not.toHaveBeenCalled();
    expect(shutdown).not.toHaveBeenCalled();
  });

  it('ordinary App fails closed when bootstrap is unavailable', async () => {
    const addEventListener = vi.spyOn(window, 'addEventListener');
    const bridge = createMockBridge();
    bridge.subscribeUiEvents = vi.fn(async () => {
      throw new Error('Native application is unavailable');
    });
    const shutdown = vi.spyOn(bridge, 'shutdown');
    (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__ = true;

    render(<App bridge={bridge} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Native application is unavailable');
    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    });

    expect(addEventListener).not.toHaveBeenCalledWith(
      'sky-desktop-gui-smoke',
      expect.any(Function),
    );
    expect(shutdown).not.toHaveBeenCalled();
  });

  it('isolated smoke hook ignores triggers when authorized is false', async () => {
    const addEventListener = vi.spyOn(window, 'addEventListener');
    const fixture = await createPackagedSmokeFixture(false);
    (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__ = true;

    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    });

    expect(addEventListener).not.toHaveBeenCalledWith(
      'sky-desktop-gui-smoke',
      expect.any(Function),
    );
    expect(fixture.patchSettings).not.toHaveBeenCalled();
    expect(fixture.setDiagnosticsEnabled).not.toHaveBeenCalled();
    expect(fixture.shutdown).not.toHaveBeenCalled();
  });

  it('native-authorized App runs the smoke once from the persistent global', async () => {
    const bridge = createMockBridge();
    authorizeGuiSmoke(bridge);
    const patchSettings = vi.spyOn(bridge, 'patchSettings');
    const setDiagnosticsEnabled = vi.spyOn(bridge, 'setDiagnosticsEnabled');
    const shutdown = vi.spyOn(bridge, 'shutdown');
    (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__ = true;

    render(
      <StrictMode>
        <App bridge={bridge} />
      </StrictMode>,
    );
    await waitFor(() => expect(shutdown).toHaveBeenCalledTimes(1), { timeout: 5_000 });

    expect(patchSettings).toHaveBeenCalledTimes(2);
    expect(setDiagnosticsEnabled).toHaveBeenCalledTimes(2);
  });

  it('flushes the catalog readiness marker before packaged smoke shutdown', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    (
      window as Window & { __SKY_STARTUP_TELEMETRY_ENABLED__?: boolean }
    ).__SKY_STARTUP_TELEMETRY_ENABLED__ = true;
    (window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__ = true;
    const telemetryStarted = deferred<void>();
    const telemetryRelease = deferred<void>();
    acknowledgeStartupReveal.mockImplementation(async (...args) => {
      const [command, request] = args;
      const marker =
        typeof request === 'object' && request !== null && 'params' in request
          ? (request as { params?: { marker?: unknown } }).params?.marker
          : undefined;
      if (command === 'subscribe_ui_events' && marker === 'react.catalog_ready') {
        telemetryStarted.resolve();
        await telemetryRelease.promise;
      }
    });

    const bridge = createMockBridge();
    authorizeGuiSmoke(bridge);
    const searchSongs = vi.spyOn(bridge, 'searchSongs');
    const shutdown = vi.spyOn(bridge, 'shutdown');
    render(
      <StrictMode>
        <App bridge={bridge} />
      </StrictMode>,
    );

    await telemetryStarted.promise;
    expect(shutdown).not.toHaveBeenCalled();
    telemetryRelease.resolve();
    await waitFor(() => expect(shutdown).toHaveBeenCalledTimes(1), { timeout: 5_000 });

    const catalogReadyCalls = acknowledgeStartupReveal.mock.calls.filter(([command, request]) => {
      if (command !== 'subscribe_ui_events' || typeof request !== 'object' || request === null) {
        return false;
      }
      return (
        'params' in request &&
        (request as { params?: { marker?: unknown } }).params?.marker === 'react.catalog_ready'
      );
    });
    expect(catalogReadyCalls).toHaveLength(1);
    const smokeSearch = searchSongs.mock.calls.find(([request]) => request.limit === 200);
    // The smoke query is intentionally unpinned to the store's generation and checks the returned
    // catalog generation independently before allowing shutdown.
    expect(smokeSearch?.[0]).toMatchObject({
      query: '',
      offset: 0,
      limit: 200,
      source: { kind: 'smart', id: 'all' },
    });
    expect(smokeSearch?.[0]).not.toHaveProperty('generation');
  });

  it('native-authorized App starts from a post-mount event and stays one-shot', async () => {
    const bridge = createMockBridge();
    authorizeGuiSmoke(bridge);
    const shutdown = vi.spyOn(bridge, 'shutdown');

    render(<App bridge={bridge} />);
    await screen.findByRole('grid', { name: 'Built-in Songs, page 1' });
    await act(async () => {
      await new Promise((resolve) => window.setTimeout(resolve, 0));
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
    });
    await waitFor(() => expect(shutdown).toHaveBeenCalledTimes(1), { timeout: 5_000 });

    await act(async () => {
      window.dispatchEvent(new Event('sky-desktop-gui-smoke'));
      await new Promise((resolve) => window.setTimeout(resolve, 50));
    });
    expect(shutdown).toHaveBeenCalledTimes(1);
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
    expect(
      await screen.findByRole('tab', { name: 'Performance' }, { timeout: 5_000 }),
    ).toBeInTheDocument();
    fireEvent.keyDown(diagnostics, { key: 'Escape' });
    await waitFor(() =>
      expect(screen.queryByRole('region', { name: 'Utility: Diagnostics' })).toBeNull(),
    );
    expect(document.activeElement).toBe(trigger);

    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    await screen.findByRole('dialog', { name: 'Settings' });
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
    await screen.findByRole('region', { name: 'Appearance' });
  });
});
