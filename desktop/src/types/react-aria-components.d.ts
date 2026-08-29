import type { ButtonHTMLAttributes, ReactElement, RefAttributes } from 'react';

/**
 * React Aria Components 1.20.0 currently conflicts with TypeScript 7's
 * `exactOptionalPropertyTypes` in its published Group/OverlayArrow types.
 * Keep the runtime package, but type the two headless buttons used by this
 * initial shell locally until the upstream declarations catch up.
 */
declare module 'react-aria-components' {
  export interface ButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'onClick'> {
    onPress?: () => void;
    isDisabled?: boolean;
  }

  export const Button: (
    props: ButtonProps & RefAttributes<HTMLButtonElement>,
  ) => ReactElement | null;
}
