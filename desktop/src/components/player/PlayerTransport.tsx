import { CircleStop, Pause, Play, SkipForward } from 'lucide-react';
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
  const prepare = useStore((store) => store.prepareSelectedPlayback);
  const start = useStore((store) => store.startPreparedPlayback);
  const stop = useStore((store) => store.stopPlayback);
  const pause = useStore((store) => store.pausePlayback);
  const resume = useStore((store) => store.resumePlayback);
  const skip = useStore((store) => store.skipPlayback);

  const active = ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const snapshot = playback.snapshot;
  const selectedDurationUs =
    playback.prepared?.song.duration_us ?? selectedRow?.duration_us ?? snapshot?.total_us ?? 0;
  const totalUs = snapshot?.total_us ?? selectedDurationUs;
  const currentUs = Math.min(Math.max(0, snapshot?.current_us ?? 0), totalUs || 0);
  const progressLabel = !selectedSongId
    ? 'Playback progress unavailable until a sheet is selected'
    : totalUs
      ? `Playback progress, ${formatPlayerDuration(currentUs)} of ${formatPlayerDuration(totalUs)}`
      : 'Playback progress unavailable';

  const prepareAndMaybeStart = async (dryRun: boolean) => {
    await prepare({ dry_run: dryRun });
    const current = useStore.getState();
    const prepared = current.playback.prepared;
    if (prepared?.song.song_id !== current.library.selectedSongId) return;
    if (prepared?.admission === 'ready') await start();
  };

  return (
    <div className="player-transport">
      <div className="transport-actions">
        <div className="transport-slot transport-stop-slot">
          {active && playback.state !== 'stopping' && (
            <button
              className="icon-button player-secondary-action"
              type="button"
              aria-label="Stop"
              title="Stop"
              onClick={() => void stop()}
            >
              <CircleStop size={16} aria-hidden="true" />
            </button>
          )}
        </div>
        <div className="transport-slot transport-primary-slot">
          {!active && !playback.prepared && (
            <button
              className="icon-button button-primary player-primary-action"
              type="button"
              aria-label="Play"
              title="Play"
              disabled={!selectedSongId}
              onClick={() => void prepareAndMaybeStart(false)}
            >
              <Play size={18} aria-hidden="true" />
            </button>
          )}
          {active &&
            playback.state !== 'stopping' &&
            (playback.state === 'paused' ? (
              <button
                className="icon-button button-primary player-primary-action"
                type="button"
                aria-label="Resume"
                title="Resume"
                disabled={playback.pendingCommand !== null}
                onClick={() => void resume()}
              >
                <Play size={18} aria-hidden="true" />
              </button>
            ) : (
              <button
                className="icon-button button-primary player-primary-action"
                type="button"
                aria-label="Pause"
                title="Pause"
                disabled={playback.pendingCommand !== null}
                onClick={() => void pause()}
              >
                <Pause size={18} aria-hidden="true" />
              </button>
            ))}
        </div>
        <div className="transport-slot transport-skip-slot">
          {active && playback.state !== 'stopping' && (
            <button
              className="icon-button player-secondary-action"
              type="button"
              aria-label="Skip"
              title="Skip"
              onClick={() => void skip()}
            >
              <SkipForward size={16} aria-hidden="true" />
            </button>
          )}
        </div>
      </div>
      <div
        className={`player-timeline${selectedSongId ? '' : ' is-disabled'}`}
        aria-label={progressLabel}
      >
        {selectedSongId && (
          <div className="player-timeline-labels">
            <span>{formatPlayerDuration(currentUs)}</span>
            <span>{formatPlayerDuration(totalUs)}</span>
          </div>
        )}
        <progress
          value={selectedSongId ? currentUs : 0}
          max={selectedSongId ? totalUs || 1 : 1}
          aria-label={progressLabel}
          aria-disabled={!selectedSongId}
        />
      </div>
      {playback.prepared && !active && (
        <span className="player-transport-status" role="status">
          {playback.prepared.admission === 'confirmation_required'
            ? 'Awaiting confirmation'
            : 'Preparing playback'}
        </span>
      )}
      {playback.prepared?.admission === 'blocked' && (
        <span className="player-message player-message-danger" role="alert">
          {playback.prepared.error_message}
        </span>
      )}
      {playback.error && !playback.prepared && (
        <span className="player-message player-message-danger" role="alert">
          {playback.error}
        </span>
      )}
    </div>
  );
}
