import type { DesktopBridge, PlaybackStatus } from '../bridge/DesktopBridge';
import { identityFromRow, selectSongById } from './selectors';
import type { PlaybackSessionLedger } from './playbackSessionLedger';
import type { DesktopStore, TransportOperation } from './types';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export const TRANSPORT_OPERATION_TIMEOUT_MS = 15_000;
const PLAYBACK_RECONCILIATION_TIMEOUT_MS = 2_000;

export interface PlaybackReconciliationContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  getLibrary: () => DesktopStore['library'];
  set: DesktopStoreSetter;
  sessionLedger: PlaybackSessionLedger;
}

export interface PlaybackReconciliation {
  beginOperation: (
    operation: Exclude<TransportOperation, null>,
    preemptTransport?: boolean,
  ) => number | null;
  finishOperation: (epoch: number, error?: string | null) => boolean;
  operationIsCurrent: (epoch: number) => boolean;
  getTransportOperationEpoch: () => number;
  nextReconciliationToken: () => number;
  reconcilePlaybackAfterTimeout: (
    token: number,
    operation: Exclude<TransportOperation, null>,
    epoch: number,
    expectedSessionId: string | null,
    expectedStartRequestId: number | null,
  ) => Promise<void>;
  clearOperationFromEvent: () => void;
  armTerminalReconciliationTimer: (sessionId: string, startRequestId: number | null) => void;
  clearTerminalReconciliationTimer: () => void;
}

export function createPlaybackReconciliation(
  context: PlaybackReconciliationContext,
): PlaybackReconciliation {
  const { bridge, get, getLibrary, set, sessionLedger } = context;
  let transportOperationEpoch = 0;
  let playbackReconciliationEpoch = 0;
  let transportOperationTimer: ReturnType<typeof setTimeout> | null = null;
  let terminalReconciliationTimer: ReturnType<typeof setTimeout> | null = null;

  const errorMessage = (error: unknown): string =>
    error instanceof Error ? error.message : String(error);

  const clearTransportTimer = () => {
    if (transportOperationTimer !== null) clearTimeout(transportOperationTimer);
    transportOperationTimer = null;
  };

  const clearTerminalReconciliationTimer = () => {
    if (terminalReconciliationTimer !== null) clearTimeout(terminalReconciliationTimer);
    terminalReconciliationTimer = null;
  };

  const emitReconciledTerminal = (terminal: NonNullable<PlaybackStatus['last_terminal']>) => {
    if (sessionLedger.isRetired(terminal.session_id)) return;
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
  ): Promise<void> => {
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
      const pending = status.active
        ? sessionLedger.pendingStartForPreparedId(status.active.prepared_id)
        : null;

      if (status.active) {
        const nativeSession = status.active;
        const library = getLibrary();
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
        if (current.sessionId && conflict) sessionLedger.retire(current.sessionId, 'failed');
        clearTransportTimer();
        clearTerminalReconciliationTimer();
        transportOperationEpoch += 1;
        if (pending) sessionLedger.removePendingStart(pending.epoch);
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
        expectedStartRequestId === null
          ? null
          : (sessionLedger.getPendingStart(expectedStartRequestId) ?? null);
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
        sessionLedger.removePendingStart(pendingRequestId);
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

  return {
    beginOperation,
    finishOperation,
    operationIsCurrent,
    getTransportOperationEpoch: () => transportOperationEpoch,
    nextReconciliationToken: () => ++playbackReconciliationEpoch,
    reconcilePlaybackAfterTimeout,
    clearOperationFromEvent,
    armTerminalReconciliationTimer,
    clearTerminalReconciliationTimer,
  };
}
