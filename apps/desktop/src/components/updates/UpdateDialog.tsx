import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { AlertCircle, ArrowDownToLine, Check, LoaderCircle, Sparkles, X } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import type { UpdateErrorCode, UpdateRetryAction } from '../../bridge/DesktopBridge';
import { formatUpdateError } from '../../state/updateHelpers';

function getErrorHeading(
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
    errorCode === 'calibration_active' ||
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

interface UpdateDialogProps {
  useStore: DesktopStoreHook;
}

export function UpdateDialog({ useStore }: UpdateDialogProps) {
  const update = useStore((store) => store.update);
  const close = useStore((store) => store.setUpdateDialogOpen);
  const check = useStore((store) => store.checkForUpdate);
  const handoff = useStore((store) => store.beginUpdateHandoff);
  const patchSettings = useStore((store) => store.patchSettings);

  if (!update.dialogOpen) return null;

  const busy =
    update.installRequestPending || ['downloading', 'ready', 'installing'].includes(update.state);
  const isChecking = update.checkRequestPending || update.state === 'checking';
  const isError = update.state === 'error' || Boolean(update.transportError);
  const isAvailable = !isError && update.state === 'available' && Boolean(update.availableVersion);
  const isCurrent = !isError && update.state === 'current';

  const effectiveRetryAction = update.transportError
    ? (update.transportErrorAction ?? 'check')
    : update.retryAction;

  const errorText = update.transportError
    ? formatUpdateError(null, update.transportError)
    : formatUpdateError(update.errorCode, update.errorDetail);

  const hasDeterminateProgress =
    busy && typeof update.progress?.total === 'number' && update.progress.total > 0;
  const progressValue = hasDeterminateProgress
    ? Math.min(update.progress?.total ?? 0, Math.max(0, update.progress?.completed ?? 0))
    : undefined;

  return (
    <ModalOverlay
      className="modal-backdrop"
      isOpen
      isDismissable={!busy}
      onOpenChange={(open) => {
        if (!open && !busy) close(false);
      }}
    >
      <Modal>
        <Dialog
          aria-label="Software update"
          className={`update-dialog${isError ? ' update-dialog-error' : ''}`}
        >
          <div className="dialog-heading update-dialog-heading">
            {isAvailable ? (
              <div className="update-header-lead">
                <div className="update-icon update-icon-accent" aria-hidden="true">
                  <Sparkles size={18} />
                </div>
                <div className="update-status" role="status" aria-live="polite">
                  <h2>Version {update.availableVersion} is available</h2>
                  <p className="update-version-subtitle">
                    You are running {update.currentVersion ?? 'the current version'} on the{' '}
                    {update.channel} channel.
                  </p>
                </div>
              </div>
            ) : (
              <h2>Software update</h2>
            )}
            <button
              className="icon-button icon-button-boxed update-close-button"
              type="button"
              aria-label="Close update"
              disabled={busy}
              onClick={() => close(false)}
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>
          {isAvailable ? (
            <>
              {update.releaseNotes && (
                <section
                  aria-labelledby="update-release-notes-heading"
                  className="update-notes-region"
                  role="region"
                  tabIndex={0}
                >
                  <h3 id="update-release-notes-heading">What’s new</h3>
                  <p className="update-notes">{update.releaseNotes}</p>
                </section>
              )}
              <div className="update-actions update-actions-split">
                <button
                  className="button button-ghost update-skip-action"
                  type="button"
                  disabled={busy}
                  onClick={async () => {
                    const targetVersion = update.availableVersion;
                    if (targetVersion) {
                      const result = await patchSettings({
                        updatePreferences: { skipVersion: targetVersion },
                      });
                      if (
                        result !== null &&
                        result.update_preferences.skip_version === targetVersion
                      ) {
                        close(false);
                      }
                    }
                  }}
                >
                  Skip this version
                </button>
                <div className="update-actions-primary-group">
                  <button
                    className="button"
                    type="button"
                    disabled={busy}
                    onClick={() => close(false)}
                  >
                    Later
                  </button>
                  <button
                    className="button button-primary"
                    type="button"
                    disabled={busy}
                    onClick={() => void handoff()}
                  >
                    <ArrowDownToLine size={15} aria-hidden="true" /> Update and restart
                  </button>
                </div>
              </div>
            </>
          ) : isError ? (
            <>
              <div className="update-state-row">
                <div className="update-icon update-icon-danger" aria-hidden="true">
                  <AlertCircle size={18} />
                </div>
                <div className="update-status" role="status" aria-live="polite">
                  <h3>
                    {getErrorHeading(
                      update.errorCode,
                      effectiveRetryAction,
                      Boolean(update.transportError),
                    )}
                  </h3>
                </div>
              </div>
              <p className="inline-error update-error-region">{errorText}</p>
              <div className="update-actions">
                <button
                  className="button"
                  type="button"
                  disabled={busy}
                  onClick={() => close(false)}
                >
                  Close
                </button>
                {effectiveRetryAction === 'check' && (
                  <button
                    className="button button-primary"
                    type="button"
                    disabled={isChecking || busy}
                    onClick={() => void check('manual')}
                  >
                    Check again
                  </button>
                )}
                {effectiveRetryAction === 'install' && (
                  <button
                    className="button button-primary"
                    type="button"
                    disabled={busy}
                    onClick={() => void handoff()}
                  >
                    Try again
                  </button>
                )}
              </div>
            </>
          ) : isChecking ? (
            <>
              <div className="update-state-row">
                <div className="update-icon update-icon-accent" aria-hidden="true">
                  <LoaderCircle className="spin" size={18} />
                </div>
                <div className="update-status" role="status" aria-live="polite">
                  <h3>Checking for updates…</h3>
                  <p className="muted">
                    Looking for available updates on the {update.channel} channel…
                  </p>
                </div>
              </div>
              <div className="update-actions">
                <button
                  className="button"
                  type="button"
                  disabled={busy}
                  onClick={() => close(false)}
                >
                  Close
                </button>
              </div>
            </>
          ) : isCurrent ? (
            <>
              <div className="update-state-row">
                <div className="update-icon update-icon-success" aria-hidden="true">
                  <Check size={18} />
                </div>
                <div className="update-status" role="status" aria-live="polite">
                  <h3>You're up to date</h3>
                  <p className="muted">
                    You are running version {update.currentVersion ?? 'the latest version'} on the{' '}
                    {update.channel} channel.
                  </p>
                </div>
              </div>
              <div className="update-actions">
                <button
                  className="button"
                  type="button"
                  disabled={busy}
                  onClick={() => close(false)}
                >
                  Close
                </button>
              </div>
            </>
          ) : (
            <>
              <div className="update-state-row">
                <div
                  className={`update-icon ${update.state === 'ready' ? 'update-icon-success' : 'update-icon-accent'}`}
                  aria-hidden="true"
                >
                  {busy && update.state !== 'ready' ? (
                    <LoaderCircle className="spin" size={18} />
                  ) : (
                    <Check size={18} />
                  )}
                </div>
                <div className="update-status" role="status" aria-live="polite">
                  <h3>
                    {update.state === 'downloading'
                      ? 'Downloading update…'
                      : update.state === 'ready'
                        ? 'Update ready to install'
                        : update.state === 'installing'
                          ? 'Installing update…'
                          : 'No update available'}
                  </h3>
                  <p className="muted">
                    {busy
                      ? `${update.progress?.message || 'The update is being applied.'}${update.progress?.total ? ` (${update.progress.completed}/${update.progress.total} bytes)` : ''}`
                      : 'The installed application is up to date.'}
                  </p>
                </div>
              </div>
              {busy && (
                <div className="update-progress-wrap" aria-hidden="true">
                  {hasDeterminateProgress ? (
                    <progress
                      className="update-progress-bar"
                      value={progressValue}
                      max={update.progress?.total ?? 100}
                    />
                  ) : (
                    <div className="update-progress-indeterminate">
                      <span />
                    </div>
                  )}
                </div>
              )}
              {!busy && (
                <div className="update-actions">
                  <button className="button" type="button" onClick={() => close(false)}>
                    Close
                  </button>
                </div>
              )}
            </>
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
