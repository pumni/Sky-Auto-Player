import { Tabs as TabsPrimitive } from '@base-ui/react/tabs';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from 'cn';

function Tabs({ className, orientation = 'horizontal', ...props }: TabsPrimitive.Root.Props) {
  return (
    <TabsPrimitive.Root
      data-slot="tabs"
      data-orientation={orientation}
      className={cn('group/tabs flex gap-2 data-horizontal:flex-col', className)}
      {...props}
    />
  );
}

const tabsListVariants = cva(
  'group/tabs-list relative inline-flex w-fit items-center justify-center rounded-control p-1 text-muted-foreground group-data-horizontal/tabs:h-9 group-data-vertical/tabs:h-fit group-data-vertical/tabs:flex-col',
  {
    variants: {
      variant: {
        default: 'bg-muted',
        line: 'gap-1 rounded-none border-b border-border bg-transparent p-0',
        segmented: 'gap-0.5 rounded-control bg-muted p-1',
      },
    },
    defaultVariants: {
      variant: 'default',
    },
  },
);

function TabsList({
  className,
  variant = 'default',
  ...props
}: TabsPrimitive.List.Props & VariantProps<typeof tabsListVariants>) {
  return (
    <TabsPrimitive.List
      data-slot="tabs-list"
      data-variant={variant}
      className={cn('relative', tabsListVariants({ variant }), className)}
      {...props}
    />
  );
}

function TabsIndicator({
  className,
  variant = 'line',
  ...props
}: TabsPrimitive.Indicator.Props & { variant?: 'line' | 'segmented' }) {
  return (
    <TabsPrimitive.Indicator
      data-slot="tabs-indicator"
      data-variant={variant}
      className={cn(
        'pointer-events-none absolute left-[var(--active-tab-left)] z-0 w-[var(--active-tab-width)] transition-[left,width,top,bottom,height] duration-tab ease-standard motion-reduce:transition-none',
        variant === 'line'
          ? 'bottom-0 h-0.5 bg-accent-ink'
          : 'inset-y-1 rounded-control border border-border bg-background',
        className,
      )}
      {...props}
    />
  );
}

function TabsTrigger({ className, ...props }: TabsPrimitive.Tab.Props) {
  return (
    <TabsPrimitive.Tab
      data-slot="tabs-trigger"
      className={cn(
        "relative z-[1] inline-flex min-h-8 flex-1 items-center justify-center gap-1.5 rounded-control border border-transparent px-2 py-1 text-sm font-medium whitespace-nowrap text-muted-foreground transition-colors duration-tab group-data-vertical/tabs:w-full group-data-vertical/tabs:justify-start hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 motion-reduce:transition-none",
        'group-data-[variant=line]/tabs-list:rounded-none group-data-[variant=line]/tabs-list:bg-transparent group-data-[variant=line]/tabs-list:data-active:bg-transparent group-data-[variant=line]/tabs-list:data-active:text-accent-ink group-data-[variant=line]/tabs-list:data-active:shadow-none',
        'group-data-[variant=segmented]/tabs-list:data-active:bg-transparent group-data-[variant=segmented]/tabs-list:data-active:text-accent-ink group-data-[variant=segmented]/tabs-list:data-active:shadow-none',
        'data-active:bg-background data-active:text-foreground data-active:shadow-sm',
        className,
      )}
      {...props}
    />
  );
}

function TabsContent({ className, ...props }: TabsPrimitive.Panel.Props) {
  return (
    <TabsPrimitive.Panel
      data-slot="tabs-content"
      className={cn('flex-1 text-sm outline-none', className)}
      {...props}
    />
  );
}

export { Tabs, TabsContent, TabsIndicator, TabsList, TabsTrigger };
