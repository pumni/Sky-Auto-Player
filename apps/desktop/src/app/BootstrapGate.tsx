import type { ReactNode } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../state/store';

interface BootstrapGateProps {
  useStore: DesktopStoreHook;
  children: ReactNode;
}

export function BootstrapGate({ useStore, children }: BootstrapGateProps) {
  const status = useStore((store: DesktopStore) => store.bootstrapState);
  const fatal = useStore((store: DesktopStore) => store.fatal);

  if (status === 'loading' || status === 'idle') {
    return (
      <main
        className="grid min-h-dvh content-center justify-items-center gap-2 bg-background p-6 text-center"
        aria-live="polite"
      >
        <div
          className="mb-2 grid size-12 place-items-center rounded-pane bg-primary text-2xl text-primary-foreground"
          aria-hidden="true"
        >
          ♪
        </div>
        <p className="m-0 text-[11px] font-semibold tracking-wide text-text-tertiary">
          SKY AUTO PLAYER
        </p>
        <h1 className="m-0 text-xl font-semibold leading-7 text-foreground">
          Opening your library…
        </h1>
        <p className="m-0 text-sm text-muted-foreground">Starting the native application.</p>
      </main>
    );
  }

  if (status === 'fatal') {
    return (
      <main
        className="grid min-h-dvh content-center justify-items-center gap-2 bg-background p-6 text-center"
        role="alert"
      >
        <p className="m-0 text-[11px] font-semibold tracking-wide text-danger">
          APPLICATION UNAVAILABLE
        </p>
        <h1 className="m-0 max-w-[580px] text-xl font-semibold leading-7 text-foreground">
          Sky Auto Player could not start.
        </h1>
        <p className="m-0 max-w-[580px] text-sm text-text-secondary">
          {fatal ?? 'The native application returned an unknown error.'}
        </p>
        <p className="m-0 text-sm text-muted-foreground">
          Close and reopen the application after correcting the native setup.
        </p>
      </main>
    );
  }

  return children;
}
