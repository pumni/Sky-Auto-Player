'use client';

import { Dialog as DialogPrimitive } from '@base-ui/react/dialog';
import { cn } from 'cn';

import { Button } from '@/components/ui/button';
import { XIcon } from 'lucide-react';

function Dialog({ ...props }: DialogPrimitive.Root.Props) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}

function DialogPortal({ ...props }: DialogPrimitive.Portal.Props) {
  return <DialogPrimitive.Portal data-slot="dialog-portal" {...props} />;
}

function DialogOverlay({ className, ...props }: DialogPrimitive.Backdrop.Props) {
  return (
    <DialogPrimitive.Backdrop
      data-slot="dialog-overlay"
      className={cn(
        'fixed inset-0 isolate z-50 bg-scrim/55 transition-opacity duration-standard ease-standard data-[starting-style]:opacity-0 data-[starting-style]:duration-[200ms] data-[ending-style]:opacity-0 data-[ending-style]:duration-[140ms] data-[ending-style]:ease-exit motion-reduce:transition-none',
        className,
      )}
      {...props}
    />
  );
}

function DialogContent({
  className,
  children,
  showCloseButton = true,
  ...props
}: DialogPrimitive.Popup.Props & {
  showCloseButton?: boolean;
}) {
  return (
    <DialogPortal>
      <DialogOverlay />
      <DialogPrimitive.Popup
        data-slot="dialog-content"
        className={cn(
          'fixed top-1/2 left-1/2 z-50 grid w-[min(560px,calc(100%-2rem))] -translate-x-1/2 -translate-y-1/2 gap-4 rounded-dialog border border-border bg-popover p-6 text-sm text-popover-foreground shadow-2xl outline-none transition-[opacity,scale,translate] duration-standard ease-enter data-[starting-style]:scale-[0.98] data-[starting-style]:opacity-0 data-[starting-style]:duration-[200ms] data-[ending-style]:scale-[0.98] data-[ending-style]:opacity-0 data-[ending-style]:duration-[140ms] data-[ending-style]:ease-exit motion-reduce:transition-none',
          className,
        )}
        {...props}
      >
        {children}
        {showCloseButton && (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            render={<Button variant="ghost" className="absolute top-4 right-4" size="icon-sm" />}
          >
            <XIcon />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Popup>
    </DialogPortal>
  );
}

function DialogTitle({ className, ...props }: DialogPrimitive.Title.Props) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn('text-2xl font-semibold leading-8', className)}
      {...props}
    />
  );
}

export { Dialog, DialogContent, DialogTitle };
