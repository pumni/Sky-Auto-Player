import { FlaskConical, ListVideo, PanelRight, SlidersHorizontal } from 'lucide-react';
import {
  Button as AriaButton,
  Dialog,
  DialogTrigger,
  Popover,
  Tooltip,
  TooltipTrigger,
} from 'react-aria-components';
import { useRef, useState, type RefObject } from 'react';
import { TimingMarginControl } from '../settings/TimingMarginControl';
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
  const playback = useStore((store) => store.playback);
  const hasPreparedPlayback = playback.prepared !== null;
  const sessionStatusConcern = useStore((store) => playbackSessionStatusConcern(store.playback));
  const [profileOpen, setProfileOpen] = useState(false);
  const [autoPlayPending, setAutoPlayPending] = useState(false);
  const autoPlayMutationPending = useRef(false);
  const profileTriggerRef = useRef<HTMLButtonElement>(null);
  const canPreparePlayback =
    Boolean(playback.currentSong || selectedSongId) &&
    playback.sessionId === null &&
    playback.transportOperation === null &&
    playback.startRequestId === null &&
    !playback.statusRetryPending &&
    sessionStatusConcern === null &&
    !hasPreparedPlayback;

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
      <TooltipTrigger delay={350} closeDelay={100}>
        <AriaButton
          className={`icon-button player-tool-button player-auto-play-button${autoPlayEnabled ? ' is-on' : ''}`}
          aria-label="Auto Play"
          aria-pressed={autoPlayEnabled}
          isDisabled={!settings || autoPlayPending}
          onPress={() => void toggleAutoPlay()}
        >
          <ListVideo size={18} aria-hidden="true" />
        </AriaButton>
        <Tooltip className="player-tool-tooltip" placement="top" offset={8}>
          Auto Play {autoPlayEnabled ? 'on' : 'off'} · Continue to the next song
        </Tooltip>
      </TooltipTrigger>
      <div className="player-profile">
        <DialogTrigger isOpen={profileOpen} onOpenChange={setProfileOpen}>
          <AriaButton
            ref={profileTriggerRef}
            className="player-tool-button profile-summary-button"
            type="button"
            aria-label="Configure playback profile"
            title="Configure playback profile"
          >
            <SlidersHorizontal size={18} aria-hidden="true" />
          </AriaButton>
          <Popover className="profile-popover" placement="top end" offset={8}>
            <Dialog aria-label="Playback profile">
              <header className="profile-popover-header">
                <h3>Playback profile</h3>
                <p>Adjust note timing for your next playback session.</p>
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
                <p className="profile-test-explanation">
                  Tests the current profile with dry-run playback; no physical keyboard input is
                  sent.
                </p>
                <button
                  className="button profile-test-action"
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
      </div>
      <TooltipTrigger delay={350} closeDelay={100}>
        <AriaButton
          className="icon-button player-tool-button player-utility-button"
          aria-label={utility.open ? 'Close utility panel' : 'Open utility panel'}
          aria-expanded={utility.open}
          aria-controls={utility.open ? 'utility-inspector' : undefined}
          ref={utilityTriggerRef}
          onPress={() => toggleUtility()}
        >
          <PanelRight size={16} aria-hidden="true" />
        </AriaButton>
        <Tooltip className="player-tool-tooltip" placement="top" offset={8}>
          {utility.open ? 'Close Inspector' : 'Open Inspector'}
        </Tooltip>
      </TooltipTrigger>
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
  if (!defaults || !bootstrap || !recommendation) {
    return <span className="muted">Profile unavailable</span>;
  }
  const frameUs = Math.ceil(1_000_000 / defaults.fps);
  const frameBaseHoldUs = Math.ceil(defaults.hold_frames * frameUs);
  const minHoldUs = frameBaseHoldUs + defaults.timing_margin_us;
  const minReleaseGapUs = frameUs + defaults.timing_margin_us;
  return (
    <>
      <label>
        Base Hold
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
        <summary>Calculated timing</summary>
        <dl className="profile-timing-summary">
          <div>
            <dt>Target hold</dt>
            <dd>{(minHoldUs / 1_000).toFixed(3)} ms</dd>
          </div>
          <div>
            <dt>Release gap</dt>
            <dd>{(minReleaseGapUs / 1_000).toFixed(3)} ms</dd>
          </div>
        </dl>
      </details>
    </>
  );
}
