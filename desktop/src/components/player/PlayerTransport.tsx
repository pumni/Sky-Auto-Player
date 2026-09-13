import { LoaderCircle, Pause, Play, Shuffle, SkipBack, SkipForward, Square } from 'lucide-react';
import type { DesktopStoreHook } from '../../state/store';
import { nextPlaybackPosition, playbackIssuePresentation, selectSongById } from '../../state/store';
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
  const setShuffleEnabled = useStore((store) => store.setShuffleEnabled);

  const operationPending = playback.transportOperation !== null;
  const confirmationPending = playback.prepared?.admission === 'confirmation_required';
  const hasSession = playback.sessionId !== null;
  const issue = playbackIssuePresentation(playback);
  const recoveryPending =
    playback.statusRetryPending ||
    issue?.kind === 'recoverable_status' ||
    issue?.kind === 'conflict';
  const active =
    hasSession &&
    ['starting', 'playing', 'paused', 'stopping', 'finished', 'failed'].includes(playback.state);
  const contextCurrent =
    playback.context !== null &&
    playback.context.valid &&
    playback.context.generation === libraryGeneration;
  const canPrevious = contextCurrent;
  const canNext = contextCurrent && nextPlaybackPosition(playback.context!) !== null;
  const transportOwnsControls = operationPending || confirmationPending || recoveryPending;
  const primaryPending =
    operationPending ||
    playback.startRequestId !== null ||
    recoveryPending ||
    (hasSession && ['starting', 'stopping', 'finished', 'failed'].includes(playback.state));
  const primaryCanStartPrepared = playback.prepared?.admission === 'ready';
  const primaryCanPlay = Boolean(selectedSongId || (playback.currentSong && contextCurrent));
  const stopCanAct =
    hasSession && ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
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

  return (
    <div className="player-transport">
      <div className="player-controls-row" data-testid="player-controls-row">
        <div className="player-core-controls" data-testid="player-core-controls">
          <button
            className={`icon-button player-secondary-action player-shuffle-action${playback.shuffleEnabled ? ' is-active' : ''}`}
            type="button"
            aria-label="Shuffle"
            aria-pressed={playback.shuffleEnabled}
            title={playback.shuffleEnabled ? 'Shuffle on' : 'Shuffle off'}
            disabled={transportOwnsControls}
            onClick={() => setShuffleEnabled(!playback.shuffleEnabled)}
          >
            <Shuffle size={16} aria-hidden="true" />
          </button>
          <button
            className="icon-button player-secondary-action player-previous-action"
            type="button"
            aria-label="Previous"
            title="Previous"
            disabled={!canPrevious || transportOwnsControls}
            onClick={() => void previous()}
          >
            <SkipBack size={16} aria-hidden="true" />
          </button>
          <div className="player-primary-slot" data-testid="player-primary-slot">
            {primaryPending ? (
              <button
                className="icon-button button-primary player-primary-action is-pending"
                type="button"
                aria-label={transportStatus ?? 'Playback transition pending'}
                title={transportStatus ?? 'Playback transition pending'}
                disabled
              >
                <LoaderCircle size={18} aria-hidden="true" />
              </button>
            ) : active && playback.state === 'playing' ? (
              <button
                className="icon-button button-primary player-primary-action"
                type="button"
                aria-label="Pause"
                title="Pause"
                onClick={() => void pause()}
              >
                <Pause size={18} aria-hidden="true" />
              </button>
            ) : active && playback.state === 'paused' ? (
              <button
                className="icon-button button-primary player-primary-action"
                type="button"
                aria-label="Resume"
                title="Resume"
                onClick={() => void resume()}
              >
                <Play size={18} aria-hidden="true" />
              </button>
            ) : (
              <button
                className="icon-button button-primary player-primary-action"
                type="button"
                aria-label="Play"
                title={
                  confirmationPending
                    ? 'Playback confirmation required'
                    : playback.prepared?.admission === 'blocked'
                      ? 'Playback is blocked'
                      : 'Play'
                }
                disabled={
                  transportOwnsControls ||
                  (!primaryCanStartPrepared &&
                    (confirmationPending || playback.prepared?.admission === 'blocked')) ||
                  (!primaryCanStartPrepared && !primaryCanPlay)
                }
                onClick={() =>
                  primaryCanStartPrepared ? void start() : void prepareAndMaybeStart(false)
                }
              >
                <Play size={18} aria-hidden="true" />
              </button>
            )}
          </div>
          <button
            className="icon-button player-secondary-action player-next-action"
            type="button"
            aria-label="Next"
            title="Next"
            disabled={!canNext || transportOwnsControls}
            onClick={() => void next()}
          >
            <SkipForward size={16} aria-hidden="true" />
          </button>
        </div>
        <button
          className="icon-button player-secondary-action player-stop-action"
          type="button"
          aria-label="Stop"
          title="Stop"
          disabled={!stopCanAct || playback.transportOperation === 'stopping'}
          onClick={() => void stop()}
        >
          <Square size={12} aria-hidden="true" />
        </button>
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
