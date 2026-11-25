import {
  Activity,
  Heart,
  Info,
  type LucideIcon,
  Palette,
  RefreshCw,
  SlidersHorizontal,
  X,
} from 'lucide-react';
import { useState } from 'react';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook, DesktopStore as StoreState } from '../../state/store';
import { Button } from '../ui/button';
import { Dialog, DialogContent, DialogTitle } from '../ui/dialog';
import {
  ScrollAreaContent,
  ScrollAreaCorner,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';
import type { SettingsCategory } from './SettingsCategoryContent';
import { SETTINGS_CATEGORIES, SettingsCategoryContent } from './SettingsCategoryContent';

const CATEGORY_ICONS: Record<SettingsCategory, LucideIcon> = {
  playback: SlidersHorizontal,
  appearance: Palette,
  diagnostics: Activity,
  updates: RefreshCw,
  donate: Heart,
  about: Info,
};

interface SettingsPanelProps {
  bootstrap: Bootstrap;
  useStore: DesktopStoreHook;
  onUpdateDialogOpen?: () => void;
}

export function SettingsPanel({ bootstrap, useStore, onUpdateDialogOpen }: SettingsPanelProps) {
  const settings = useStore((store: StoreState) => store.settings);
  const settingsError = useStore((store: StoreState) => store.settingsError);
  const open = useStore((store: StoreState) => store.settingsOpen);
  const setOpen = useStore((store: StoreState) => store.setSettingsOpen);
  const [category, setCategory] = useState<SettingsCategory>('playback');

  if (!settings) return null;

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        className="flex h-[min(640px,88dvh)] w-[min(820px,calc(100vw-32px))] min-h-0 flex-col gap-0 overflow-hidden rounded-dialog border-border bg-card px-5 pb-3.5 pt-[18px] shadow-2xl"
        aria-labelledby="settings-title"
        showCloseButton={false}
      >
        <div className="mb-3 flex flex-none items-start justify-between gap-4">
          <div className="grid gap-px">
            <p className="m-0 text-[11px] font-semibold tracking-wide text-text-tertiary">
              PREFERENCES
            </p>
            <DialogTitle className="m-0 text-2xl font-semibold leading-8" id="settings-title">
              Settings
            </DialogTitle>
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            className="size-8 border-0 bg-transparent p-0 hover:bg-control-hover"
            type="button"
            onClick={() => setOpen(false)}
            aria-label="Close settings"
            title="Close settings"
          >
            <X size={16} aria-hidden="true" />
          </Button>
        </div>

        <div className="grid min-h-0 flex-1 grid-cols-[164px_minmax(0,1fr)] border-y border-border max-[600px]:grid-cols-1">
          <ScrollAreaRoot className="min-h-0 min-w-0 border-r border-border max-[600px]:border-b max-[600px]:border-r-0">
            <ScrollAreaViewport
              className="size-full"
              role="navigation"
              aria-label="Settings categories"
            >
              <ScrollAreaContent className="grid min-w-0 content-start gap-0.5 py-3 pr-3 max-[600px]:flex max-[600px]:w-max max-[600px]:items-center max-[600px]:gap-1 max-[600px]:py-2 max-[600px]:pr-0">
                {SETTINGS_CATEGORIES.map((item) => {
                  const Icon = CATEGORY_ICONS[item.id];
                  const isGroupStart = item.id === 'donate';
                  return (
                    <Button
                      variant="ghost"
                      size="sm"
                      key={item.id}
                      className={`relative flex min-h-[34px] justify-start gap-2 px-3 text-left text-[13px] font-medium text-text-secondary hover:bg-control-hover hover:text-foreground after:absolute after:inset-y-[7px] after:left-0.5 after:w-[3px] after:rounded-full after:bg-accent-ink after:opacity-0 data-[active=true]:bg-selection data-[active=true]:text-accent-ink data-[active=true]:after:opacity-100 motion-reduce:transition-none max-[600px]:flex-none ${isGroupStart ? "mt-2.5 before:absolute before:-top-1.5 before:left-2 before:right-2 before:h-px before:bg-border before:content-[''] max-[600px]:mt-0" : ''}`}
                      type="button"
                      aria-current={category === item.id ? 'page' : undefined}
                      data-active={category === item.id}
                      data-slot="settings-nav-item"
                      onClick={() => setCategory(item.id)}
                    >
                      <Icon size={15} className="shrink-0 opacity-80" aria-hidden="true" />
                      <span>{item.label}</span>
                    </Button>
                  );
                })}
              </ScrollAreaContent>
            </ScrollAreaViewport>
            <ScrollAreaScrollbar orientation="vertical">
              <ScrollAreaThumb />
            </ScrollAreaScrollbar>
            <ScrollAreaScrollbar orientation="horizontal">
              <ScrollAreaThumb />
            </ScrollAreaScrollbar>
            <ScrollAreaCorner />
          </ScrollAreaRoot>

          <div data-slot="settings-content-scroll-region" className="min-h-0 min-w-0">
            <ScrollAreaRoot className="size-full">
              <ScrollAreaViewport data-settings-content className="size-full">
                <ScrollAreaContent className="min-w-0 pl-5 pr-6 max-[600px]:pl-3">
                  <SettingsCategoryContent
                    category={category}
                    bootstrap={bootstrap}
                    settings={settings}
                    useStore={useStore}
                    {...(onUpdateDialogOpen ? { onUpdateDialogOpen } : {})}
                  />
                </ScrollAreaContent>
              </ScrollAreaViewport>
              <ScrollAreaScrollbar orientation="vertical">
                <ScrollAreaThumb />
              </ScrollAreaScrollbar>
            </ScrollAreaRoot>
          </div>
        </div>

        <div className="flex flex-none items-center justify-between gap-3 pt-2">
          {settingsError !== null && (
            <p className="m-0 text-xs text-danger" role="alert">
              Could not save settings. {settingsError}
            </p>
          )}
          {category !== 'donate' ? (
            <p className="m-0 text-xs leading-relaxed text-text-tertiary">
              Settings are validated and persisted by the native runtime.
            </p>
          ) : (
            <span aria-hidden="true" />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
