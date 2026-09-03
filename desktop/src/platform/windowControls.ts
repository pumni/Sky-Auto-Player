import { getCurrentWindow } from '@tauri-apps/api/window';

export interface WindowControls {
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
  isMaximized(): Promise<boolean>;
  onResize(listener: () => void): Promise<() => void>;
}

export function createTauriWindowControls(): WindowControls {
  const appWindow = getCurrentWindow();
  return {
    minimize: () => appWindow.minimize(),
    toggleMaximize: () => appWindow.toggleMaximize(),
    close: () => appWindow.close(),
    isMaximized: () => appWindow.isMaximized(),
    onResize: async (listener) => appWindow.onResized(listener),
  };
}

export function createBrowserWindowControls(): WindowControls {
  let maximized = false;
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((listener) => listener());

  return {
    async minimize() {
      // Browser shells cannot minimize their host window. Keeping this action a
      // deterministic no-op makes the chrome testable outside Tauri.
    },
    async toggleMaximize() {
      maximized = !maximized;
      notify();
    },
    async close() {
      // Browser shells cannot close their host window.
    },
    async isMaximized() {
      return maximized;
    },
    async onResize(listener) {
      const onWindowResize = () => listener();
      listeners.add(listener);
      window.addEventListener('resize', onWindowResize);
      return () => {
        listeners.delete(listener);
        window.removeEventListener('resize', onWindowResize);
      };
    },
  };
}

export function createWindowControls(): WindowControls {
  const isTauri =
    '__TAURI_INTERNALS__' in window ||
    'isTauri' in window ||
    window.location.protocol === 'tauri:' ||
    window.location.hostname === 'tauri.localhost';
  return isTauri ? createTauriWindowControls() : createBrowserWindowControls();
}
