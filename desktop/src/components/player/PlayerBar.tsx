import {
  Activity,
  CircleStop,
  Music2,
  Pause,
  Play,
  SkipForward,
  SlidersHorizontal,
} from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface PlayerBarProps {
  useStore: DesktopStoreHook;
}

function formatDuration(value: number): string {
  const seconds = Math.max(0, Math.round(value / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

function stateLabel(state: string): string {
  return state.replaceAll('_', ' ');
}

export function PlayerBar({ useStore }: PlayerBarProps) {
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
  const bootstrap = useStore((store: DesktopStore) => store.bootstrap);
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
  const [profileOpen, setProfileOpen] = useState(false);
  const profileTriggerRef = useRef<HTMLButtonElement>(null);
  const profileDialogRef = useRef<HTMLDivElement>(null);
  const profileWasOpen = useRef(false);

  useEffect(() => {
    if (profileOpen) {
      profileWasOpen.current = true;
      profileDialogRef.current?.focus();
    } else if (profileWasOpen.current) {
      profileWasOpen.current = false;
      profileTriggerRef.current?.focus();
    }
  }, [profileOpen]);

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
  const profileSummary = defaults
    ? `${defaults.hold_frames}f · ${defaults.tempo_scale.toFixed(2)}× · ${defaults.fps} FPS${dryRun ? ' · Dry-run' : ''}`
    : 'Unavailable';
  const progressLabel = snapshot
    ? `Playback progress, ${formatDuration(snapshot.current_us)} of ${formatDuration(snapshot.total_us)}`
    : 'Playback progress unavailable';

  return (
    <footer className="player-bar" aria-label="Player controls">
      {playback.prepared?.admission === 'confirmation_required' && (
        <div className="player-admission" role="dialog" aria-label="Playback confirmation">
          <div className="admission-copy">
            <strong>Playback confirmation</strong>
            <span>{playback.prepared.risk.headline}</span>
            {playback.prepared.risk.reasons.length > 0 && (
              <span className="muted">{playback.prepared.risk.reasons[0]}</span>
            )}
          </div>
          <div className="admission-actions">
            {playback.prepared.decisions.map((decision) => (
              <button
                key={decision.decision}
                className={`button${decision.decision === 'proceed' ? ' button-primary' : ''}`}
                type="button"
                onClick={() => void start(decision.decision)}
              >
                {decision.label}
              </button>
            ))}
          </div>
        </div>
      )}

      <div className="player-track-info">
        <span className="player-track-icon" aria-hidden="true">
          <Music2 size={17} />
        </span>
        <div className="player-track-copy">
          <strong title={selectedTitle}>{selectedTitle}</strong>
          <span className="muted">
            {selectedSongId ? stateLabel(playback.state) : 'Choose a sheet from Library'}
          </span>
        </div>
        {playback.prepared?.risk.level &&
          ['medium', 'high'].includes(playback.prepared.risk.level) && (
            <span
              className={`player-risk risk-${playback.prepared.risk.level}`}
              aria-label={playback.prepared.risk.headline}
            >
              {playback.prepared.risk.level}
            </span>
          )}
      </div>

      <div className="player-transport">
        <div className="transport-actions">
          {!active && !playback.prepared && (
            <button
              className="button button-primary player-primary-action"
              type="button"
              disabled={!selectedSongId}
              onClick={() => void prepareAndMaybeStart()}
            >
              <Play size={15} aria-hidden="true" /> Play
            </button>
          )}
          {active && playback.state !== 'stopping' && (
            <>
              {playback.state === 'paused' ? (
                <button
                  className="button button-primary player-primary-action"
                  type="button"
                  disabled={playback.pendingCommand !== null}
                  onClick={() => void resume()}
                >
                  <Play size={15} aria-hidden="true" /> Resume
                </button>
              ) : (
                <button
                  className="button button-primary player-primary-action"
                  type="button"
                  disabled={playback.pendingCommand !== null}
                  onClick={() => void pause()}
                >
                  <Pause size={15} aria-hidden="true" /> Pause
                </button>
              )}
              <button className="button" type="button" onClick={() => void skip()}>
                <SkipForward size={15} aria-hidden="true" /> Skip
              </button>
              <button className="button" type="button" onClick={() => void stop()}>
                <CircleStop size={15} aria-hidden="true" /> Stop
              </button>
            </>
          )}
        </div>
        {snapshot && active && (
          <div className="player-timeline" aria-label={progressLabel}>
            <div className="player-timeline-labels">
              <span>{formatDuration(snapshot.current_us)}</span>
              <span>{formatDuration(snapshot.total_us)}</span>
            </div>
            <progress
              value={snapshot.current_us}
              max={snapshot.total_us || undefined}
              aria-label={progressLabel}
            />
          </div>
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

      <div
        className="player-profile"
        onKeyDown={(event) => {
          if (event.key === 'Escape') {
            event.preventDefault();
            setProfileOpen(false);
          }
        }}
      >
        <button
          ref={profileTriggerRef}
          className="profile-summary-button"
          type="button"
          aria-label="Configure timing profile"
          aria-expanded={profileOpen}
          aria-controls="playback-profile-popover"
          onClick={() => setProfileOpen((open) => !open)}
        >
          <SlidersHorizontal size={15} aria-hidden="true" />
          <span>
            <strong>Playback profile</strong>
            <small>{profileSummary}</small>
          </span>
        </button>
        <div className="profile-fields" aria-label="Playback profile">
          <ProfileFields
            defaults={defaults}
            bootstrap={bootstrap}
            dryRun={dryRun}
            setDryRun={setDryRun}
            patchSettings={patchSettings}
          />
        </div>
        {profileOpen && (
          <div
            ref={profileDialogRef}
            id="playback-profile-popover"
            className="profile-popover"
            role="dialog"
            aria-label="Playback profile"
            tabIndex={-1}
          >
            <ProfileFields
              defaults={defaults}
              bootstrap={bootstrap}
              dryRun={dryRun}
              setDryRun={setDryRun}
              patchSettings={patchSettings}
            />
          </div>
        )}
      </div>

      <button
        className="icon-button player-diagnostics-button"
        type="button"
        aria-label={diagnosticsOpen ? 'Close diagnostics' : 'Open diagnostics'}
        aria-pressed={diagnosticsOpen}
        onClick={() => setDiagnosticsOpen(!diagnosticsOpen)}
      >
        <Activity size={16} aria-hidden="true" />
      </button>
    </footer>
  );
}

interface ProfileFieldsProps {
  defaults: NonNullable<DesktopStore['settings']>['playback_defaults'] | undefined;
  bootstrap: DesktopStore['bootstrap'];
  dryRun: boolean;
  setDryRun: (value: boolean) => void;
  patchSettings: DesktopStore['patchSettings'];
}

function ProfileFields({
  defaults,
  bootstrap,
  dryRun,
  setDryRun,
  patchSettings,
}: ProfileFieldsProps) {
  if (!defaults || !bootstrap) return <span className="muted">Profile unavailable</span>;
  return (
    <>
      <label>
        Hold
        <select
          value={defaults.hold_frames}
          onChange={(event) =>
            void patchSettings({ playbackDefaults: { holdFrames: Number(event.target.value) } })
          }
        >
          {bootstrap.option_sets.hold_frames.map((value) => (
            <option key={value} value={value}>
              {value}f
            </option>
          ))}
        </select>
      </label>
      <label>
        Tempo
        <select
          value={defaults.tempo_scale}
          onChange={(event) =>
            void patchSettings({ playbackDefaults: { tempoScale: Number(event.target.value) } })
          }
        >
          {bootstrap.option_sets.tempo_scales.map((value) => (
            <option key={value} value={value}>
              {value}×
            </option>
          ))}
        </select>
      </label>
      <label>
        FPS
        <select
          value={defaults.fps}
          onChange={(event) =>
            void patchSettings({ playbackDefaults: { fps: Number(event.target.value) } })
          }
        >
          {bootstrap.option_sets.fps.map((value) => (
            <option key={value} value={value}>
              {value}
            </option>
          ))}
        </select>
      </label>
      <label className="profile-checkbox">
        <input
          type="checkbox"
          checked={dryRun}
          onChange={(event) => setDryRun(event.target.checked)}
        />
        Dry-run
      </label>
    </>
  );
}
