import { RefreshCw } from 'lucide-react';
import { useId } from 'react';
import appIcon40 from '../../assets/brand/app-icon-40.png';
import { resolveAppearance } from '../../appearance/resolveAppearance';
import { useSystemAppearance } from '../../appearance/useSystemAppearance';
import type {
  AppearanceMode,
  Bootstrap,
  PaletteId,
  SettingsPatch,
} from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { AppSelect } from '../ui/AppSelect';
import { formatFpsFrameHint, formatHoldFramesHint, formatTempoHint } from '../ui/appSelectHelpers';
import { AutoPlaySwitch } from './AutoPlaySwitch';
import { DonateSettings } from './DonateSettings';
import { TimingMarginControl } from './TimingMarginControl';

export type SettingsCategory =
  | 'playback'
  | 'appearance'
  | 'diagnostics'
  | 'updates'
  | 'donate'
  | 'about';

export const SETTINGS_CATEGORIES: Array<{ id: SettingsCategory; label: string }> = [
  { id: 'playback', label: 'Playback' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'diagnostics', label: 'Diagnostics' },
  { id: 'updates', label: 'Updates' },
  { id: 'donate', label: 'Donate' },
  { id: 'about', label: 'About' },
];

export interface PaletteMeta {
  id: PaletteId;
  label: string;
  swatches: readonly [string, string, string, string];
}

export const DEFAULT_PALETTE: PaletteMeta = {
  id: 'graphite',
  label: 'Graphite (Default)',
  swatches: ['#121314', '#191a1b', '#242526', '#8e9dff'],
};

export const PALETTES: readonly PaletteMeta[] = [
  DEFAULT_PALETTE,
  { id: 'aurora', label: 'Aurora', swatches: ['#0b1018', '#101722', '#151e2b', '#785ddd'] },
  { id: 'minimalist', label: 'Minimalist', swatches: ['#17191d', '#202328', '#292d33', '#a0b8d5'] },
  { id: 'slate', label: 'Slate', swatches: ['#10171d', '#17232c', '#20313d', '#6ca7d8'] },
  { id: 'cyberpunk', label: 'Cyberpunk', swatches: ['#130f1d', '#1d1529', '#291c37', '#d95abf'] },
  { id: 'classic', label: 'Classic', swatches: ['#171512', '#211d18', '#2c261f', '#c99242'] },
];

