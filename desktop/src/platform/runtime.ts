import { isTauri } from '@tauri-apps/api/core';

export function isTauriRuntime(): boolean {
  return (
    isTauri() ||
    window.location.protocol === 'tauri:' ||
    window.location.hostname === 'tauri.localhost'
  );
}
