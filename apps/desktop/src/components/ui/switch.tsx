'use client';

import { Switch as SwitchPrimitive } from '@base-ui/react/switch';
import { cn } from 'cn';

function Switch({
  className,
  size = 'default',
  ...props
}: SwitchPrimitive.Root.Props & {
  size?: 'sm' | 'default';
}) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      className={cn(
        'peer group/switch relative inline-flex h-5 w-9 shrink-0 items-center rounded-full border border-transparent bg-input transition-colors duration-fast outline-none after:absolute after:-inset-x-2 after:-inset-y-2 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background aria-invalid:ring-2 aria-invalid:ring-destructive data-checked:bg-primary data-disabled:cursor-not-allowed data-disabled:opacity-50',
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className="pointer-events-none block size-4 translate-x-0 rounded-full bg-background shadow-sm transition-transform duration-fast group-data-[size=default]/switch:data-checked:translate-x-4"
      />
    </SwitchPrimitive.Root>
  );
}

export { Switch };
