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

function ScrollAreaScrollbar({ className, ...props }: ScrollAreaPrimitive.Scrollbar.Props) {
  return (
    <ScrollAreaPrimitive.Scrollbar
      data-slot="scroll-area-scrollbar"
      className={cn(
        'z-10 flex touch-none select-none bg-transparent p-[2px] opacity-75 transition-opacity duration-[120ms] ease-standard data-[hovering]:opacity-100 data-[scrolling]:opacity-100 group-focus-within/scroll-area:opacity-100 data-[orientation=vertical]:w-[10px] data-[orientation=vertical]:flex-col data-[orientation=horizontal]:h-[10px] data-[orientation=horizontal]:flex-row',
        className,
      )}
      {...props}
    />
  );
}

function ScrollAreaThumb({ className, ...props }: ScrollAreaPrimitive.Thumb.Props) {
  return (
    <ScrollAreaPrimitive.Thumb
      data-slot="scroll-area-thumb"
      className={cn(
        'relative rounded-full bg-muted-foreground transition-colors duration-[120ms] hover:bg-foreground data-[scrolling]:bg-foreground',
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
