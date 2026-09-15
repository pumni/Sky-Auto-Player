import type { SongRow } from '../bridge/DesktopBridge';
import { LIBRARY_PAGE_SIZE } from './types';
import type {
  DetailEntry,
  DesktopStore,
  LibraryState,
  PlaybackContext,
  PlaybackIssuePresentation,
  PlaybackSongIdentity,
} from './types';

function greatestCommonDivisor(left: number, right: number): number {
  let a = Math.abs(left);
  let b = Math.abs(right);
  while (b !== 0) [a, b] = [b, a % b];
  return a;
}

export function createShuffleTraversal(
  total: number,
  originIndex: number,
  seed: string,
): PlaybackContext['shuffleTraversal'] {
  if (!Number.isInteger(total) || total < 1 || !Number.isInteger(originIndex)) return null;
  const origin = ((originIndex % total) + total) % total;
  if (total === 1) return { originIndex: origin, step: 0, position: 0 };
  let hash = 2_166_136_261;
  for (let index = 0; index < seed.length; index += 1) {
    hash ^= seed.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619) >>> 0;
  }
  let step = (hash % (total - 1)) + 1;
  while (greatestCommonDivisor(step, total) !== 1) {
    step = (step % (total - 1)) + 1;
  }
  return { originIndex: origin, step, position: 0 };
}

export function playbackContextIndexAt(context: PlaybackContext, position: number): number | null {
  if (!Number.isInteger(position) || position < 0 || position >= context.total) return null;
  const traversal = context.shuffleTraversal;
  return traversal ? (traversal.originIndex + position * traversal.step) % context.total : position;
}

export function nextPlaybackPosition(context: PlaybackContext): number | null {
  const position = context.shuffleTraversal?.position ?? context.currentIndex;
  return position + 1 < context.total ? position + 1 : null;
}

export function previousPlaybackPosition(
  context: PlaybackContext,
  currentUs: number,
): number | null {
  const position = context.shuffleTraversal?.position ?? context.currentIndex;
  if (currentUs > 3_000_000) return position;
  return position > 0 ? position - 1 : null;
}

const TARGET_FAILURE_TITLES: Record<string, string> = {
  target_not_found: 'Sky window was not found',
  target_integrity_mismatch: 'Sky is running with different permissions',
  target_focus_failed: 'Sky could not be focused',
};

export function playbackIssuePresentation(
  playback: Pick<DesktopStore['playback'], 'error' | 'sessionId' | 'startRequestId' | 'state'>,
): PlaybackIssuePresentation | null {
  const message = playback.error;
  if (!message) return null;

  if (message.startsWith('Playback status is unavailable:')) {
    return {
      kind: 'recoverable_status',
      title: 'Playback status is unavailable',
      message: playback.sessionId
        ? 'Sky Auto Player cannot confirm the current session.'
        : 'Sky Auto Player cannot confirm whether playback started.',
    };
  }

  const targetFailure = /^(target_[a-z_]+):\s*(.*)$/s.exec(message);
  const targetFailureCode = targetFailure?.[1];
  if (targetFailure && targetFailureCode && TARGET_FAILURE_TITLES[targetFailureCode]) {
    return {
      kind: 'target_failure',
      title: TARGET_FAILURE_TITLES[targetFailureCode],
      message: targetFailure[2] || message,
    };
  }

  if (playback.sessionId !== null || playback.startRequestId !== null) {
    const conflict = message.startsWith('A different native playback session is active.');
    return {
      kind: conflict ? 'conflict' : 'recoverable_status',
      title: conflict
        ? 'Another native playback session is active'
        : playback.startRequestId !== null
          ? 'Playback status is unavailable'
          : 'Playback status needs attention',
      message,
    };
  }

  if (message.startsWith('Native playback did not create a session.')) {
    return {
      kind: 'playback_failure',
      title: 'Playback could not start',
      message: 'No active playback session was created.',
    };
  }

  return { kind: 'playback_failure', title: 'Playback failed', message };
}

export function selectSelectedDetail(
  store: Pick<DesktopStore, 'library' | 'details'>,
): DetailEntry {
  const selectedSongId = store.library.selectedSongId;
  return selectedSongId
    ? (store.details.bySongId.get(selectedSongId) ?? EMPTY_DETAIL)
    : EMPTY_DETAIL;
}

export function selectRowAtIndex(library: Pick<LibraryState, 'pages'>, index: number) {
  if (index < 0) return undefined;
  const offset = Math.floor(index / LIBRARY_PAGE_SIZE) * LIBRARY_PAGE_SIZE;
  return library.pages.get(offset)?.[index - offset];
}

export function selectSongById(
  library: Pick<LibraryState, 'pages' | 'indexById'>,
  songId: string | null,
) {
  const index = songId ? library.indexById.get(songId) : undefined;
  return index === undefined ? undefined : selectRowAtIndex(library, index);
}

export function playbackContextMatchesLibrary(
  context: PlaybackContext | null,
  library: LibraryState,
): context is PlaybackContext {
  return Boolean(
    context?.valid &&
    context.generation === library.generation &&
    context.query === library.query &&
    context.source.kind === library.searchSource.kind &&
    context.source.id === library.searchSource.id,
  );
}

export function selectNowPlayingSongId(
  store: Pick<DesktopStore, 'library' | 'playback'>,
): string | null {
  const { currentSong } = store.playback;
  return currentSong?.generation === store.library.generation ? currentSong.songId : null;
}

export function identityFromRow(row: SongRow, generation: number): PlaybackSongIdentity {
  return {
    songId: row.song_id,
    title: row.title,
    liked: row.liked,
    durationUs: row.duration_us,
    formatLabel: row.format_label,
    noteCount: row.note_count,
    riskLevel: row.risk_level,
    generation,
  };
}

const EMPTY_DETAIL: DetailEntry = { state: 'idle', value: null, error: null };
