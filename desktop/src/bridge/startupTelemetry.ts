import { Channel, invoke } from '@tauri-apps/api/core';

interface StartupTelemetryWindow extends Window {
  __SKY_STARTUP_TELEMETRY_ENABLED__?: boolean;
}

const STARTUP_TELEMETRY_COMMAND = 'subscribe_ui_events';
let telemetryQueue = Promise.resolve();

function isTauriWindow(): boolean {
  return window.location.protocol === 'tauri:' || window.location.hostname === 'tauri.localhost';
}

export function recordStartupTelemetry(marker: string): void {
  if (!isTauriWindow()) return;
  if ((window as StartupTelemetryWindow).__SKY_STARTUP_TELEMETRY_ENABLED__ !== true) return;

  telemetryQueue = telemetryQueue.then(() =>
    invoke(STARTUP_TELEMETRY_COMMAND, {
      channel: new Channel<never>(),
      params: { marker },
    })
      .then(() => undefined)
      .catch(() => undefined),
  );
}
