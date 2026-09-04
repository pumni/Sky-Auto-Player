import { Download, ListRestart, Settings } from 'lucide-react';
import { Button } from 'react-aria-components';
import { useMemo, type RefObject } from 'react';
import brandMark32Url from '../../assets/brand/app-icon-32.png';
import brandMark40Url from '../../assets/brand/app-icon-40.png';
import brandMark48Url from '../../assets/brand/app-icon-48.png';
import brandMark64Url from '../../assets/brand/app-icon-64.png';
import type { Bootstrap } from '../../bridge/DesktopBridge';
import type { DesktopStore, DesktopStoreHook } from '../../state/store';
import { createWindowControls, type WindowControls } from '../../platform/windowControls';
import { GlobalSearch } from './GlobalSearch';
import { WindowCaptionControls } from './WindowCaptionControls';

interface AppTitleBarProps {
  bootstrap: Bootstrap;
  useStore: DesktopStoreHook;
  settingsTriggerRef?: RefObject<HTMLButtonElement | null>;
  windowControls?: WindowControls;
}

export function AppTitleBar({
  useStore,
  settingsTriggerRef,
  windowControls,
}: AppTitleBarProps) {
  const controls = useMemo(() => windowControls ?? createWindowControls(), [windowControls]);
  const reload = useStore((store: DesktopStore) => store.reloadLibrary);
  const setSettingsOpen = useStore((store: DesktopStore) => store.setSettingsOpen);
  const update = useStore((store: DesktopStore) => store.update);
  const setUpdateDialogOpen = useStore((store: DesktopStore) => store.setUpdateDialogOpen);

  return (
    <header className="app-titlebar" aria-label="Sky Auto Player" data-tauri-drag-region="deep">
      <div className="app-titlebar-brand">
        <img
          className="app-titlebar-icon"
          src={brandMark32Url}
          srcSet={`${brandMark32Url} 1x, ${brandMark40Url} 1.25x, ${brandMark48Url} 1.5x, ${brandMark64Url} 2x`}
          sizes="24px"
          alt=""
          width="24"
          height="24"
          draggable={false}
        />
        <span className="visually-hidden">Sky Auto Player</span>
      </div>
      <div className="titlebar-drag-space" aria-hidden="true" />
      <GlobalSearch useStore={useStore} />
      <div className="app-titlebar-actions" data-tauri-drag-region="false">
        {update.state === 'available' && (
          <Button
            className="update-indicator"
            aria-label={`Open update ${update.availableVersion}`}
            title={`Update available: ${update.availableVersion}`}
            onPress={() => setUpdateDialogOpen(true)}
          >
            <Download size={15} aria-hidden="true" />
            <span className="visually-hidden">Update</span>
          </Button>
        )}
        <Button
          className="icon-button"
          aria-label="Reload library"
          title="Reload library"
          onPress={() => void reload()}
        >
          <ListRestart size={17} aria-hidden="true" />
        </Button>
        <Button
          ref={settingsTriggerRef}
          className="icon-button"
          aria-label="Open settings"
          title="Open settings"
          onPress={() => setSettingsOpen(true)}
        >
          <Settings size={17} aria-hidden="true" />
        </Button>
      </div>
      <WindowCaptionControls controls={controls} />
    </header>
  );
}
