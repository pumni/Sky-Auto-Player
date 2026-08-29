import { Button } from 'react-aria-components';
import type { ReactNode } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../state/store';

interface BootstrapGateProps {
  useStore: DesktopStoreHook;
  children: ReactNode;
}

export function BootstrapGate({ useStore, children }: BootstrapGateProps) {
  const status = useStore((store: DesktopStore) => store.bootstrapState);
  const fatal = useStore((store: DesktopStore) => store.fatal);
  const initialize = useStore((store: DesktopStore) => store.initialize);

  if (status === 'loading' || status === 'idle') {
    return (
      <main className="startup-screen" aria-live="polite">
        <div className="startup-mark" aria-hidden="true">
          ♪
        </div>
        <p className="eyebrow">SKY AUTO PLAYER</p>
        <h1>Opening your library…</h1>
        <p className="muted">Starting the local application core.</p>
      </main>
    );
  }

  if (status === 'fatal') {
    return (
      <main className="fatal-screen" role="alert">
        <p className="eyebrow">CORE UNAVAILABLE</p>
        <h1>Sky Auto Player could not start.</h1>
        <p className="fatal-message">{fatal ?? 'The local Core returned an unknown error.'}</p>
        <Button className="button button-primary" onPress={() => void initialize()}>
          Try again
        </Button>
      </main>
    );
  }

  return children;
}
