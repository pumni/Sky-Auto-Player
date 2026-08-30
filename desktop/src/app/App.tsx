import { useEffect, useMemo, useRef } from 'react';
import type { DesktopBridge } from '../bridge/DesktopBridge';
import { BootstrapGate } from './BootstrapGate';
import { LibraryPanel } from '../components/library/LibraryPanel';
import { SettingsPanel } from '../components/settings/SettingsPanel';
import { SongInspector } from '../components/inspector/SongInspector';
import { Toolbar } from '../components/shell/Toolbar';
import { PlayerDock } from '../components/player/PlayerDock';
import { DiagnosticsDrawer } from '../components/diagnostics/DiagnosticsDrawer';
import { CalibrationDialog } from '../components/calibration/CalibrationDialog';
import { UpdateDialog } from '../components/updates/UpdateDialog';
import { createDesktopStore } from '../state/store';

interface AppProps {
  bridge: DesktopBridge;
}

export function App({ bridge }: AppProps) {
  const storeRef = useRef<ReturnType<typeof createDesktopStore> | null>(null);
  const useStore = useMemo(() => {
    if (storeRef.current === null) storeRef.current = createDesktopStore(bridge);
    return storeRef.current;
  }, [bridge]);
  const bootstrap = useStore((store) => store.bootstrap);
  const settingsOpen = useStore((store) => store.settingsOpen);

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
        document.querySelector<HTMLInputElement>('input[type="search"]')?.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [settingsOpen, useStore]);

  return (
    <BootstrapGate useStore={useStore}>
      {bootstrap && (
        <div className="app-shell">
          <Toolbar bootstrap={bootstrap} useStore={useStore} />
          <main className="main-layout">
            <LibraryPanel useStore={useStore} />
            <SongInspector useStore={useStore} />
          </main>
          <PlayerDock useStore={useStore} />
          <DiagnosticsDrawer useStore={useStore} />
          <SettingsPanel bootstrap={bootstrap} useStore={useStore} />
          <CalibrationDialog useStore={useStore} />
          <UpdateDialog useStore={useStore} />
        </div>
      )}
    </BootstrapGate>
  );
}
