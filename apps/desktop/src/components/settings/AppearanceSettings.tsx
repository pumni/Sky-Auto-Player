import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
import { Check, type LucideIcon, Monitor, Moon, Sun } from 'lucide-react';
import { useId } from 'react';
import { resolveAppearance } from '../../appearance/resolveAppearance';
import { useSystemAppearance } from '../../appearance/useSystemAppearance';
import type { AppearanceMode, PaletteId, SettingsPatch } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { DEFAULT_PALETTE, PALETTES, type PaletteMeta } from './paletteMetadata';
import { SettingsCategoryHeader } from './SettingsLayout';

interface ThemeOption {
  value: AppearanceMode;
  label: string;
  icon: LucideIcon;
}

const THEME_OPTIONS: readonly ThemeOption[] = [
  { value: 'system', label: 'System', icon: Monitor },
  { value: 'light', label: 'Light', icon: Sun },
  { value: 'dark', label: 'Dark', icon: Moon },
];

function PaletteSwatches({ palette }: { palette: PaletteMeta }) {
  const swatches = [
    { role: 'canvas', hex: palette.swatches[0] },
    { role: 'chrome', hex: palette.swatches[1] },
    { role: 'accent', hex: palette.swatches[3] },
  ] as const;
  return (
    <span className="inline-flex shrink-0 items-center gap-1" aria-hidden="true">
      {swatches.map(({ role, hex }) => (
        <span
          key={`${palette.id}-${role}`}
          className="size-3 rounded-full border border-border"
          style={{ backgroundColor: hex }}
        />
      ))}
    </span>
  );
}

interface AppearanceSettingsProps {
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
}

