import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { useState } from 'react';

interface PlaylistRenameDialogProps {
  open: boolean;
  initialName: string;
  pending: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string) => Promise<void>;
}

export function PlaylistRenameDialog({
  open,
  initialName,
  pending,
  error,
  onOpenChange,
  onSubmit,
}: PlaylistRenameDialogProps) {
  const [name, setName] = useState(initialName);
  const [validationError, setValidationError] = useState<string | null>(null);

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setValidationError('Enter a playlist name.');
      return;
    }
    setValidationError(null);
    try {
      await onSubmit(trimmed);
      onOpenChange(false);
    } catch {
      // The store keeps the safe native error in the navigator state.
    }
  };

  return (
    <ModalOverlay
      isOpen={open}
      isDismissable={!pending}
      onOpenChange={onOpenChange}
      className="modal-backdrop"
    >
      <Modal className="library-dialog">
        <Dialog aria-label={`Rename ${initialName}`}>
          <div className="dialog-heading">
            <div>
              <p className="eyebrow">PLAYLIST</p>
              <h2>Rename playlist</h2>
            </div>
            <button
              className="icon-button"
              type="button"
              aria-label="Close rename dialog"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              ×
            </button>
          </div>
          <form
            className="library-dialog-form"
            onSubmit={(event) => {
              event.preventDefault();
              void submit();
            }}
          >
            <label>
              Playlist name
              <input
                autoFocus
                value={name}
                onChange={(event) => setName(event.target.value)}
                aria-invalid={validationError !== null}
              />
            </label>
            {(validationError || error) && (
              <p className="inline-error" role="alert">
                {validationError ?? error}
              </p>
            )}
            <div className="library-dialog-actions">
              <button
                className="button"
                type="button"
                onClick={() => onOpenChange(false)}
                disabled={pending}
              >
                Cancel
              </button>
              <button className="button button-primary" type="submit" disabled={pending}>
                {pending ? 'Saving…' : 'Save'}
              </button>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
