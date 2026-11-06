import type { DesktopBridge, SongRow } from '../bridge/DesktopBridge';
import { identityFromRow, playbackContextIndexAt } from './selectors';
import { AUTO_PLAY_HANDOFF_MS } from './types';
import type { DesktopStore, PlaybackContext, PlaybackSongIdentity } from './types';

type PreparedPlayback = NonNullable<DesktopStore['playback']['prepared']>;

export interface PlaybackSourceTraversalContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  getLibrary: () => DesktopStore['library'];
  playbackContextIsCurrent: (context: PlaybackContext) => boolean;
  resolveContextRow: (context: PlaybackContext, index: number) => Promise<SongRow>;
  prepareForOperation: (
    identity: PlaybackSongIdentity,
    context: PlaybackContext,
    overrides: undefined,
    operationEpoch: number,
  ) => Promise<PreparedPlayback | null>;
  startForOperation: (
    prepared: PreparedPlayback,
    identity: PlaybackSongIdentity,
    context: PlaybackContext,
    decision: undefined,
    operationEpoch: number,
  ) => Promise<void>;
  beginOperation: (operation: 'advancing' | 'restarting') => number | null;
  finishOperation: (epoch: number, error?: string | null) => boolean;
  operationIsCurrent: (epoch: number) => boolean;
  waitForRetirement: (sessionId: string) => Promise<'finished' | 'failed' | null>;
}

export interface PlaybackSourceTraversal {
  transitionContextPlayback: (
    context: PlaybackContext,
    targetPosition: number,
    operation: 'advancing' | 'restarting',
    sessionId: string | null,
    naturalAutoPlayRevision?: number,
  ) => Promise<void>;
  cancelStaleAutoAdvanceHandoff: (state: DesktopStore) => void;
  autoPlaySettingsRevision: () => number;
  onAutoPlayChanged: () => void;
}

export function createPlaybackSourceTraversal(
  context: PlaybackSourceTraversalContext,
): PlaybackSourceTraversal {
  const {
    bridge,
    get,
    getLibrary,
    playbackContextIsCurrent,
    resolveContextRow,
    prepareForOperation,
    startForOperation,
    beginOperation,
    finishOperation,
    operationIsCurrent,
    waitForRetirement,
  } = context;
  let autoPlaySettingsRevision = 0;
  let pendingAutoAdvanceHandoff: {
    cancel: () => void;
    isCurrent: (state: DesktopStore) => boolean;
  } | null = null;

  const transitionContextPlayback = async (
    playbackContext: PlaybackContext,
    targetPosition: number,
    operation: 'advancing' | 'restarting',
    sessionId: string | null,
    naturalRevision?: number,
  ): Promise<void> => {
    const epoch = beginOperation(operation);
    if (epoch === null) return;
    try {
      if (
        playbackContext.generation !== getLibrary().generation ||
        !playbackContextIsCurrent(playbackContext)
      ) {
        throw new Error(
          'The Library context changed after playback started. Select a song to create a new playback context.',
        );
      }
      if (sessionId) {
        const retirement = waitForRetirement(sessionId);
        const state = get().playback.state;
        if (!['finished', 'failed'].includes(state)) {
          await bridge.skipPlayback({ sessionId });
        }
        const outcome = await retirement;
        if (outcome === null) {
          throw new Error('The current playback session did not retire in time.');
        }
        if (outcome === 'failed') return;
      }
      if (!operationIsCurrent(epoch)) return;
      if (naturalRevision !== undefined) {
        const handoffIsCurrent = (current: DesktopStore) => {
          const owner = current.playback.context;
          return (
            operationIsCurrent(epoch) &&
            current.playback.transportOperation === 'advancing' &&
            autoPlaySettingsRevision === naturalRevision &&
            current.settings?.auto_play === true &&
            playbackContext.generation === getLibrary().generation &&
            playbackContextIsCurrent(playbackContext) &&
            owner !== null &&
            owner === playbackContext &&
            owner.currentSongId === playbackContext.currentSongId &&
            owner.currentIndex === playbackContext.currentIndex
          );
        };
        const handoffElapsed = await new Promise<boolean>((resolve) => {
          const timer = setTimeout(() => {
            if (pendingAutoAdvanceHandoff?.cancel === cancel) {
              pendingAutoAdvanceHandoff = null;
            }
            resolve(true);
          }, AUTO_PLAY_HANDOFF_MS);
          const cancel = () => {
            clearTimeout(timer);
            if (pendingAutoAdvanceHandoff?.cancel === cancel) {
              pendingAutoAdvanceHandoff = null;
            }
            resolve(false);
          };
          pendingAutoAdvanceHandoff = { cancel, isCurrent: handoffIsCurrent };
        });
        if (!operationIsCurrent(epoch)) return;
        if (!handoffElapsed || !handoffIsCurrent(get())) {
          finishOperation(epoch);
          return;
        }
      }
      const targetIndex = playbackContextIndexAt(playbackContext, targetPosition);
      if (targetIndex === null) throw new Error('There is no next playback-context item.');
      const row = await resolveContextRow(playbackContext, targetIndex);
      if (!operationIsCurrent(epoch)) return;
      const nextContext: PlaybackContext = {
        ...playbackContext,
        currentIndex: targetIndex,
        currentSongId: row.song_id,
        shuffleTraversal: playbackContext.shuffleTraversal
          ? { ...playbackContext.shuffleTraversal, position: targetPosition }
          : null,
      };
      const identity = identityFromRow(row, playbackContext.generation);
      const prepared = await prepareForOperation(identity, nextContext, undefined, epoch);
      if (!prepared || !operationIsCurrent(epoch)) return;
      if (prepared.admission === 'blocked') {
        finishOperation(epoch, prepared.error_message ?? 'Playback is blocked for this song.');
        return;
      }
      if (prepared.admission === 'confirmation_required') {
        finishOperation(epoch);
        return;
      }
      await startForOperation(prepared, identity, nextContext, undefined, epoch);
    } catch (error) {
      finishOperation(epoch, error instanceof Error ? error.message : String(error));
    }
  };

  return {
    transitionContextPlayback,
    cancelStaleAutoAdvanceHandoff(state) {
      const pending = pendingAutoAdvanceHandoff;
      if (pending && !pending.isCurrent(state)) pending.cancel();
    },
    autoPlaySettingsRevision: () => autoPlaySettingsRevision,
    onAutoPlayChanged: () => {
      autoPlaySettingsRevision += 1;
    },
  };
}
