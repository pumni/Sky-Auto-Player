import { LoaderCircle, Pause, Play, Shuffle, SkipBack, SkipForward, Square } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import type { DesktopStoreHook } from '../../state/store';
import {
  nextPlaybackPosition,
  playbackIssuePresentation,
  previousPlaybackPosition,
  selectSongById,
} from '../../state/store';
import { Button } from '../ui/button';
import { formatPlayerDuration } from './playerFormatting';

interface PlayerTransportProps {
  useStore: DesktopStoreHook;
}

export function PlayerTransport({ useStore }: PlayerTransportProps) {
  const playback = useStore(
    useShallow((store) => ({
      shuffleEnabled: store.playback.shuffleEnabled,
      state: store.playback.state,
      sessionId: store.playback.sessionId,
      hasCurrentSong: store.playback.currentSong !== null,
      context: store.playback.context,
      startRequestId: store.playback.startRequestId,
      transportOperation: store.playback.transportOperation,
      prepared: store.playback.prepared,
      error: store.playback.error,
      statusRetryPending: store.playback.statusRetryPending,
    })),
  );
  const selectedSongId = useStore((store) => store.library.selectedSongId);
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
    playback.context?.valid && playback.context.generation === libraryGeneration;
  const canNext = contextCurrent && nextPlaybackPosition(playback.context!) !== null;
  const transportOwnsControls = operationPending || confirmationPending || recoveryPending;
  const primaryPending =
    operationPending ||
    playback.startRequestId !== null ||
    recoveryPending ||
    (hasSession && ['starting', 'stopping', 'finished', 'failed'].includes(playback.state));
  const primaryCanStartPrepared = playback.prepared?.admission === 'ready';
  const primaryIsPlaying = active && playback.state === 'playing';
  const primaryIsPaused = active && playback.state === 'paused';
  const primaryCanPlay = Boolean(selectedSongId || (playback.hasCurrentSong && contextCurrent));
  const stopCanAct =
    hasSession && ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const canPrevious = useStore((store) => {
    const context = store.playback.context;
    if (!context?.valid || context.generation !== store.library.generation) {
      return false;
    }
    const snapshot = store.playback.snapshot;
    const snapshotCurrent = snapshot?.session_id === store.playback.sessionId;
    const selectedSongId = store.library.selectedSongId;
    const selectedDurationUs =
      store.playback.currentSong?.durationUs ??
      selectSongById(store.library, selectedSongId)?.duration_us ??
      snapshot?.total_us ??
      0;
    const totalUs = snapshot?.total_us ?? selectedDurationUs;
    const currentUs = Math.min(
      Math.max(0, snapshotCurrent ? (snapshot?.current_us ?? 0) : 0),
      totalUs || 0,
    );
    return previousPlaybackPosition(context, currentUs) !== null;
  });

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
    <div
      data-slot="player-transport"
      className="relative col-start-2 row-start-1 grid min-h-[70px] min-w-0 grid-rows-[44px_20px] content-center gap-1.5 [--player-primary-half:21px] [--player-control-gap:8px] [--player-secondary-target:34px] [--player-stop-gap:4px]"
    >
      <div className="relative h-11 min-w-0" data-testid="player-controls-row">
        <div className="absolute inset-0" data-testid="player-core-controls">
          <Button
            variant="ghost"
            size="icon-sm"
            className="absolute left-[calc(50%-var(--player-primary-half)-var(--player-control-gap)-var(--player-secondary-target)-var(--player-control-gap)-var(--player-secondary-target))] top-1/2 size-[34px] -translate-y-1/2 rounded-full p-0 text-text-secondary hover:bg-control-hover hover:text-foreground disabled:cursor-default disabled:text-control-disabled disabled:opacity-100 disabled:hover:!bg-transparent aria-pressed:text-accent-ink"
            type="button"
            aria-label="Shuffle"
            aria-pressed={playback.shuffleEnabled}
            title={playback.shuffleEnabled ? 'Shuffle on' : 'Shuffle off'}
            disabled={transportOwnsControls}
            onClick={() => setShuffleEnabled(!playback.shuffleEnabled)}
          >
            <Shuffle size={16} aria-hidden="true" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            className="absolute left-[calc(50%-var(--player-primary-half)-var(--player-control-gap)-var(--player-secondary-target))] top-1/2 size-[34px] -translate-y-1/2 rounded-full p-0 text-text-secondary hover:bg-control-hover hover:text-foreground disabled:cursor-default disabled:text-control-disabled disabled:opacity-100 disabled:hover:!bg-transparent"
            type="button"
            aria-label="Previous"
            title="Previous"
            disabled={!canPrevious || transportOwnsControls}
            onClick={() => void previous()}
          >
            <SkipBack size={16} aria-hidden="true" />
          </Button>
          <div
            className="absolute left-1/2 top-1/2 grid size-[42px] -translate-x-1/2 -translate-y-1/2 place-items-center"
            data-testid="player-primary-slot"
          >
            <Button
              data-slot="primary-transport"
              data-transport-state={
                primaryPending
                  ? 'pending'
                  : primaryIsPlaying
                    ? 'playing'
                    : primaryIsPaused
                      ? 'paused'
                      : 'idle'
              }
              variant="default"
              size="transport"
              className="disabled:opacity-100"
              type="button"
              aria-label={
                primaryPending
                  ? (transportStatus ?? 'Playback transition pending')
                  : primaryIsPlaying
                    ? 'Pause'
                    : primaryIsPaused
                      ? 'Resume'
                      : primaryCanStartPrepared && playback.prepared?.config.dry_run
                        ? 'Start test playback (no input)'
                        : 'Play'
              }
              aria-busy={primaryPending}
              title={
                primaryPending
                  ? (transportStatus ?? 'Playback transition pending')
                  : primaryIsPlaying
                    ? 'Pause'
                    : primaryIsPaused
                      ? 'Resume'
                      : confirmationPending
                        ? 'Playback confirmation required'
                        : playback.prepared?.admission === 'blocked'
                          ? 'Playback is blocked'
                          : primaryCanStartPrepared && playback.prepared?.config.dry_run
                            ? 'Start test playback: no keyboard input'
                            : 'Play: may send physical keyboard input'
              }
              disabled={
                primaryPending ||
                (!(primaryIsPlaying || primaryIsPaused) &&
                  ((!primaryCanStartPrepared &&
                    (confirmationPending || playback.prepared?.admission === 'blocked')) ||
                    (!primaryCanStartPrepared && !primaryCanPlay)))
              }
              onClick={() => {
                if (primaryIsPlaying) void pause();
                else if (primaryIsPaused) void resume();
                else if (primaryCanStartPrepared) void start();
                else void prepareAndMaybeStart(false);
              }}
            >
              <span className="grid size-[18px] place-items-center" aria-hidden="true">
                <Play
                  className={`col-start-1 row-start-1 size-4 transition-opacity duration-[100ms] ease-standard motion-reduce:transition-none ${!primaryPending && !primaryIsPlaying ? 'opacity-100' : 'opacity-0'}`}
                  size={16}
                  fill="currentColor"
                  strokeWidth={1.5}
                />
                <Pause
                  className={`col-start-1 row-start-1 size-4 transition-opacity duration-[100ms] ease-standard motion-reduce:transition-none ${!primaryPending && primaryIsPlaying ? 'opacity-100' : 'opacity-0'}`}
                  size={16}
                  fill="currentColor"
                  strokeWidth={1.5}
                />
                <LoaderCircle
                  className={`col-start-1 row-start-1 size-4 transition-opacity duration-[100ms] ease-standard motion-reduce:transition-none ${primaryPending ? 'animate-spin opacity-100 motion-reduce:animate-none' : 'opacity-0'}`}
                  size={16}
                />
              </span>
            </Button>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            className="absolute left-[calc(50%+var(--player-primary-half)+var(--player-control-gap))] top-1/2 size-[34px] -translate-y-1/2 rounded-full p-0 text-text-secondary hover:bg-control-hover hover:text-foreground disabled:cursor-default disabled:text-control-disabled disabled:opacity-100 disabled:hover:!bg-transparent"
            type="button"
            aria-label="Next"
            title="Next"
            disabled={!canNext || transportOwnsControls}
            onClick={() => void next()}
          >
            <SkipForward size={16} aria-hidden="true" />
          </Button>
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          className="absolute left-[calc(50%+var(--player-primary-half)+var(--player-control-gap)+var(--player-secondary-target)+var(--player-stop-gap))] top-1/2 size-[34px] -translate-y-1/2 rounded-full p-0 text-text-secondary hover:bg-control-hover hover:text-foreground disabled:cursor-default disabled:text-control-disabled disabled:opacity-100 disabled:hover:!bg-transparent"
          type="button"
          aria-label="Stop"
          title="Stop"
          disabled={!stopCanAct || playback.transportOperation === 'stopping'}
          onClick={() => void stop()}
        >
          <Square size={13} fill="currentColor" strokeWidth={1.5} aria-hidden="true" />
        </Button>
      </div>
      <PlayerTransportTimeline useStore={useStore} transportStatus={transportStatus} />
    </div>
  );
}

