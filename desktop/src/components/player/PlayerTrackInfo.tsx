import { Heart, Music2 } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { selectSongById } from '../../state/store';
import { formatPlayerDuration, playerStateLabel } from './playerFormatting';

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
  const active = useStore((store) =>
    ['starting', 'playing', 'paused', 'stopping'].includes(store.playback.state),
  );
  const playbackState = useStore((store) => store.playback.state);
  const playbackError = useStore((store) => store.playback.error);
  const setSongLiked = useStore((store) => store.setSongLiked);

  const fallbackIdentity = selectedRow;
  const title =
    currentSong?.title ?? preparedSong?.title ?? fallbackIdentity?.title ?? 'No song selected';
  const liked = currentSong?.liked ?? fallbackIdentity?.liked ?? false;
  const likeSongId = currentSong?.songId ?? selectedSongId;
  const metadata = currentSong
    ? `${currentSong.formatLabel} · ${
        currentSong.durationUs === null ? '…' : formatPlayerDuration(currentSong.durationUs)
      } · ${currentSong.noteCount === null ? '…' : currentSong.noteCount} notes`
    : fallbackIdentity
      ? `${fallbackIdentity.format_label} · ${
          fallbackIdentity.duration_us === null
            ? '…'
            : formatPlayerDuration(fallbackIdentity.duration_us)
        } · ${fallbackIdentity.note_count === null ? '…' : fallbackIdentity.note_count} notes`
      : 'Preparing metadata…';
  const trackSubtitle =
    !currentSong && !selectedSongId
      ? 'Select a song from your Library'
      : playbackError
        ? 'Playback error'
        : active
          ? playerStateLabel(playbackState)
          : currentSong && playbackState === 'idle'
            ? 'Ready to play'
            : metadata;

  return (
    <div className="player-track-info">
      <span className="player-track-icon" aria-hidden="true">
        <Music2 size={17} />
      </span>
      <div className="player-track-copy">
        <strong title={title}>{title}</strong>
        <span className="muted">{trackSubtitle}</span>
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
