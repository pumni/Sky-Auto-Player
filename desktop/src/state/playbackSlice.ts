import type {
  DesktopBridge,
  LibrarySource,
  PlaybackConfig,
  PlaybackDecisionAcceptance,
  PlaybackDecisionId,
  PlaybackStatus,
  SearchResult,
  SongRow,
  UiEvent,
} from '../bridge/DesktopBridge';
import { rememberRetiredSession } from './retiredSessions';
import { sourceKey } from './libraryHelpers';
import {
  createShuffleTraversal,
  identityFromRow,
  nextPlaybackPosition,
  playbackContextIndexAt,
  previousPlaybackPosition,
  selectRowAtIndex,
  selectSongById,
} from './selectors';
import { AUTO_PLAY_HANDOFF_MS, LIBRARY_PAGE_SIZE } from './types';
import type {
  DesktopStore,
  LibraryState,
  PlaybackContext,
  PlaybackSongIdentity,
  PlaybackUiState,
  TransportOperation,
} from './types';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

interface PendingPlaybackStart {
  epoch: number;
  preparedId: string;
  identity: PlaybackSongIdentity;
  context: PlaybackContext;
}

export interface PlaybackSliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
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
  const { bridge, get, set, loadPage } = context;
  let startRequestEpoch = 0;
  let shuffleTraversalSequence = 0;
  const pendingStarts = new Map<number, PendingPlaybackStart>();
  let transportOperationEpoch = 0;
  let playbackReconciliationEpoch = 0;
  let playbackStatusRetryPromise: Promise<void> | null = null;
  let playbackStatusRetryToken = 0;
  let transportOperationTimer: ReturnType<typeof setTimeout> | null = null;
  let terminalReconciliationTimer: ReturnType<typeof setTimeout> | null = null;
  const retiredSessions = new Map<string, 'finished' | 'failed'>();
  const retirementWaiters = new Map<string, Set<(outcome: 'finished' | 'failed' | null) => void>>();
  const TRANSPORT_OPERATION_TIMEOUT_MS = 15_000;
  const PLAYBACK_RECONCILIATION_TIMEOUT_MS = 2_000;
  const MAX_PENDING_PLAYBACK_STARTS = 16;
  const sourceMembershipRevisions = new Map<string, number>();
  let autoPlaySettingsRevision = 0;
  let pendingAutoAdvanceHandoff: {
    cancel: () => void;
    isCurrent: (state: DesktopStore) => boolean;
  } | null = null;

  const pendingStartForSong = (songId: string): PendingPlaybackStart | null => {
    const matches = [...pendingStarts.values()].filter((item) => item.identity.songId === songId);
    return matches[matches.length - 1] ?? null;
  };
  const pendingStartForPreparedId = (preparedId: string): PendingPlaybackStart | null =>
    [...pendingStarts.values()].find((item) => item.preparedId === preparedId) ?? null;
  const membershipRevisionFor = (source: LibrarySource) =>
    sourceMembershipRevisions.get(sourceKey(source)) ?? 0;
  const playbackContextIsCurrent = (playbackContext: PlaybackContext) =>
    playbackContext.valid &&
    playbackContext.membershipRevision === membershipRevisionFor(playbackContext.source);

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
    if (retiredSessions.has(sessionId)) return false;
    if (current.sessionId === sessionId) return true;
    if (current.sessionId !== null) return false;
    const pending = pendingStartForSong(songId);
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
    const pending = pendingStartForSong(songId);
    if (pending) return pending.identity;
    if (current.currentSong?.songId === songId) return current.currentSong;
    return null;
  };

  const finishRetirementWaiters = (sessionId: string, outcome: 'finished' | 'failed') => {
    const waiters = retirementWaiters.get(sessionId);
    retirementWaiters.delete(sessionId);
    waiters?.forEach((resolve) => resolve(outcome));
  };

  const retireSession = (sessionId: string, outcome: 'finished' | 'failed') => {
    rememberRetiredSession(retiredSessions, sessionId, outcome);
    finishRetirementWaiters(sessionId, outcome);
  };

  const waitForRetirement = (sessionId: string): Promise<'finished' | 'failed' | null> => {
    const retired = retiredSessions.get(sessionId);
    if (retired) return Promise.resolve(retired);
    return new Promise((resolve) => {
      const waiters = retirementWaiters.get(sessionId) ?? new Set();
      const timeout = setTimeout(() => {
        waiters.delete(onRetirement);
        if (waiters.size === 0) retirementWaiters.delete(sessionId);
        resolve(null);
      }, TRANSPORT_OPERATION_TIMEOUT_MS);
      const onRetirement = (outcome: 'finished' | 'failed' | null) => {
        clearTimeout(timeout);
        resolve(outcome);
      };
      waiters.add(onRetirement);
      retirementWaiters.set(sessionId, waiters);
    });
  };

  const clearTransportTimer = () => {
    if (transportOperationTimer !== null) clearTimeout(transportOperationTimer);
    transportOperationTimer = null;
  };

  const clearTerminalReconciliationTimer = () => {
    if (terminalReconciliationTimer !== null) clearTimeout(terminalReconciliationTimer);
    terminalReconciliationTimer = null;
  };

  const armTerminalReconciliationTimer = (sessionId: string, startRequestId: number | null) => {
    clearTerminalReconciliationTimer();
    const token = ++playbackReconciliationEpoch;
    const epoch = transportOperationEpoch;
    terminalReconciliationTimer = setTimeout(() => {
      terminalReconciliationTimer = null;
      const current = get().playback;
      if (current.sessionId !== sessionId || current.startRequestId !== startRequestId) return;
      void reconcilePlaybackAfterTimeout(token, 'stopping', epoch, sessionId, startRequestId);
    }, TRANSPORT_OPERATION_TIMEOUT_MS);
  };

  const emitReconciledTerminal = (terminal: NonNullable<PlaybackStatus['last_terminal']>) => {
    if (retiredSessions.has(terminal.session_id)) return;
    if (terminal.state === 'failed') {
      get().applyEvent({
        v: 1,
        name: 'playback.failed',
        payload: {
          session_id: terminal.session_id,
          song_id: terminal.song_id,
          code: terminal.failure_code ?? 'playback_failed',
          message:
            terminal.failure_message ??
            'Playback failed before the terminal event reached the interface.',
        },
      });
    } else {
      const outcome = terminal.outcome ?? 'quit';
      get().applyEvent({
        v: 1,
        name: 'playback.finished',
        payload: {
          session_id: terminal.session_id,
          song_id: terminal.song_id,
          outcome,
          total_us: 0,
          message: outcome === 'finished' ? 'Playback finished' : 'Playback stopped',
        },
      });
    }
  };

  const queryPlaybackStatus = async (): Promise<PlaybackStatus> => {
    let timeout: ReturnType<typeof setTimeout> | null = null;
    try {
      return await Promise.race([
        bridge.getPlaybackStatus(),
        new Promise<never>((_, reject) => {
          timeout = setTimeout(
            () => reject(new Error('Native playback status query timed out.')),
            PLAYBACK_RECONCILIATION_TIMEOUT_MS,
          );
        }),
      ]);
    } finally {
      if (timeout !== null) clearTimeout(timeout);
    }
  };

  const reconcilePlaybackAfterTimeout = async (
    token: number,
    operation: Exclude<TransportOperation, null>,
    epoch: number,
    expectedSessionId: string | null,
    expectedStartRequestId: number | null,
  ) => {
    const canApply = () => {
      if (playbackReconciliationEpoch !== token) return false;
      const current = get().playback;
      return (
        (current.sessionId === expectedSessionId &&
          current.startRequestId === expectedStartRequestId) ||
        (current.transportOperation === operation && transportOperationEpoch === epoch)
      );
    };

    try {
      const status = await queryPlaybackStatus();
      if (!canApply()) return;
      const current = get().playback;
      const pending = status.active ? pendingStartForPreparedId(status.active.prepared_id) : null;

      if (status.active) {
        const nativeSession = status.active;
        const library = get().library;
        const row = selectSongById(library, nativeSession.song_id);
        const identity =
          pending?.identity ??
          (current.currentSong?.songId === nativeSession.song_id
            ? current.currentSong
            : row
              ? identityFromRow(row, library.generation)
              : {
                  songId: nativeSession.song_id,
                  title: nativeSession.title,
                  liked: false,
                  durationUs: null,
                  formatLabel: 'SKY',
                  noteCount: null,
                  riskLevel: 'unknown' as const,
                  generation: library.generation,
                });
        const playbackContext =
          pending?.context ??
          (current.currentSong?.songId === nativeSession.song_id ? current.context : null);
        const conflict =
          (current.sessionId !== null && current.sessionId !== nativeSession.session_id) ||
          (current.startRequestId !== null && current.startRequestId !== pending?.epoch);
        if (current.sessionId && conflict) retireSession(current.sessionId, 'failed');
        clearTransportTimer();
        clearTerminalReconciliationTimer();
        transportOperationEpoch += 1;
        if (pending) pendingStarts.delete(pending.epoch);
        set({
          playback: {
            ...current,
            sessionId: nativeSession.session_id,
            currentSong: { ...identity, title: nativeSession.title },
            context: playbackContext,
            prepared: null,
            preparedIdentity: null,
            preparedContext: null,
            startRequestId: null,
            transportOperation: null,
            state: nativeSession.state,
            snapshot:
              current.snapshot?.session_id === nativeSession.session_id ? current.snapshot : null,
            error: conflict
              ? 'A different native playback session is active. Stop it before starting another song.'
              : ['starting', 'stopping'].includes(nativeSession.state)
                ? 'The app could not confirm the current session state.'
                : null,
            statusRetryPending: false,
          },
        });
        return;
      }

      const terminal = status.last_terminal;
      const expectedPendingStart =
        expectedStartRequestId === null ? null : pendingStarts.get(expectedStartRequestId);
      const matchingTerminal =
        terminal &&
        (terminal.session_id === expectedSessionId ||
          terminal.session_id === current.sessionId ||
          expectedPendingStart?.preparedId === terminal.prepared_id);
      if (matchingTerminal) {
        clearTerminalReconciliationTimer();
        emitReconciledTerminal(terminal);
        if (
          (operation === 'advancing' || operation === 'restarting') &&
          get().playback.transportOperation === operation &&
          transportOperationEpoch === epoch
        ) {
          armOperationTimer(epoch, operation);
        }
        return;
      }

      if (current.sessionId !== null) {
        clearTerminalReconciliationTimer();
        get().applyEvent({
          v: 1,
          name: 'playback.failed',
          payload: {
            session_id: current.sessionId,
            song_id: current.currentSong?.songId ?? '',
            code: 'session_retired',
            message: 'The native playback session has retired. Press Play to start again.',
          },
        });
        return;
      }

      clearTransportTimer();
      clearTerminalReconciliationTimer();
      transportOperationEpoch += 1;
      const pendingRequestId = current.startRequestId;
      if (pendingRequestId !== null) {
        pendingStarts.delete(pendingRequestId);
        set({
          playback: {
            ...current,
            prepared: null,
            preparedIdentity: null,
            preparedContext: null,
            startRequestId: null,
            transportOperation: null,
            state: 'idle',
            snapshot: null,
            error:
              'Native playback did not create a session. No active playback session was created. Press Play to try again.',
          },
        });
      } else {
        set({
          playback: {
            ...current,
            transportOperation: null,
            error:
              'Playback did not confirm the requested operation. Check the session and try again.',
          },
        });
      }
    } catch (error) {
      if (!canApply()) return;
      clearTransportTimer();
      clearTerminalReconciliationTimer();
      transportOperationEpoch += 1;
      const current = get().playback;
      set({
        playback: {
          ...current,
          transportOperation: null,
          error: `Playback status is unavailable: ${errorMessage(error)}`,
        },
      });
    }
  };

  const operationTimedOut = (epoch: number, operation: Exclude<TransportOperation, null>) => {
    if (transportOperationEpoch !== epoch) return;
    transportOperationTimer = null;
    const current = get().playback;
    const token = ++playbackReconciliationEpoch;
    const transitionCanContinue = operation === 'advancing' || operation === 'restarting';
    if (!transitionCanContinue) {
      transportOperationEpoch += 1;
      set({
        playback: {
          ...current,
          transportOperation: null,
          error: 'Playback confirmation timed out. Checking the native session state…',
        },
      });
    }
    void reconcilePlaybackAfterTimeout(
      token,
      operation,
      epoch,
      current.sessionId,
      current.startRequestId,
    );
  };

  const armOperationTimer = (epoch: number, operation: Exclude<TransportOperation, null>) => {
    clearTransportTimer();
    clearTerminalReconciliationTimer();
    transportOperationTimer = setTimeout(
      () => operationTimedOut(epoch, operation),
      TRANSPORT_OPERATION_TIMEOUT_MS,
    );
  };

  const beginOperation = (
    operation: Exclude<TransportOperation, null>,
    preemptTransport = false,
  ): number | null => {
    const current = get().playback;
    if (current.transportOperation !== null && !(preemptTransport && operation === 'stopping'))
      return null;
    clearTransportTimer();
    const epoch = ++transportOperationEpoch;
    playbackReconciliationEpoch += 1;
    set({
      playback: {
        ...current,
        transportOperation: operation,
        error: operation === 'stopping' ? current.error : null,
      },
    });
    armOperationTimer(epoch, operation);
    return epoch;
  };

  const operationIsCurrent = (epoch: number) => transportOperationEpoch === epoch;

  const finishOperation = (epoch: number, error: string | null = null) => {
    if (!operationIsCurrent(epoch)) return false;
    clearTransportTimer();
    transportOperationEpoch += 1;
    set({ playback: { ...get().playback, transportOperation: null, error } });
    return true;
  };

  const clearOperationFromEvent = () => {
    clearTransportTimer();
    transportOperationEpoch += 1;
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

  const contextForSong = (library: LibraryState, songId: string): PlaybackContext | null => {
    const currentIndex = library.indexById.get(songId);
    if (currentIndex === undefined) return null;
    const shuffleEnabled = get().playback.shuffleEnabled;
    return {
      source: library.searchSource,
      query: library.query,
      generation: library.generation,
      total: library.resultTotal,
      currentIndex,
      currentSongId: songId,
      shuffleTraversal: shuffleEnabled
        ? createShuffleTraversal(
            library.resultTotal,
            currentIndex,
            `${library.generation}:${sourceKey(library.searchSource)}:${library.query}:${songId}:${++shuffleTraversalSequence}`,
          )
        : null,
      dryRun: false,
      membershipRevision: membershipRevisionFor(library.searchSource),
      valid: true,
    };
  };

  const resolveContextRow = async (
    playbackContext: PlaybackContext,
    index: number,
  ): Promise<SongRow> => {
    const library = get().library;
    if (
      playbackContext.generation !== library.generation ||
      !playbackContextIsCurrent(playbackContext)
    ) {
      throw new Error(
        'The Library context changed after playback started. Select a song to create a new playback context.',
      );
    }
    if (!Number.isInteger(index) || index < 0 || index >= playbackContext.total) {
      throw new Error('There is no song at that playback-context position.');
    }
    const offset = Math.floor(index / LIBRARY_PAGE_SIZE) * LIBRARY_PAGE_SIZE;
    const visibleContext =
      library.generation === playbackContext.generation &&
      library.query === playbackContext.query &&
      sourceKey(library.searchSource) === sourceKey(playbackContext.source);
    const visibleRow = visibleContext ? selectRowAtIndex(library, index) : undefined;
    if (visibleRow) return visibleRow;
    const result = await loadPage(
      playbackContext.query,
      playbackContext.source,
      offset,
      playbackContext.generation,
      library.searchRequestGeneration,
      false,
    );
    if (get().library.generation !== playbackContext.generation) {
      throw new Error('The Library changed while the adjacent song was being resolved.');
    }
    const row = result.items[index - result.offset];
    if (!row) throw new Error('The adjacent song is no longer available in this Library context.');
    return row;
  };

  const makePlaybackConfig = (
    overrides?: Partial<PlaybackConfig>,
    inheritedDryRun = false,
  ): PlaybackConfig => {
    const settings = get().settings;
    if (!settings) throw new Error('Playback settings are not ready.');
    return {
      hold_frames: overrides?.hold_frames ?? settings.playback_defaults.hold_frames,
      timing_margin_us: overrides?.timing_margin_us ?? settings.playback_defaults.timing_margin_us,
      tempo_scale: overrides?.tempo_scale ?? settings.playback_defaults.tempo_scale,
      fps: overrides?.fps ?? settings.playback_defaults.fps,
      dry_run: overrides?.dry_run ?? inheritedDryRun,
    };
  };

  const prepareForOperation = async (
    identity: PlaybackSongIdentity,
    playbackContext: PlaybackContext,
    overrides: Partial<PlaybackConfig> | undefined,
    operationEpoch: number,
  ): Promise<NonNullable<DesktopStore['playback']['prepared']> | null> => {
    if (!playbackContextIsCurrent(playbackContext)) {
      throw new Error(
        'The Library source changed. Select a song to create a new playback context.',
      );
    }
    const config = makePlaybackConfig(overrides, playbackContext.dryRun);
    const prepared = await bridge.preparePlayback({
      songId: identity.songId,
      generation: playbackContext.generation,
      config,
    });
    if (
      playbackContext.generation !== get().library.generation ||
      !playbackContextIsCurrent(playbackContext)
    ) {
      throw new Error(
        'The Library context changed while playback was being prepared. Select a song to create a new context.',
      );
    }
    if (!operationIsCurrent(operationEpoch)) return null;
    const preparedContext = { ...playbackContext, dryRun: config.dry_run };
    set({
      playback: {
        ...get().playback,
        prepared,
        preparedIdentity: identity,
        preparedContext,
        error: prepared.error_message,
      },
    });
    return prepared;
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
    const startEpoch = ++startRequestEpoch;
    pendingStarts.set(startEpoch, {
      epoch: startEpoch,
      preparedId: prepared.prepared_id,
      identity,
      context: playbackContext,
    });
    while (pendingStarts.size > MAX_PENDING_PLAYBACK_STARTS) {
      const oldest = pendingStarts.keys().next().value;
      if (oldest === undefined) break;
      pendingStarts.delete(oldest);
    }
    set({
      playback: {
        ...get().playback,
        startRequestId: startEpoch,
        transportOperation: 'starting',
        error: null,
      },
    });
    const session = await bridge
      .startPlayback({
        preparedId: prepared.prepared_id,
        decisions,
      })
      .catch((error: unknown) => {
        pendingStarts.delete(startEpoch);
        const current = get().playback;
        if (current.startRequestId === startEpoch) {
          set({ playback: { ...current, startRequestId: null } });
        }
        throw error;
      });
    if (!operationIsCurrent(operationEpoch) || retiredSessions.has(session.session_id)) {
      pendingStarts.delete(startEpoch);
      const current = get().playback;
      const ownsRequest = current.startRequestId === startEpoch;
      if (retiredSessions.has(session.session_id)) {
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
    pendingStarts.delete(startEpoch);
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

  const transitionContextPlayback = async (
    playbackContext: PlaybackContext,
    targetPosition: number,
    operation: 'advancing' | 'restarting',
    sessionId: string | null,
    naturalAutoPlayRevision?: number,
  ) => {
    const epoch = beginOperation(operation);
    if (epoch === null) return;
    try {
      if (
        playbackContext.generation !== get().library.generation ||
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
      if (naturalAutoPlayRevision !== undefined) {
        const handoffIsCurrent = (current: DesktopStore) => {
          const owner = current.playback.context;
          return (
            operationIsCurrent(epoch) &&
            current.playback.transportOperation === 'advancing' &&
            autoPlaySettingsRevision === naturalAutoPlayRevision &&
            current.settings?.auto_play === true &&
            playbackContext.generation === current.library.generation &&
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
      finishOperation(epoch, errorMessage(error));
    }
  };

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
      const pending = pendingStartForSong(event.payload.song_id);
      const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
      const playbackContext = pending?.context ?? current.context;
      const confirmsOperation =
        (current.transportOperation === 'starting' && event.payload.state === 'playing') ||
        (current.transportOperation === 'pausing' && event.payload.state === 'paused') ||
        (current.transportOperation === 'resuming' && event.payload.state === 'playing');
      if (confirmsOperation) clearOperationFromEvent();
      if (pending) pendingStarts.delete(pending.epoch);
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
      const pending = pendingStartForSong(event.payload.song_id);
      const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
      const playbackContext = pending?.context ?? current.context;
      const confirmsOperation =
        (current.transportOperation === 'starting' && event.payload.state === 'playing') ||
        (current.transportOperation === 'pausing' && event.payload.state === 'paused') ||
        (current.transportOperation === 'resuming' && event.payload.state === 'playing');
      if (confirmsOperation) clearOperationFromEvent();
      if (pending) pendingStarts.delete(pending.epoch);
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
      retireSession(event.payload.session_id, 'finished');
      const pending = pendingStartForSong(event.payload.song_id);
      const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
      const playbackContext = pending?.context ?? current.context;
      if (pending) pendingStarts.delete(pending.epoch);
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
        playbackContext.generation === get().library.generation &&
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
        void transitionContextPlayback(
          playbackContext,
          nextPosition,
          'advancing',
          null,
          autoPlaySettingsRevision,
        );
      }
      return true;
    }

    const current = get().playback;
    if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return true;
    clearTerminalReconciliationTimer();
    retireSession(event.payload.session_id, 'failed');
    const pending = pendingStartForSong(event.payload.song_id);
    const identity = bindSessionIdentity(event.payload.session_id, event.payload.song_id);
    const playbackContext = pending?.context ?? current.context;
    if (pending) pendingStarts.delete(pending.epoch);
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
    async prepareSelectedPlayback(overrides) {
      const current = get();
      const playback = current.playback;
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
          ? current.library.selectedSongId
          : (playback.currentSong?.songId ?? current.library.selectedSongId);
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
        : contextForSong(current.library, selectedSongId);
      if (!playbackContext) {
        set({
          playback: {
            ...playback,
            error: 'Select a loaded Library row before preparing playback.',
          },
        });
        return;
      }
      const row = selectSongById(current.library, selectedSongId);
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
        const prepared = await prepareForOperation(
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

    async stopPlayback() {
      const playback = get().playback;
      const sessionId = playback.sessionId;
      const preemptTransport = playback.transportOperation !== null;
      if (
        !sessionId ||
        !['starting', 'playing', 'paused', 'stopping', 'finished', 'failed'].includes(
          playback.state,
        )
      )
        return;
      const operationEpoch = beginOperation('stopping', preemptTransport);
      if (operationEpoch === null) return;
      try {
        const acknowledgement = await bridge.stopPlayback({ sessionId });
        if (['finished', 'failed'].includes(acknowledgement.state)) {
          const token = ++playbackReconciliationEpoch;
          await reconcilePlaybackAfterTimeout(
            token,
            'stopping',
            operationEpoch,
            sessionId,
            playback.startRequestId,
          );
        }
      } catch (error) {
        finishOperation(operationEpoch, errorMessage(error));
      }
    },

    retryPlaybackStatus() {
      if (playbackStatusRetryPromise) return playbackStatusRetryPromise;
      const playback = get().playback;
      const token = ++playbackReconciliationEpoch;
      playbackStatusRetryToken = token;
      const operation = playback.transportOperation ?? 'starting';
      const epoch = transportOperationEpoch;
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
        finishOperation(operationEpoch, errorMessage(error));
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
        finishOperation(operationEpoch, errorMessage(error));
      }
    },

    setShuffleEnabled(enabled) {
      const playback = get().playback;
      if (
        playback.shuffleEnabled === enabled ||
        playback.transportOperation !== null ||
        playback.prepared?.admission === 'confirmation_required'
      )
        return;
      const updateTraversal = (playbackContext: PlaybackContext | null): PlaybackContext | null => {
        if (!playbackContext || !playbackContextIsCurrent(playbackContext)) return playbackContext;
        return {
          ...playbackContext,
          shuffleTraversal: enabled
            ? createShuffleTraversal(
                playbackContext.total,
                playbackContext.currentIndex,
                `${playbackContext.generation}:${sourceKey(playbackContext.source)}:${playbackContext.query}:${playbackContext.currentSongId}:${++shuffleTraversalSequence}`,
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
      )
        return;
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
      )
        return;
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

  return {
    actions,
    clearOperationFromEvent,
    invalidatePlaybackMembershipContext,
    playbackAfterBrowserNavigation,
    playbackContextIsCurrent,
    isSessionRetired: (sessionId) => retiredSessions.has(sessionId),
    cancelStaleAutoAdvanceHandoff: (state) => {
      const pending = pendingAutoAdvanceHandoff;
      if (pending && !pending.isCurrent(state)) pending.cancel();
    },
    handleEvent,
    onAutoPlayChanged: () => {
      autoPlaySettingsRevision += 1;
    },
  };
}
