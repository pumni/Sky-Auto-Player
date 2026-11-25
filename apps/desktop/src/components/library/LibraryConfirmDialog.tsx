import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '../ui/alert-dialog';

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
    <AlertDialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!pending || nextOpen) onOpenChange(nextOpen);
      }}
    >
      <AlertDialogContent className="w-[min(520px,calc(100vw-32px))] gap-4 p-6">
        <AlertDialogHeader className="items-start text-left">
          <p className="m-0 text-xs font-semibold leading-4 text-text-tertiary">Confirm action</p>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription className="text-sm leading-relaxed text-muted-foreground">
            {message}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter className="mt-2 flex justify-end gap-2">
          <AlertDialogCancel variant="outline" disabled={pending}>
            Cancel
          </AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
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
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
