import { Heart, LocateFixed, Music2 } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import {
  playbackSessionStatusConcern,
  selectNowPlayingIndex,
  selectSongById,
} from '../../state/store';
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
  const dryRun = useStore(
    (store) =>
      (store.playback.context?.dryRun === true ||
        store.playback.preparedContext?.dryRun === true) &&
      (active || sessionStatusConcern !== null),
  );
  const setSongLiked = useStore((store) => store.setSongLiked);
  const nowPlayingIndex = useStore((store) => selectNowPlayingIndex(store));
  const showNowPlaying = useStore((store) => store.showNowPlaying);

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
      {nowPlayingIndex !== null && (
        <button
          className="icon-button player-show-now-playing"
          type="button"
          aria-label={`Show now playing, row ${nowPlayingIndex + 1}`}
          title="Show now playing"
          onClick={() => void showNowPlaying()}
        >
          <LocateFixed size={16} aria-hidden="true" />
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
