import { RefreshCw } from 'lucide-react';
import { useId } from 'react';
import { resolveAppearance } from '../../appearance/resolveAppearance';
import { useSystemAppearance } from '../../appearance/useSystemAppearance';
import appIcon40 from '../../assets/brand/app-icon-40.png';
import type {
  AppearanceMode,
  Bootstrap,
  PaletteId,
  SettingsPatch,
} from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { Button } from '../ui/button';
import {
  formatFpsFrameHint,
  formatHoldFramesHint,
  formatTempoHint,
} from '../ui/playbackOptionFormatting';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
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
  onUpdateDialogOpen?: () => void;
}

export function SettingsCategoryContent({
  category,
  bootstrap,
  settings,
  useStore,
  onUpdateDialogOpen,
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
        <section
          data-settings-category={category}
          data-slot="settings-category-content"
          className="grid content-start gap-3 py-4"
          aria-labelledby="playback-settings-title"
        >
          <h3
            className="m-0 text-[15px] font-semibold tracking-tight text-foreground"
            id="playback-settings-title"
          >
            Playback
          </h3>
          <div className="grid gap-2 border-t border-border pt-2.5">
            <h4 className="m-0 text-xs font-semibold text-text-secondary">Behavior</h4>
            <div className="rounded-control border border-border bg-surface px-3">
              <AutoPlaySwitch
                checked={settings.auto_play}
                onChange={(autoPlay) => patch({ autoPlay })}
              />
            </div>
          </div>
          <div className="grid gap-2 border-t border-border pt-2.5">
            <h4 className="m-0 text-xs font-semibold text-text-secondary">Playback defaults</h4>
            <div className="grid grid-cols-3 gap-3 max-[700px]:grid-cols-1">
              <div className="grid min-w-0 content-start gap-1.5">
                <label
                  className="cursor-pointer text-[13px] font-medium text-text-secondary"
                  htmlFor={baseHoldId}
                >
                  Base Hold
                </label>
                <Select
                  value={String(defaults.hold_frames)}
                  onValueChange={(nextValue) => {
                    if (nextValue !== null)
                      patch({ playbackDefaults: { holdFrames: Number(nextValue) } });
                  }}
                >
                  <SelectTrigger id={baseHoldId} aria-label="Base Hold">
                    <SelectValue>
                      {(value: string | null) =>
                        value === null
                          ? 'Base Hold'
                          : `${value} ${Number(value) === 1 ? 'frame' : 'frames'}`
                      }
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent className="min-w-44">
                    {bootstrap.option_sets.hold_frames.map((value) => (
                      <SelectItem key={value} value={String(value)}>
                        <span>{`${value} ${value === 1 ? 'frame' : 'frames'}`}</span>
                        <span className="text-text-tertiary">
                          {formatHoldFramesHint(value, defaults.fps)}
                        </span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid min-w-0 content-start gap-1.5">
                <label
                  className="cursor-pointer text-[13px] font-medium text-text-secondary"
                  htmlFor={tempoId}
                >
                  Tempo
                </label>
                <Select
                  value={String(defaults.tempo_scale)}
                  onValueChange={(nextValue) => {
                    if (nextValue !== null)
                      patch({ playbackDefaults: { tempoScale: Number(nextValue) } });
                  }}
                >
                  <SelectTrigger id={tempoId} aria-label="Tempo">
                    <SelectValue>
                      {(value: string | null) => (value === null ? 'Tempo' : `${value}×`)}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent className="min-w-40">
                    {bootstrap.option_sets.tempo_scales.map((value) => {
                      const hint = formatTempoHint(value);
                      return (
                        <SelectItem key={value} value={String(value)}>
                          <span>{value}×</span>
                          {hint && <span className="text-text-tertiary">{hint}</span>}
                        </SelectItem>
                      );
                    })}
                  </SelectContent>
                </Select>
              </div>
              <div className="grid min-w-0 content-start gap-1.5">
                <label
                  className="cursor-pointer text-[13px] font-medium text-text-secondary"
                  htmlFor={fpsId}
                >
                  FPS
                </label>
                <Select
                  value={String(defaults.fps)}
                  onValueChange={(nextValue) => {
                    if (nextValue !== null) patch({ playbackDefaults: { fps: Number(nextValue) } });
                  }}
                >
                  <SelectTrigger id={fpsId} aria-label="FPS">
                    <SelectValue>{(value: string | null) => value ?? 'FPS'}</SelectValue>
                  </SelectTrigger>
                  <SelectContent className="min-w-40">
                    {bootstrap.option_sets.fps.map((value) => (
                      <SelectItem key={value} value={String(value)}>
                        <span>{value}</span>
                        <span className="text-text-tertiary">{formatFpsFrameHint(value)}</span>
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            </div>
          </div>
          <div className="mt-0 grid gap-2 border-t border-border pt-2.5">
            <h4 className="m-0 text-xs font-semibold text-text-secondary">Timing</h4>
            <div className="grid gap-2.5 rounded-pane border border-border bg-surface p-3">
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
              <dl className="m-0 grid grid-cols-2 gap-x-5 gap-y-1.5 border-t border-border pt-2.5 text-xs tabular-nums">
                <div>
                  <dt className="text-text-tertiary">1 frame</dt>
                  <dd className="m-0 font-medium text-foreground">
                    {(frameUs / 1_000).toFixed(3)} ms
                  </dd>
                </div>
                <div>
                  <dt className="text-text-tertiary">Base hold</dt>
                  <dd className="m-0 font-medium text-foreground">
                    {(frameBaseHoldUs / 1_000).toFixed(3)} ms
                  </dd>
                </div>
                <div>
                  <dt className="text-text-tertiary">Target hold</dt>
                  <dd className="m-0 font-medium text-foreground">
                    {(minHoldUs / 1_000).toFixed(3)} ms
                  </dd>
                </div>
              </dl>
            </div>
            <p className="m-0 text-xs leading-relaxed text-text-tertiary">
              Changes apply to the next prepared session; an active session keeps its frozen timing
              values.
            </p>
          </div>
        </section>
      )}

      {category === 'appearance' && (
        <section
          data-settings-category={category}
          data-slot="settings-category-content"
          className="grid content-start gap-3 py-4"
          aria-labelledby="appearance-settings-title"
        >
          <h3
            className="m-0 text-[15px] font-semibold tracking-tight text-foreground"
            id="appearance-settings-title"
          >
            Appearance
          </h3>
          <div className="grid w-full max-w-[360px] gap-1.5">
            <label className="text-[13px] font-medium text-text-secondary" htmlFor={themeId}>
              Theme
            </label>
            <Select
              value={settings.appearance_mode}
              onValueChange={(nextValue) => {
                if (nextValue !== null) patch({ appearanceMode: nextValue as AppearanceMode });
              }}
            >
              <SelectTrigger id={themeId} aria-label="Theme">
                <SelectValue>
                  {(value: string | null) =>
                    value === 'system'
                      ? 'Follow Windows (recommended)'
                      : value === 'light'
                        ? 'Light'
                        : value === 'dark'
                          ? 'Dark'
                          : 'Theme'
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="system">
                  <span>Follow Windows (recommended)</span>
                  <span className="text-text-tertiary">Auto</span>
                </SelectItem>
                <SelectItem value="light">Light</SelectItem>
                <SelectItem value="dark">Dark</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="grid w-full max-w-[360px] gap-1.5">
            <label className="text-[13px] font-medium text-text-secondary" htmlFor={paletteId}>
              Dark palette
            </label>
            <Select
              value={settings.palette}
              onValueChange={(nextValue) => {
                if (nextValue !== null) patch({ palette: nextValue as PaletteId });
              }}
            >
              <SelectTrigger id={paletteId} aria-label="Dark palette">
                <SelectValue>
                  {(value: string | null) =>
                    palettes.find((item) => item.id === value)?.label ?? 'Dark palette'
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                {palettes.map((palette) => (
                  <SelectItem key={palette.id} value={palette.id}>
                    <span className="inline-flex items-center gap-2">
                      <span className="inline-flex gap-1" aria-hidden="true">
                        {[palette.swatches[1], palette.swatches[3]].map((hex) => (
                          <span
                            key={hex}
                            className="size-3 rounded-full border border-border"
                            style={{ backgroundColor: hex }}
                          />
                        ))}
                      </span>
                      {palette.label}
                    </span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div
            data-slot="appearance-preview"
            data-effective-mode={effectiveAppearance.colorMode}
            role="img"
            aria-label={`${effectiveAppearance.colorMode === 'light' ? 'Light' : 'Dark'} appearance preview; saved dark palette is ${activePaletteMeta.label}`}
            className="w-full max-w-[360px] overflow-hidden rounded-pane border border-border bg-background text-foreground"
          >
            <div className="flex h-7 items-center gap-1.5 border-b border-border bg-card px-2">
              <span className="size-2 rounded-full bg-primary" />
              <span className="h-1.5 w-14 rounded-full bg-text-secondary opacity-70" />
              <span className="ml-auto h-1.5 w-8 rounded-full bg-text-tertiary opacity-70" />
            </div>
            <div className="grid grid-cols-[minmax(0,1fr)_68px] gap-2 bg-background p-2">
              <div className="grid h-10 grid-cols-[1fr_40px] items-center gap-2 rounded-control border border-border bg-surface px-2">
                <span className="h-1.5 rounded-full bg-text-secondary opacity-70" />
                <span className="h-5 rounded-control bg-selection" />
              </div>
              <div className="h-10 rounded-control border border-border bg-popover" />
              <span
                data-slot="appearance-preview-primary-action"
                className="col-span-2 inline-flex w-fit items-center rounded-control bg-primary px-2 py-1 text-[10px] font-medium text-primary-foreground"
              >
                Primary action
              </span>
            </div>
          </div>
          <p
            data-slot="appearance-active-summary"
            className="m-0 text-xs leading-relaxed text-foreground"
          >
            <strong>Active: {effectiveAppearance.colorMode === 'light' ? 'Light' : 'Dark'}.</strong>{' '}
            Saved dark palette: {activePaletteMeta.label}.
          </p>
          <p className="m-0 text-xs leading-relaxed text-text-tertiary">
            Follow Windows uses the Windows app color preference while Theme is set to Follow
            Windows.
          </p>
        </section>
      )}

      {category === 'diagnostics' && (
        <section
          data-settings-category={category}
          data-slot="settings-category-content"
          className="grid content-start gap-3 py-4"
          aria-labelledby="diagnostic-settings-title"
        >
          <h3
            className="m-0 text-[15px] font-semibold tracking-tight text-foreground"
            id="diagnostic-settings-title"
          >
            Telemetry and diagnostics
          </h3>
          <div className="grid gap-2.5">
            <div className="grid gap-1 rounded-control border border-border bg-surface px-3.5 py-3">
              <label
                className="flex cursor-pointer items-center gap-2 text-[13px] text-text-secondary"
                htmlFor={telemetryId}
              >
                <input
                  className="size-[15px] flex-none cursor-pointer accent-primary"
                  id={telemetryId}
                  type="checkbox"
                  aria-describedby={telemetryDescId}
                  checked={settings.telemetry_enabled}
                  onChange={(event) => patch({ telemetryEnabled: event.target.checked })}
                />
                <span className="text-[13px] font-semibold text-foreground">
                  Allow anonymous telemetry
                </span>
              </label>
              <p
                id={telemetryDescId}
                className="m-0 pl-6 text-xs leading-relaxed text-text-secondary"
              >
                Share anonymized diagnostics to help improve playback stability and timing quality.
              </p>
            </div>
            <div className="grid gap-1 rounded-control border border-border bg-surface px-3.5 py-3">
              <label
                className="flex cursor-pointer items-center gap-2 text-[13px] text-text-secondary"
                htmlFor={verboseHudId}
              >
                <input
                  className="size-[15px] flex-none cursor-pointer accent-primary"
                  id={verboseHudId}
                  type="checkbox"
                  aria-describedby={verboseHudDescId}
                  checked={settings.verbose_hud}
                  onChange={(event) => patch({ verboseHud: event.target.checked })}
                />
                <span className="text-[13px] font-semibold text-foreground">
                  Verbose playback HUD
                </span>
              </label>
              <p
                id={verboseHudDescId}
                className="m-0 pl-6 text-xs leading-relaxed text-text-secondary"
              >
                Expose extended scheduling and native dispatch metrics in the diagnostics view.
              </p>
            </div>
          </div>
        </section>
      )}

      {category === 'updates' && (
        <section
          data-settings-category={category}
          data-slot="settings-category-content"
          className="grid content-start gap-3 py-4"
          aria-labelledby="update-settings-title"
        >
          <h3
            className="m-0 text-[15px] font-semibold tracking-tight text-foreground"
            id="update-settings-title"
          >
            Updates
          </h3>
          <div className="grid gap-1 rounded-control border border-border bg-surface px-3.5 py-3">
            <label
              className="flex cursor-pointer items-center gap-2 text-[13px] text-text-secondary"
              htmlFor={autoCheckId}
            >
              <input
                className="size-[15px] flex-none cursor-pointer accent-primary"
                id={autoCheckId}
                type="checkbox"
                aria-describedby={autoCheckDescId}
                checked={settings.update_preferences.auto_check}
                onChange={(event) =>
                  patch({ updatePreferences: { autoCheck: event.target.checked } })
                }
              />
              <span className="text-[13px] font-semibold text-foreground">
                Check for updates automatically
              </span>
            </label>
            <p
              id={autoCheckDescId}
              className="m-0 pl-6 text-xs leading-relaxed text-text-secondary"
            >
              Check for signed releases in the background when Sky Auto Player starts.
            </p>
          </div>
          <div className="grid w-full max-w-[360px] gap-1.5">
            <label className="text-[13px] font-medium text-text-secondary" htmlFor={channelId}>
              Channel
            </label>
            <Select
              value={settings.update_preferences.channel}
              onValueChange={(nextValue) => {
                if (nextValue === 'stable' || nextValue === 'beta') {
                  patch({ updatePreferences: { channel: nextValue } });
                }
              }}
            >
              <SelectTrigger id={channelId} aria-label="Channel">
                <SelectValue>
                  {(value: string | null) => (value === 'beta' ? 'Beta (unavailable)' : 'Stable')}
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="stable">Stable</SelectItem>
                <SelectItem value="beta" disabled>
                  Beta (unavailable)
                </SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="mt-0.5 flex flex-wrap items-center gap-3">
            <Button
              variant="outline"
              className="h-9 w-auto justify-self-start gap-2 whitespace-nowrap"
              type="button"
              disabled={checkingUpdates}
              onClick={() => {
                onUpdateDialogOpen?.();
                void checkForUpdate('manual');
              }}
            >
              <RefreshCw
                size={14}
                className={checkingUpdates ? 'animate-spin motion-reduce:animate-none' : undefined}
                aria-hidden="true"
              />
              {checkingUpdates ? 'Checking…' : 'Check for updates'}
            </Button>
            <span className="text-xs tabular-nums text-text-tertiary">
              Current version · {bootstrap.app_version}
            </span>
          </div>
        </section>
      )}

      {category === 'donate' && <DonateSettings />}

      {category === 'about' && (
        <section
          data-settings-category={category}
          data-slot="settings-category-content"
          className="grid content-start gap-3 py-4"
          aria-labelledby="about-settings-title"
        >
          <div className="flex items-center gap-3">
            <img
              className="size-9 flex-none rounded-control"
              src={appIcon40}
              alt=""
              width={36}
              height={36}
              aria-hidden="true"
            />
            <div className="grid min-w-0 gap-1">
              <h3
                className="m-0 text-[15px] font-semibold text-foreground"
                id="about-settings-title"
              >
                About Sky Auto Player
              </h3>
              <p className="m-0 max-w-[520px] text-[13px] leading-relaxed text-text-secondary">
                A focused Windows workbench for preparing and playing Sky music sheets.
              </p>
            </div>
          </div>
          <dl className="m-0 grid gap-0 rounded-pane border border-border bg-surface px-3.5 py-0.5">
            <div className="grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-2 last:border-b-0">
              <dt className="text-xs text-text-tertiary">Version</dt>
              <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                {bootstrap.app_version}
              </dd>
            </div>
            <div className="grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-2 last:border-b-0">
              <dt className="text-xs text-text-tertiary">Native version</dt>
              <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                {bootstrap.native_build.native_version}
              </dd>
            </div>
            <div className="grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-2 last:border-b-0">
              <dt className="text-xs text-text-tertiary">Build commit</dt>
              <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                {bootstrap.native_build.native_build_commit}
              </dd>
            </div>
            <div className="grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-2 last:border-b-0">
              <dt className="text-xs text-text-tertiary">Native ABI</dt>
              <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                {bootstrap.native_build.native_abi}
              </dd>
            </div>
            <div className="grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-3 border-b border-border py-2 last:border-b-0">
              <dt className="text-xs text-text-tertiary">Rust</dt>
              <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                {bootstrap.native_build.rustc_version}
              </dd>
            </div>
            <div className="grid grid-cols-[140px_minmax(0,1fr)] items-baseline gap-3 py-2">
              <dt className="text-xs text-text-tertiary">Win32 backend</dt>
              <dd className="m-0 min-w-0 break-words text-xs text-foreground">
                {bootstrap.native_build.win32_backend ? 'Enabled' : 'Disabled'}
              </dd>
            </div>
          </dl>
        </section>
      )}
    </>
  );
}
