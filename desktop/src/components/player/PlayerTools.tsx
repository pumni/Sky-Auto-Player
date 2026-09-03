import { PanelRight, SlidersHorizontal } from 'lucide-react';
import { Button as AriaButton, Dialog, DialogTrigger, Popover } from 'react-aria-components';
import { useRef, useState, type RefObject } from 'react';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';

interface PlayerToolsProps {
  useStore: DesktopStoreHook;
  utilityTriggerRef?: RefObject<HTMLButtonElement | null> | undefined;
}

export function PlayerTools({ useStore, utilityTriggerRef }: PlayerToolsProps) {
  const defaults = useStore((store) => store.settings?.playback_defaults);
  const bootstrap = useStore((store) => store.bootstrap);
  const utility = useStore((store) => store.utility);
  const patchSettings = useStore((store) => store.patchSettings);
  const toggleUtility = useStore((store) => store.toggleUtility);
  const prepare = useStore((store) => store.prepareSelectedPlayback);
  const start = useStore((store) => store.startPreparedPlayback);
  const selectedSongId = useStore((store) => store.library.selectedSongId);
  const playbackState = useStore((store) => store.playback.state);
  const pendingCommand = useStore((store) => store.playback.pendingCommand);
  const hasPreparedPlayback = useStore((store) => store.playback.prepared !== null);
  const [profileOpen, setProfileOpen] = useState(false);
  const profileTriggerRef = useRef<HTMLButtonElement>(null);
  const canPreparePlayback =
    Boolean(selectedSongId) &&
    !['starting', 'playing', 'paused', 'stopping'].includes(playbackState) &&
    pendingCommand === null &&
    !hasPreparedPlayback;
  const profileSummary = defaults
    ? `${defaults.hold_frames}f · ${defaults.tempo_scale.toFixed(2)}× · ${defaults.fps} FPS`
    : 'Unavailable';

  const prepareAndMaybeStart = async () => {
    await prepare({ dry_run: true });
    const current = useStore.getState();
    const prepared = current.playback.prepared;
    if (prepared?.song.song_id !== current.library.selectedSongId) return;
    if (prepared?.admission === 'ready') await start();
  };

  return (
    <div className="player-tools">
      <div className="player-profile">
        <DialogTrigger isOpen={profileOpen} onOpenChange={setProfileOpen}>
          <AriaButton
            ref={profileTriggerRef}
            className="player-tool-button profile-summary-button"
            type="button"
            aria-label="Configure playback profile"
            title="Configure playback profile"
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
                  patchSettings={patchSettings}
                />
              </div>
              <button
                className="button profile-test-action"
                type="button"
                disabled={!canPreparePlayback}
                onClick={() => {
                  setProfileOpen(false);
                  void prepareAndMaybeStart();
                }}
              >
                Test playback (no input)
              </button>
            </Dialog>
          </Popover>
        </DialogTrigger>
      </div>
      <button
        className="icon-button player-tool-button player-utility-button"
        type="button"
        aria-label={utility.open ? 'Close utility panel' : 'Open utility panel'}
        title={utility.open ? 'Close utility panel' : 'Open utility panel'}
        aria-pressed={utility.open}
        ref={utilityTriggerRef}
        onClick={() => toggleUtility()}
      >
        <PanelRight size={16} aria-hidden="true" />
      </button>
    </div>
  );
}

interface ProfileFieldsProps {
  defaults: NonNullable<DesktopStore['settings']>['playback_defaults'] | undefined;
  bootstrap: DesktopStore['bootstrap'];
  patchSettings: DesktopStore['patchSettings'];
}

function ProfileFields({ defaults, bootstrap, patchSettings }: ProfileFieldsProps) {
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
    </>
  );
}
