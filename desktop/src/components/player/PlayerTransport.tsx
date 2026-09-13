import { LoaderCircle, Pause, Play, SkipBack, SkipForward, Square } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { selectSongById } from '../../state/store';
import { formatPlayerDuration } from './playerFormatting';

interface PlayerTransportProps {
  useStore: DesktopStoreHook;
}

export function PlayerTransport({ useStore }: PlayerTransportProps) {
  const playback = useStore((store) => store.playback);
  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const selectedRow = useStore((store) => selectSongById(store.library, selectedSongId));
  const libraryGeneration = useStore((store) => store.library.generation);
  const prepare = useStore((store) => store.prepareSelectedPlayback);
  const start = useStore((store) => store.startPreparedPlayback);
  const stop = useStore((store) => store.stopPlayback);
  const pause = useStore((store) => store.pausePlayback);
  const resume = useStore((store) => store.resumePlayback);
  const previous = useStore((store) => store.previousPlayback);
  const next = useStore((store) => store.nextPlayback);

  const operationPending = playback.transportOperation !== null;
  const confirmationPending = playback.prepared?.admission === 'confirmation_required';
  const hasSession = playback.sessionId !== null;
  const active =
    hasSession &&
    ['starting', 'playing', 'paused', 'stopping', 'finished', 'failed'].includes(playback.state);
  const contextCurrent =
    playback.context !== null &&
    playback.context.valid &&
    playback.context.generation === libraryGeneration;
  const canPrevious = contextCurrent;
  const canNext = contextCurrent && playback.context!.currentIndex + 1 < playback.context!.total;
  const timelineSongExists = Boolean(playback.currentSong || selectedSongId);
  const selectedDurationUs =
    playback.currentSong?.durationUs ??
    selectedRow?.duration_us ??
    playback.snapshot?.total_us ??
    0;
  const totalUs = playback.snapshot?.total_us ?? selectedDurationUs;
  const snapshotCurrent = playback.snapshot?.session_id === playback.sessionId;
  const currentUs = Math.min(
    Math.max(0, snapshotCurrent ? (playback.snapshot?.current_us ?? 0) : 0),
    totalUs || 0,
  );
  const progressLabel = !timelineSongExists
    ? 'Playback progress unavailable until a song is selected'
    : totalUs
      ? `Playback progress, ${formatPlayerDuration(currentUs)} of ${formatPlayerDuration(totalUs)}`
      : 'Playback progress unavailable';

  const transportStatus = (() => {
    switch (playback.transportOperation) {
      case 'preparing':
        return 'Preparing playback';
      case 'starting':
        return 'Starting playback';
      case 'pausing':
        return 'Pausing playback';
      case 'resuming':
        return 'Resuming playback';
      case 'stopping':
        return 'Stopping playback';
      case 'advancing':
        return 'Starting next song';
      case 'restarting':
        return 'Restarting song';
      default:
        if (playback.prepared?.admission === 'confirmation_required')
          return 'Awaiting confirmation';
        return playback.startRequestId !== null && ['idle', 'starting'].includes(playback.state)
          ? 'Waiting for native playback session'
          : null;
    }
  })();

  const prepareAndMaybeStart = async (dryRun: boolean) => {
    await prepare({ dry_run: dryRun });
    const current = useStore.getState();
    if (current.playback.prepared?.admission === 'ready') await start();
  };

  const otherTransportOwnsControls = operationPending || confirmationPending;

  return (
    <div className="player-transport">
      <div className="transport-actions">
        <div className="transport-slot transport-previous-slot">
          <button
            className="icon-button player-secondary-action"
            type="button"
            aria-label="Previous"
            title="Previous"
            disabled={!canPrevious || otherTransportOwnsControls}
            onClick={() => void previous()}
          >
            <SkipBack size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="transport-slot transport-stop-balance-slot" aria-hidden="true" />
        <div className="transport-slot transport-primary-slot">
          {!active && !playback.prepared && playback.startRequestId === null && (
            <button
              className="icon-button button-primary player-primary-action"
              type="button"
              aria-label="Play"
              title="Play"
              disabled={
                playback.startRequestId !== null ||
                (!selectedSongId && (!playback.currentSong || !contextCurrent)) ||
                operationPending
              }
              onClick={() => void prepareAndMaybeStart(false)}
            >
              <Play size={18} aria-hidden="true" />
            </button>
          )}
          {!active && playback.startRequestId !== null && (
            <button
              className="icon-button button-primary player-primary-action is-pending"
              type="button"
              aria-label="Starting playback"
              title="Starting playback"
              disabled
            >
              <LoaderCircle size={18} aria-hidden="true" />
            </button>
          )}
          {active && playback.state === 'playing' && (
            <button
              className="icon-button button-primary player-primary-action"
              type="button"
              aria-label="Pause"
              title="Pause"
              disabled={operationPending}
              onClick={() => void pause()}
            >
              <Pause size={18} aria-hidden="true" />
            </button>
          )}
          {active && playback.state === 'paused' && (
            <button
              className="icon-button button-primary player-primary-action"
              type="button"
              aria-label="Resume"
              title="Resume"
              disabled={operationPending}
              onClick={() => void resume()}
            >
              <Play size={18} aria-hidden="true" />
            </button>
          )}
          {active && playback.state !== 'playing' && playback.state !== 'paused' && (
            <button
              className="icon-button button-primary player-primary-action is-pending"
              type="button"
              aria-label={
                playback.state === 'starting' ? 'Starting playback' : 'Playback transition pending'
              }
              title={
                playback.state === 'starting' ? 'Starting playback' : 'Playback transition pending'
              }
              disabled
            >
              <LoaderCircle size={18} aria-hidden="true" />
            </button>
          )}
        </div>
        <div className="transport-slot transport-next-slot">
          <button
            className="icon-button player-secondary-action"
            type="button"
            aria-label="Next"
            title="Next"
            disabled={!canNext || otherTransportOwnsControls}
            onClick={() => void next()}
          >
            <SkipForward size={16} aria-hidden="true" />
          </button>
        </div>
        <div className="transport-slot transport-stop-slot">
          {hasSession &&
            ['starting', 'playing', 'paused', 'stopping', 'finished', 'failed'].includes(
              playback.state,
            ) && (
              <button
                className="icon-button player-secondary-action"
                type="button"
                aria-label="Stop"
                title="Stop"
                disabled={playback.transportOperation === 'stopping'}
                onClick={() => void stop()}
              >
                <Square size={12} aria-hidden="true" />
              </button>
            )}
        </div>
      </div>
      <div
        className={`player-timeline${timelineSongExists ? '' : ' is-disabled'}`}
        aria-label={progressLabel}
      >
        <div className="player-timeline-label-row">
          <div
            className={`player-timeline-labels${transportStatus ? ' is-hidden' : ''}`}
            aria-hidden={!timelineSongExists || transportStatus !== null}
          >
            <span>{timelineSongExists ? formatPlayerDuration(currentUs) : ''}</span>
            <span>{timelineSongExists ? formatPlayerDuration(totalUs) : ''}</span>
          </div>
          {transportStatus && (
            <span className="player-transport-status" role="status">
              {transportStatus}
            </span>
          )}
        </div>
        <progress
          value={timelineSongExists ? currentUs : 0}
          max={timelineSongExists ? totalUs || 1 : 1}
          aria-label={progressLabel}
          aria-disabled={!timelineSongExists}
        />
      </div>
    </div>
  );
}
