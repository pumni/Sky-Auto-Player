import type { RefObject } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import type { TrackTableHandle } from '../tracks/TrackTable';
import { PlaybackAdmission } from './PlaybackAdmission';
import { PlayerTools } from './PlayerTools';
import { PlayerTrackInfo } from './PlayerTrackInfo';
import { PlayerTransport } from './PlayerTransport';

interface PlayerBarProps {
  useStore: DesktopStoreHook;
  utilityTriggerRef?: RefObject<HTMLButtonElement | null>;
  locateSelectedRef?: RefObject<TrackTableHandle | null>;
}

export function PlayerBar({ useStore, utilityTriggerRef, locateSelectedRef }: PlayerBarProps) {
  return (
    <footer className="player-bar" aria-label="Player controls">
      <PlaybackAdmission useStore={useStore} />
      <PlayerTrackInfo useStore={useStore} />
      <PlayerTransport useStore={useStore} />
      <PlayerTools
        useStore={useStore}
        utilityTriggerRef={utilityTriggerRef}
        locateSelectedRef={locateSelectedRef}
      />
    </footer>
  );
}
