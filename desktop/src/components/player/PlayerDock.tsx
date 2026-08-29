import { Activity, Pause, Play, SkipForward, Square } from 'lucide-react';
import { useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface PlayerDockProps {
  useStore: DesktopStoreHook;
}

function progress(current: number, total: number): string {
  if (!total) return '0:00 / 0:00';
  const format = (value: number) => {
    const seconds = Math.max(0, Math.round(value / 1_000_000));
    return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
  };
  return `${format(current)} / ${format(total)}`;
}

export function PlayerDock({ useStore }: PlayerDockProps) {
  const playback = useStore((store: DesktopStore) => store.playback);
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);
  const selectedTitle = useStore(
    (store: DesktopStore) =>
      store.playback.songTitle ??
      store.playback.prepared?.song.title ??
      store.detail.value?.title ??
      'No song selected',
  );
  const settings = useStore((store: DesktopStore) => store.settings);
  const prepare = useStore((store: DesktopStore) => store.prepareSelectedPlayback);
  const start = useStore((store: DesktopStore) => store.startPreparedPlayback);
  const stop = useStore((store: DesktopStore) => store.stopPlayback);
  const pause = useStore((store: DesktopStore) => store.pausePlayback);
  const resume = useStore((store: DesktopStore) => store.resumePlayback);
  const skip = useStore((store: DesktopStore) => store.skipPlayback);
  const patchSettings = useStore((store: DesktopStore) => store.patchSettings);
  const diagnosticsOpen = useStore((store: DesktopStore) => store.diagnostics.open);
  const setDiagnosticsOpen = useStore((store: DesktopStore) => store.setDiagnosticsOpen);
  const [dryRun, setDryRun] = useState(true);

  const prepareAndMaybeStart = async () => {
    await prepare({ dry_run: dryRun });
    const current = useStore.getState();
    const prepared = current.playback.prepared;
    if (prepared?.song.song_id !== current.library.selectedSongId) return;
    if (prepared?.admission === 'ready') await start();
  };

  const active = ['starting', 'playing', 'paused', 'stopping'].includes(playback.state);
  const snapshot = playback.snapshot;
  const defaults = settings?.playback_defaults;

  return (
    <footer className="player-dock" aria-label="Player controls">
      <div className={`status-led status-${playback.state}`} aria-hidden="true" />
      <div className="dock-song">
        <strong>{selectedTitle}</strong>
        <span className="muted">{playback.state.replace('_', ' ')}</span>
      </div>
      <div className="dock-controls">
        {!active && !playback.prepared && (
          <button
            className="button button-primary"
            type="button"
            disabled={!selectedSongId}
            onClick={() => void prepareAndMaybeStart()}
          >
            <Play size={14} aria-hidden="true" /> Play
          </button>
        )}
        {active && playback.state !== 'stopping' && (
          <>
            {playback.state === 'paused' ? (
              <button
                className="button"
                type="button"
                disabled={playback.pendingCommand !== null}
                onClick={() => void resume()}
              >
                <Play size={14} aria-hidden="true" /> Resume
              </button>
            ) : (
              <button
                className="button"
                type="button"
                disabled={playback.pendingCommand !== null}
                onClick={() => void pause()}
              >
                <Pause size={14} aria-hidden="true" /> Pause
              </button>
            )}
            <button className="button" type="button" onClick={() => void skip()}>
              <SkipForward size={14} aria-hidden="true" /> Skip
            </button>
            <button className="button" type="button" onClick={() => void stop()}>
              <Square size={14} aria-hidden="true" /> Stop
            </button>
          </>
        )}
      </div>
      {defaults && !active && (
        <div className="dock-settings" aria-label="Playback settings">
          <label>
            Hold{' '}
            <select
              value={defaults.hold_frames}
              onChange={(event) =>
                void patchSettings({ playbackDefaults: { holdFrames: Number(event.target.value) } })
              }
            >
              {useStore.getState().bootstrap?.option_sets.hold_frames.map((value) => (
                <option key={value} value={value}>
                  {value}f
                </option>
              ))}
            </select>
          </label>
          <label>
            Tempo{' '}
            <select
              value={defaults.tempo_scale}
              onChange={(event) =>
                void patchSettings({ playbackDefaults: { tempoScale: Number(event.target.value) } })
              }
            >
              {useStore.getState().bootstrap?.option_sets.tempo_scales.map((value) => (
                <option key={value} value={value}>
                  {value}×
                </option>
              ))}
            </select>
          </label>
          <label>
            FPS{' '}
            <select
              value={defaults.fps}
              onChange={(event) =>
                void patchSettings({ playbackDefaults: { fps: Number(event.target.value) } })
              }
            >
              {useStore.getState().bootstrap?.option_sets.fps.map((value) => (
                <option key={value} value={value}>
                  {value}
                </option>
              ))}
            </select>
          </label>
          <label className="dock-checkbox">
            <input
              type="checkbox"
              checked={dryRun}
              onChange={(event) => setDryRun(event.target.checked)}
            />{' '}
            Dry-run
          </label>
        </div>
      )}
      {snapshot && active && (
        <span className="dock-progress">{progress(snapshot.current_us, snapshot.total_us)}</span>
      )}
      <button
        className="icon-button dock-diagnostics-button"
        type="button"
        aria-label={diagnosticsOpen ? 'Close diagnostics' : 'Open diagnostics'}
        aria-pressed={diagnosticsOpen}
        onClick={() => setDiagnosticsOpen(!diagnosticsOpen)}
      >
        <Activity size={15} aria-hidden="true" />
      </button>
      {playback.prepared?.admission === 'confirmation_required' && (
        <div className="risk-confirmation" role="dialog" aria-label="Playback confirmation">
          <span>{playback.prepared.risk.headline}</span>
          {playback.prepared.decisions.map((decision) => (
            <button
              key={decision.decision}
              className="button"
              type="button"
              onClick={() => void start(decision.decision)}
            >
              {decision.label}
            </button>
          ))}
        </div>
      )}
      {playback.prepared?.admission === 'blocked' && (
        <span className="dock-error" role="alert">
          {playback.prepared.error_message}
        </span>
      )}
      {playback.error && !playback.prepared && (
        <span className="dock-error" role="alert">
          {playback.error}
        </span>
      )}
    </footer>
  );
}
