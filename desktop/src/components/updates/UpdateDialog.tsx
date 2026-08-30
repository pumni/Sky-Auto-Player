import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { Check, Download, X } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';

interface UpdateDialogProps {
  useStore: DesktopStoreHook;
}

export function UpdateDialog({ useStore }: UpdateDialogProps) {
  const update = useStore((store) => store.update);
  const close = useStore((store) => store.setUpdateDialogOpen);
  const check = useStore((store) => store.checkForUpdate);
  const handoff = useStore((store) => store.beginUpdateHandoff);
  if (!update.dialogOpen) return null;
  const available = update.state === 'available' && update.availableVersion;
  const busy = update.state === 'checking' || update.state === 'handoff_in_progress';
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
              className="icon-button"
              type="button"
              aria-label="Close update"
              disabled={busy}
              onClick={() => close(false)}
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>
          <div className="update-status" role="status" aria-live="polite">
            {available ? (
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
                    onClick={() => void handoff()}
                  >
                    <Check size={15} aria-hidden="true" /> Update and restart
                  </button>
                  <button className="button" type="button" onClick={() => close(false)}>
                    Later
                  </button>
                </div>
              </>
            ) : update.state === 'error' ? (
              <>
                <h3>Update check failed</h3>
                <p className="inline-error">
                  {update.error ?? 'The update service is unavailable.'}
                </p>
                <div className="update-actions">
                  <button
                    className="button button-primary"
                    type="button"
                    onClick={() => void check()}
                  >
                    Check again
                  </button>
                  <button className="button" type="button" onClick={() => close(false)}>
                    Close
                  </button>
                </div>
              </>
            ) : (
              <>
                <h3>
                  {update.state === 'handoff_in_progress'
                    ? 'Preparing restart…'
                    : update.state === 'handoff_ready'
                      ? 'Restart handoff ready'
                      : 'No update available'}
                </h3>
                <p className="muted">
                  {update.state === 'handoff_in_progress'
                    ? 'The verified native updater is preparing its handoff.'
                    : update.state === 'handoff_ready'
                      ? 'The verified updater is ready. The application will close and restart.'
                      : 'The installed application is up to date.'}
                </p>
                {!busy && (
                  <button className="button" type="button" onClick={() => close(false)}>
                    Close
                  </button>
                )}
              </>
            )}
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
