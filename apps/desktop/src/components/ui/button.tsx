import { Button as ButtonPrimitive } from '@base-ui/react/button';
import { cva, type VariantProps } from 'class-variance-authority';
import { cn } from 'cn';

const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-control border-0 border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-colors duration-fast ease-standard outline-none select-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:pointer-events-none disabled:opacity-50 aria-invalid:ring-2 aria-invalid:ring-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 motion-reduce:transition-none",
  {
    variants: {
      variant: {
        default:
          'bg-primary text-primary-foreground hover:bg-primary-hover active:bg-primary-pressed',
        outline:
          'border border-border bg-background hover:bg-secondary active:bg-control-pressed aria-expanded:bg-secondary',
        secondary:
          'bg-secondary text-secondary-foreground hover:bg-control-hover active:bg-control-pressed',
        ghost:
          'hover:bg-control-hover hover:text-accent-foreground active:bg-control-pressed aria-expanded:bg-control-active',
        destructive:
          'bg-destructive text-destructive-foreground hover:bg-destructive-hover active:bg-destructive-pressed',
      },
      size: {
        default:
          'h-9 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2',
        xs: "h-6 gap-1 rounded-control px-2 text-xs [&_svg:not([class*='size-'])]:size-3",
        sm: 'h-8 gap-1 px-2.5 has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5',
        icon: 'size-9',
        'icon-xs': "size-6 rounded-control [&_svg:not([class*='size-'])]:size-3",
        'icon-sm': 'size-8 rounded-control',
        'icon-lg': 'size-10',
        transport: 'size-[42px] rounded-full',
      },
    },
    defaultVariants: {
      variant: 'default',
      size: 'default',
    },
  },
);

function Button({
  className,
  variant = 'default',
  size = 'default',
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button };
