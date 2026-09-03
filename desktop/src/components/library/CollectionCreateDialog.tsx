import { Dialog, Modal, ModalOverlay } from 'react-aria-components';
import { useState } from 'react';

interface CollectionCreateDialogProps {
  open: boolean;
  pending: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string) => Promise<void>;
}

export function CollectionCreateDialog({
  open,
  pending,
  error,
  onOpenChange,
  onSubmit,
}: CollectionCreateDialogProps) {
  const [name, setName] = useState('');
  const [validationError, setValidationError] = useState<string | null>(null);

  const submit = async () => {
    const trimmed = name.trim();
    if (!trimmed) {
      setValidationError('Enter a collection name.');
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
        <Dialog aria-label="New collection">
          <div className="dialog-heading">
            <div>
              <p className="eyebrow">YOUR LIBRARY</p>
              <h2>New collection</h2>
            </div>
            <button
              className="icon-button"
              type="button"
              aria-label="Close new collection dialog"
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
              Collection name
              <input
                autoFocus
                value={name}
                onChange={(event) => setName(event.target.value)}
                aria-invalid={validationError !== null}
                aria-describedby={validationError ? 'collection-create-error' : undefined}
              />
            </label>
            {(validationError || error) && (
              <p id="collection-create-error" className="inline-error" role="alert">
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
                {pending ? 'Creating…' : 'Create'}
              </button>
            </div>
          </form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
