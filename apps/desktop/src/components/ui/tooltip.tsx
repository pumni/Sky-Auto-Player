'use client';

import { Tooltip as TooltipPrimitive } from '@base-ui/react/tooltip';
import { cn } from 'cn';

function TooltipProvider({
  delay = 350,
  closeDelay = 80,
  timeout = 450,
  ...props
}: TooltipPrimitive.Provider.Props) {
  return (
    <TooltipPrimitive.Provider
      data-slot="tooltip-provider"
      delay={delay}
      closeDelay={closeDelay}
      timeout={timeout}
      {...props}
    />
  );
}

function Tooltip({ ...props }: TooltipPrimitive.Root.Props) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />;
}

function TooltipTrigger({ ...props }: TooltipPrimitive.Trigger.Props) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

function TooltipContent({
  className,
  side = 'top',
  sideOffset = 6,
  align = 'center',
  alignOffset = 0,
  children,
  ...props
}: TooltipPrimitive.Popup.Props &
  Pick<TooltipPrimitive.Positioner.Props, 'align' | 'alignOffset' | 'side' | 'sideOffset'>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Positioner
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
        className="isolate z-50"
      >
        <TooltipPrimitive.Popup
          role="tooltip"
          data-slot="tooltip-content"
          className={cn(
            'z-50 inline-flex w-fit max-w-[min(320px,calc(100vw-32px))] origin-(--transform-origin) items-center gap-1.5 break-words rounded-control border border-overlay-border bg-popover px-2.5 py-1.5 text-xs leading-[1.4] text-popover-foreground shadow-lg transition-opacity duration-[100ms] ease-enter data-[starting-style]:opacity-0 data-[ending-style]:opacity-0 data-[ending-style]:duration-[80ms] data-[ending-style]:ease-exit motion-reduce:transition-none',
            className,
          )}
          {...props}
        >
          {children}
        </TooltipPrimitive.Popup>
      </TooltipPrimitive.Positioner>
    </TooltipPrimitive.Portal>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
