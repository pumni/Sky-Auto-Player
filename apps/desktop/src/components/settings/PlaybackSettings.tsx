import { useId } from 'react';
import type { Bootstrap, SettingsPatch } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import {
  computePlaybackHoldTiming,
  formatFpsFrameHint,
  formatHoldFramesHint,
  formatTempoHint,
} from '../ui/playbackOptionFormatting';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { AutoPlaySwitch } from './AutoPlaySwitch';
import { SettingsCategoryHeader, SettingsSection } from './SettingsLayout';
import { TimingMarginControl } from './TimingMarginControl';

interface PlaybackSettingsProps {
  bootstrap: Bootstrap;
  settings: NonNullable<DesktopStore['settings']>;
  useStore: DesktopStoreHook;
}

export function PlaybackSettings({ bootstrap, settings, useStore }: PlaybackSettingsProps) {
  const baseHoldId = useId();
  const tempoId = useId();
  const fpsId = useId();
  const patchSettings = useStore((store) => store.patchSettings);
  const patch = (value: SettingsPatch) => void patchSettings(value);
  const defaults = settings.playback_defaults;
  const { frameUs, frameBaseHoldUs, minHoldUs } = computePlaybackHoldTiming(
    defaults.fps,
    defaults.hold_frames,
    defaults.timing_margin_us,
  );

  return (
    <section
      data-settings-category="playback"
      data-slot="settings-category-content"
      className="grid content-start gap-5"
      aria-labelledby="playback-settings-title"
    >
      <SettingsCategoryHeader
        id="playback-settings-title"
        title="Playback"
        description="Configure playback behavior, defaults, and key-hold timing."
      />

      <SettingsSection title="Behavior" divider>
        <AutoPlaySwitch checked={settings.auto_play} onChange={(autoPlay) => patch({ autoPlay })} />
      </SettingsSection>

      <SettingsSection title="Playback defaults" divider>
        <div className="grid min-w-0 grid-cols-1 gap-3 @min-[420px]/settings-content:grid-cols-2 @min-[560px]/settings-content:grid-cols-3">
          <div className="grid min-w-0 content-start gap-1.5">
            <label
              className="cursor-pointer text-[13px] font-medium leading-5 text-foreground"
              htmlFor={baseHoldId}
            >
              Base hold
            </label>
            <Select
              value={String(defaults.hold_frames)}
              onValueChange={(nextValue) => {
                if (nextValue !== null)
                  patch({ playbackDefaults: { holdFrames: Number(nextValue) } });
              }}
            >
              <SelectTrigger id={baseHoldId} aria-label="Base hold">
                <SelectValue>
                  {(value: string | null) =>
                    value === null
                      ? 'Base hold'
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
              className="cursor-pointer text-[13px] font-medium leading-5 text-foreground"
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
              className="cursor-pointer text-[13px] font-medium leading-5 text-foreground"
              htmlFor={fpsId}
            >
              Target FPS
            </label>
            <Select
              value={String(defaults.fps)}
              onValueChange={(nextValue) => {
                if (nextValue !== null) patch({ playbackDefaults: { fps: Number(nextValue) } });
              }}
            >
              <SelectTrigger id={fpsId} aria-label="Target FPS">
                <SelectValue>
                  {(value: string | null) => (value === null ? 'Target FPS' : `${value} FPS`)}
                </SelectValue>
              </SelectTrigger>
              <SelectContent className="min-w-44">
                {bootstrap.option_sets.fps.map((value) => (
                  <SelectItem key={value} value={String(value)}>
                    <span>{value} FPS</span>
                    <span className="text-text-tertiary">{formatFpsFrameHint(value)}</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        </div>
      </SettingsSection>

      <SettingsSection title="Timing" divider>
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

        <dl
          data-slot="playback-timing-readout"
          className="m-0 grid gap-3 rounded-pane border border-border bg-card px-3.5 py-3 tabular-nums"
        >
          <div className="grid gap-0.5">
            <dt className="text-xs leading-[18px] text-text-tertiary">Target hold</dt>
            <dd className="m-0 text-lg font-semibold leading-6 text-foreground">
              {(minHoldUs / 1_000).toFixed(3)} ms
            </dd>
          </div>
          <div className="grid grid-cols-1 gap-x-5 gap-y-2 border-t border-border pt-2.5 @min-[320px]/settings-content:grid-cols-2">
            <div className="grid gap-0.5">
              <dt className="text-xs leading-[18px] text-text-tertiary">1 frame</dt>
              <dd className="m-0 text-[13px] font-medium leading-5 text-foreground">
                {(frameUs / 1_000).toFixed(3)} ms
              </dd>
            </div>
            <div className="grid gap-0.5">
              <dt className="text-xs leading-[18px] text-text-tertiary">Base hold</dt>
              <dd className="m-0 text-[13px] font-medium leading-5 text-foreground">
                {(frameBaseHoldUs / 1_000).toFixed(3)} ms
              </dd>
            </div>
          </div>
        </dl>

        <p className="m-0 text-xs leading-[18px] text-text-tertiary">
          Changes apply to the next prepared session; an active session keeps its frozen timing
          values.
        </p>
      </SettingsSection>
    </section>
  );
}
