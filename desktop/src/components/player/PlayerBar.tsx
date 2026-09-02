import {
  Activity,
  CircleStop,
  Music2,
  Pause,
  Play,
  SkipForward,
  SlidersHorizontal,
} from 'lucide-react';
import { Button as AriaButton, Dialog, DialogTrigger, Popover } from 'react-aria-components';
import { useEffect, useRef, useState, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface PlayerBarProps {
  useStore: DesktopStoreHook;
  diagnosticsTriggerRef?: RefObject<HTMLButtonElement | null>;
}

function formatDuration(value: number): string {
  const seconds = Math.max(0, Math.round(value / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

function stateLabel(state: string): string {
  return state.replaceAll('_', ' ');
}

export function PlayerBar({ useStore, diagnosticsTriggerRef }: PlayerBarProps) {
  const playback = useStore((store: DesktopStore) => store.playback);
  const selectedSongId = useStore((store: DesktopStore) => store.library.selectedSongId);
  const detail = useStore((store: DesktopStore) => store.detail);
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
  const admissionActionRef = useRef<HTMLButtonElement>(null);

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
  const admissionRequired = playback.prepared?.admission === 'confirmation_required';
  const selectedDurationUs =
    playback.prepared?.song.duration_us ?? detail.value?.duration_us ?? snapshot?.total_us ?? 0;
  const totalUs = snapshot?.total_us ?? selectedDurationUs;
  const currentUs = Math.min(Math.max(0, snapshot?.current_us ?? 0), totalUs || 0);
  const trackSubtitle = !selectedSongId
    ? 'Choose a sheet from Library'
    : playback.error
      ? 'Playback error'
      : active
        ? stateLabel(playback.state)
        : 'Ready';
  const profileSummary = defaults
    ? `${defaults.hold_frames}f · ${defaults.tempo_scale.toFixed(2)}× · ${defaults.fps} FPS${dryRun ? ' · Dry-run' : ''}`
    : 'Unavailable';
  const progressLabel = totalUs
    ? `Playback progress, ${formatDuration(currentUs)} of ${formatDuration(totalUs)}`
    : 'Playback progress unavailable';

  useEffect(() => {
    if (!admissionRequired) return;
    const frame = window.requestAnimationFrame(() => admissionActionRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [admissionRequired]);

  return (
    <footer className="player-bar" aria-label="Player controls">
      {admissionRequired && playback.prepared && (
        <section
          className="player-admission"
          role="group"
          aria-labelledby="playback-confirmation-title"
          aria-describedby="playback-confirmation-reason"
        >
          <div className="admission-copy">
            <strong id="playback-confirmation-title">Playback confirmation</strong>
            <span>{playback.prepared.risk.headline}</span>
            <span id="playback-confirmation-reason" className="muted">
              {playback.prepared.risk.reasons[0] ?? 'Review the selected sheet before starting.'}
            </span>
          </div>
          <div className="admission-actions">
            {playback.prepared.decisions.map((decision, index) => (
              <button
                key={decision.decision}
                className={`button${decision.decision === 'proceed' ? ' button-primary' : ''}`}
                type="button"
                ref={index === 0 ? admissionActionRef : undefined}
                onClick={() => void start(decision.decision)}
              >
                {decision.label}
              </button>
            ))}
          </div>
        </section>
      )}

      <div className="player-track-info">
        <span className="player-track-icon" aria-hidden="true">
          <Music2 size={17} />
        </span>
        <div className="player-track-copy">
          <strong title={selectedTitle}>{selectedTitle}</strong>
          <span className="muted">{trackSubtitle}</span>
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
              className="icon-button button-primary player-primary-action"
              type="button"
              aria-label="Play"
              title="Play"
              disabled={!selectedSongId}
              onClick={() => void prepareAndMaybeStart()}
            >
              <Play size={18} aria-hidden="true" />
            </button>
          )}
          {active && playback.state !== 'stopping' && (
            <>
              {playback.state === 'paused' ? (
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
              )}
              <button
                className="icon-button player-secondary-action"
                type="button"
                aria-label="Skip"
                title="Skip"
                onClick={() => void skip()}
              >
                <SkipForward size={16} aria-hidden="true" />
              </button>
              <button
                className="icon-button player-secondary-action"
                type="button"
                aria-label="Stop"
                title="Stop"
                onClick={() => void stop()}
              >
                <CircleStop size={16} aria-hidden="true" />
              </button>
            </>
          )}
        </div>
        <div className="player-timeline" aria-label={progressLabel}>
          <div className="player-timeline-labels">
            <span>{formatDuration(currentUs)}</span>
            <span>{formatDuration(totalUs)}</span>
          </div>
          <progress value={currentUs} max={totalUs || 1} aria-label={progressLabel} />
        </div>
        {playback.prepared && !active && (
          <span className="player-transport-status" role="status">
            {admissionRequired ? 'Awaiting confirmation' : 'Preparing playback'}
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

      <div className="player-profile">
        <DialogTrigger isOpen={profileOpen} onOpenChange={setProfileOpen}>
          <AriaButton
            ref={profileTriggerRef}
            className="profile-summary-button"
            type="button"
            aria-label="Configure timing profile"
          >
            <SlidersHorizontal size={15} aria-hidden="true" />
            <span>
              <strong>Playback profile</strong>
              <small>{profileSummary}</small>
            </span>
          </AriaButton>
          <Popover
            id="playback-profile-popover"
            className="profile-popover"
            placement="top end"
            offset={8}
          >
            <Dialog aria-label="Playback profile">
              <div className="profile-fields">
                <ProfileFields
                  defaults={defaults}
                  bootstrap={bootstrap}
                  dryRun={dryRun}
                  setDryRun={setDryRun}
                  patchSettings={patchSettings}
                />
              </div>
            </Dialog>
          </Popover>
        </DialogTrigger>
        <div className="profile-fields" aria-label="Playback profile">
          <ProfileFields
            defaults={defaults}
            bootstrap={bootstrap}
            dryRun={dryRun}
            setDryRun={setDryRun}
            patchSettings={patchSettings}
          />
        </div>
      </div>

      <button
        className="icon-button player-diagnostics-button"
        type="button"
        aria-label={diagnosticsOpen ? 'Close diagnostics' : 'Open diagnostics'}
        aria-pressed={diagnosticsOpen}
        ref={diagnosticsTriggerRef}
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
