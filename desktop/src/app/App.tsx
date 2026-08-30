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

  useEffect(() => {
    let completed = false;
    const runPackagedSmoke = async () => {
      if (completed) return;
      completed = true;
      const waitForReady = async () => {
        for (let attempt = 0; attempt < 300; attempt += 1) {
          const state = useStore.getState();
          if (state.bootstrapState === 'ready' && state.settings) return;
          if (state.bootstrapState === 'fatal') {
            throw new Error(state.fatal ?? 'packaged GUI bootstrap failed');
          }
          await new Promise((resolve) => window.setTimeout(resolve, 100));
        }
        throw new Error('packaged GUI bootstrap timed out');
      };

      await waitForReady();
      const state = useStore.getState();
      if (!document.querySelector('.app-shell')) {
        throw new Error('packaged GUI shell did not render');
      }
      await state.search('');
      await state.patchSettings({ theme: state.bootstrap?.theme ?? 'aurora' });
      await state.setDiagnosticsEnabled(true);
      await state.setDiagnosticsEnabled(false);
      // The controlled-close command destroys this WebView after starting
      // bounded Core cleanup. Do not wait for an invoke response from a
      // window that is intentionally being destroyed.
      void bridge.shutdown();
    };

    const onSmokeEvent = () => {
      void runPackagedSmoke();
    };
    window.addEventListener('sky-phase8-gui-smoke', onSmokeEvent);
    if ((window as Window & { __SKY_PHASE8_GUI_SMOKE__?: boolean }).__SKY_PHASE8_GUI_SMOKE__) {
      void runPackagedSmoke();
    }
    return () => window.removeEventListener('sky-phase8-gui-smoke', onSmokeEvent);
  }, [bridge, useStore]);

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
