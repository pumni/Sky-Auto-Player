import type { DesktopBridge } from '../bridge/DesktopBridge';
import {
  createShuffleTraversal,
  nextPlaybackPosition,
  previousPlaybackPosition,
} from './selectors';
import { sourceKey } from './libraryHelpers';
import type { DesktopStore, PlaybackContext, TransportOperation } from './types';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface PlaybackTransportActions {
  stopPlayback: DesktopStore['stopPlayback'];
  retryPlaybackStatus: DesktopStore['retryPlaybackStatus'];
  pausePlayback: DesktopStore['pausePlayback'];
  resumePlayback: DesktopStore['resumePlayback'];
  setShuffleEnabled: DesktopStore['setShuffleEnabled'];
  previousPlayback: DesktopStore['previousPlayback'];
  nextPlayback: DesktopStore['nextPlayback'];
}

export interface PlaybackTransportContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  beginOperation: (
    operation: Exclude<TransportOperation, null>,
    preemptTransport?: boolean,
  ) => number | null;
  finishOperation: (epoch: number, error?: string | null) => boolean;
  getTransportOperationEpoch: () => number;
  nextReconciliationToken: () => number;
  reconcilePlaybackAfterTimeout: (
    token: number,
    operation: Exclude<TransportOperation, null>,
    epoch: number,
    expectedSessionId: string | null,
    expectedStartRequestId: number | null,
  ) => Promise<void>;
  playbackContextIsCurrent: (context: PlaybackContext) => boolean;
  transitionContextPlayback: (
    context: PlaybackContext,
    targetPosition: number,
    operation: 'advancing' | 'restarting',
    sessionId: string | null,
  ) => Promise<void>;
  nextShuffleTraversalSequence: () => number;
}

export function createPlaybackTransportActions(
  context: PlaybackTransportContext,
): PlaybackTransportActions {
  const {
    bridge,
    get,
    set,
    beginOperation,
    finishOperation,
    getTransportOperationEpoch,
    nextReconciliationToken,
    reconcilePlaybackAfterTimeout,
    playbackContextIsCurrent,
    transitionContextPlayback,
    nextShuffleTraversalSequence,
  } = context;
  let playbackStatusRetryPromise: Promise<void> | null = null;
  let playbackStatusRetryToken = 0;

  return {
    async stopPlayback() {
      const playback = get().playback;
      const sessionId = playback.sessionId;
      const preemptTransport = playback.transportOperation !== null;
      if (
        !sessionId ||
        !['starting', 'playing', 'paused', 'stopping', 'finished', 'failed'].includes(
          playback.state,
        )
      ) {
        return;
      }
      const operationEpoch = beginOperation('stopping', preemptTransport);
      if (operationEpoch === null) return;
      try {
        const acknowledgement = await bridge.stopPlayback({ sessionId });
        if (['finished', 'failed'].includes(acknowledgement.state)) {
          const token = nextReconciliationToken();
          await reconcilePlaybackAfterTimeout(
            token,
            'stopping',
            operationEpoch,
            sessionId,
            playback.startRequestId,
          );
        }
      } catch (error) {
        finishOperation(operationEpoch, error instanceof Error ? error.message : String(error));
      }
    },

    retryPlaybackStatus() {
      if (playbackStatusRetryPromise) return playbackStatusRetryPromise;
      const playback = get().playback;
      const token = nextReconciliationToken();
      playbackStatusRetryToken = token;
      const operation = playback.transportOperation ?? 'starting';
      const epoch = getTransportOperationEpoch();
      set({ playback: { ...playback, statusRetryPending: true } });
      const retryPromise = reconcilePlaybackAfterTimeout(
        token,
        operation,
        epoch,
        playback.sessionId,
        playback.startRequestId,
      ).finally(() => {
        if (playbackStatusRetryPromise !== retryPromise || playbackStatusRetryToken !== token)
          return;
        playbackStatusRetryPromise = null;
        const latest = get().playback;
        set({ playback: { ...latest, statusRetryPending: false } });
      });
      playbackStatusRetryPromise = retryPromise;
      return retryPromise;
    },

    async pausePlayback() {
      const playback = get().playback;
      const sessionId = playback.sessionId;
      if (!sessionId || playback.state !== 'playing' || playback.transportOperation !== null)
        return;
      const operationEpoch = beginOperation('pausing');
      if (operationEpoch === null) return;
      try {
        await bridge.pausePlayback({ sessionId });
      } catch (error) {
        finishOperation(operationEpoch, error instanceof Error ? error.message : String(error));
      }
    },

    async resumePlayback() {
      const playback = get().playback;
      const sessionId = playback.sessionId;
      if (!sessionId || playback.state !== 'paused' || playback.transportOperation !== null) return;
      const operationEpoch = beginOperation('resuming');
      if (operationEpoch === null) return;
      try {
        await bridge.resumePlayback({ sessionId });
      } catch (error) {
        finishOperation(operationEpoch, error instanceof Error ? error.message : String(error));
      }
    },

    setShuffleEnabled(enabled) {
      const playback = get().playback;
      if (
        playback.shuffleEnabled === enabled ||
        playback.transportOperation !== null ||
        playback.prepared?.admission === 'confirmation_required'
      ) {
        return;
      }
      const updateTraversal = (playbackContext: PlaybackContext | null): PlaybackContext | null => {
        if (!playbackContext || !playbackContextIsCurrent(playbackContext)) return playbackContext;
        return {
          ...playbackContext,
          shuffleTraversal: enabled
            ? createShuffleTraversal(
                playbackContext.total,
                playbackContext.currentIndex,
                `${playbackContext.generation}:${sourceKey(playbackContext.source)}:${playbackContext.query}:${playbackContext.currentSongId}:${nextShuffleTraversalSequence()}`,
              )
            : null,
        };
      };
      const playbackContext = updateTraversal(playback.context);
      const preparedContext = updateTraversal(playback.preparedContext);
      set({
        playback: {
          ...playback,
          shuffleEnabled: enabled,
          context: playbackContext,
          preparedContext,
        },
      });
    },

    async previousPlayback() {
      const playback = get().playback;
      const playbackContext = playback.context;
      if (
        !playbackContext ||
        !playbackContextIsCurrent(playbackContext) ||
        playback.prepared?.admission === 'confirmation_required' ||
        playback.transportOperation !== null
      ) {
        return;
      }
      const currentUs =
        playback.snapshot?.session_id === playback.sessionId ? playback.snapshot.current_us : 0;
      const targetPosition = previousPlaybackPosition(playbackContext, currentUs);
      if (targetPosition === null) return;
      await transitionContextPlayback(
        playbackContext,
        targetPosition,
        'restarting',
        playback.sessionId,
      );
    },

    async nextPlayback() {
      const playback = get().playback;
      const playbackContext = playback.context;
      if (
        !playbackContext ||
        !playbackContextIsCurrent(playbackContext) ||
        playback.prepared?.admission === 'confirmation_required' ||
        playback.transportOperation !== null
      ) {
        return;
      }
      const targetPosition = nextPlaybackPosition(playbackContext);
      if (targetPosition === null) return;
      await transitionContextPlayback(
        playbackContext,
        targetPosition,
        'advancing',
        playback.sessionId,
      );
    },
  };
}
