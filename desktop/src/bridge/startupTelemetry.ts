import { Channel, invoke } from '@tauri-apps/api/core';
import { isTauriRuntime } from '../platform/runtime';

interface StartupTelemetryWindow extends Window {
  __SKY_STARTUP_TELEMETRY_ENABLED__?: boolean;
}

const STARTUP_TELEMETRY_COMMAND = 'subscribe_ui_events';
let telemetryQueue = Promise.resolve();

export function recordStartupTelemetry(marker: string): void {
  if (!isTauriRuntime()) return;
  if ((window as StartupTelemetryWindow).__SKY_STARTUP_TELEMETRY_ENABLED__ !== true) return;

  const frontendElapsedUs = Math.round(performance.now() * 1000);
  telemetryQueue = telemetryQueue.then(() =>
    invoke(STARTUP_TELEMETRY_COMMAND, {
      channel: new Channel<never>(),
      params: { marker, frontend_elapsed_us: frontendElapsedUs },
    })
      .then(() => undefined)
      .catch(() => undefined),
  );
}
