import { Heart, Music2 } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { playbackSessionStatusConcern, selectSongById } from '../../state/store';
import { playerStateLabel } from './playerFormatting';

interface PlayerTrackInfoProps {
  useStore: DesktopStoreHook;
}

export function PlayerTrackInfo({ useStore }: PlayerTrackInfoProps) {
  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const selectedRow = useStore((store) =>
    selectSongById(store.library, store.library.selectedSongId),
  );
  const currentSong = useStore((store) => store.playback.currentSong);
  const preparedSong = useStore((store) => store.playback.prepared?.song);
  const preparedRisk = useStore((store) => store.playback.prepared?.risk);
  const playback = useStore((store) => store.playback);
  const active = ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const sessionStatusConcern = playbackSessionStatusConcern(playback);
  const setSongLiked = useStore((store) => store.setSongLiked);

  const fallbackIdentity = selectedRow;
  const title =
    (active || sessionStatusConcern !== null ? currentSong?.title : undefined) ??
    preparedSong?.title ??
    currentSong?.title ??
    fallbackIdentity?.title ??
    'No song selected';
  const liked = currentSong?.liked ?? fallbackIdentity?.liked ?? false;
  const likeSongId = currentSong?.songId ?? selectedSongId;
  const statusConcernLabel =
    sessionStatusConcern === 'recoverable_status'
      ? 'Playback status uncertain'
      : sessionStatusConcern === 'conflict'
        ? 'Native session conflict'
        : null;
  const baseStatus =
    statusConcernLabel ??
    (active
      ? playerStateLabel(playback.state)
      : !currentSong && !preparedSong && !fallbackIdentity
        ? 'Select a song from your Library'
        : preparedSong
          ? 'Prepared'
          : currentSong
            ? 'Last played'
            : 'Ready to play');
  const dryRun =
    (playback.context?.dryRun === true || playback.preparedContext?.dryRun === true) &&
    (active || sessionStatusConcern !== null);
  const status = dryRun ? `Dry run · ${baseStatus}` : baseStatus;

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
            aria-label={`${title}: ${status}${dryRun ? '. No physical input is sent.' : ''}`}
            aria-live="polite"
            aria-atomic="true"
            title={status}
          >
            {status}
          </span>
        </div>
      </div>
      {likeSongId && (currentSong || fallbackIdentity) && (
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
        <span
          className={`player-risk risk-${preparedRisk.level}`}
          aria-label={preparedRisk.headline}
        >
          {preparedRisk.level}
        </span>
      )}
    </div>
  );
}
