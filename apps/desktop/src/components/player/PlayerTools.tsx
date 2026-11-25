import { ChevronRight, FlaskConical, ListVideo, PanelRight, SlidersHorizontal } from 'lucide-react';
import { type RefObject, useId, useRef, useState } from 'react';
import {
  type DesktopStore,
  type DesktopStoreHook,
  playbackSessionStatusConcern,
} from '../../state/store';
import { TimingMarginControl } from '../settings/TimingMarginControl';
import { Button } from '../ui/button';
import {
  ScrollAreaContent,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';
import {
  formatFpsFrameHint,
  formatHoldFramesHint,
  formatTempoHint,
} from '../ui/playbackOptionFormatting';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/tooltip';

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
  const [profileAdvancedOpen, setProfileAdvancedOpen] = useState(false);
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
    <div
      data-slot="player-tools"
      className="col-start-3 row-start-1 flex min-w-0 flex-nowrap items-center justify-self-end gap-2"
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-8 text-text-tertiary aria-pressed:text-accent-ink hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring"
              aria-label="Auto Play"
              aria-pressed={autoPlayEnabled}
              disabled={!settings}
              aria-busy={autoPlayPending}
              onClick={() => void toggleAutoPlay()}
            />
          }
        >
          <ListVideo size={18} aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent>
          Auto Play {autoPlayEnabled ? 'on' : 'off'} · Continue to the next song
        </TooltipContent>
      </Tooltip>
      <div className="relative w-8 min-w-0">
        <Tooltip disabled={profileOpen}>
          <Popover open={profileOpen} onOpenChange={setProfileOpen}>
            <TooltipTrigger
              render={
                <PopoverTrigger
                  render={
                    <Button
                      ref={profileTriggerRef}
                      variant="ghost"
                      size="icon-sm"
                      className="relative size-8 text-text-tertiary aria-expanded:text-accent-ink after:absolute after:bottom-0.5 after:left-1/2 after:size-1 after:-translate-x-1/2 after:rounded-full after:bg-accent-ink after:opacity-0 aria-expanded:after:opacity-100"
                      type="button"
                      aria-label="Configure playback profile"
                      aria-expanded={profileOpen}
                    />
                  }
                />
              }
            >
              <SlidersHorizontal size={18} aria-hidden="true" />
            </TooltipTrigger>
            <TooltipContent>
              {profileSummary ? `Playback profile · ${profileSummary}` : 'Playback profile'}
            </TooltipContent>
            <PopoverContent
              side="top"
              align="end"
              sideOffset={8}
              className="flex h-max max-h-(--available-height) max-[700px]:max-h-[min(360px,var(--available-height))] w-[min(360px,calc(100vw-24px))] min-w-[min(320px,calc(100vw-24px))] max-w-[calc(100vw-24px)] flex-col overflow-hidden p-0 shadow-2xl focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
              role="dialog"
              aria-label="Playback profile"
            >
              <ScrollAreaRoot className="min-h-0 flex-1">
                <ScrollAreaViewport className="rounded-popover">
                  <ScrollAreaContent className="grid min-w-0 gap-2.5 p-4">
                    <header className="flex min-w-0 items-baseline justify-between gap-2">
                      <h3 className="m-0 shrink-0 text-sm font-semibold leading-5 text-foreground">
                        Playback profile
                      </h3>
                      <p className="m-0 min-w-0 text-right text-xs leading-4 text-text-tertiary">
                        Fine-tune how songs play.
                      </p>
                    </header>
                    <div
                      data-slot="profile-fields"
                      className="grid min-w-0 grid-cols-2 gap-x-2.5 gap-y-2"
                    >
                      <ProfileFields
                        defaults={defaults}
                        bootstrap={bootstrap}
                        patchSettings={patchSettings}
                        advancedTimingOpen={profileAdvancedOpen}
                        onAdvancedTimingOpenChange={setProfileAdvancedOpen}
                      />
                    </div>
                    <div className="grid min-w-0 gap-2 border-t border-border pt-2.5">
                      <div className="grid min-w-0 gap-1">
                        <div className="flex min-w-0 items-baseline gap-2">
                          <strong className="shrink-0 text-[13px] font-semibold text-foreground">
                            Safe preview
                          </strong>
                          {selectedSongTitle && (
                            <span
                              className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-xs text-text-secondary"
                              title={selectedSongTitle}
                            >
                              {selectedSongTitle}
                            </span>
                          )}
                        </div>
                        <p className="m-0 text-xs leading-relaxed text-text-secondary">
                          {canPreparePlayback ? 'No keyboard input is sent.' : previewDescription}
                        </p>
                      </div>
                      <Button
                        variant="default"
                        size="default"
                        className="min-h-9 w-full text-xs disabled:border disabled:border-border disabled:bg-card disabled:text-muted-foreground"
                        type="button"
                        disabled={!canPreparePlayback}
                        onClick={() => {
                          setProfileOpen(false);
                          void prepareAndMaybeStart();
                        }}
                      >
                        <FlaskConical size={15} aria-hidden="true" />
                        Test playback (no input)
                      </Button>
                    </div>
                  </ScrollAreaContent>
                </ScrollAreaViewport>
                <ScrollAreaScrollbar orientation="vertical">
                  <ScrollAreaThumb />
                </ScrollAreaScrollbar>
              </ScrollAreaRoot>
            </PopoverContent>
          </Popover>
        </Tooltip>
      </div>
      <Tooltip>
        <TooltipTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              className="size-8 flex-none text-text-secondary aria-expanded:text-accent-ink"
              aria-label={utility.open ? 'Close utility panel' : 'Open utility panel'}
              aria-expanded={utility.open}
              {...(utility.open ? { 'aria-controls': 'utility-inspector' } : {})}
              ref={utilityTriggerRef}
              onClick={() => toggleUtility()}
            />
          }
        >
          <PanelRight size={16} aria-hidden="true" />
        </TooltipTrigger>
        <TooltipContent>
          {utility.open ? 'Close Inspector · Ctrl+I' : 'Open Inspector · Ctrl+I'}
        </TooltipContent>
      </Tooltip>
    </div>
  );
}

