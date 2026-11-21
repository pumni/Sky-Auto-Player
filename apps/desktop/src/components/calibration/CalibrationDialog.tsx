import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { AlertCircle, Check, Gauge, LoaderCircle, X, XCircle } from 'lucide-react';
import type { ReactNode, RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';

interface CalibrationDialogProps {
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  useStore: DesktopStoreHook;
}

export function CalibrationDialog({ settingsTriggerRef, useStore }: CalibrationDialogProps) {
  const calibration = useStore((store) => store.calibration);
  const setOpen = useStore((store) => store.setCalibrationOpen);
  const start = useStore((store) => store.startCalibration);
  const cancel = useStore((store) => store.cancelCalibration);
  const close = () => {
    setOpen(false);
    queueMicrotask(() => settingsTriggerRef?.current?.focus());
  };
  const running = ['starting', 'running', 'cancelling'].includes(calibration.state);
  if (!calibration.open) return null;

  const badgeTone =
    calibration.state === 'succeeded'
      ? 'update-icon-success'
      : calibration.state === 'failed'
        ? 'update-icon-danger'
        : calibration.state === 'cancelled'
          ? 'calibration-icon-warning'
          : 'update-icon-accent';

  const badgeIcon =
    calibration.state === 'succeeded' ? (
      <Check size={17} aria-hidden="true" />
    ) : calibration.state === 'failed' ? (
      <AlertCircle size={17} aria-hidden="true" />
    ) : calibration.state === 'cancelled' ? (
      <XCircle size={17} aria-hidden="true" />
    ) : running ? (
      <LoaderCircle className="spin" size={17} aria-hidden="true" />
    ) : (
      <Gauge size={17} aria-hidden="true" />
    );

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
        <Dialog aria-label="Timing calibration" className="calibration-dialog">
          <div className="dialog-heading calibration-dialog-heading">
            <div className="calibration-header-lead">
              <span className={`update-icon ${badgeTone}`} aria-hidden="true">
                {badgeIcon}
              </span>
              <div className="calibration-header-copy">
                <p className="eyebrow">NATIVE TIMING</p>
                <h2>Calibration</h2>
              </div>
            </div>
            {!running && (
              <button
                className="icon-button icon-button-boxed"
                type="button"
                aria-label="Close calibration"
                title="Close calibration"
                onClick={close}
              >
                <X size={16} aria-hidden="true" />
              </button>
            )}
          </div>

          {calibration.state === 'idle' && (
            <div className="calibration-body">
              <p className="calibration-description">
                Measure the native dispatch path on this machine before trusting tight timing
                margins.
              </p>
              <div className="calibration-actions">
                <button
                  className="button button-primary"
                  type="button"
                  onClick={() => void start('quick')}
                >
                  Start quick calibration
                </button>
              </div>
            </div>
          )}

          {running && (
            <div className="calibration-body" aria-live="polite">
              <div className="calibration-status-card">
                <div className="calibration-status-meta">
                  <strong>{calibration.phase || 'Preparing'}</strong>
                  <span className="calibration-progress-count">
                    {calibration.total
                      ? `${calibration.completed} of ${calibration.total}`
                      : 'Starting…'}
                  </span>
                </div>
                {calibration.message && (
                  <p className="calibration-description">{calibration.message}</p>
                )}
                {calibration.total ? (
                  <progress
                    className="update-progress-bar"
                    value={calibration.completed}
                    max={calibration.total}
                  />
                ) : (
                  <div className="update-progress-indeterminate" aria-hidden="true">
                    <span />
                  </div>
                )}
              </div>
              <div className="calibration-actions">
                <button
                  className="button"
                  type="button"
                  disabled={calibration.state === 'cancelling'}
                  onClick={() => void cancel()}
                >
                  {calibration.state === 'cancelling' ? 'Cancelling…' : 'Cancel'}
                </button>
              </div>
            </div>
          )}

          {calibration.state === 'succeeded' && (
            <TerminalState
              tone="success"
              icon={<Check size={16} aria-hidden="true" />}
              title="Calibration complete"
              message={calibration.result?.message ?? calibration.message}
              onClose={close}
            />
          )}

          {calibration.state === 'failed' && (
            <TerminalState
              tone="danger"
              icon={<AlertCircle size={16} aria-hidden="true" />}
              title="Calibration failed"
              message={calibration.error ?? calibration.message}
              onClose={close}
            />
          )}

          {calibration.state === 'cancelled' && (
            <TerminalState
              tone="warning"
              icon={<XCircle size={16} aria-hidden="true" />}
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
  tone,
  icon,
  title,
  message,
  onClose,
}: {
  tone: 'success' | 'danger' | 'warning';
  icon: ReactNode;
  title: string;
  message: string;
  onClose: () => void;
}) {
  return (
    <div className="calibration-body" role="status">
      <div className={`calibration-result-card calibration-result-card--${tone}`}>
        <div className="calibration-result-heading">
          <span className={`calibration-result-icon calibration-result-icon--${tone}`}>{icon}</span>
          <h3>{title}</h3>
        </div>
        <p className="calibration-description">{message}</p>
      </div>
      <div className="calibration-actions">
        <button className="button button-primary" type="button" onClick={onClose}>
          Close
        </button>
      </div>
    </div>
  );
}
