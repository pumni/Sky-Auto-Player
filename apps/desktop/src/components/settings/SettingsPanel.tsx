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
import { useRef, useState } from 'react';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import type { DesktopStoreHook, DesktopStore as StoreState } from '../../state/store';
import { Button } from '../ui/button';
import { Dialog, DialogContent, DialogTitle } from '../ui/dialog';
import {
  ScrollAreaContent,
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
  const settingsState = useStore((store: StoreState) => store.settingsState);
  const settingsError = useStore((store: StoreState) => store.settingsError);
  const open = useStore((store: StoreState) => store.settingsOpen);
  const setOpen = useStore((store: StoreState) => store.setSettingsOpen);
  const [category, setCategory] = useState<SettingsCategory>('playback');
  const contentViewportRef = useRef<HTMLDivElement>(null);

  if (!settings) return null;

  const hasStatusNotice = settingsError !== null || settingsState === 'loading';

  const selectCategory = (nextCategory: SettingsCategory) => {
    setCategory(nextCategory);
    if (contentViewportRef.current) {
      contentViewportRef.current.scrollTop = 0;
    }
  };

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        className="flex h-[min(640px,88dvh,calc(100dvh-32px))] w-[min(820px,calc(100vw-32px))] min-h-0 flex-col gap-2 overflow-hidden bg-card p-2 shadow-2xl"
        aria-labelledby="settings-title"
        showCloseButton={false}
      >
        <div
          data-slot="settings-header"
          className="flex flex-none items-center justify-between gap-4 px-3 pt-1"
        >
          <DialogTitle className="m-0 text-xl font-semibold leading-7" id="settings-title">
            Settings
          </DialogTitle>
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

        <div className="grid min-h-0 flex-1 grid-cols-[164px_minmax(0,1fr)] gap-2">
          <ScrollAreaRoot className="min-h-0 min-w-0">
            <ScrollAreaViewport
              className="size-full"
              role="navigation"
              aria-label="Settings categories"
            >
              <ScrollAreaContent className="grid min-w-0 content-start gap-1 px-1 py-0.5">
                {SETTINGS_CATEGORIES.map((item) => {
                  const Icon = CATEGORY_ICONS[item.id];
                  const isGroupStart = item.id === 'donate';
                  return (
                    <Button
                      variant="ghost"
                      size="sm"
                      key={item.id}
                      className={`group/nav-item relative flex min-h-[34px] justify-start gap-2 rounded-control px-2.5 text-left text-[13px] font-medium text-text-secondary transition-colors duration-fast hover:bg-control-hover hover:text-foreground after:absolute after:inset-y-[7px] after:left-0 after:w-[3px] after:rounded-full after:bg-accent-ink after:opacity-0 data-[active=true]:bg-selection data-[active=true]:text-foreground data-[active=true]:after:opacity-100 motion-reduce:transition-none ${isGroupStart ? "mt-2.5 before:absolute before:-top-1.5 before:left-2 before:right-2 before:h-px before:bg-border before:content-['']" : ''}`}
                      type="button"
                      aria-current={category === item.id ? 'page' : undefined}
                      data-active={category === item.id}
                      data-slot="settings-nav-item"
                      onClick={() => selectCategory(item.id)}
                    >
                      <Icon
                        size={16}
                        className="shrink-0 opacity-80 group-data-[active=true]/nav-item:text-accent-ink group-data-[active=true]/nav-item:opacity-100"
                        aria-hidden="true"
                      />
                      <span>{item.label}</span>
                    </Button>
                  );
                })}
              </ScrollAreaContent>
            </ScrollAreaViewport>
            <ScrollAreaScrollbar orientation="vertical">
              <ScrollAreaThumb />
            </ScrollAreaScrollbar>
          </ScrollAreaRoot>

          <div
            data-slot="settings-content-scroll-region"
            className="min-h-0 min-w-0 overflow-hidden rounded-pane border border-border bg-surface"
          >
            <ScrollAreaRoot className="size-full">
              <ScrollAreaViewport
                ref={contentViewportRef}
                data-settings-content
                className="size-full @container/settings-content"
              >
                <ScrollAreaContent className="min-w-0 px-3 py-4 @min-[420px]/settings-content:px-4 @min-[520px]/settings-content:p-5">
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

        {hasStatusNotice && (
          <div
            data-slot="settings-status-region"
            className="flex flex-none items-center justify-between gap-3 px-3 pb-1"
          >
            {settingsError !== null ? (
              <p className="m-0 text-xs text-danger" role="alert">
                Could not save settings. {settingsError}
              </p>
            ) : (
              <p className="m-0 text-xs text-text-tertiary" role="status" aria-live="polite">
                Saving…
              </p>
            )}
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
