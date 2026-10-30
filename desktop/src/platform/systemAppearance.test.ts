import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { subscribeSystemAppearance } from './systemAppearance';

const { isTauriRuntimeMock, nativeWindow } = vi.hoisted(() => {
  const listeners = new Set<(event: { payload: 'light' | 'dark' }) => void>();
  const nativeWindow = {
    theme: vi.fn(async (): Promise<'light' | 'dark'> => 'dark'),
    onThemeChanged: vi.fn(async (listener: (event: { payload: 'light' | 'dark' }) => void) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    }),
    emitTheme(payload: 'light' | 'dark') {
      for (const listener of listeners) listener({ payload });
    },
    clearListeners() {
      listeners.clear();
    },
  };
  return { isTauriRuntimeMock: vi.fn(() => false), nativeWindow };
});

vi.mock('./runtime', () => ({ isTauriRuntime: isTauriRuntimeMock }));
vi.mock('@tauri-apps/api/window', () => ({ getCurrentWindow: () => nativeWindow }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

describe('system appearance subscription', () => {
  let originalMatchMedia: typeof window.matchMedia;

  beforeEach(() => {
    originalMatchMedia = window.matchMedia;
    isTauriRuntimeMock.mockReturnValue(false);
    nativeWindow.theme.mockReset().mockResolvedValue('dark');
    nativeWindow.onThemeChanged.mockClear();
    nativeWindow.clearListeners();
  });

  afterEach(() => {
    if (originalMatchMedia) {
      Object.defineProperty(window, 'matchMedia', {
        configurable: true,
        value: originalMatchMedia,
      });
    } else {
      Reflect.deleteProperty(window, 'matchMedia');
    }
    vi.restoreAllMocks();
  });

  it('uses the native theme event over a stale asynchronous getter result', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    const pendingTheme = deferred<'light' | 'dark'>();
    nativeWindow.theme.mockReturnValue(pendingTheme.promise);
    const modes: string[] = [];
    const subscription = subscribeSystemAppearance((mode) => modes.push(mode));

    await vi.waitFor(() => expect(nativeWindow.theme).toHaveBeenCalledTimes(1));
    nativeWindow.emitTheme('light');
    pendingTheme.resolve('dark');
    await subscription.ready;

    expect(modes.at(-1)).toBe('light');
    subscription.unsubscribe();
  });

  it('tracks browser media changes and removes its listener on cleanup', async () => {
    const listeners: Array<(event: MediaQueryListEvent) => void> = [];
    let mediaMatches = false;
    const removeListener = vi.fn();
    const mediaQuery = {
      get matches() {
        return mediaMatches;
      },
      addEventListener: vi.fn((_name: string, callback: (event: MediaQueryListEvent) => void) => {
        listeners.push(callback);
      }),
      removeEventListener: removeListener,
    } as unknown as MediaQueryList;
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => mediaQuery),
    });
    const modes: string[] = [];

    const subscription = subscribeSystemAppearance((mode) => modes.push(mode));
    await subscription.ready;
    expect(modes).toEqual(['light']);

    mediaMatches = true;
    listeners[0]?.({ matches: true } as MediaQueryListEvent);
    expect(modes).toEqual(['light', 'dark']);

    subscription.unsubscribe();
    expect(removeListener).toHaveBeenCalledTimes(1);
  });

  it('uses media events if the packaged native theme listener is unavailable', async () => {
    isTauriRuntimeMock.mockReturnValue(true);
    nativeWindow.onThemeChanged.mockRejectedValueOnce(new Error('native listener unavailable'));
    const pendingTheme = deferred<'light' | 'dark'>();
    nativeWindow.theme.mockReturnValue(pendingTheme.promise);
    let mediaMatches = false;
    const listeners: Array<(event: MediaQueryListEvent) => void> = [];
    const mediaQuery = {
      get matches() {
        return mediaMatches;
      },
      addEventListener: vi.fn((_name: string, callback: (event: MediaQueryListEvent) => void) => {
        listeners.push(callback);
      }),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList;
    Object.defineProperty(window, 'matchMedia', {
      configurable: true,
      value: vi.fn(() => mediaQuery),
    });
    const modes: string[] = [];

    const subscription = subscribeSystemAppearance((mode) => modes.push(mode));
    await vi.waitFor(() => expect(nativeWindow.theme).toHaveBeenCalledTimes(1));
    mediaMatches = true;
    listeners[0]?.({ matches: true } as MediaQueryListEvent);
    pendingTheme.resolve('light');
    await subscription.ready;
    expect(modes.at(-1)).toBe('dark');

    mediaMatches = false;
    listeners[0]?.({ matches: false } as MediaQueryListEvent);
    expect(modes.at(-1)).toBe('light');
    subscription.unsubscribe();
    expect(mediaQuery.removeEventListener).toHaveBeenCalledTimes(1);
  });
});
