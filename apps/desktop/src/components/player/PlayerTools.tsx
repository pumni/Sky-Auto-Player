import { ChevronRight, FlaskConical, ListVideo, PanelRight, SlidersHorizontal } from 'lucide-react';
import { Button as AriaButton, Dialog, DialogTrigger, Popover } from 'react-aria-components';
import { useId, useRef, useState, type RefObject } from 'react';
import { TimingMarginControl } from '../settings/TimingMarginControl';
import { AppSelect } from '../ui/AppSelect';
import { formatFpsFrameHint, formatHoldFramesHint, formatTempoHint } from '../ui/appSelectHelpers';
import {
  playbackSessionStatusConcern,
  type DesktopStore,
  type DesktopStoreHook,
} from '../../state/store';

interface PlayerToolsProps {
  useStore: DesktopStoreHook;
  utilityTriggerRef?: RefObject<HTMLButtonElement | null> | undefined;
}

export function PlayerTools({ useStore, utilityTriggerRef }: PlayerToolsProps) {
  const settings = useStore((store) => store.settings);
  const autoPlayEnabled = settings?.auto_play ?? true;
  const defaults = settings?.playback_defaults;
  const bootstrap = useStore((store) => store.bootstrap);
  const utility = useStore((store) => store.utility);
  const patchSettings = useStore((store) => store.patchSettings);
  const toggleUtility = useStore((store) => store.toggleUtility);
  const prepare = useStore((store) => store.prepareSelectedPlayback);
  const start = useStore((store) => store.startPreparedPlayback);
  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const selectedSongTitle = useStore((store) => {
    const active = store.playback.currentSong;
    if (active) return active.title;
    if (!store.library.selectedSongId) return null;
    if (store.library.selectedSongSummary?.song_id === store.library.selectedSongId) {
      return store.library.selectedSongSummary.title;
    }
    for (const rows of store.library.pages.values()) {
      const match = rows.find((row) => row.song_id === store.library.selectedSongId);
      if (match) return match.title;
    }
    return null;
  });
  const playback = useStore((store) => store.playback);
  const hasPreparedPlayback = playback.prepared !== null;
  const sessionStatusConcern = useStore((store) => playbackSessionStatusConcern(store.playback));
  const [profileOpen, setProfileOpen] = useState(false);
  const [autoPlayPending, setAutoPlayPending] = useState(false);
  const autoPlayMutationPending = useRef(false);
  const profileTriggerRef = useRef<HTMLButtonElement>(null);
  const hasTestSong = Boolean(playback.currentSong || selectedSongId);
  const canPreparePlayback =
    hasTestSong &&
    playback.sessionId === null &&
    playback.transportOperation === null &&
    playback.startRequestId === null &&
    !playback.statusRetryPending &&
    sessionStatusConcern === null &&
    !hasPreparedPlayback;
  const previewDescription = !hasTestSong
    ? 'Select a song to enable a dry-run preview. No keyboard input will be sent.'
    : !canPreparePlayback
      ? 'Finish the current playback or pending preparation before starting another test.'
      : 'Try these settings without sending any physical keyboard input.';
  const profileSummary = defaults
    ? `${defaults.hold_frames}f · ${defaults.tempo_scale}× · ${defaults.fps} FPS`
    : null;

  const prepareAndMaybeStart = async () => {
    await prepare({ dry_run: true });
    const current = useStore.getState();
    const prepared = current.playback.prepared;
    if (prepared?.admission === 'ready') await start();
  };

  const toggleAutoPlay = async () => {
    if (!settings || autoPlayMutationPending.current) return;
    autoPlayMutationPending.current = true;
    setAutoPlayPending(true);
    try {
      await patchSettings({ autoPlay: !settings.auto_play });
    } finally {
      autoPlayMutationPending.current = false;
      setAutoPlayPending(false);
    }
  };

  return (
    <div className="player-tools">
      <div className="player-tooltip-anchor">
        <AriaButton
          className={`icon-button player-tool-button player-auto-play-button${autoPlayEnabled ? ' is-on' : ''}`}
          aria-label="Auto Play"
          aria-describedby="player-auto-play-tooltip"
          aria-pressed={autoPlayEnabled}
          isDisabled={!settings}
          aria-busy={autoPlayPending}
          onPress={() => void toggleAutoPlay()}
        >
          <ListVideo size={18} aria-hidden="true" />
        </AriaButton>
        <span id="player-auto-play-tooltip" className="player-tool-tooltip" role="tooltip">
          Auto Play {autoPlayEnabled ? 'on' : 'off'} · Continue to the next song
        </span>
      </div>
      <div className={`player-profile player-tooltip-anchor${profileOpen ? ' is-open' : ''}`}>
        <DialogTrigger isOpen={profileOpen} onOpenChange={setProfileOpen}>
          <AriaButton
            ref={profileTriggerRef}
            className={`player-tool-button profile-summary-button${profileOpen ? ' is-open' : ''}`}
            type="button"
            aria-label="Configure playback profile"
            aria-describedby="player-profile-tooltip"
            aria-expanded={profileOpen}
          >
            <SlidersHorizontal size={18} aria-hidden="true" />
          </AriaButton>
          <Popover className="profile-popover" placement="top end" offset={8}>
            <Dialog aria-label="Playback profile">
              <header className="profile-popover-header">
                <div className="profile-popover-heading">
                  <h3>Playback profile</h3>
                  <p>Fine-tune how each song is played.</p>
                </div>
                {profileSummary && (
                  <span className="profile-header-summary" aria-hidden="true">
                    {defaults?.hold_frames}f · {defaults?.tempo_scale}× · {defaults?.fps}
                  </span>
                )}
              </header>
              <div className="profile-fields">
                <ProfileFields
                  defaults={defaults}
                  bootstrap={bootstrap}
                  recommendation={settings?.timing_margin_recommendation}
                  patchSettings={patchSettings}
                />
              </div>
              <div className="profile-preview">
                <div className="profile-preview-copy">
                  <div className="profile-preview-heading">
                    <strong>Safe preview</strong>
                    {selectedSongTitle && (
                      <span className="profile-preview-song" title={selectedSongTitle}>
                        {selectedSongTitle}
                      </span>
                    )}
                  </div>
                  <p className="profile-test-explanation">{previewDescription}</p>
                </div>
                <button
                  className={`button profile-test-action${canPreparePlayback ? ' is-ready' : ''}`}
                  type="button"
                  disabled={!canPreparePlayback}
                  onClick={() => {
                    setProfileOpen(false);
                    void prepareAndMaybeStart();
                  }}
                >
                  <FlaskConical size={15} aria-hidden="true" />
                  Test playback (no input)
                </button>
              </div>
            </Dialog>
          </Popover>
        </DialogTrigger>
        <span id="player-profile-tooltip" className="player-tool-tooltip" role="tooltip">
          {profileSummary ? `Playback profile · ${profileSummary}` : 'Playback profile'}
        </span>
      </div>
      <div className="player-tooltip-anchor">
        <AriaButton
          className="icon-button player-tool-button player-utility-button"
          aria-label={utility.open ? 'Close utility panel' : 'Open utility panel'}
          aria-describedby="player-utility-tooltip"
          aria-expanded={utility.open}
          {...(utility.open ? { 'aria-controls': 'utility-inspector' } : {})}
          ref={utilityTriggerRef}
          onPress={() => toggleUtility()}
        >
          <PanelRight size={16} aria-hidden="true" />
        </AriaButton>
        <span id="player-utility-tooltip" className="player-tool-tooltip" role="tooltip">
          {utility.open ? 'Close Inspector' : 'Open Inspector'}
        </span>
      </div>
    </div>
  );
}

