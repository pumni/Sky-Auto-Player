export const DEFAULT_CHECK_INTERVAL_S = 86_400;
export const RETRY_INTERVAL_S = 300;

export interface AutoCheckPreferences {
  auto_check: boolean;
  check_interval_s?: number;
  last_check_ts?: number;
  last_error_ts?: number;
}

export function shouldAutoCheck(
  preferences: AutoCheckPreferences,
  nowSec: number = Math.floor(Date.now() / 1000),
): boolean {
  if (!preferences.auto_check) {
    return false;
  }
  const checkIntervalS = preferences.check_interval_s ?? DEFAULT_CHECK_INTERVAL_S;
  const lastCheckTs = preferences.last_check_ts ?? 0;
  const lastErrorTs = preferences.last_error_ts ?? 0;

  const successElapsed = Math.max(0, nowSec - lastCheckTs);
  if (nowSec < lastCheckTs || successElapsed >= checkIntervalS) {
    return true;
  }
  if (lastErrorTs !== 0) {
    const errorElapsed = Math.max(0, nowSec - lastErrorTs);
    if (nowSec < lastErrorTs || errorElapsed >= RETRY_INTERVAL_S) {
      return true;
    }
  }
  return false;
}

export function formatUpdateError(error: unknown): string {
  if (!error) {
    return 'The update service is temporarily unavailable. Please try again later.';
  }
  const raw = error instanceof Error ? error.message : String(error);

  if (raw.includes('playback_active')) {
    return 'Updates cannot be installed while music playback is active. Stop playback and try again.';
  }
  if (raw.includes('calibration_active')) {
    return 'Updates cannot be installed while timing calibration is running. Complete or cancel calibration and try again.';
  }
  if (raw.includes('update_busy')) {
    return 'An update installation is already in progress.';
  }
  if (raw.includes('closing')) {
    return 'The application is closing.';
  }
  if (raw.includes('update_service_unavailable')) {
    return 'The update service is not available. Please try again later.';
  }
  if (raw.includes('Failed to fetch') || raw.includes('network') || raw.includes('connect')) {
    return 'Could not check for updates. Check your network connection and try again.';
  }

  // Strip known technical prefixes if present to present clean user copy
  const stripped = raw
    .replace(/^(error:\s*)+/i, '')
    .replace(/^[a-z_]+:\s*/i, (match) => {
      // If prefix looks like an internal code (e.g. "invalid_params: "), strip it
      return match.includes('_') ? '' : match;
    })
    .trim();

  return stripped.length > 0
    ? stripped
    : 'An unexpected error occurred while checking for updates.';
}
