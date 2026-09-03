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
    <footer className="player-bar" aria-label="Player controls">
      <PlaybackAdmission useStore={useStore} />
      <PlayerTrackInfo useStore={useStore} />
      <PlayerTransport useStore={useStore} />
      <PlayerTools useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
    </footer>
  );
}
