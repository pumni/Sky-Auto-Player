import type { UpdateErrorCode } from '../bridge/DesktopBridge';

export function stripTechnicalPrefixes(raw: string): string {
  return raw
    .replace(/^(error:\s*)+/i, '')
    .replace(/^[a-z0-9_]+:\s*/i, '')
    .trim();
}

export function formatUpdateError(
  code?: UpdateErrorCode | string | null,
  detail?: string | null,
): string {
  if (!code && !detail) {
    return 'The update service is temporarily unavailable. Please try again later.';
  }

  switch (code) {
    case 'playback_active':
      return 'Updates cannot be installed while music playback is active. Stop playback and try again.';
    case 'calibration_active':
      return 'Updates cannot be installed while timing calibration is running. Complete or cancel calibration and try again.';
    case 'update_busy':
      return 'An update installation is already in progress.';
    case 'closing':
      return 'The application is closing.';
    case 'channel_unavailable':
      return 'The selected update channel is not available.';
    case 'update_service_unavailable':
      return 'The update service is not available. Please try again later.';
    case 'stale_update':
      return 'The requested update is no longer available.';
    case 'update_unavailable':
      return 'No update is available for this version.';
    case 'state_persistence_failed':
      return 'Failed to save update state.';
    case 'check_failed':
      if (detail) {
        const stripped = stripTechnicalPrefixes(detail);
        if (stripped.length > 0) return stripped;
      }
      return 'Could not check for updates. Check your network connection and try again.';
    case 'download_failed':
      if (detail) {
        const stripped = stripTechnicalPrefixes(detail);
        if (stripped.length > 0) return stripped;
      }
      return 'Failed to download the update. Please try again later.';
    case 'install_failed':
      if (detail) {
        const stripped = stripTechnicalPrefixes(detail);
        if (stripped.length > 0) return stripped;
      }
      return 'Failed to install the update. Please try again later.';
    case 'unknown':
      if (detail) {
        const stripped = stripTechnicalPrefixes(detail);
        if (stripped.length > 0) return stripped;
      }
      return 'An unexpected error occurred while updating.';
  }

  const raw = typeof code === 'string' ? code : (detail ?? '');
  const stripped = stripTechnicalPrefixes(raw);
  return stripped.length > 0
    ? stripped
    : 'An unexpected error occurred while checking for updates.';
}
