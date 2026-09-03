import { Dialog, Modal, ModalOverlay } from 'react-aria-components';

interface LibraryConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  confirmLabel: string;
  pending: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: () => Promise<void>;
}

export function LibraryConfirmDialog({
  open,
  title,
  message,
  confirmLabel,
  pending,
  onOpenChange,
  onConfirm,
}: LibraryConfirmDialogProps) {
  return (
    <ModalOverlay
      isOpen={open}
      isDismissable={!pending}
      onOpenChange={onOpenChange}
      className="modal-backdrop"
    >
      <Modal className="library-dialog library-confirm-dialog">
        <Dialog aria-label={title}>
          <div className="dialog-heading">
            <div>
              <p className="eyebrow">CONFIRM ACTION</p>
              <h2>{title}</h2>
            </div>
            <button
              className="icon-button"
              type="button"
              aria-label="Close confirmation dialog"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              ×
            </button>
          </div>
          <p className="library-confirm-message">{message}</p>
          <div className="library-dialog-actions">
            <button
              className="button"
              type="button"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              Cancel
            </button>
            <button
              className="button button-danger"
              type="button"
              onClick={() => {
                void (async () => {
                  try {
                    await onConfirm();
                    onOpenChange(false);
                  } catch {
                    // The mutation surface reports the error inline; keep the dialog open.
                  }
                })();
              }}
              disabled={pending}
            >
              {pending ? 'Working…' : confirmLabel}
            </button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
