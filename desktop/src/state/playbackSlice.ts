import type {
  DesktopBridge,
  LibrarySource,
  PlaybackDecisionAcceptance,
  PlaybackDecisionId,
  SearchResult,
  UiEvent,
} from '../bridge/DesktopBridge';
import { createPlaybackSessionLedger } from './playbackSessionLedger';
import type { PendingPlaybackStart } from './playbackSessionLedger';
import { sourceKey } from './libraryHelpers';
import { identityFromRow, nextPlaybackPosition, selectSongById } from './selectors';
import { createPlaybackSourceTraversal } from './playbackSourceTraversal';
import { createPlaybackTransportActions } from './playbackTransport';
import { createPlaybackPreparationController } from './playbackPreparation';
import {
  createPlaybackReconciliation,
  TRANSPORT_OPERATION_TIMEOUT_MS,
} from './playbackReconciliation';
import type {
  DesktopStore,
  LibraryState,
  PlaybackContext,
  PlaybackSongIdentity,
  PlaybackUiState,
} from './types';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface PlaybackSliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  getLibrary: () => LibraryState;
  set: DesktopStoreSetter;
  loadPage: (
    query: string,
    source: LibrarySource,
    offset: number,
    generation: number,
    token: number,
    mergeIntoLibrary?: boolean,
  ) => Promise<SearchResult>;
}

export type PlaybackSliceActions = Pick<
  DesktopStore,
  | 'prepareSelectedPlayback'
  | 'startPreparedPlayback'
  | 'cancelPreparedPlayback'
  | 'stopPlayback'
  | 'retryPlaybackStatus'
  | 'pausePlayback'
  | 'resumePlayback'
  | 'previousPlayback'
  | 'nextPlayback'
  | 'setShuffleEnabled'
>;

export interface PlaybackSlice {
  actions: PlaybackSliceActions;
  clearOperationFromEvent: () => void;
  invalidatePlaybackMembershipContext: (source: LibrarySource) => void;
  playbackAfterBrowserNavigation: (playback: DesktopStore['playback']) => DesktopStore['playback'];
  playbackContextIsCurrent: (context: PlaybackContext) => boolean;
  isSessionRetired: (sessionId: string) => boolean;
  cancelStaleAutoAdvanceHandoff: (state: DesktopStore) => void;
  handleEvent: (event: UiEvent) => boolean;
  onAutoPlayChanged: () => void;
}

