import { Heart, Music2 } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { playbackSessionStatusConcern, selectSongById } from '../../state/store';
import { StatusBadge } from '../ui/StatusBadge';
import { Button } from '../ui/button';
import {
  playerStateLabel,
  TEST_MODE_ACCESSIBLE_DESCRIPTION,
  TEST_MODE_LABEL,
  TEST_MODE_PREPARED_DESCRIPTION,
} from './playerFormatting';

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
      if (playback.preparedContext) {
        return playback.preparedContext.dryRun ? 'prepared-test' : 'prepared-live';
      }
    }
    if (playback.prepared) {
      return playback.prepared.config.dry_run ? 'prepared-test' : 'prepared-live';
    }
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
            : 'Selected');
  const testModeDescription =
    playbackMode === 'test'
      ? TEST_MODE_ACCESSIBLE_DESCRIPTION
      : playbackMode === 'prepared-test'
        ? TEST_MODE_PREPARED_DESCRIPTION
        : null;
  const statusAccessibilityLabel =
    `${title}: ${baseStatus}` +
    (testModeDescription
      ? `. ${testModeDescription}`
      : playbackMode === 'live'
        ? '. Normal playback may send physical input.'
        : playbackMode === 'prepared-live'
          ? '. Prepared for normal playback.'
          : '');

  return (
    <div
      className="col-start-1 row-start-1 flex min-w-0 items-center gap-2.5"
      role="group"
      aria-label={`Playback for ${title}`}
    >
      <span
        className="grid size-[30px] flex-none place-items-center rounded-control text-text-secondary max-[1100px]:hidden"
        aria-hidden="true"
      >
        <Music2 size={17} />
      </span>
      <div className="grid min-w-0 flex-1 gap-0.5">
        <div className="flex min-w-0 items-center gap-1.5 max-[700px]:gap-1">
          <strong
            className="min-w-0 flex-[0_1_auto] overflow-hidden text-ellipsis whitespace-nowrap text-sm font-semibold text-foreground"
            title={title}
          >
            {title}
          </strong>
          {likeSongId && (hasCurrentSong || fallbackIdentity) && (
            <Button
              variant="ghost"
              size="icon-sm"
              className={`size-[30px] max-[700px]:size-[28px] flex-none rounded-full p-0 text-text-tertiary hover:bg-control-hover hover:text-accent-ink focus-visible:text-accent-ink ${liked ? 'text-accent-ink' : ''}`}
              type="button"
              aria-label={liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
              aria-pressed={liked}
              title={liked ? 'Remove from Liked Songs' : 'Add to Liked Songs'}
              onClick={() => void setSongLiked(likeSongId, !liked)}
            >
              <Heart size={16} fill={liked ? 'currentColor' : 'none'} aria-hidden="true" />
            </Button>
          )}
        </div>
        <div className="flex min-w-0 items-center gap-1.5 max-[700px]:gap-1">
          <span
            data-slot="player-track-status"
            className={`block min-w-0 flex-[0_1_auto] overflow-hidden text-ellipsis whitespace-nowrap text-xs leading-4 ${sessionStatusConcern !== null ? 'text-warning' : active ? 'text-accent-ink' : 'text-muted-foreground'}`}
            role="status"
            aria-label={statusAccessibilityLabel}
            aria-live="polite"
            aria-atomic="true"
            title={baseStatus}
          >
            {baseStatus}
          </span>
          {testModeDescription && (
            <StatusBadge
              className="shrink-0"
              appearance="plain"
              indicator={false}
              aria-label={testModeDescription}
              title={testModeDescription}
            >
              {TEST_MODE_LABEL}
            </StatusBadge>
          )}
        </div>
        {preparedRisk && ['medium', 'high'].includes(preparedRisk.level) && (
          <StatusBadge
            className="w-fit shrink-0"
            tone={preparedRisk.level === 'high' ? 'danger' : 'warning'}
            aria-label={preparedRisk.headline}
            title={preparedRisk.headline}
          >
            {preparedRisk.level === 'high' ? 'High risk' : 'Medium risk'}
          </StatusBadge>
        )}
      </div>
    </div>
  );
}
