import { useState } from 'react';
import { Button } from '../ui/button';
import { Dialog, DialogContent, DialogTitle } from '../ui/dialog';
import { Input } from '../ui/input';

interface PlaylistCreateDialogProps {
  open: boolean;
  pending: boolean;
  error: string | null;
  onOpenChange: (open: boolean) => void;
  onSubmit: (name: string) => Promise<void>;
}

export function PlaylistCreateDialog({
  open,
  pending,
  error,
  onOpenChange,
  onSubmit,
}: PlaylistCreateDialogProps) {
  const [name, setName] = useState('');
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
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!pending || nextOpen) onOpenChange(nextOpen);
      }}
    >
      <DialogContent
        className="w-[min(420px,calc(100vw-32px))] gap-0 rounded-dialog border-border bg-card p-5"
        aria-labelledby="playlist-create-title"
        showCloseButton={false}
      >
        <div className="mb-4 flex items-start justify-between gap-4">
          <div className="grid gap-px">
            <p className="m-0 text-[11px] font-semibold tracking-wide text-text-tertiary">
              YOUR LIBRARY
            </p>
            <DialogTitle
              className="m-0 text-2xl font-semibold leading-8"
              id="playlist-create-title"
            >
              New playlist
            </DialogTitle>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-8 border-0 bg-transparent p-0 hover:bg-control-hover"
            type="button"
            aria-label="Close new playlist dialog"
            onClick={() => onOpenChange(false)}
            disabled={pending}
          >
            <span className="text-xl leading-none" aria-hidden="true">
              ×
            </span>
          </Button>
        </div>
        <form
          className="grid min-w-0 gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <label className="grid gap-1.5 text-xs font-medium text-text-secondary">
            Playlist name
            <Input
              className="mt-1"
              autoFocus
              aria-label="Playlist name"
              value={name}
              onChange={(event) => setName(event.target.value)}
              aria-invalid={validationError !== null}
              aria-describedby={validationError ? 'playlist-create-error' : undefined}
            />
          </label>
          {(validationError || error) && (
            <p id="playlist-create-error" className="m-0 text-xs text-danger" role="alert">
              {validationError ?? error}
            </p>
          )}
          <div className="mt-2 flex justify-end gap-2">
            <Button
              variant="outline"
              className="h-9"
              type="button"
              onClick={() => onOpenChange(false)}
              disabled={pending}
            >
              Cancel
            </Button>
            <Button type="submit" disabled={pending}>
              {pending ? 'Creating…' : 'Create'}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
