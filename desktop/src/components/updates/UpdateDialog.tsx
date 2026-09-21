import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { Check, Download, LoaderCircle, X } from 'lucide-react';
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
        <Dialog aria-label="Software update" className="settings-dialog update-dialog">
          <div className="dialog-heading">
            <div>
              <p className="eyebrow">APPLICATION UPDATE</p>
              <h2>Update</h2>
            </div>
            <button
              className="icon-button icon-button-boxed"
              type="button"
              aria-label="Close update"
              disabled={busy}
              onClick={() => close(false)}
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>
          <div className="update-status" role="status" aria-live="polite">
            {isChecking ? (
              <>
                <div className="update-icon" aria-hidden="true">
                  <LoaderCircle className="spin" size={20} />
                </div>
                <h3>Checking for updates…</h3>
                <p className="muted">
                  Looking for available updates on the {update.channel} channel…
                </p>
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
            ) : isError ? (
              <>
                <h3>
                  {getErrorHeading(
                    update.errorCode,
                    effectiveRetryAction,
                    Boolean(update.transportError),
                  )}
                </h3>
                <p className="inline-error">{errorText}</p>
                <div className="update-actions">
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
            ) : isAvailable ? (
              <>
                <div className="update-icon" aria-hidden="true">
                  <Download size={20} />
                </div>
                <h3>Version {update.availableVersion} is available</h3>
                <p>
                  You are running {update.currentVersion ?? 'the current version'} on the{' '}
                  {update.channel} channel.
                </p>
                {update.releaseNotes && <p className="update-notes">{update.releaseNotes}</p>}
                <div className="update-actions">
                  <button
                    className="button button-primary"
                    type="button"
                    disabled={busy}
                    onClick={() => void handoff()}
                  >
                    <Check size={15} aria-hidden="true" /> Update and restart
                  </button>
                  <button
                    className="button"
                    type="button"
                    disabled={busy}
                    onClick={() => close(false)}
                  >
                    Later
                  </button>
                  <button
                    className="button button-ghost"
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
                </div>
              </>
            ) : isCurrent ? (
              <>
                <div className="update-icon" aria-hidden="true">
                  <Check size={20} />
                </div>
                <h3>You're up to date</h3>
                <p className="muted">
                  You are running version {update.currentVersion ?? 'the latest version'} on the{' '}
                  {update.channel} channel.
                </p>
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
                {!busy && (
                  <div className="update-actions">
                    <button className="button" type="button" onClick={() => close(false)}>
                      Close
                    </button>
                  </div>
                )}
              </>
            )}
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