export function createPlaybackSlice(context: PlaybackSliceContext): PlaybackSlice {
  const { bridge, get, getLibrary, set, loadPage } = context;
  const sessionLedger = createPlaybackSessionLedger();
  let shuffleTraversalSequence = 0;
  const sourceMembershipRevisions = new Map<string, number>();

  const membershipRevisionFor = (source: LibrarySource) =>
    sourceMembershipRevisions.get(sourceKey(source)) ?? 0;
  const playbackContextIsCurrent = (playbackContext: PlaybackContext) =>
    playbackContext.valid &&
    playbackContext.membershipRevision === membershipRevisionFor(playbackContext.source);

  const reconciliation = createPlaybackReconciliation({
    bridge,
    get,
    getLibrary,
    set,
    sessionLedger,
  });
  const {
    beginOperation,
    finishOperation,
    operationIsCurrent,
    getTransportOperationEpoch,
    nextReconciliationToken,
    reconcilePlaybackAfterTimeout,
    clearOperationFromEvent,
    armTerminalReconciliationTimer,
    clearTerminalReconciliationTimer,
  } = reconciliation;
  const preparation = createPlaybackPreparationController({
    bridge,
    get,
    getLibrary,
    set,
    loadPage,
    playbackContextIsCurrent,
    membershipRevisionFor,
    nextShuffleTraversalSequence: () => ++shuffleTraversalSequence,
    operationIsCurrent,
  });

  const errorMessage = (error: unknown): string =>
    error instanceof Error ? error.message : String(error);

  const invalidatePlaybackMembershipContext = (source: LibrarySource): void => {
    const key = sourceKey(source);
    sourceMembershipRevisions.set(key, (sourceMembershipRevisions.get(key) ?? 0) + 1);
    const playback = get().playback;
    const contextMatches = playback.context !== null && sourceKey(playback.context.source) === key;
    const playbackContext =
      playback.context && sourceKey(playback.context.source) === key
        ? { ...playback.context, valid: false }
        : playback.context;
    const preparedMatches =
      playback.preparedContext !== null && sourceKey(playback.preparedContext.source) === key;
    const cancelConfirmation =
      preparedMatches &&
      playback.prepared?.admission === 'confirmation_required' &&
      playback.transportOperation === null;
    if (!contextMatches && !preparedMatches) return;
    set({
      playback: {
        ...playback,
        context: playbackContext,
        prepared: cancelConfirmation ? null : playback.prepared,
        preparedIdentity: cancelConfirmation ? null : playback.preparedIdentity,
        preparedContext: preparedMatches
          ? { ...playback.preparedContext!, valid: false }
          : playback.preparedContext,
        error: cancelConfirmation
          ? 'The Library changed while playback was awaiting confirmation. Prepare the song again.'
          : playback.error,
      },
    });
  };

  const acceptsSessionEvent = (sessionId: string, songId: string): boolean => {
    const current = get().playback;
    if (sessionLedger.isRetired(sessionId)) return false;
    if (current.sessionId === sessionId) return true;
    if (current.sessionId !== null) return false;
    const pending = sessionLedger.pendingStartForSong(songId);
    if (
      pending &&
      (current.startRequestId === null || current.startRequestId === pending.epoch) &&
      ['idle', 'starting', 'finished', 'failed'].includes(current.state)
    ) {
      // The native runtime may publish the first state/terminal event before the
      // start promise continuation runs. Bind the event only to the matching
      // request; a timed-out request cannot steal a newer start's ownership.
      return true;
    }
    return false;
  };

  const bindSessionIdentity = (sessionId: string, songId: string): PlaybackSongIdentity | null => {
    const current = get().playback;
    if (current.sessionId === sessionId && current.currentSong?.songId === songId) {
      return current.currentSong;
    }
    const pending = sessionLedger.pendingStartForSong(songId);
    if (pending) return pending.identity;
    if (current.currentSong?.songId === songId) return current.currentSong;
    return null;
  };

  const confirmsPlaybackStart = (
    current: DesktopStore['playback'],
    sessionId: string,
    songId: string,
    pending: PendingPlaybackStart | null,
    state: PlaybackUiState,
  ): boolean => {
    if (
      state !== 'playing' ||
      !['starting', 'advancing', 'restarting'].includes(current.transportOperation ?? '')
    )
      return false;
    if (pending !== null) return true;
    return (
      current.state === 'starting' &&
      current.sessionId === sessionId &&
      current.currentSong?.songId === songId &&
      current.context?.currentSongId === songId
    );
  };

  const playbackAfterBrowserNavigation = (
    playback: DesktopStore['playback'],
  ): DesktopStore['playback'] => {
    const cancelPreparing = playback.transportOperation === 'preparing';
    if (cancelPreparing) clearOperationFromEvent();
    const preserveIdentity =
      playback.sessionId !== null ||
      playback.startRequestId !== null ||
      (!cancelPreparing && playback.transportOperation !== null);
    return {
      ...playback,
      prepared: null,
      preparedIdentity: null,
      preparedContext: null,
      currentSong: preserveIdentity ? playback.currentSong : null,
      context: preserveIdentity ? playback.context : null,
      sessionId: preserveIdentity ? playback.sessionId : null,
      state: preserveIdentity ? playback.state : 'idle',
      snapshot: preserveIdentity ? playback.snapshot : null,
      transportOperation: cancelPreparing ? null : playback.transportOperation,
      error: null,
    };
  };

  const startForOperation = async (
    prepared: NonNullable<DesktopStore['playback']['prepared']>,
    identity: PlaybackSongIdentity,
    playbackContext: PlaybackContext,
    decision: PlaybackDecisionId | undefined,
    operationEpoch: number,
  ): Promise<void> => {
    if (!operationIsCurrent(operationEpoch)) return;
    if (!prepared.prepared_id)
      throw new Error('The prepared playback plan is no longer available.');
    const decisions: PlaybackDecisionAcceptance[] = decision ? [{ decision, accepted: true }] : [];
    const pendingStart = sessionLedger.beginStart({
      preparedId: prepared.prepared_id,
      identity,
      context: playbackContext,
    });
    const startEpoch = pendingStart.epoch;
    const startingPlayback = get().playback;
    const transportOperation =
      startingPlayback.transportOperation === 'advancing' ||
      startingPlayback.transportOperation === 'restarting'
        ? startingPlayback.transportOperation
        : 'starting';
    set({
      playback: {
        ...startingPlayback,
        startRequestId: startEpoch,
        transportOperation,
        error: null,
      },
    });
    const session = await bridge
      .startPlayback({
        preparedId: prepared.prepared_id,
        decisions,
      })
      .catch((error: unknown) => {
        sessionLedger.removePendingStart(startEpoch);
        const current = get().playback;
        if (current.startRequestId === startEpoch) {
          set({ playback: { ...current, startRequestId: null } });
        }
        throw error;
      });
    if (!operationIsCurrent(operationEpoch) || sessionLedger.isRetired(session.session_id)) {
      sessionLedger.removePendingStart(startEpoch);
      const current = get().playback;
      const ownsRequest = current.startRequestId === startEpoch;
      if (sessionLedger.isRetired(session.session_id)) {
        if (ownsRequest) set({ playback: { ...current, startRequestId: null } });
        return;
      }
      if (current.sessionId === session.session_id) {
        if (ownsRequest) set({ playback: { ...current, startRequestId: null } });
        return;
      }
      if (current.sessionId === null) {
        set({
          playback: {
            ...current,
            sessionId: session.session_id,
            currentSong: identity,
            context: playbackContext,
            prepared: null,
            preparedIdentity: null,
            preparedContext: null,
            startRequestId: ownsRequest ? null : current.startRequestId,
            state: 'starting',
            snapshot: null,
          },
        });
        const stopEpoch = beginOperation('stopping');
        try {
          await bridge.stopPlayback({ sessionId: session.session_id });
        } catch (error) {
          if (stopEpoch !== null) finishOperation(stopEpoch, errorMessage(error));
        }
      } else {
        if (ownsRequest) set({ playback: { ...current, startRequestId: null } });
        void bridge.stopPlayback({ sessionId: session.session_id }).catch(() => undefined);
      }
      return;
    }
    sessionLedger.removePendingStart(startEpoch);
    if (!playbackContextIsCurrent(playbackContext)) {
      const current = get().playback;
      if (current.startRequestId === startEpoch) {
        set({ playback: { ...current, startRequestId: null } });
      }
      void bridge.stopPlayback({ sessionId: session.session_id }).catch(() => undefined);
      throw new Error('The Library source changed while playback was starting.');
    }
    const current = get().playback;
    if (current.sessionId !== null && current.sessionId !== session.session_id) {
      if (current.startRequestId === startEpoch) {
        set({ playback: { ...current, startRequestId: null } });
      }
      void bridge.stopPlayback({ sessionId: session.session_id }).catch(() => undefined);
      return;
    }
    const earlyEventBound = current.sessionId === session.session_id;
    set({
      playback: {
        ...current,
        prepared: null,
        preparedIdentity: null,
        preparedContext: null,
        startRequestId: null,
        sessionId: session.session_id,
        currentSong: identity,
        context: playbackContext,
        state: earlyEventBound ? current.state : 'starting',
        snapshot: earlyEventBound ? current.snapshot : null,
        error: null,
      },
    });
  };

  const sourceTraversal = createPlaybackSourceTraversal({
    bridge,
    get,
    getLibrary,
    playbackContextIsCurrent,
    resolveContextRow: preparation.resolveContextRow,
    prepareForOperation: preparation.prepareForOperation,
    startForOperation,
    beginOperation: (operation) => beginOperation(operation),
    finishOperation,
    operationIsCurrent,
    waitForRetirement: (sessionId) =>
      sessionLedger.waitForRetirement(sessionId, TRANSPORT_OPERATION_TIMEOUT_MS),
  });

  const transportActions = createPlaybackTransportActions({
    bridge,
    get,
    set,
    beginOperation,
    finishOperation,
    getTransportOperationEpoch,
    nextReconciliationToken,
    reconcilePlaybackAfterTimeout,
    playbackContextIsCurrent,
    transitionContextPlayback: sourceTraversal.transitionContextPlayback,
    nextShuffleTraversalSequence: () => ++shuffleTraversalSequence,
  });

  const handleEvent = (event: UiEvent): boolean => {
    if (
      event.name !== 'playback.state_changed' &&
      event.name !== 'playback.snapshot' &&
      event.name !== 'playback.finished' &&
      event.name !== 'playback.failed'
    ) {
      return false;
    }
    if (event.name === 'playback.state_changed') {
      const current = get().playback;
      if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return true;
      const pending = sessionLedger.pendingStartForSong(event.payload.song_id);
      const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
      const playbackContext = pending?.context ?? current.context;
      const confirmsOperation =
        confirmsPlaybackStart(
          current,
          event.payload.session_id,
          event.payload.song_id,
          pending,
          event.payload.state,
        ) ||
        (current.transportOperation === 'pausing' && event.payload.state === 'paused') ||
        (current.transportOperation === 'resuming' && event.payload.state === 'playing');
      if (confirmsOperation) clearOperationFromEvent();
      if (pending) sessionLedger.removePendingStart(pending.epoch);
      const startRequestId =
        current.sessionId === event.payload.session_id || current.startRequestId === pending?.epoch
          ? null
          : current.startRequestId;
      set({
        playback: {
          ...current,
          sessionId: event.payload.session_id,
          currentSong: identity ?? current.currentSong,
          context: playbackContext,
          transportOperation: confirmsOperation ? null : current.transportOperation,
          startRequestId,
          snapshot: current.sessionId === event.payload.session_id ? current.snapshot : null,
          state:
            event.payload.state === 'failed' ? 'failed' : (event.payload.state as PlaybackUiState),
          error: event.payload.state === 'failed' ? event.payload.message : null,
        },
      });
      if (event.payload.state === 'finished' || event.payload.state === 'failed') {
        armTerminalReconciliationTimer(event.payload.session_id, startRequestId);
      } else {
        clearTerminalReconciliationTimer();
      }
      return true;
    }
    if (event.name === 'playback.snapshot') {
      const current = get().playback;
      if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return true;
      const pending = sessionLedger.pendingStartForSong(event.payload.song_id);
      const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
      const playbackContext = pending?.context ?? current.context;
      const confirmsOperation =
        confirmsPlaybackStart(
          current,
          event.payload.session_id,
          event.payload.song_id,
          pending,
          event.payload.state,
        ) ||
        (current.transportOperation === 'pausing' && event.payload.state === 'paused') ||
        (current.transportOperation === 'resuming' && event.payload.state === 'playing');
      if (confirmsOperation) clearOperationFromEvent();
      if (pending) sessionLedger.removePendingStart(pending.epoch);
      set({
        playback: {
          ...current,
          sessionId: event.payload.session_id,
          currentSong: identity ? { ...identity, title: event.payload.title } : current.currentSong,
          context: playbackContext,
          transportOperation: confirmsOperation ? null : current.transportOperation,
          startRequestId:
            current.sessionId === event.payload.session_id ||
            current.startRequestId === pending?.epoch
              ? null
              : current.startRequestId,
          state: event.payload.state as PlaybackUiState,
          snapshot: event.payload,
          error: null,
        },
      });
      return true;
    }
    if (event.name === 'playback.finished') {
      const current = get().playback;
      if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return true;
      clearTerminalReconciliationTimer();
      sessionLedger.retire(event.payload.session_id, 'finished');
      const pending = sessionLedger.pendingStartForSong(event.payload.song_id);
      const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
      const playbackContext = pending?.context ?? current.context;
      if (pending) sessionLedger.removePendingStart(pending.epoch);
      const continueOperation =
        current.transportOperation === 'advancing' || current.transportOperation === 'restarting';
      const autoAdvance =
        event.payload.outcome === 'finished' &&
        current.transportOperation !== 'stopping' &&
        !continueOperation &&
        !playbackContext?.dryRun &&
        get().settings?.auto_play === true &&
        playbackContext !== null &&
        playbackContext.currentSongId === event.payload.song_id &&
        playbackContextIsCurrent(playbackContext) &&
        playbackContext.generation === getLibrary().generation &&
        nextPlaybackPosition(playbackContext) !== null;
      const nextPosition = playbackContext ? nextPlaybackPosition(playbackContext) : null;
      if (!continueOperation) clearOperationFromEvent();
      set({
        playback: {
          ...current,
          sessionId: null,
          currentSong: identity ?? current.currentSong,
          context: playbackContext,
          prepared: null,
          preparedIdentity: null,
          preparedContext: null,
          startRequestId:
            current.sessionId === event.payload.session_id ||
            current.startRequestId === pending?.epoch
              ? null
              : current.startRequestId,
          transportOperation: continueOperation ? current.transportOperation : null,
          state: 'idle',
          snapshot: null,
          error: null,
        },
      });
      if (autoAdvance && playbackContext && nextPosition !== null) {
        void sourceTraversal.transitionContextPlayback(
          playbackContext,
          nextPosition,
          'advancing',
          null,
          sourceTraversal.autoPlaySettingsRevision(),
        );
      }
      return true;
    }

    const current = get().playback;
    if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return true;
    clearTerminalReconciliationTimer();
    sessionLedger.retire(event.payload.session_id, 'failed');
    const pending = sessionLedger.pendingStartForSong(event.payload.song_id);
    const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
    const playbackContext = pending?.context ?? current.context;
    if (pending) sessionLedger.removePendingStart(pending.epoch);
    clearOperationFromEvent();
    set({
      playback: {
        ...current,
        sessionId: null,
        currentSong: identity ?? current.currentSong,
        context: playbackContext,
        prepared: null,
        preparedIdentity: null,
        preparedContext: null,
        startRequestId:
          current.sessionId === event.payload.session_id ||
          current.startRequestId === pending?.epoch
            ? null
            : current.startRequestId,
        transportOperation: null,
        state: 'failed',
        snapshot: null,
        error: `${event.payload.code}: ${event.payload.message}`,
      },
    });
    return true;
  };

  const actions: PlaybackSliceActions = {
    ...transportActions,
    async prepareSelectedPlayback(overrides) {
      const current = get();
      const playback = current.playback;
      const library = getLibrary();
      if (playback.prepared?.admission === 'confirmation_required') return;
      if (
        playback.startRequestId !== null ||
        playback.transportOperation !== null ||
        playback.sessionId !== null ||
        ['starting', 'playing', 'paused', 'stopping'].includes(playback.state)
      ) {
        return;
      }
      const selectedSongId =
        playback.context && !playbackContextIsCurrent(playback.context)
          ? library.selectedSongId
          : (playback.currentSong?.songId ?? library.selectedSongId);
      if (!selectedSongId || !current.settings) {
        set({
          playback: { ...playback, error: 'Select a song before preparing playback.' },
        });
        return;
      }
      const reuseCurrentContext =
        playback.currentSong?.songId === selectedSongId &&
        playback.context !== null &&
        playbackContextIsCurrent(playback.context);
      const playbackContext = reuseCurrentContext
        ? playback.context!
        : preparation.contextForSong(library, selectedSongId);
      if (!playbackContext) {
        set({
          playback: {
            ...playback,
            error: 'Select a loaded Library row before preparing playback.',
          },
        });
        return;
      }
      const row = selectSongById(library, selectedSongId);
      const identity =
        reuseCurrentContext && playback.currentSong
          ? playback.currentSong
          : row
            ? identityFromRow(row, playbackContext.generation)
            : null;
      if (!identity) {
        set({
          playback: {
            ...playback,
            error: 'The selected song is not available in this Library context.',
          },
        });
        return;
      }
      const operationEpoch = beginOperation('preparing');
      if (operationEpoch === null) return;
      try {
        const prepared = await preparation.prepareForOperation(
          identity,
          playbackContext,
          overrides,
          operationEpoch,
        );
        if (!prepared || !operationIsCurrent(operationEpoch)) return;
        finishOperation(operationEpoch, prepared.error_message);
      } catch (error) {
        finishOperation(operationEpoch, errorMessage(error));
      }
    },

    async startPreparedPlayback(decision) {
      const playback = get().playback;
      const prepared = playback.prepared;
      const identity = playback.preparedIdentity;
      const playbackContext = playback.preparedContext;
      if (playback.startRequestId !== null) return;
      if (playback.transportOperation !== null) return;
      if (
        playback.sessionId !== null ||
        ['playing', 'paused', 'stopping'].includes(playback.state)
      ) {
        return;
      }
      if (!prepared?.prepared_id || !identity || !playbackContext) {
        set({ playback: { ...playback, error: 'Prepare playback before starting.' } });
        return;
      }
      if (prepared.admission === 'blocked') return;
      if (
        prepared.admission === 'confirmation_required' &&
        (!decision || !prepared.decisions.some((item) => item.decision === decision))
      )
        return;
      if (!playbackContextIsCurrent(playbackContext)) {
        set({
          playback: {
            ...playback,
            prepared: null,
            preparedIdentity: null,
            preparedContext: null,
            error: 'The Library source changed. Select a song and prepare playback again.',
          },
        });
        return;
      }
      const operationEpoch = beginOperation('starting');
      if (operationEpoch === null) return;
      try {
        await startForOperation(prepared, identity, playbackContext, decision, operationEpoch);
      } catch (error) {
        finishOperation(operationEpoch, errorMessage(error));
      }
    },

    cancelPreparedPlayback() {
      const playback = get().playback;
      if (
        playback.transportOperation !== null ||
        !['confirmation_required', 'blocked'].includes(playback.prepared?.admission ?? '')
      )
        return;
      set({
        playback: {
          ...playback,
          prepared: null,
          preparedIdentity: null,
          preparedContext: null,
          error: null,
        },
      });
    },
  };

  return {
    actions,
    clearOperationFromEvent,
    invalidatePlaybackMembershipContext,
    playbackAfterBrowserNavigation,
    playbackContextIsCurrent,
    isSessionRetired: (sessionId) => sessionLedger.isRetired(sessionId),
    cancelStaleAutoAdvanceHandoff: sourceTraversal.cancelStaleAutoAdvanceHandoff,
    handleEvent,
    onAutoPlayChanged: sourceTraversal.onAutoPlayChanged,
  };
}
