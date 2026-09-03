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
  const playbackTitle = useStore((store) => store.playback.songTitle);
  const preparedSong = useStore((store) => store.playback.prepared?.song);
  const preparedRisk = useStore((store) => store.playback.prepared?.risk);
  const active = useStore((store) =>
    ['starting', 'playing', 'paused', 'stopping'].includes(store.playback.state),
  );
  const playbackState = useStore((store) => store.playback.state);
  const playbackError = useStore((store) => store.playback.error);
  const setSongLiked = useStore((store) => store.setSongLiked);

  const selectedTitle =
    playbackTitle ?? preparedSong?.title ?? selectedRow?.title ?? 'No song selected';
  const selectedMetadata = selectedRow
    ? `${selectedRow.format_label} · ${
        selectedRow.duration_us === null ? '…' : formatPlayerDuration(selectedRow.duration_us)
      } · ${selectedRow.note_count === null ? '…' : selectedRow.note_count} notes`
    : 'Preparing metadata…';
  const trackSubtitle = !selectedSongId
    ? 'Choose a sheet from Library'
    : playbackError
      ? 'Playback error'
      : active
        ? playerStateLabel(playbackState)
        : selectedMetadata;

  return (
    <div className="player-track-info">
      <span className="player-track-icon" aria-hidden="true">
        <Music2 size={17} />
      </span>
      <div className="player-track-copy">
        <strong title={selectedTitle}>{selectedTitle}</strong>
        <span className="muted">{trackSubtitle}</span>
      </div>
      {selectedSongId && selectedRow && (
        <button
          className={`icon-button player-like-button${selectedRow.liked ? ' is-liked' : ''}`}
          type="button"
          aria-label={selectedRow.liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
          aria-pressed={selectedRow.liked}
          title={selectedRow.liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
          onClick={() => void setSongLiked(selectedSongId, !selectedRow.liked)}
        >
          <Heart size={16} fill={selectedRow.liked ? 'currentColor' : 'none'} aria-hidden="true" />
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
