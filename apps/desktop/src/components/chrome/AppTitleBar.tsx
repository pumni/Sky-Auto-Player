import { Download, RefreshCw, Settings } from 'lucide-react';
import { useMemo, type RefObject } from 'react';
import brandMarkDarkUrl from '../../assets/brand/mark-no-bg.svg';
import brandMarkLightUrl from '../../assets/brand/mark-mono-dark.svg';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { createWindowControls, type WindowControls } from '../../platform/windowControls';
import { GlobalSearch } from './GlobalSearch';
import { WindowCaptionControls } from './WindowCaptionControls';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/tooltip';

interface AppTitleBarProps {
  bootstrap: Bootstrap;
  searchInputRef?: RefObject<HTMLInputElement | null>;
  useStore: DesktopStoreHook;
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  windowControls?: WindowControls;
  onSettingsOpen?: () => void;
  onUpdateOpen?: () => void;
}

export function AppTitleBar({
  searchInputRef,
  useStore,
  settingsTriggerRef,
  windowControls,
  onSettingsOpen,
  onUpdateOpen,
}: AppTitleBarProps) {
  const controls = useMemo(() => windowControls ?? createWindowControls(), [windowControls]);
  const reload = useStore((store: DesktopStore) => store.reloadLibrary);
  const setSettingsOpen = useStore((store: DesktopStore) => store.setSettingsOpen);
  const update = useStore((store: DesktopStore) => store.update);
  const setUpdateDialogOpen = useStore((store: DesktopStore) => store.setUpdateDialogOpen);
  const updateVersion = update.availableVersion ?? 'new version';

  return (
    <header
      className="relative z-10 grid h-10 min-w-0 grid-cols-[minmax(0,1fr)_minmax(0,min(480px,42vw))_minmax(0,1fr)] items-center bg-background max-[1023px]:grid-cols-[minmax(0,1fr)_minmax(260px,min(480px,42vw))_auto]"
      aria-label="Sky Auto Player"
      data-tauri-drag-region="deep"
      data-slot="titlebar"
      data-update-available={update.state === 'available' ? 'true' : 'false'}
    >
      <div
        className="flex h-full min-w-min items-center"
        data-tauri-drag-region="deep"
        data-slot="titlebar-leading"
      >
        <div className="flex h-full flex-none items-center pl-3" data-tauri-drag-region="deep">
          <div data-slot="titlebar-brand" className="grid size-8 flex-none place-items-center">
            <img
              className="hidden size-7 object-contain dark:block"
              src={brandMarkDarkUrl}
              alt=""
              width="28"
              height="28"
              draggable={false}
            />
            <img
              className="size-7 object-contain dark:hidden"
              src={brandMarkLightUrl}
              alt=""
              width="28"
              height="28"
              draggable={false}
            />
            <span className="sr-only">Sky Auto Player</span>
          </div>
          <span
            className="ml-2 whitespace-nowrap text-[13px] font-semibold text-foreground max-[1100px]:hidden"
            data-slot="titlebar-product-name"
            data-tauri-drag-region="deep"
          >
            Sky Auto Player
          </span>
        </div>
        <div
          className="h-full min-w-6 flex-1"
          aria-hidden="true"
          data-tauri-drag-region="true"
          data-slot="titlebar-drag-space"
        />
      </div>
      <GlobalSearch {...(searchInputRef ? { inputRef: searchInputRef } : {})} useStore={useStore} />
      <div
        className="flex h-full min-w-min items-center"
        data-tauri-drag-region="deep"
        data-slot="titlebar-trailing"
      >
        <div
          className="h-full min-w-6 flex-1"
          aria-hidden="true"
          data-tauri-drag-region="true"
          data-slot="titlebar-drag-space-trailing"
        />
        <div
          className="flex h-10 flex-none items-stretch pl-2"
          data-tauri-drag-region="false"
          data-slot="titlebar-controls"
        >
          <div className="flex min-w-0 items-center gap-1 px-1" data-tauri-drag-region="false">
            {update.state === 'available' && (
              <Tooltip>
                <TooltipTrigger
                  render={
                    <Button
                      variant="ghost"
                      className="relative size-[34px]"
                      aria-label={`Open update ${updateVersion}`}
                      onClick={() => {
                        onUpdateOpen?.();
                        setUpdateDialogOpen(true);
                      }}
                    />
                  }
                >
                  <Download size={15} aria-hidden="true" />
                  <span
                    data-update-indicator
                    className="pointer-events-none absolute right-[5px] top-[5px] size-1.5 rounded-full bg-accent-ink ring-2 ring-background"
                    aria-hidden="true"
                  />
                  <span className="sr-only">Update available</span>
                </TooltipTrigger>
                <TooltipContent side="bottom">
                  {`Update available: ${updateVersion}. Open update details.`}
                </TooltipContent>
              </Tooltip>
            )}
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    className="size-[34px]"
                    aria-label="Reload library"
                    onClick={() => void reload()}
                  />
                }
              >
                <RefreshCw size={17} aria-hidden="true" />
              </TooltipTrigger>
              <TooltipContent side="bottom">Reload library</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    ref={settingsTriggerRef}
                    className="size-[34px]"
                    aria-label="Open settings"
                    onClick={() => {
                      onSettingsOpen?.();
                      setSettingsOpen(true);
                    }}
                  />
                }
              >
                <Settings size={17} aria-hidden="true" />
              </TooltipTrigger>
              <TooltipContent side="bottom">Open settings</TooltipContent>
            </Tooltip>
          </div>
          <WindowCaptionControls controls={controls} />
        </div>
      </div>
    </header>
  );
}