export const palettes = PALETTES;

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
  const baseHoldId = useId();
  const tempoId = useId();
  const fpsId = useId();
  const themeId = useId();
  const paletteId = useId();
  const channelId = useId();
  const telemetryId = useId();
  const telemetryDescId = useId();
  const verboseHudId = useId();
  const verboseHudDescId = useId();
  const autoCheckId = useId();
  const autoCheckDescId = useId();
  const patchSettings = useStore((store) => store.patchSettings);
  const update = useStore((store) => store.update);
  const checkForUpdate = useStore((store) => store.checkForUpdate);
  const patch = (value: SettingsPatch) => void patchSettings(value);
  const { mode: systemColorMode } = useSystemAppearance();
  const effectiveAppearance = resolveAppearance(
    settings.appearance_mode,
    systemColorMode,
    settings.palette,
  );
  const darkPaletteStandby = effectiveAppearance.colorMode === 'light';
  const activePaletteMeta =
    palettes.find((item) => item.id === settings.palette) ?? DEFAULT_PALETTE;
  const defaults = settings.playback_defaults;
  const frameUs = Math.ceil(1_000_000 / defaults.fps);
  const frameBaseHoldUs = Math.ceil(defaults.hold_frames * frameUs);
  const minHoldUs = frameBaseHoldUs + defaults.timing_margin_us;
  const checkingUpdates = update.checkRequestPending || update.state === 'checking';

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
              <div className="settings-field">
                <label htmlFor={baseHoldId}>Base Hold</label>
                <AppSelect
                  id={baseHoldId}
                  listboxLabel="Base Hold"
                  popoverMinWidth={176}
                  value={String(defaults.hold_frames)}
                  options={bootstrap.option_sets.hold_frames.map((value) => ({
                    value: String(value),
                    label: `${value} ${value === 1 ? 'frame' : 'frames'}`,
                    hint: formatHoldFramesHint(value, defaults.fps),
                  }))}
                  onChange={(nextValue) =>
                    patch({ playbackDefaults: { holdFrames: Number(nextValue) } })
                  }
                />
              </div>
              <div className="settings-field">
                <label htmlFor={tempoId}>Tempo</label>
                <AppSelect
                  id={tempoId}
                  listboxLabel="Tempo"
                  popoverMinWidth={154}
                  value={String(defaults.tempo_scale)}
                  options={bootstrap.option_sets.tempo_scales.map((value) => {
                    const hint = formatTempoHint(value);
                    return {
                      value: String(value),
                      label: `${value}×`,
                      ...(hint ? { hint } : {}),
                    };
                  })}
                  onChange={(nextValue) =>
                    patch({ playbackDefaults: { tempoScale: Number(nextValue) } })
                  }
                />
              </div>
              <div className="settings-field">
                <label htmlFor={fpsId}>FPS</label>
                <AppSelect
                  id={fpsId}
                  listboxLabel="FPS"
                  popoverMinWidth={160}
                  value={String(defaults.fps)}
                  options={bootstrap.option_sets.fps.map((value) => ({
                    value: String(value),
                    label: String(value),
                    hint: formatFpsFrameHint(value),
                  }))}
                  onChange={(nextValue) => patch({ playbackDefaults: { fps: Number(nextValue) } })}
                />
              </div>
            </div>
          </div>
          <div className="settings-timing-group">
            <h4>Timing</h4>
            <div className="settings-timing-card">
              <TimingMarginControl
                value={defaults.timing_margin_us}
                options={bootstrap.option_sets}
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
              </dl>
            </div>
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
          <div className="settings-field-group">
            <label htmlFor={themeId}>Theme</label>
            <AppSelect<AppearanceMode>
              id={themeId}
              listboxLabel="Theme"
              value={settings.appearance_mode}
              options={[
                { value: 'system', label: 'Follow Windows (recommended)', hint: 'Auto' },
                { value: 'light', label: 'Light' },
                { value: 'dark', label: 'Dark' },
              ]}
              onChange={(nextValue) => patch({ appearanceMode: nextValue })}
            />
          </div>
          <div
            className={
              darkPaletteStandby ? 'settings-field-group is-standby' : 'settings-field-group'
            }
          >
            <label htmlFor={paletteId}>Dark palette</label>
            <AppSelect<PaletteId>
              id={paletteId}
              listboxLabel="Dark palette"
              value={settings.palette}
              ariaDescribedBy={darkPaletteStandby ? 'dark-palette-standby-hint' : undefined}
              options={palettes.map((palette) => ({
                value: palette.id,
                label: palette.label,
                swatches: [palette.swatches[1], palette.swatches[3]],
              }))}
              onChange={(nextValue) => patch({ palette: nextValue })}
            />
            {darkPaletteStandby && (
              <span id="dark-palette-standby-hint" className="settings-field-hint">
                Saved for Dark mode (Light mode is currently active).
              </span>
            )}
          </div>
          <div
            className={`settings-palette-preview${darkPaletteStandby ? ' is-standby' : ''}`}
            aria-hidden="true"
          >
            <div className="settings-palette-preview-copy">
              <span className="settings-palette-preview-label">{activePaletteMeta.label}</span>
              <span className="settings-palette-preview-meta">
                {darkPaletteStandby ? 'Standby · Light active' : 'Active Dark surface tokens'}
              </span>
            </div>
            <div className="settings-palette-swatches">
              {activePaletteMeta.swatches.map((hex) => (
                <span
                  key={`${activePaletteMeta.id}-${hex}`}
                  className="settings-palette-swatch"
                  style={{ backgroundColor: hex }}
                />
              ))}
            </div>
          </div>
          <p className="settings-note">
            The saved palette is used whenever Dark is effective. Follow Windows uses the Windows
            app color preference while Theme is set to Follow Windows.
          </p>
        </section>
      )}

      {category === 'diagnostics' && (
        <section className="settings-section" aria-labelledby="diagnostic-settings-title">
          <h3 id="diagnostic-settings-title">Telemetry and diagnostics</h3>
          <div className="settings-toggle-list">
            <div className="settings-toggle-card">
              <label className="checkbox-row" htmlFor={telemetryId}>
                <input
                  id={telemetryId}
                  type="checkbox"
                  aria-describedby={telemetryDescId}
                  checked={settings.telemetry_enabled}
                  onChange={(event) => patch({ telemetryEnabled: event.target.checked })}
                />
                <span className="settings-toggle-title">Allow anonymous telemetry</span>
              </label>
              <p id={telemetryDescId} className="settings-toggle-description">
                Share anonymized diagnostics to help improve playback stability and timing quality.
              </p>
            </div>
            <div className="settings-toggle-card">
              <label className="checkbox-row" htmlFor={verboseHudId}>
                <input
                  id={verboseHudId}
                  type="checkbox"
                  aria-describedby={verboseHudDescId}
                  checked={settings.verbose_hud}
                  onChange={(event) => patch({ verboseHud: event.target.checked })}
                />
                <span className="settings-toggle-title">Verbose playback HUD</span>
              </label>
              <p id={verboseHudDescId} className="settings-toggle-description">
                Expose extended scheduling and native dispatch metrics in the diagnostics view.
              </p>
            </div>
          </div>
        </section>
      )}

      {category === 'updates' && (
        <section className="settings-section" aria-labelledby="update-settings-title">
          <h3 id="update-settings-title">Updates</h3>
          <div className="settings-toggle-card">
            <label className="checkbox-row" htmlFor={autoCheckId}>
              <input
                id={autoCheckId}
                type="checkbox"
                aria-describedby={autoCheckDescId}
                checked={settings.update_preferences.auto_check}
                onChange={(event) =>
                  patch({ updatePreferences: { autoCheck: event.target.checked } })
                }
              />
              <span className="settings-toggle-title">Check for updates automatically</span>
            </label>
            <p id={autoCheckDescId} className="settings-toggle-description">
              Check for signed releases in the background when Sky Auto Player starts.
            </p>
          </div>
          <div className="settings-field-group">
            <label htmlFor={channelId}>Channel</label>
            <AppSelect<'stable' | 'beta'>
              id={channelId}
              listboxLabel="Channel"
              value={settings.update_preferences.channel}
              options={[
                { value: 'stable', label: 'Stable' },
                { value: 'beta', label: 'Beta (unavailable)', disabled: true },
              ]}
              onChange={(nextValue) => patch({ updatePreferences: { channel: nextValue } })}
            />
          </div>
          <div className="settings-action-row">
            <button
              className="button settings-inline-action"
              type="button"
              disabled={checkingUpdates}
              onClick={() => void checkForUpdate('manual')}
            >
              <RefreshCw
                size={14}
                className={checkingUpdates ? 'spin' : undefined}
                aria-hidden="true"
              />
              {checkingUpdates ? 'Checking…' : 'Check for updates'}
            </button>
            <span className="settings-version-meta">Current version · {bootstrap.app_version}</span>
          </div>
        </section>
      )}

      {category === 'donate' && <DonateSettings />}

      {category === 'about' && (
        <section className="settings-section" aria-labelledby="about-settings-title">
          <div className="about-brand-header">
            <img
              className="about-brand-icon"
              src={appIcon40}
              alt=""
              width={36}
              height={36}
              aria-hidden="true"
            />
            <div className="about-brand-copy">
              <h3 id="about-settings-title">About Sky Auto Player</h3>
              <p className="about-intro">
                A focused Windows workbench for preparing and playing Sky music sheets.
              </p>
            </div>
          </div>
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
