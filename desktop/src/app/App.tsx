import { useEffect, useMemo, useRef } from 'react';
import type { DesktopBridge } from '../bridge/DesktopBridge';
import { BootstrapGate } from './BootstrapGate';
import { SettingsPanel } from '../components/settings/SettingsPanel';
import { AppTitleBar } from '../components/chrome/AppTitleBar';
import { PlayerBar } from '../components/player/PlayerBar';
import { Workbench } from '../components/workbench/Workbench';
import { CalibrationDialog } from '../components/calibration/CalibrationDialog';
import { UpdateDialog } from '../components/updates/UpdateDialog';
import { createDesktopStore } from '../state/store';
import { createWindowControls } from '../platform/windowControls';
import { usePackagedSmokeTest } from './usePackagedSmokeTest';

interface AppProps {
  bridge: DesktopBridge;
}

export function App({ bridge }: AppProps) {
  const utilityTriggerRef = useRef<HTMLButtonElement>(null);
  const settingsTriggerRef = useRef<HTMLButtonElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const useStore = useMemo(() => createDesktopStore(bridge), [bridge]);
  const bootstrap = useStore((store) => store.bootstrap);
  const settingsOpen = useStore((store) => store.settingsOpen);
  const windowControls = useMemo(() => createWindowControls(), []);

  useEffect(() => {
    void useStore.getState().initialize();
  }, [useStore]);

  useEffect(() => {
    if (bootstrap) document.documentElement.dataset.theme = bootstrap.theme;
  }, [bootstrap]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (
        (event.key === '/' || (event.key.toLowerCase() === 'f' && event.ctrlKey)) &&
        !settingsOpen
      ) {
        event.preventDefault();
        searchInputRef.current?.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [settingsOpen, useStore]);

  usePackagedSmokeTest({ bridge, useStore });

  return (
    <BootstrapGate useStore={useStore}>
      {bootstrap && (
        <div className="app-shell">
          <AppTitleBar
            bootstrap={bootstrap}
            useStore={useStore}
            settingsTriggerRef={settingsTriggerRef}
            searchInputRef={searchInputRef}
            windowControls={windowControls}
          />
          <Workbench useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
          <PlayerBar useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
          <SettingsPanel
            bootstrap={bootstrap}
            useStore={useStore}
            settingsTriggerRef={settingsTriggerRef}
          />
          <CalibrationDialog useStore={useStore} settingsTriggerRef={settingsTriggerRef} />
          <UpdateDialog useStore={useStore} />
        </div>
      )}
    </BootstrapGate>
  );
}
