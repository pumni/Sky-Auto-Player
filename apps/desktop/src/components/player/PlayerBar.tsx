import type { RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { PlaybackAdmission } from './PlaybackAdmission';
import { PlayerTools } from './PlayerTools';
import { PlayerTrackInfo } from './PlayerTrackInfo';
import { PlayerTransport } from './PlayerTransport';

interface PlayerBarProps {
  useStore: DesktopStoreHook;
  utilityTriggerRef?: RefObject<HTMLButtonElement | null>;
}

export function PlayerBar({ useStore, utilityTriggerRef }: PlayerBarProps) {
  return (
    <footer
      data-slot="player-dock"
      className="relative grid min-w-0 grid-cols-[minmax(140px,1fr)_minmax(320px,600px)_minmax(140px,1fr)] items-center gap-2 border-t border-border bg-card px-3 py-2 text-text-secondary"
      aria-label="Player controls"
    >
      <PlaybackAdmission useStore={useStore} />
      <PlayerTrackInfo useStore={useStore} />
      <PlayerTransport useStore={useStore} />
      <PlayerTools useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
    </footer>
  );
}