function PlayerTransportTimeline({
  useStore,
  transportStatus,
}: PlayerTransportProps & { transportStatus: string | null }) {
  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const selectedRow = useStore((store) => selectSongById(store.library, selectedSongId));
  const hasCurrentSong = useStore((store) => store.playback.currentSong !== null);
  const currentSongDurationUs = useStore((store) => store.playback.currentSong?.durationUs ?? null);
  const sessionId = useStore((store) => store.playback.sessionId);
  const snapshotSessionId = useStore((store) => store.playback.snapshot?.session_id ?? null);
  const snapshotCurrentUs = useStore((store) => store.playback.snapshot?.current_us ?? 0);
  const snapshotTotalUs = useStore((store) => store.playback.snapshot?.total_us ?? null);

  const timelineSongExists = Boolean(hasCurrentSong || selectedSongId);
  const selectedDurationUs =
    currentSongDurationUs ?? selectedRow?.duration_us ?? snapshotTotalUs ?? 0;
  const totalUs = snapshotTotalUs ?? selectedDurationUs;
  const snapshotCurrent = snapshotSessionId === sessionId;
  const currentUs = Math.min(Math.max(0, snapshotCurrent ? snapshotCurrentUs : 0), totalUs || 0);
  const progressLabel = !timelineSongExists
    ? 'Playback progress unavailable until a song is selected'
    : totalUs
      ? `Playback progress, ${formatPlayerDuration(currentUs)} of ${formatPlayerDuration(totalUs)}`
      : 'Playback progress unavailable';

  return (
    <div
      className={`grid min-h-6 min-w-0 grid-rows-[16px_6px] gap-[3px] ${timelineSongExists ? '' : 'text-text-tertiary'}`}
      aria-label={progressLabel}
      data-slot="player-timeline"
    >
      <div className="relative grid h-4 min-w-0 items-center">
        <div
          className="col-start-1 row-start-1 flex h-4 justify-between text-[11px] leading-4 tabular-nums text-text-tertiary aria-hidden:invisible"
          aria-hidden={!timelineSongExists || transportStatus !== null}
          data-slot="player-timeline-labels"
        >
          <span>{timelineSongExists ? formatPlayerDuration(currentUs) : ''}</span>
          <span>{timelineSongExists ? formatPlayerDuration(totalUs) : ''}</span>
        </div>
        {transportStatus && (
          <span
            className="col-start-1 row-start-1 overflow-hidden text-center text-xs leading-4 text-text-tertiary"
            role="status"
          >
            {transportStatus}
          </span>
        )}
      </div>
      <progress
        className="block h-1 w-full self-center appearance-none overflow-hidden rounded-full border-0 bg-control-hover accent-primary aria-disabled:bg-control-hover"
        value={timelineSongExists ? currentUs : 0}
        max={timelineSongExists ? totalUs || 1 : 1}
        aria-label={progressLabel}
        aria-disabled={!timelineSongExists}
      />
    </div>
  );
}
