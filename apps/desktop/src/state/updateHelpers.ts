import type { UpdateErrorCode, UpdateRetryAction, UpdateState } from '../bridge/DesktopBridge';

export interface UpdatePresentationState {
  state: UpdateState;
  availableVersion: string | null;
  checkRequestPending: boolean;
  installRequestPending: boolean;
  transportError: string | null;
  transportErrorAction: 'check' | 'install' | null;
  errorCode: UpdateErrorCode | null;
  errorDetail: string | null;
  retryAction: UpdateRetryAction;
  progress: { completed: number; total: number | null; message: string } | null;
}

export interface UpdatePresentation {
  isChecking: boolean;
  isNativeBusy: boolean;
  isBusy: boolean;
  isError: boolean;
  isAvailable: boolean;
  isCurrent: boolean;
  effectiveRetryAction: UpdateRetryAction;
  errorHeading: string;
  errorText: string | null;
  busyHeading: string;
  busyDetail: string;
}

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
      return 'Could not check for updates. Check your network connection and try again.';
    case 'download_failed':
      return 'Failed to download the update. Please try again later.';
    case 'install_failed':
      return 'Failed to install the update. Please try again later.';
    case 'unknown':
      return 'An unexpected error occurred while updating.';
  }

  const raw = typeof code === 'string' ? code : (detail ?? '');
  const stripped = stripTechnicalPrefixes(raw);
  return stripped.length > 0
    ? stripped
    : 'An unexpected error occurred while checking for updates.';
}

function getUpdateErrorHeading(
  errorCode: UpdateErrorCode | null,
  retryAction: UpdateRetryAction,
  hasTransportError = false,
): string {
  if (hasTransportError) {
    return retryAction === 'install' ? 'Update failed' : 'Update check failed';
  }
  if (
    errorCode === 'check_failed' ||
    errorCode === 'state_persistence_failed' ||
    errorCode === 'channel_unavailable' ||
    errorCode === 'update_service_unavailable'
  ) {
    return 'Update check failed';
  }
  if (
    errorCode === 'playback_active' ||
    errorCode === 'update_busy' ||
    errorCode === 'closing' ||
    errorCode === 'stale_update' ||
    errorCode === 'update_unavailable' ||
    errorCode === 'download_failed' ||
    errorCode === 'install_failed' ||
    retryAction === 'install'
  ) {
    return 'Update failed';
  }
  return 'Update check failed';
}

export function deriveUpdatePresentation(update: UpdatePresentationState): UpdatePresentation {
  const isChecking = update.checkRequestPending || update.state === 'checking';
  const isNativeBusy =
    update.state === 'downloading' || update.state === 'ready' || update.state === 'installing';
  const isPendingHandoff = update.installRequestPending && !isNativeBusy;
  const isBusy = update.installRequestPending || isNativeBusy;
  const isError = update.state === 'error' || Boolean(update.transportError);
  const isAvailable =
    !isError && !isBusy && update.state === 'available' && Boolean(update.availableVersion);
  const isCurrent = !isError && !isBusy && update.state === 'current';

  const effectiveRetryAction: UpdateRetryAction = update.transportError
    ? (update.transportErrorAction ?? 'check')
    : update.retryAction;

  const errorText = isError
    ? update.transportError
      ? formatUpdateError(null, update.transportError)
      : formatUpdateError(update.errorCode, update.errorDetail)
    : null;

  const errorHeading = getUpdateErrorHeading(
    update.errorCode,
    effectiveRetryAction,
    Boolean(update.transportError),
  );

  const busyHeading =
    update.state === 'downloading'
      ? 'Downloading update…'
      : update.state === 'ready'
        ? 'Update ready to install'
        : update.state === 'installing'
          ? 'Installing update…'
          : isPendingHandoff
            ? 'Preparing update…'
            : 'No update available';

  const busyDetail = isPendingHandoff
    ? 'Starting the update handoff…'
    : isNativeBusy
      ? `${update.progress?.message || 'The update is being applied.'}${update.progress?.total ? ` (${update.progress.completed}/${update.progress.total} bytes)` : ''}`
      : 'The installed application is up to date.';

  return {
    isChecking,
    isNativeBusy,
    isBusy,
    isError,
    isAvailable,
    isCurrent,
    effectiveRetryAction,
    errorHeading,
    errorText,
    busyHeading,
    busyDetail,
  };
}
