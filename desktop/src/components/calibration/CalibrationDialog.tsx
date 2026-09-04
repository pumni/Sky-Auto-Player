import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { CheckCircle2, LoaderCircle, XCircle } from 'lucide-react';
import type { ReactNode } from 'react';
import type { DesktopStoreHook } from '../../state/store';

interface CalibrationDialogProps {
  useStore: DesktopStoreHook;
}

export function CalibrationDialog({ useStore }: CalibrationDialogProps) {
  const calibration = useStore((store) => store.calibration);
  const setOpen = useStore((store) => store.setCalibrationOpen);
  const start = useStore((store) => store.startCalibration);
  const cancel = useStore((store) => store.cancelCalibration);
  const close = () => {
    setOpen(false);
    queueMicrotask(() =>
      document.querySelector<HTMLElement>('[aria-label="Open settings"]')?.focus(),
    );
  };
  const running = ['starting', 'running', 'cancelling'].includes(calibration.state);
  if (!calibration.open) return null;
  return (
    <ModalOverlay
      className="modal-backdrop"
      isOpen
      isDismissable={!running}
      onOpenChange={(open) => {
        if (!open && !running) close();
      }}
    >
      <Modal>
        <Dialog aria-label="Timing calibration" className="settings-dialog calibration-dialog">
          <div className="dialog-heading">
            <div>
              <p className="eyebrow">NATIVE TIMING</p>
              <h2>Calibration</h2>
            </div>
            {!running && (
              <button
                className="icon-button"
                type="button"
                aria-label="Close calibration"
                onClick={close}
              >
                ×
              </button>
            )}
          </div>
          {calibration.state === 'idle' && (
            <div className="calibration-copy">
              <p>
                Measure the native dispatch path on this machine before trusting tight timing
                margins.
              </p>
              <button
                className="button button-primary"
                type="button"
                onClick={() => void start('quick')}
              >
                Start quick calibration
              </button>
            </div>
          )}
          {running && (
            <div className="calibration-copy" aria-live="polite">
              <LoaderCircle className="spin" size={22} aria-hidden="true" />
              <p>
                <strong>{calibration.phase || 'Preparing'}</strong>
                <br />
                {calibration.message}
              </p>
              <progress
                value={calibration.total ? calibration.completed : undefined}
                max={calibration.total || undefined}
              />
              <span className="muted">
                {calibration.total
                  ? `${calibration.completed} of ${calibration.total}`
                  : 'Starting…'}
              </span>
              <button
                className="button"
                type="button"
                disabled={calibration.state === 'cancelling'}
                onClick={() => void cancel()}
              >
                {calibration.state === 'cancelling' ? 'Cancelling…' : 'Cancel'}
              </button>
            </div>
          )}
          {calibration.state === 'succeeded' && (
            <TerminalState
              icon={<CheckCircle2 size={24} aria-hidden="true" />}
              title="Calibration complete"
              message={calibration.result?.message ?? calibration.message}
              onClose={close}
            />
          )}
          {calibration.state === 'failed' && (
            <TerminalState
              icon={<XCircle size={24} aria-hidden="true" />}
              title="Calibration failed"
              message={calibration.error ?? calibration.message}
              onClose={close}
            />
          )}
          {calibration.state === 'cancelled' && (
            <TerminalState
              icon={<XCircle size={24} aria-hidden="true" />}
              title="Calibration cancelled"
              message={calibration.message || 'No changes were applied.'}
              onClose={close}
            />
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function TerminalState({
  icon,
  title,
  message,
  onClose,
}: {
  icon: ReactNode;
  title: string;
  message: string;
  onClose: () => void;
}) {
  return (
    <div className="calibration-copy" role="status">
      <div className="calibration-result-icon">{icon}</div>
      <h3>{title}</h3>
      <p>{message}</p>
      <button className="button button-primary" type="button" onClick={onClose}>
        Close
      </button>
    </div>
  );
}