interface ProfileFieldsProps {
  defaults: NonNullable<DesktopStore['settings']>['playback_defaults'] | undefined;
  bootstrap: DesktopStore['bootstrap'];
  recommendation: NonNullable<DesktopStore['settings']>['timing_margin_recommendation'] | undefined;
  patchSettings: DesktopStore['patchSettings'];
}

function ProfileFields({ defaults, bootstrap, recommendation, patchSettings }: ProfileFieldsProps) {
  const holdId = useId();
  const tempoId = useId();
  const fpsId = useId();

  if (!defaults || !bootstrap || !recommendation) {
    return <span className="muted">Profile unavailable</span>;
  }
  const frameUs = Math.ceil(1_000_000 / defaults.fps);
  const frameBaseHoldUs = Math.ceil(defaults.hold_frames * frameUs);
  const minHoldUs = frameBaseHoldUs + defaults.timing_margin_us;
  const resumeCleanupDelayUs = frameUs + defaults.timing_margin_us;

  const holdOptions = bootstrap.option_sets.hold_frames.map((value) => ({
    value: String(value),
    label: `${value}f`,
    hint: formatHoldFramesHint(value, defaults.fps),
  }));
  const tempoOptions = bootstrap.option_sets.tempo_scales.map((value) => {
    const hint = formatTempoHint(value);
    return {
      value: String(value),
      label: `${value}×`,
      ...(hint ? { hint } : {}),
    };
  });
  const fpsOptions = bootstrap.option_sets.fps.map((value) => ({
    value: String(value),
    label: String(value),
    hint: formatFpsFrameHint(value),
  }));

  return (
    <>
      <div className="profile-field">
        <label htmlFor={holdId}>Base Hold</label>
        <AppSelect
          id={holdId}
          listboxLabel="Base Hold"
          density="compact"
          popoverMinWidth={154}
          value={String(defaults.hold_frames)}
          options={holdOptions}
          onChange={(nextValue) =>
            void patchSettings({ playbackDefaults: { holdFrames: Number(nextValue) } })
          }
        />
      </div>
      <div className="profile-field">
        <label htmlFor={tempoId}>Tempo</label>
        <AppSelect
          id={tempoId}
          listboxLabel="Tempo"
          density="compact"
          popoverMinWidth={144}
          value={String(defaults.tempo_scale)}
          options={tempoOptions}
          onChange={(nextValue) =>
            void patchSettings({ playbackDefaults: { tempoScale: Number(nextValue) } })
          }
        />
      </div>
      <div className="profile-field">
        <label htmlFor={fpsId}>FPS</label>
        <AppSelect
          id={fpsId}
          listboxLabel="FPS"
          density="compact"
          popoverMinWidth={154}
          value={String(defaults.fps)}
          options={fpsOptions}
          onChange={(nextValue) =>
            void patchSettings({ playbackDefaults: { fps: Number(nextValue) } })
          }
        />
      </div>
      <TimingMarginControl
        value={defaults.timing_margin_us}
        options={bootstrap.option_sets}
        recommendation={recommendation}
        density="compact"
        onChange={(value) =>
          patchSettings({ playbackDefaults: { timingMarginUs: value } }).then(
            (authoritative) => authoritative?.playback_defaults.timing_margin_us ?? null,
          )
        }
      />
      <details className="profile-advanced">
        <summary>
          <ChevronRight className="profile-advanced-chevron" size={14} aria-hidden="true" />
          <span>Calculated timing</span>
        </summary>
        <dl className="profile-timing-summary">
          <div>
            <dt>Target hold</dt>
            <dd>{(minHoldUs / 1_000).toFixed(3)} ms</dd>
          </div>
          <div>
            <dt>Resume cleanup delay</dt>
            <dd>{(resumeCleanupDelayUs / 1_000).toFixed(3)} ms</dd>
          </div>
        </dl>
      </details>
    </>
  );
}
