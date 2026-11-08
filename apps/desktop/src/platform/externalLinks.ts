import { openUrl } from '@tauri-apps/plugin-opener';

/** Open a donation destination in the system browser from the packaged desktop app. */
export async function openExternalUrl(url: string): Promise<void> {
  await openUrl(url);
}
