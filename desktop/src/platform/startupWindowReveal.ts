import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { STARTUP_WINDOW_REVEALED_COMMAND } from '../bridge/generated/command_names';
import { recordStartupTelemetry } from '../bridge/startupTelemetry';
import { isTauriRuntime } from './runtime';

export const STARTUP_WINDOW_REVEAL_FALLBACK_MS = 2_500;

export type StartupWindowRevealReason =
  'bootstrap-ready' | 'bootstrap-failed' | 'slow-bootstrap' | 'render-error';

export interface StartupWindowRevealCoordinator {
  scheduleFallback(): void;
  reveal(reason: StartupWindowRevealReason): void;
}

const COORDINATOR_KEY = '__SKY_STARTUP_WINDOW_REVEAL_COORDINATOR__';

declare global {
  interface Window {
    __SKY_STARTUP_WINDOW_REVEAL_COORDINATOR__?: StartupWindowRevealCoordinator;
  }
}

export function createStartupWindowRevealCoordinator(
  showWindow: () => Promise<void>,
  onShown: () => void,
  onFailure: (reason: StartupWindowRevealReason, error: unknown) => void,
  onSlowBootstrap: () => void,
  fallbackDelayMs = STARTUP_WINDOW_REVEAL_FALLBACK_MS,
): StartupWindowRevealCoordinator {
  let revealRequested = false;
  let fallbackTimer: ReturnType<typeof setTimeout> | null = null;

  const reveal = (reason: StartupWindowRevealReason) => {
    if (revealRequested) return;
    revealRequested = true;
    if (fallbackTimer !== null) {
      clearTimeout(fallbackTimer);
      fallbackTimer = null;
    }
    if (reason === 'slow-bootstrap') onSlowBootstrap();

    try {
      void showWindow()
        .then(onShown)
        .catch((error: unknown) => onFailure(reason, error));
    } catch (error) {
      onFailure(reason, error);
    }
  };

  return {
    scheduleFallback() {
      if (revealRequested || fallbackTimer !== null) return;
      fallbackTimer = setTimeout(() => {
        fallbackTimer = null;
        reveal('slow-bootstrap');
      }, fallbackDelayMs);
    },
    reveal,
  };
}

function getCoordinator(): StartupWindowRevealCoordinator | null {
  if (!isTauriRuntime()) return null;
  const existing = window[COORDINATOR_KEY];
  if (existing) return existing;

  const coordinator = createStartupWindowRevealCoordinator(
    () =>
      getCurrentWindow()
        .show()
        .then(() => invoke<void>(STARTUP_WINDOW_REVEALED_COMMAND)),
    () => recordStartupTelemetry('window.reveal.shown'),
    (reason, error) => {
      console.error(
        `Could not reveal the Sky Auto Player window during ${reason}; the native startup fallback may retry until startup is acknowledged or shutdown is accepted.`,
        error,
      );
      recordStartupTelemetry('window.reveal.failed');
    },
    () => recordStartupTelemetry('window.reveal.slow_fallback'),
  );
  window[COORDINATOR_KEY] = coordinator;
  return coordinator;
}

export function scheduleStartupWindowRevealFallback(): void {
  getCoordinator()?.scheduleFallback();
}

export function revealStartupWindow(reason: StartupWindowRevealReason): void {
  getCoordinator()?.reveal(reason);
}
