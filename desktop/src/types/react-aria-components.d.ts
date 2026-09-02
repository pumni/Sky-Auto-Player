import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  ReactElement,
  ReactNode,
  RefAttributes,
} from 'react';

/**
 * React Aria Components 1.20.0 currently conflicts with TypeScript 7's
 * `exactOptionalPropertyTypes` in its published Group/OverlayArrow types.
 * Keep the runtime package, but type the small headless primitive surface used
 * by this initial shell locally until the upstream declarations catch up.
 */
declare module 'react-aria-components' {
  export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'onClick'> {
    onPress?: () => void;
    isDisabled?: boolean;
  }

  export const Button: (
    props: ButtonProps & RefAttributes<HTMLButtonElement>,
  ) => ReactElement | null;

  export interface DialogTriggerProps {
    isOpen?: boolean;
    defaultOpen?: boolean;
    onOpenChange?: (isOpen: boolean) => void;
    children: ReactNode;
  }

  export const DialogTrigger: (props: DialogTriggerProps) => ReactElement | null;

  export interface PopoverProps extends HTMLAttributes<HTMLDivElement> {
    placement?: string;
    offset?: number;
    children?: ReactNode;
  }

  export const Popover: (props: PopoverProps & RefAttributes<HTMLElement>) => ReactElement | null;

  export interface ModalOverlayProps extends HTMLAttributes<HTMLDivElement> {
    isOpen?: boolean;
    isDismissable?: boolean;
    onOpenChange?: (isOpen: boolean) => void;
    children?: ReactNode;
  }

  export const ModalOverlay: (
    props: ModalOverlayProps & RefAttributes<HTMLDivElement>,
  ) => ReactElement | null;

  export const Modal: (
    props: HTMLAttributes<HTMLDivElement> & RefAttributes<HTMLDivElement>,
  ) => ReactElement | null;

  export const Dialog: (
    props: HTMLAttributes<HTMLElement> & RefAttributes<HTMLElement>,
  ) => ReactElement | null;

  export interface TabsProps extends HTMLAttributes<HTMLDivElement> {
    defaultSelectedKey?: string;
  }

  export const Tabs: (props: TabsProps & RefAttributes<HTMLDivElement>) => ReactElement | null;

  export const TabList: (
    props: HTMLAttributes<HTMLElement> & RefAttributes<HTMLElement>,
  ) => ReactElement | null;

  export const Tab: (
    props: HTMLAttributes<HTMLButtonElement> & { id: string } & RefAttributes<HTMLButtonElement>,
  ) => ReactElement | null;

  export const TabPanel: (
    props: HTMLAttributes<HTMLElement> & { id: string } & RefAttributes<HTMLElement>,
  ) => ReactElement | null;
}
