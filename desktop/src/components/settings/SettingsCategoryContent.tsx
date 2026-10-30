import type {
  AppearanceMode,
  Bootstrap,
  PaletteId,
  SettingsPatch,
} from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { AutoPlaySwitch } from './AutoPlaySwitch';
import { TimingMarginControl } from './TimingMarginControl';

export type SettingsCategory =
  'playback' | 'appearance' | 'diagnostics' | 'updates' | 'advanced' | 'about';

export const SETTINGS_CATEGORIES: Array<{ id: SettingsCategory; label: string }> = [
  { id: 'playback', label: 'Playback' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'updates', label: 'Updates' },
  { id: 'advanced', label: 'Advanced' },
  { id: 'about', label: 'About' },
];

const palettes: Array<{ id: PaletteId; label: string }> = [
  { id: 'graphite', label: 'Graphite (Default)' },
  { id: 'aurora', label: 'Aurora' },
  { id: 'minimalist', label: 'Minimalist' },
  { id: 'slate', label: 'Slate' },
  { id: 'cyberpunk', label: 'Cyberpunk' },
  { id: 'classic', label: 'Classic' },
];

interface SettingsCategoryContentProps {
  category: SettingsCategory;
  bootstrap: Bootstrap;
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
}

export function SettingsCategoryContent({
  category,
  bootstrap,
  settings,
  useStore,
}: SettingsCategoryContentProps) {
  const patchSettings = useStore((store) => store.patchSettings);
  const setOpen = useStore((store) => store.setSettingsOpen);
  const setCalibrationOpen = useStore((store) => store.setCalibrationOpen);
  const update = useStore((store) => store.update);
  const checkForUpdate = useStore((store) => store.checkForUpdate);
  const patch = (value: SettingsPatch) => void patchSettings(value);
  const defaults = settings.playback_defaults;
  const frameUs = Math.ceil(1_000_000 / defaults.fps);
  const frameBaseHoldUs = Math.ceil(defaults.hold_frames * frameUs);
  const minHoldUs = frameBaseHoldUs + defaults.timing_margin_us;
  const minReleaseGapUs = frameUs + defaults.timing_margin_us;

  return (
    <>
      {category === 'playback' && (
        <section className="settings-section" aria-labelledby="playback-settings-title">
          <h3 id="playback-settings-title">Playback</h3>
          <div className="settings-playback-group">
            <h4>Behavior</h4>
            <AutoPlaySwitch
              checked={settings.auto_play}
              onChange={(autoPlay) => patch({ autoPlay })}
            />
          </div>
          <div className="settings-playback-group">
            <h4>Playback defaults</h4>
            <div className="settings-grid">
              <label>
                Base Hold
                <select
                  value={defaults.hold_frames}
                  onChange={(event) =>
                    patch({ playbackDefaults: { holdFrames: Number(event.target.value) } })
                  }
                >
                  {bootstrap.option_sets.hold_frames.map((value) => (
                    <option key={value} value={value}>
                      {value} {value === 1 ? 'frame' : 'frames'}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Tempo
                <select
                  value={defaults.tempo_scale}
                  onChange={(event) =>
                    patch({ playbackDefaults: { tempoScale: Number(event.target.value) } })
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
                    patch({ playbackDefaults: { fps: Number(event.target.value) } })
                  }
                >
                  {bootstrap.option_sets.fps.map((value) => (
                    <option key={value} value={value}>
                      {value}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          </div>
          <div className="settings-timing-group">
            <h4>Timing</h4>
            <TimingMarginControl
              value={defaults.timing_margin_us}
              options={bootstrap.option_sets}
              recommendation={settings.timing_margin_recommendation}
              density="full"
              onChange={(value) =>
                patchSettings({ playbackDefaults: { timingMarginUs: value } }).then(
                  (authoritative) => authoritative?.playback_defaults.timing_margin_us ?? null,
                )
              }
            />
            <dl className="settings-timing-summary">
              <div>
                <dt>1 frame</dt>
                <dd>{(frameUs / 1_000).toFixed(3)} ms</dd>
              </div>
              <div>
                <dt>Base hold</dt>
                <dd>{(frameBaseHoldUs / 1_000).toFixed(3)} ms</dd>
              </div>
              <div>
                <dt>Target hold</dt>
                <dd>{(minHoldUs / 1_000).toFixed(3)} ms</dd>
              </div>
              <div>
                <dt>Release gap</dt>
                <dd>{(minReleaseGapUs / 1_000).toFixed(3)} ms</dd>
              </div>
            </dl>
            <p className="settings-note">
              Changes apply to the next prepared session; an active session keeps its frozen timing
              values.
            </p>
          </div>
        </section>
      )}

      {category === 'appearance' && (
        <section className="settings-section" aria-labelledby="appearance-settings-title">
          <h3 id="appearance-settings-title">Appearance</h3>
          <label>
            Theme
            <select
              value={settings.appearance_mode}
              onChange={(event) => patch({ appearanceMode: event.target.value as AppearanceMode })}
            >
              <option value="system">Follow Windows (recommended)</option>
              <option value="light">Light</option>
              <option value="dark">Dark</option>
            </select>
          </label>
          <label>
            Dark palette
            <select
              value={settings.palette}
              onChange={(event) => patch({ palette: event.target.value as PaletteId })}
            >
              {palettes.map((palette) => (
                <option key={palette.id} value={palette.id}>
                  {palette.label}
                </option>
              ))}
            </select>
          </label>
          <p className="settings-note">
            The saved palette is used whenever Dark is effective. Follow Windows uses the Windows
            app color preference while Theme is set to Follow Windows.
          </p>
        </section>
      )}

      {category === 'diagnostics' && (
        <section className="settings-section" aria-labelledby="diagnostic-settings-title">
          <h3 id="diagnostic-settings-title">Telemetry and diagnostics</h3>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={settings.telemetry_enabled}
              onChange={(event) => patch({ telemetryEnabled: event.target.checked })}
            />
            Allow anonymous telemetry
          </label>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={settings.verbose_hud}
              onChange={(event) => patch({ verboseHud: event.target.checked })}
            />
            Verbose playback HUD
          </label>
        </section>
      )}

      {category === 'updates' && (
        <section className="settings-section" aria-labelledby="update-settings-title">
          <h3 id="update-settings-title">Updates</h3>
          <label className="checkbox-row">
            <input
              type="checkbox"
              checked={settings.update_preferences.auto_check}
              onChange={(event) =>
                patch({ updatePreferences: { autoCheck: event.target.checked } })
              }
            />
            Check for updates automatically
          </label>
          <label>
            Channel
            <select
              value={settings.update_preferences.channel}
              onChange={(event) =>
                patch({
                  updatePreferences: { channel: event.target.value as 'stable' | 'beta' },
                })
              }
            >
              <option value="stable">Stable</option>
              <option value="beta" disabled>
                Beta (unavailable)
              </option>
            </select>
          </label>
          <button
            className="button"
            type="button"
            disabled={update.checkRequestPending || update.state === 'checking'}
            onClick={() => void checkForUpdate('manual')}
          >
            {update.checkRequestPending || update.state === 'checking'
              ? 'Checking…'
              : 'Check for updates'}
          </button>
        </section>
      )}

      {category === 'advanced' && (
        <section className="settings-section" aria-labelledby="advanced-settings-title">
          <h3 id="advanced-settings-title">Advanced</h3>
          <p className="settings-note">
            Native timing, input and security policy remain owned by the local Rust runtime.
          </p>
          <div className="settings-subsection">
            <h3>Advanced timing</h3>
            <p className="settings-note">
              Run a safe, testable timing calibration for this machine.
            </p>
            <button
              className="button"
              type="button"
              onClick={() => {
                setOpen(false);
                setCalibrationOpen(true);
              }}
            >
              Open calibration
            </button>
          </div>
        </section>
      )}

      {category === 'about' && (
        <section className="settings-section" aria-labelledby="about-settings-title">
          <h3 id="about-settings-title">About Sky Auto Player</h3>
          <p className="about-intro">
            A focused Windows workbench for preparing and playing Sky music sheets.
          </p>
          <dl className="about-details">
            <div>
              <dt>Version</dt>
              <dd>{bootstrap.app_version}</dd>
            </div>
            <div>
              <dt>Native version</dt>
              <dd>{bootstrap.native_build.native_version}</dd>
            </div>
            <div>
              <dt>Build commit</dt>
              <dd>{bootstrap.native_build.native_build_commit}</dd>
            </div>
            <div>
              <dt>Native ABI</dt>
              <dd>{bootstrap.native_build.native_abi}</dd>
            </div>
            <div>
              <dt>Rust</dt>
              <dd>{bootstrap.native_build.rustc_version}</dd>
            </div>
            <div>
              <dt>Win32 backend</dt>
              <dd>{bootstrap.native_build.win32_backend ? 'Enabled' : 'Disabled'}</dd>
            </div>
          </dl>
        </section>
      )}
    </>
  );
}