export function AppearanceSettings({ settings, useStore }: AppearanceSettingsProps) {
  const themeGroupLabelId = useId();
  const themeHelpId = useId();
  const paletteId = useId();
  const paletteHelpId = useId();
  const patchSettings = useStore((store) => store.patchSettings);
  const patch = (value: SettingsPatch) => void patchSettings(value);
  const { mode: systemColorMode } = useSystemAppearance();
  const effectiveAppearance = resolveAppearance(
    settings.appearance_mode,
    systemColorMode,
    settings.palette,
  );
  const activePaletteMeta =
    PALETTES.find((item) => item.id === settings.palette) ?? DEFAULT_PALETTE;
  const usesUnifiedShell =
    effectiveAppearance.colorMode === 'light' || effectiveAppearance.palette === 'graphite';
  const previewLabel =
    effectiveAppearance.colorMode === 'light'
      ? `Light appearance preview (saved dark palette: ${activePaletteMeta.label})`
      : `Dark appearance preview (${activePaletteMeta.label} palette)`;

  return (
    <section
      data-settings-category="appearance"
      data-slot="settings-category-content"
      className="grid content-start gap-4"
      aria-labelledby="appearance-settings-title"
    >
      <SettingsCategoryHeader
        id="appearance-settings-title"
        title="Appearance"
        description="Choose how Sky Auto Player looks."
      />

      <div className="grid min-w-0 gap-2 border-t border-border pt-3">
        <h4 id={themeGroupLabelId} className="m-0 text-sm font-semibold leading-5 text-foreground">
          Theme
        </h4>
        <RadioGroup
          aria-labelledby={themeGroupLabelId}
          aria-describedby={themeHelpId}
          value={settings.appearance_mode}
          onValueChange={(value) => {
            if (value === 'system' || value === 'light' || value === 'dark') {
              patch({ appearanceMode: value });
            }
          }}
          className="grid w-full max-w-[440px] min-w-0 grid-cols-1 gap-2 @min-[360px]/settings-content:grid-cols-3"
        >
          {THEME_OPTIONS.map((option) => {
            const Icon = option.icon;
            return (
              <Radio.Root
                key={option.value}
                value={option.value}
                data-slot="theme-radio-option"
                data-value={option.value}
                className="group relative flex min-h-10 min-w-0 cursor-pointer items-center justify-center gap-2 rounded-control border border-input bg-card px-2.5 text-[13px] font-medium leading-5 text-foreground outline-none transition-colors duration-fast hover:bg-control-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-surface data-[checked]:border-accent-ink data-[checked]:bg-selection motion-reduce:transition-none forced-colors:data-[checked]:border-[Highlight]"
              >
                <Icon
                  size={16}
                  className="shrink-0 text-text-secondary group-data-[checked]:text-accent-ink"
                  aria-hidden="true"
                />
                <span className="whitespace-nowrap">{option.label}</span>
                <span className="grid size-4 shrink-0 place-items-center" aria-hidden="true">
                  <Radio.Indicator className="grid place-items-center text-accent-ink">
                    <Check size={14} strokeWidth={2.25} />
                  </Radio.Indicator>
                </span>
              </Radio.Root>
            );
          })}
        </RadioGroup>
        <p id={themeHelpId} className="m-0 text-xs leading-[18px] text-text-secondary">
          System follows Windows appearance.
        </p>
      </div>

      <div className="grid min-w-0 gap-2 border-t border-border pt-3">
        <div className="grid w-full max-w-[440px] min-w-0 gap-1.5">
          <label
            className="cursor-pointer text-sm font-semibold leading-5 text-foreground"
            htmlFor={paletteId}
          >
            Dark palette
          </label>
          <Select
            value={settings.palette}
            onValueChange={(nextValue) => {
              if (nextValue !== null) patch({ palette: nextValue as PaletteId });
            }}
          >
            <SelectTrigger
              id={paletteId}
              className="w-full min-w-0"
              aria-label="Dark palette"
              aria-describedby={paletteHelpId}
            >
              <SelectValue>
                {(value: string | null) => {
                  const matched = PALETTES.find((item) => item.id === value);
                  if (!matched) return 'Dark palette';
                  return (
                    <span className="inline-flex min-w-0 items-center gap-2">
                      <PaletteSwatches palette={matched} />
                      <span className="truncate">{matched.label}</span>
                    </span>
                  );
                }}
              </SelectValue>
            </SelectTrigger>
            <SelectContent>
              {PALETTES.map((palette) => (
                <SelectItem key={palette.id} value={palette.id}>
                  <span className="inline-flex items-center gap-2">
                    <PaletteSwatches palette={palette} />
                    <span>{palette.label}</span>
                  </span>
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <p id={paletteHelpId} className="m-0 text-xs leading-[18px] text-text-secondary">
            Your dark palette is saved for when Dark appearance is active.
          </p>
        </div>
      </div>

      <div className="grid min-w-0 gap-2 border-t border-border pt-3">
        <h4 className="m-0 text-sm font-semibold leading-5 text-foreground">Preview</h4>
        <div
          data-slot="appearance-preview"
          data-effective-mode={effectiveAppearance.colorMode}
          role="img"
          aria-label={previewLabel}
          className={`pointer-events-none w-full max-w-[440px] min-w-0 select-none overflow-hidden rounded-pane border border-border text-foreground ${usesUnifiedShell ? 'bg-card' : 'bg-background'}`}
        >
          {/* Mini title bar */}
          <div
            data-slot="appearance-preview-titlebar"
            className={`flex h-6 items-center gap-1.5 px-2.5 ${usesUnifiedShell ? 'bg-transparent' : 'border-b border-border bg-card'}`}
          >
            <span className="size-2 rounded-full bg-primary" />
            <span className="h-1.5 w-14 rounded-full bg-text-secondary opacity-70" />
            <span className="ml-auto h-1.5 w-8 rounded-full bg-text-tertiary opacity-60" />
          </div>

          {/* Mini workbench panes */}
          <div className="grid min-w-0 grid-cols-[minmax(28px,44px)_minmax(0,1fr)_minmax(44px,76px)] gap-1.5 px-1.5 pb-1.5">
            {/* Mini navigator */}
            <div className="grid min-w-0 content-start gap-1 rounded-control bg-card p-1.5">
              <span className="h-2 w-full rounded-sm bg-selection" />
              <span className="h-1.5 w-4/5 rounded-sm bg-text-tertiary opacity-50" />
              <span className="h-1.5 w-3/5 rounded-sm bg-text-tertiary opacity-40" />
            </div>

            {/* Mini track browser canvas */}
            <div
              data-slot="appearance-preview-canvas"
              className="grid min-w-0 content-start gap-1 rounded-control border border-border bg-surface p-1.5"
            >
              <div className="flex min-w-0 items-center gap-1.5 rounded-sm bg-selection px-1.5 py-1">
                <span className="h-2.5 w-0.5 shrink-0 rounded-full bg-accent-ink" />
                <span className="h-1.5 w-16 max-w-full rounded-full bg-foreground opacity-85" />
              </div>
              <div className="flex min-w-0 items-center gap-1.5 px-1.5 py-0.5">
                <span className="h-1.5 w-12 max-w-full rounded-full bg-text-secondary opacity-55" />
              </div>
              <div className="flex min-w-0 items-center gap-1.5 px-1.5 py-0.5">
                <span className="h-1.5 w-14 max-w-full rounded-full bg-text-tertiary opacity-45" />
              </div>
            </div>

            {/* Mini inspector */}
            <div className="grid min-w-0 content-start gap-1.5 rounded-control border border-border bg-card p-1.5">
              <span className="h-1.5 w-10 max-w-full rounded-full bg-text-secondary opacity-70" />
              <span className="h-1.5 w-full rounded-full bg-text-tertiary opacity-45" />
              <span className="h-1.5 w-3/4 rounded-full bg-text-tertiary opacity-45" />
            </div>
          </div>

          {/* Mini player dock */}
          <div className="flex h-8 items-center justify-between gap-2 border-t border-border bg-card px-2.5">
            <span className="h-1.5 w-14 rounded-full bg-text-secondary opacity-70" />
            <span
              data-slot="appearance-preview-primary-action"
              className="inline-flex h-5 items-center rounded-control bg-primary px-2 text-[10px] font-medium text-primary-foreground"
            >
              Play
            </span>
            <span className="h-1.5 w-10 rounded-full bg-text-tertiary opacity-60" />
          </div>
        </div>

        <p
          data-slot="appearance-active-summary"
          className="m-0 text-xs leading-[18px] text-foreground"
        >
          <strong>Active: {effectiveAppearance.colorMode === 'light' ? 'Light' : 'Dark'}.</strong>{' '}
          Saved dark palette: {activePaletteMeta.label}.
        </p>
      </div>
    </section>
  );
}
