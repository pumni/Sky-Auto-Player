import { ScrollArea as ScrollAreaPrimitive } from '@base-ui/react/scroll-area';
import { cn } from 'cn';

function ScrollAreaRoot({ className, ...props }: ScrollAreaPrimitive.Root.Props) {
  return (
    <ScrollAreaPrimitive.Root
      data-slot="scroll-area"
      className={cn(
        'group/scroll-area relative grid grid-cols-[minmax(0,1fr)] grid-rows-[minmax(0,auto)] min-h-0 min-w-0 overflow-hidden',
        className,
      )}
      {...props}
    />
  );
}

function ScrollAreaViewport({ className, ...props }: ScrollAreaPrimitive.Viewport.Props) {
  return (
    <ScrollAreaPrimitive.Viewport
      data-slot="scroll-area-viewport"
      className={cn(
        'col-start-1 row-start-1 overscroll-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden',
        className,
      )}
      {...props}
    />
  );
}

function ScrollAreaContent({ className, style, ...props }: ScrollAreaPrimitive.Content.Props) {
  return (
    <ScrollAreaPrimitive.Content
      data-slot="scroll-area-content"
      className={cn('min-w-0', className)}
      // Base UI defaults this measurement wrapper to fit-content. App panels need to wrap text
      // within the viewport; callers can still opt into horizontal content with w-max.
      style={{ minWidth: 0, ...style }}
      {...props}
    />
  );
}

function ScrollAreaScrollbar({
  className,
  onPointerDownCapture,
  onPointerUpCapture,
  onPointerCancelCapture,
  onLostPointerCapture,
  ...props
}: ScrollAreaPrimitive.Scrollbar.Props) {
  const beginDrag: NonNullable<ScrollAreaPrimitive.Scrollbar.Props['onPointerDownCapture']> = (
    event,
  ) => {
    onPointerDownCapture?.(event);
    event.currentTarget.dataset.dragging = 'true';
  };
  const endDrag: NonNullable<ScrollAreaPrimitive.Scrollbar.Props['onPointerUpCapture']> = (
    event,
  ) => {
    onPointerUpCapture?.(event);
    delete event.currentTarget.dataset.dragging;
  };
  const cancelDrag: NonNullable<ScrollAreaPrimitive.Scrollbar.Props['onPointerCancelCapture']> = (
    event,
  ) => {
    onPointerCancelCapture?.(event);
    delete event.currentTarget.dataset.dragging;
  };
  const loseCapture: NonNullable<ScrollAreaPrimitive.Scrollbar.Props['onLostPointerCapture']> = (
    event,
  ) => {
    onLostPointerCapture?.(event);
    delete event.currentTarget.dataset.dragging;
  };

  return (
    <ScrollAreaPrimitive.Scrollbar
      data-slot="scroll-area-scrollbar"
      className={cn(
        'z-10 flex touch-none select-none bg-transparent p-[2px] opacity-0 pointer-events-none transition-opacity duration-scrollbar-exit ease-standard data-[scrolling]:opacity-100 data-[scrolling]:pointer-events-auto data-[scrolling]:duration-scrollbar-enter data-[dragging=true]:opacity-100 data-[dragging=true]:pointer-events-auto data-[dragging=true]:duration-scrollbar-enter forced-colors:opacity-100 forced-colors:pointer-events-auto motion-reduce:transition-none data-[orientation=vertical]:w-[10px] data-[orientation=vertical]:flex-col data-[orientation=horizontal]:h-[10px] data-[orientation=horizontal]:flex-row',
        className,
      )}
      onPointerDownCapture={beginDrag}
      onPointerUpCapture={endDrag}
      onPointerCancelCapture={cancelDrag}
      onLostPointerCapture={loseCapture}
      {...props}
    />
  );
}

function ScrollAreaThumb({ className, ...props }: ScrollAreaPrimitive.Thumb.Props) {
  return (
    <ScrollAreaPrimitive.Thumb
      data-slot="scroll-area-thumb"
      className={cn(
        'relative rounded-full bg-scrollbar-thumb transition-colors duration-fast hover:bg-scrollbar-thumb-active data-[scrolling]:bg-scrollbar-thumb-active motion-reduce:transition-none',
        className,
      )}
      {...props}
    />
  );
}

function ScrollAreaCorner({ className, ...props }: ScrollAreaPrimitive.Corner.Props) {
  return (
    <ScrollAreaPrimitive.Corner
      data-slot="scroll-area-corner"
      className={cn('bg-transparent', className)}
      {...props}
    />
  );
}

export {
  ScrollAreaContent,
  ScrollAreaCorner,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
};
