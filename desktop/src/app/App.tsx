import { lazy, Suspense, useEffect, useMemo, useRef } from 'react';
import type { DesktopBridge } from '../bridge/DesktopBridge';
import { BootstrapGate } from './BootstrapGate';
import { AppTitleBar } from '../components/chrome/AppTitleBar';
import { PlayerBar } from '../components/player/PlayerBar';
import { Workbench } from '../components/workbench/Workbench';
import { createDesktopStore } from '../state/store';
import { createWindowControls } from '../platform/windowControls';
import { usePackagedSmokeTest } from './usePackagedSmokeTest';
import { recordStartupTelemetry } from '../bridge/startupTelemetry';

const SettingsPanel = lazy(() =>
  import('../components/settings/SettingsPanel').then(({ SettingsPanel: component }) => ({
    default: component,
  })),
);
const CalibrationDialog = lazy(() =>
  import('../components/calibration/CalibrationDialog').then(
    ({ CalibrationDialog: component }) => ({
      default: component,
    }),
  ),
);
const UpdateDialog = lazy(() =>
  import('../components/updates/UpdateDialog').then(({ UpdateDialog: component }) => ({
    default: component,
  })),
);

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
  const calibrationOpen = useStore((store) => store.calibration.open);
  const updateDialogOpen = useStore((store) => store.update.dialogOpen);
  const windowControls = useMemo(() => createWindowControls(), []);
  const wasSettingsOpen = useRef(false);

  useEffect(() => {
    recordStartupTelemetry('react.initialize.start');
    void useStore.getState().initialize();
  }, [useStore]);

  useEffect(() => {
    if (useStore.getState().bootstrapState !== 'ready') return;
    recordStartupTelemetry('react.shell_ready');
  }, [bootstrap, useStore]);

  useEffect(() => {
    if (bootstrap) document.documentElement.dataset.theme = bootstrap.settings.theme;
  }, [bootstrap]);

  useEffect(() => {
    if (!settingsOpen && wasSettingsOpen.current) {
      window.setTimeout(() => settingsTriggerRef.current?.focus(), 0);
    }
    wasSettingsOpen.current = settingsOpen;
  }, [settingsOpen, settingsTriggerRef]);

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
          <Suspense fallback={null}>
            {settingsOpen && (
              <SettingsPanel
                bootstrap={bootstrap}
                useStore={useStore}
                settingsTriggerRef={settingsTriggerRef}
              />
            )}
            {calibrationOpen && (
              <CalibrationDialog useStore={useStore} settingsTriggerRef={settingsTriggerRef} />
            )}
            {updateDialogOpen && <UpdateDialog useStore={useStore} />}
          </Suspense>
        </div>
      )}
    </BootstrapGate>
  );
}
