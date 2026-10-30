import { getCurrentWindow } from '@tauri-apps/api/window';
import { isTauriRuntime } from './runtime';
import type { ColorMode } from '../appearance/resolveAppearance';

const COLOR_SCHEME_QUERY = '(prefers-color-scheme: dark)';

export interface SystemAppearanceSubscription {
  ready: Promise<void>;
  unsubscribe: () => void;
}

function isColorMode(value: unknown): value is ColorMode {
  return value === 'light' || value === 'dark';
}

function getMediaQuery(): MediaQueryList | null {
  if (typeof window.matchMedia !== 'function') return null;
  return window.matchMedia(COLOR_SCHEME_QUERY);
}

export function readBrowserSystemMode(): ColorMode {
  try {
    const query = getMediaQuery();
    // Keep the existing dark presentation as a safe fallback in older browser shells.
    return query ? (query.matches ? 'dark' : 'light') : 'dark';
  } catch {
    return 'dark';
  }
}

export function subscribeSystemAppearance(
  onChange: (mode: ColorMode) => void,
): SystemAppearanceSubscription {
  let active = true;
  let nativeUnlisten: (() => void) | null = null;
  let mediaUnlisten: (() => void) | null = null;

  const publish = (mode: unknown) => {
    if (active && isColorMode(mode)) onChange(mode);
  };

  if (!isTauriRuntime()) {
    try {
      const query = getMediaQuery();
      if (query) {
        publish(query.matches ? 'dark' : 'light');
        const listener = (event: MediaQueryListEvent) => publish(event.matches ? 'dark' : 'light');
        query.addEventListener('change', listener);
        mediaUnlisten = () => query.removeEventListener('change', listener);
      } else {
        publish('dark');
      }
    } catch {
      publish('dark');
    }

    return {
      ready: Promise.resolve(),
      unsubscribe() {
        if (!active) return;
        active = false;
        mediaUnlisten?.();
        mediaUnlisten = null;
      },
    };
  }

  // Use media only as a temporary startup fallback. Packaged Tauri listens to
  // the native window theme and does not keep a second OS-theme listener active.
  publish(readBrowserSystemMode());
  let nativeEventVersion = 0;
  let mediaEventVersion = 0;
  const appWindow = getCurrentWindow();
  const ready = (async () => {
    try {
      const unlisten = await appWindow.onThemeChanged(({ payload }) => {
        if (!active) return;
        nativeEventVersion += 1;
        publish(payload);
      });
      if (!active) {
        unlisten();
        return;
      }
      nativeUnlisten = unlisten;
    } catch {
      // A browser media query is the fallback when the native listener is unavailable.
      subscribeMediaFallback();
    }

    const eventVersionBeforeRead = nativeEventVersion;
    const mediaVersionBeforeRead = mediaEventVersion;
    try {
      const mode = await appWindow.theme();
      if (
        !active ||
        nativeEventVersion !== eventVersionBeforeRead ||
        mediaEventVersion !== mediaVersionBeforeRead
      ) {
        return;
      }
      if (isColorMode(mode)) {
        publish(mode);
      } else {
        publish(readBrowserSystemMode());
      }
    } catch {
      if (
        !active ||
        nativeEventVersion !== eventVersionBeforeRead ||
        mediaEventVersion !== mediaVersionBeforeRead
      ) {
        return;
      }
      publish(readBrowserSystemMode());
    }
  })();

  function subscribeMediaFallback() {
    if (!active || mediaUnlisten) return;
    try {
      const query = getMediaQuery();
      if (!query) {
        publish('dark');
        return;
      }
      const listener = (event: MediaQueryListEvent) => {
        mediaEventVersion += 1;
        publish(event.matches ? 'dark' : 'light');
      };
      query.addEventListener('change', listener);
      mediaUnlisten = () => query.removeEventListener('change', listener);
      publish(query.matches ? 'dark' : 'light');
    } catch {
      publish('dark');
    }
  }

  return {
    ready,
    unsubscribe() {
      if (!active) return;
      active = false;
      nativeUnlisten?.();
      mediaUnlisten?.();
      nativeUnlisten = null;
      mediaUnlisten = null;
    },
  };
}
