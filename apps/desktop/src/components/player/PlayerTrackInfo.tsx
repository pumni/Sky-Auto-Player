import { Heart, Music2 } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { playbackSessionStatusConcern, selectSongById } from '../../state/store';
import { StatusBadge } from '../ui/StatusBadge';
import { playerStateLabel } from './playerFormatting';

interface PlayerTrackInfoProps {
  useStore: DesktopStoreHook;
}

export function PlayerTrackInfo({ useStore }: PlayerTrackInfoProps) {
  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const selectedRow = useStore((store) =>
    selectSongById(store.library, store.library.selectedSongId),
  );
  const currentSongId = useStore((store) => store.playback.currentSong?.songId ?? null);
  const currentSongTitle = useStore((store) => store.playback.currentSong?.title);
  const currentSongLiked = useStore((store) => store.playback.currentSong?.liked);
  const hasCurrentSong = useStore((store) => store.playback.currentSong !== null);
  const preparedSongTitle = useStore((store) => store.playback.prepared?.song.title);
  const preparedRisk = useStore((store) => store.playback.prepared?.risk);
  const state = useStore((store) => store.playback.state);
  const sessionStatusConcern = useStore((store) => playbackSessionStatusConcern(store.playback));
  const active = ['starting', 'playing', 'paused', 'stopping'].includes(state);
  const playbackMode = useStore((store) => {
    const playback = store.playback;
    const ownsSession =
      ['starting', 'playing', 'paused', 'stopping'].includes(playback.state) ||
      playbackSessionStatusConcern(playback) !== null;
    if (ownsSession) {
      if (playback.context) return playback.context.dryRun ? 'test' : 'live';
      if (playback.preparedContext) return playback.preparedContext.dryRun ? 'test' : 'live';
    }
    if (playback.prepared) return playback.prepared.config.dry_run ? 'test' : 'live';
    return null;
  });
  const setSongLiked = useStore((store) => store.setSongLiked);

  const fallbackIdentity = selectedRow;
  const title =
    (active || sessionStatusConcern !== null ? currentSongTitle : undefined) ??
    preparedSongTitle ??
    currentSongTitle ??
    fallbackIdentity?.title ??
    'No song selected';
  const liked = currentSongLiked ?? fallbackIdentity?.liked ?? false;
  const likeSongId = currentSongId ?? selectedSongId;
  const statusConcernLabel =
    sessionStatusConcern === 'recoverable_status'
      ? 'Playback status uncertain'
      : sessionStatusConcern === 'conflict'
        ? 'Native session conflict'
        : null;
  const baseStatus =
    statusConcernLabel ??
    (active
      ? playerStateLabel(state)
      : !hasCurrentSong && !preparedSongTitle && !fallbackIdentity
        ? 'Select a song from your Library'
        : preparedSongTitle
          ? 'Prepared'
          : hasCurrentSong
            ? 'Last played'
            : 'Ready to play');
  const statusAccessibilityLabel =
    `${title}: ${baseStatus}` +
    (playbackMode === 'test'
      ? '. Dry run; no physical input is sent.'
      : playbackMode === 'live'
        ? '. Normal playback may send physical input.'
        : '');
  const testModeDescription = 'Dry run: no physical keyboard input is sent.';

  return (
    <div className="player-track-info" role="group" aria-label={`Playback for ${title}`}>
      <span className="player-track-icon" aria-hidden="true">
        <Music2 size={17} />
      </span>
      <div className="player-track-copy">
        <strong title={title}>{title}</strong>
        <div className="player-track-subtitle">
          <span
            className={`muted player-track-status${active ? ' is-active' : ''}${sessionStatusConcern !== null ? ' is-attention' : ''}`}
            aria-label={statusAccessibilityLabel}
            aria-live="polite"
            aria-atomic="true"
            title={baseStatus}
          >
            {baseStatus}
          </span>
          {playbackMode === 'test' && (
            <StatusBadge
              className="player-test-mode"
              appearance="plain"
              aria-label={testModeDescription}
              title={testModeDescription}
            >
              Test<span className="player-test-mode-detail"> · no keys</span>
            </StatusBadge>
          )}
        </div>
      </div>
      {likeSongId && (hasCurrentSong || fallbackIdentity) && (
        <button
          className={`icon-button player-like-button${liked ? ' is-liked' : ''}`}
          type="button"
          aria-label={liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
          aria-pressed={liked}
          title={liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
          onClick={() => void setSongLiked(likeSongId, !liked)}
        >
          <Heart size={16} fill={liked ? 'currentColor' : 'none'} aria-hidden="true" />
        </button>
      )}
      {preparedRisk && ['medium', 'high'].includes(preparedRisk.level) && (
        <StatusBadge
          className="player-risk"
          tone={preparedRisk.level === 'high' ? 'danger' : 'warning'}
          aria-label={preparedRisk.headline}
          title={preparedRisk.headline}
        >
          {preparedRisk.level === 'high' ? 'High risk' : 'Medium risk'}
        </StatusBadge>
      )}
    </div>
  );
}
