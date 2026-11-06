import { rememberRetiredSession } from './retiredSessions';
import type { PlaybackContext, PlaybackSongIdentity } from './types';

export interface PendingPlaybackStart {
  epoch: number;
  preparedId: string;
  identity: PlaybackSongIdentity;
  context: PlaybackContext;
}

export interface PlaybackSessionLedger {
  beginStart: (start: Omit<PendingPlaybackStart, 'epoch'>) => PendingPlaybackStart;
  getPendingStart: (epoch: number) => PendingPlaybackStart | undefined;
  pendingStartForSong: (songId: string) => PendingPlaybackStart | null;
  pendingStartForPreparedId: (preparedId: string) => PendingPlaybackStart | null;
  removePendingStart: (epoch: number) => void;
  retire: (sessionId: string, outcome: 'finished' | 'failed') => void;
  waitForRetirement: (
    sessionId: string,
    timeoutMs: number,
  ) => Promise<'finished' | 'failed' | null>;
  isRetired: (sessionId: string) => boolean;
}

const MAX_PENDING_PLAYBACK_STARTS = 16;

export function createPlaybackSessionLedger(): PlaybackSessionLedger {
  let startEpoch = 0;
  const pendingStarts = new Map<number, PendingPlaybackStart>();
  const retiredSessions = new Map<string, 'finished' | 'failed'>();
  const retirementWaiters = new Map<string, Set<(outcome: 'finished' | 'failed' | null) => void>>();

  const pendingStartForSong = (songId: string): PendingPlaybackStart | null => {
    const matches = [...pendingStarts.values()].filter((item) => item.identity.songId === songId);
    return matches[matches.length - 1] ?? null;
  };

  const finishRetirementWaiters = (sessionId: string, outcome: 'finished' | 'failed') => {
    const waiters = retirementWaiters.get(sessionId);
    retirementWaiters.delete(sessionId);
    waiters?.forEach((resolve) => resolve(outcome));
  };

  return {
    beginStart(start) {
      const pending: PendingPlaybackStart = { ...start, epoch: ++startEpoch };
      pendingStarts.set(pending.epoch, pending);
      while (pendingStarts.size > MAX_PENDING_PLAYBACK_STARTS) {
        const oldest = pendingStarts.keys().next().value;
        if (oldest === undefined) break;
        pendingStarts.delete(oldest);
      }
      return pending;
    },
    getPendingStart: (epoch) => pendingStarts.get(epoch),
    pendingStartForSong,
    pendingStartForPreparedId: (preparedId) =>
      [...pendingStarts.values()].find((item) => item.preparedId === preparedId) ?? null,
    removePendingStart: (epoch) => {
      pendingStarts.delete(epoch);
    },
    retire(sessionId, outcome) {
      rememberRetiredSession(retiredSessions, sessionId, outcome);
      finishRetirementWaiters(sessionId, outcome);
    },
    waitForRetirement(sessionId, timeoutMs) {
      const retired = retiredSessions.get(sessionId);
      if (retired) return Promise.resolve(retired);
      return new Promise((resolve) => {
        const waiters = retirementWaiters.get(sessionId) ?? new Set();
        const timeout = setTimeout(() => {
          waiters.delete(onRetirement);
          if (waiters.size === 0) retirementWaiters.delete(sessionId);
          resolve(null);
        }, timeoutMs);
        const onRetirement = (outcome: 'finished' | 'failed' | null) => {
          clearTimeout(timeout);
          resolve(outcome);
        };
        waiters.add(onRetirement);
        retirementWaiters.set(sessionId, waiters);
      });
    },
    isRetired: (sessionId) => retiredSessions.has(sessionId),
  };
}
