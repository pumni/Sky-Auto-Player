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
  'group/tabs-list relative h-9 w-full min-w-0 items-center justify-center text-muted-foreground group-data-vertical/tabs:h-fit',
  {
    variants: {
      variant: {
        line: 'grid grid-flow-col auto-cols-fr justify-start gap-0 rounded-none border-b border-border bg-transparent p-0',
        segmented:
          'flex h-[34px] gap-0.5 rounded-control bg-secondary p-[3px] group-data-vertical/tabs:flex-col',
      },
    },
  },
);

function TabsList({
  className,
  variant,
  ...props
}: TabsPrimitive.List.Props &
  VariantProps<typeof tabsListVariants> & {
    variant: 'line' | 'segmented';
  }) {
  return (
    <TabsPrimitive.List
      data-slot="tabs-list"
      data-variant={variant}
      className={cn('relative', tabsListVariants({ variant }), className)}
      {...props}
    />
  );
}

function TabsIndicator({ className, ...props }: TabsPrimitive.Indicator.Props) {
  return (
    <TabsPrimitive.Indicator
      data-slot="tabs-indicator"
      className={cn(
        'pointer-events-none absolute left-[var(--active-tab-left)] z-0 w-[var(--active-tab-width)] transition-[left,width,top,bottom,height] duration-tab ease-standard motion-reduce:transition-none',
        'group-data-[variant=line]/tabs-list:bottom-0 group-data-[variant=line]/tabs-list:h-0.5 group-data-[variant=line]/tabs-list:bg-accent-ink',
        'group-data-[variant=segmented]/tabs-list:top-[var(--active-tab-top)] group-data-[variant=segmented]/tabs-list:h-[var(--active-tab-height)] group-data-[variant=segmented]/tabs-list:rounded-[5px] group-data-[variant=segmented]/tabs-list:border group-data-[variant=segmented]/tabs-list:border-input dark:group-data-[variant=segmented]/tabs-list:border-border group-data-[variant=segmented]/tabs-list:bg-control-active group-data-[variant=segmented]/tabs-list:shadow-sm',
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
        "relative z-[1] inline-flex min-h-8 min-w-0 flex-none items-center justify-center gap-1.5 rounded-control border border-transparent bg-transparent px-2 py-1 text-xs font-medium whitespace-nowrap text-muted-foreground transition-colors duration-tab hover:bg-transparent hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 aria-disabled:pointer-events-none aria-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 motion-reduce:transition-none",
        'group-data-[variant=line]/tabs-list:h-full group-data-[variant=line]/tabs-list:rounded-none group-data-[variant=line]/tabs-list:px-3 group-data-[variant=line]/tabs-list:text-sm',
        'group-data-[variant=segmented]/tabs-list:min-h-7 group-data-[variant=segmented]/tabs-list:flex-1 group-data-[variant=segmented]/tabs-list:rounded-[5px] group-data-[variant=segmented]/tabs-list:px-1.5',
        'aria-selected:font-semibold aria-selected:bg-transparent aria-selected:shadow-none',
        'group-data-[variant=line]/tabs-list:aria-selected:text-accent-ink group-data-[variant=segmented]/tabs-list:aria-selected:text-foreground group-data-[variant=segmented]/tabs-list:aria-selected:forced-colors:text-[HighlightText]',
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