interface ProfileFieldsProps {
  defaults: NonNullable<DesktopStore['settings']>['playback_defaults'] | undefined;
  bootstrap: DesktopStore['bootstrap'];
  patchSettings: DesktopStore['patchSettings'];
  advancedTimingOpen: boolean;
  onAdvancedTimingOpenChange: (open: boolean) => void;
}

function ProfileFields({
  defaults,
  bootstrap,
  patchSettings,
  advancedTimingOpen,
  onAdvancedTimingOpenChange,
}: ProfileFieldsProps) {
  const holdId = useId();
  const tempoId = useId();
  const fpsId = useId();

  if (!defaults || !bootstrap) {
    return <span className="text-muted-foreground">Profile unavailable</span>;
  }
  const frameUs = Math.ceil(1_000_000 / defaults.fps);
  const frameBaseHoldUs = Math.ceil(defaults.hold_frames * frameUs);
  const minHoldUs = frameBaseHoldUs + defaults.timing_margin_us;

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
      <div className="grid min-w-0 gap-0.5">
        <label className="flex items-center gap-1 text-xs text-text-tertiary" htmlFor={holdId}>
          Base hold
        </label>
        <Select
          value={String(defaults.hold_frames)}
          onValueChange={(value) => {
            if (value !== null)
              void patchSettings({ playbackDefaults: { holdFrames: Number(value) } });
          }}
        >
          <SelectTrigger id={holdId} className="h-8" aria-label="Base hold">
            <SelectValue>
              {(value: string | null) => (value === null ? 'Base hold' : `${value}f`)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent className="min-w-40">
            {holdOptions.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                <span>{option.label}</span>
                <span className="text-text-tertiary">{option.hint}</span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="text-xs leading-4 text-text-tertiary">
          {formatHoldFramesHint(defaults.hold_frames, defaults.fps)}
        </span>
      </div>
      <div className="grid min-w-0 gap-0.5">
        <label className="flex items-center gap-1 text-xs text-text-tertiary" htmlFor={tempoId}>
          Tempo
        </label>
        <Select
          value={String(defaults.tempo_scale)}
          onValueChange={(value) => {
            if (value !== null)
              void patchSettings({ playbackDefaults: { tempoScale: Number(value) } });
          }}
        >
          <SelectTrigger id={tempoId} className="h-8" aria-label="Tempo">
            <SelectValue>
              {(value: string | null) => (value === null ? 'Tempo' : `${value}×`)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent className="min-w-36">
            {tempoOptions.map((option) => (
              <SelectItem key={option.value} value={option.value}>
                <span>{option.label}</span>
                {option.hint && <span className="text-text-tertiary">{option.hint}</span>}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="text-xs leading-4 text-text-tertiary">
          {formatTempoHint(defaults.tempo_scale) ?? '\u00a0'}
        </span>
      </div>
      <details
        className="group col-span-2 min-w-0 border-t border-border pt-2"
        open={advancedTimingOpen}
        onToggle={(event) => onAdvancedTimingOpenChange(event.currentTarget.open)}
      >
        <summary className="flex cursor-pointer list-none items-center gap-1.5 py-0.5 text-[13px] font-medium text-text-secondary hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
          <ChevronRight
            className="size-3.5 transition-[rotate] duration-fast group-open:rotate-90 motion-reduce:transition-none"
            size={14}
            aria-hidden="true"
          />
          <span>Advanced timing</span>
        </summary>
        <div className="mt-1.5 grid min-w-0 grid-cols-2 gap-x-2.5 gap-y-2">
          <div className="grid min-w-0 gap-0.5">
            <label className="flex items-center gap-1 text-xs text-text-tertiary" htmlFor={fpsId}>
              FPS
            </label>
            <Select
              value={String(defaults.fps)}
              onValueChange={(value) => {
                if (value !== null)
                  void patchSettings({ playbackDefaults: { fps: Number(value) } });
              }}
            >
              <SelectTrigger id={fpsId} className="h-8" aria-label="FPS">
                <SelectValue>{(value: string | null) => value ?? 'FPS'}</SelectValue>
              </SelectTrigger>
              <SelectContent className="min-w-40">
                {fpsOptions.map((option) => (
                  <SelectItem key={option.value} value={option.value}>
                    <span>{option.label}</span>
                    <span className="text-text-tertiary">{option.hint}</span>
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <TimingMarginControl
            value={defaults.timing_margin_us}
            options={bootstrap.option_sets}
            density="compact"
            onChange={(value) =>
              patchSettings({ playbackDefaults: { timingMarginUs: value } }).then(
                (authoritative) => authoritative?.playback_defaults.timing_margin_us ?? null,
              )
            }
          />
          <dl className="col-span-2 m-0 grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-0.5 border-t border-border pt-1.5 text-xs">
            <dt className="text-text-tertiary">Frame duration</dt>
            <dd className="m-0 font-mono tabular-nums text-foreground">
              {(frameUs / 1_000).toFixed(3)} ms
            </dd>
            <dt className="text-text-tertiary">Base hold duration</dt>
            <dd className="m-0 font-mono tabular-nums text-foreground">
              {(frameBaseHoldUs / 1_000).toFixed(3)} ms
            </dd>
            <dt className="text-text-tertiary">Effective target hold</dt>
            <dd className="m-0 font-mono tabular-nums text-foreground">
              {(minHoldUs / 1_000).toFixed(3)} ms
            </dd>
          </dl>
        </div>
      </details>
    </>
  );
}
