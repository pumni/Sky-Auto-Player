import { useEffect, useMemo, useRef } from 'react';
import type { DesktopBridge, UiEvent } from '../bridge/DesktopBridge';
import { BootstrapGate } from './BootstrapGate';
import { SettingsPanel } from '../components/settings/SettingsPanel';
import { AppTitleBar } from '../components/chrome/AppTitleBar';
import { PlayerBar } from '../components/player/PlayerBar';
import { Workbench } from '../components/workbench/Workbench';
import { CalibrationDialog } from '../components/calibration/CalibrationDialog';
import { UpdateDialog } from '../components/updates/UpdateDialog';
import { createDesktopStore } from '../state/store';
import { createWindowControls } from '../platform/windowControls';

interface AppProps {
  bridge: DesktopBridge;
}

export function App({ bridge }: AppProps) {
  const storeRef = useRef<ReturnType<typeof createDesktopStore> | null>(null);
  const utilityTriggerRef = useRef<HTMLButtonElement>(null);
  const settingsTriggerRef = useRef<HTMLButtonElement>(null);
  const useStore = useMemo(() => {
    if (storeRef.current === null) storeRef.current = createDesktopStore(bridge);
    return storeRef.current;
  }, [bridge]);
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
      const waitForStore = async (predicate: () => boolean, description: string): Promise<void> => {
        for (let attempt = 0; attempt < 100; attempt += 1) {
          if (predicate()) return;
          await new Promise((resolve) => window.setTimeout(resolve, 50));
        }
        throw new Error(`packaged GUI ${description} state timed out`);
      };

      await waitForReady();
      const state = useStore.getState();
      if (!document.querySelector('.app-shell')) {
        throw new Error('packaged GUI shell did not render');
      }
      const generation = state.library.generation;
      const search = await bridge.searchSongs({
        query: '',
        offset: 0,
        limit: 200,
        ...(generation > 0 ? { generation } : {}),
      });
      if (
        !Number.isInteger(search.generation) ||
        search.generation < 0 ||
        !Number.isInteger(search.total) ||
        search.total < 0 ||
        !Array.isArray(search.items) ||
        search.items.length > search.limit
      ) {
        throw new Error('packaged GUI search postcondition failed');
      }
      await waitForStore(() => {
        const current = useStore.getState();
        return !current.library.loading && current.library.error === null;
      }, 'library');
      const afterSearch = useStore.getState();
      if (
        afterSearch.bootstrapState !== 'ready' ||
        afterSearch.library.loading ||
        afterSearch.library.error !== null ||
        afterSearch.fatal !== null
      ) {
        throw new Error('packaged GUI library store postcondition failed');
      }
      const expectedTheme = state.bootstrap?.theme ?? 'aurora';
      const smokeTheme = expectedTheme === 'aurora' ? 'slate' : 'aurora';
      const patched = await bridge.patchSettings({ theme: smokeTheme });
      if (patched.theme !== smokeTheme) {
        throw new Error('packaged GUI settings mutation postcondition failed');
      }
      const reread = await bridge.getSettings();
      if (reread.theme !== smokeTheme) {
        throw new Error('packaged GUI settings round-trip postcondition failed');
      }
      const restored = await bridge.patchSettings({ theme: expectedTheme });
      if (restored.theme !== expectedTheme) {
        throw new Error('packaged GUI settings restore postcondition failed');
      }
      const restoredRead = await bridge.getSettings();
      if (restoredRead.theme !== expectedTheme) {
        throw new Error('packaged GUI settings restore round-trip failed');
      }
      const afterSettings = useStore.getState();
      if (afterSettings.settingsState !== 'ready' || afterSettings.fatal !== null) {
        throw new Error('packaged GUI settings store postcondition failed');
      }
      const enabled = await bridge.setDiagnosticsEnabled({ enabled: true });
      if (enabled.enabled !== true) {
        throw new Error('packaged GUI diagnostics enable postcondition failed');
      }
      const disabled = await bridge.setDiagnosticsEnabled({ enabled: false });
      if (disabled.enabled !== false) {
        throw new Error('packaged GUI diagnostics disable postcondition failed');
      }

      let calibrationOperationId: string | null = null;
      let calibrationFinished: Extract<UiEvent, { name: 'calibration.finished' }> | null = null;
      const earlyCalibrationFinished: Extract<UiEvent, { name: 'calibration.finished' }>[] = [];
      let resolveCalibration!: (event: Extract<UiEvent, { name: 'calibration.finished' }>) => void;
      const calibrationDone = new Promise<Extract<UiEvent, { name: 'calibration.finished' }>>(
        (resolve) => {
          resolveCalibration = resolve;
        },
      );
      const unsubscribe = await bridge.subscribeUiEvents((event) => {
        if (
          event.name === 'calibration.finished' &&
          calibrationOperationId !== null &&
          event.payload.operation_id === calibrationOperationId
        ) {
          resolveCalibration(event);
        } else if (event.name === 'calibration.finished') {
          earlyCalibrationFinished.push(event);
        }
      });
      try {
        const calibration = await bridge.startCalibration({
          mode: 'quick',
          className: null,
          polyphony: null,
          samples: null,
          timeoutSeconds: null,
        });
        if (calibration.state !== 'running') {
          throw new Error('packaged GUI calibration did not start');
        }
        calibrationOperationId = calibration.operation_id;
        const early = earlyCalibrationFinished.find(
          (event) => event.payload.operation_id === calibrationOperationId,
        );
        if (early) resolveCalibration(early);
        calibrationFinished = await Promise.race([
          calibrationDone,
          new Promise<never>((_, reject) =>
            window.setTimeout(
              () => reject(new Error('packaged GUI calibration timed out')),
              10_000,
            ),
          ),
        ]);
      } finally {
        unsubscribe();
      }
      if (
        !calibrationFinished ||
        !['succeeded', 'cancelled'].includes(calibrationFinished.payload.outcome)
      ) {
        throw new Error('packaged GUI calibration terminal postcondition failed');
      }
      // The controlled-close command destroys this WebView after starting
      // bounded native cleanup. Do not wait for an invoke response from a
      // window that is intentionally being destroyed.
      void bridge.shutdown();
    };

    const onSmokeEvent = () => {
      void runPackagedSmoke().catch((error: unknown) => {
        console.error('packaged GUI smoke failed', error);
        void bridge.shutdown(true).catch((shutdownError: unknown) => {
          console.error('packaged GUI failure shutdown failed', shutdownError);
        });
      });
    };
    window.addEventListener('sky-desktop-gui-smoke', onSmokeEvent);
    if ((window as Window & { __SKY_DESKTOP_GUI_SMOKE__?: boolean }).__SKY_DESKTOP_GUI_SMOKE__) {
      void runPackagedSmoke().catch((error: unknown) => {
        console.error('packaged GUI smoke failed', error);
        void bridge.shutdown(true).catch((shutdownError: unknown) => {
          console.error('packaged GUI failure shutdown failed', shutdownError);
        });
      });
    }
    return () => window.removeEventListener('sky-desktop-gui-smoke', onSmokeEvent);
  }, [bridge, useStore]);

  return (
    <BootstrapGate useStore={useStore}>
      {bootstrap && (
        <div className="app-shell">
          <AppTitleBar
            bootstrap={bootstrap}
            useStore={useStore}
            settingsTriggerRef={settingsTriggerRef}
            windowControls={windowControls}
          />
          <Workbench useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
          <PlayerBar useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
          <SettingsPanel
            bootstrap={bootstrap}
            useStore={useStore}
            settingsTriggerRef={settingsTriggerRef}
          />
          <CalibrationDialog useStore={useStore} />
          <UpdateDialog useStore={useStore} />
        </div>
      )}
    </BootstrapGate>
  );
}
