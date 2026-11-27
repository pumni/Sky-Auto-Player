import { lazy, Suspense, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { applyAppearance } from '../appearance/applyAppearance';
import { resolveAppearance } from '../appearance/resolveAppearance';
import { useSystemAppearance } from '../appearance/useSystemAppearance';
import type { DesktopBridge } from '../bridge/DesktopBridge';
import { recordStartupTelemetry } from '../bridge/startupTelemetry';
import { AppTitleBar } from '../components/chrome/AppTitleBar';
import { PlayerBar } from '../components/player/PlayerBar';
import { TooltipProvider } from '../components/ui/tooltip';
import { Workbench } from '../components/workbench/Workbench';
import {
  revealStartupWindow,
  scheduleStartupWindowRevealFallback,
} from '../platform/startupWindowReveal';
import { createWindowControls } from '../platform/windowControls';
import { createDesktopStore } from '../state/store';
import { BootstrapGate } from './BootstrapGate';
import { PackagedSmokeTest } from './usePackagedSmokeTest';

const SettingsPanel = lazy(() =>
  import('../components/settings/SettingsPanel').then(({ SettingsPanel: component }) => ({
    default: component,
  })),
);
const UpdateDialog = lazy(() =>
  import('../components/updates/UpdateDialog').then(({ UpdateDialog: component }) => ({
    default: component,
  })),
);

interface AppProps {
  bridge: DesktopBridge;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return (
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement ||
    target.isContentEditable ||
    target.closest(
      '[contenteditable="true"], [role="combobox"], [role="listbox"], [role="option"]',
    ) !== null
  );
}

export function App({ bridge }: AppProps) {
  const utilityTriggerRef = useRef<HTMLButtonElement>(null);
  const settingsTriggerRef = useRef<HTMLButtonElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const useStore = useMemo(() => createDesktopStore(bridge), [bridge]);
  const bootstrap = useStore((store) => store.bootstrap);
  const bootstrapState = useStore((store) => store.bootstrapState);
  const settings = useStore((store) => store.settings);
  const settingsOpen = useStore((store) => store.settingsOpen);
  const updateDialogOpen = useStore((store) => store.update.dialogOpen);
  const setSettingsOpen = useStore((store) => store.setSettingsOpen);
  const toggleUtility = useStore((store) => store.toggleUtility);
  const windowControls = useMemo(() => createWindowControls(), []);
  const [settingsMounted, setSettingsMounted] = useState(false);
  const [updateMounted, setUpdateMounted] = useState(false);
  const returnToSettingsAfterUpdateRef = useRef(false);
  const { mode: systemMode, ready: appearanceReady } = useSystemAppearance();

  useEffect(() => {
    scheduleStartupWindowRevealFallback();
    recordStartupTelemetry('react.initialize.start');
    void useStore.getState().initialize();
  }, [useStore]);

  useEffect(() => {
    if (bootstrapState !== 'ready') return;
    recordStartupTelemetry('react.shell_ready');
  }, [bootstrapState]);

  const appearanceMode =
    settings?.appearance_mode ?? bootstrap?.settings.appearance_mode ?? 'system';
  const palette = settings?.palette ?? bootstrap?.settings.palette ?? 'graphite';

  useLayoutEffect(() => {
    applyAppearance(resolveAppearance(appearanceMode, systemMode, palette));
    if (bootstrapState === 'ready' && bootstrap && appearanceReady) {
      revealStartupWindow('bootstrap-ready');
    } else if (bootstrapState === 'fatal' && appearanceReady) {
      revealStartupWindow('bootstrap-failed');
    }
  }, [appearanceMode, palette, bootstrap, bootstrapState, systemMode, appearanceReady]);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (typeof event.key !== 'string') return;
      const normalizedKey = event.key.toLowerCase();
      const hasActiveModal = document.querySelector('[aria-modal="true"]') !== null;
      const isFindShortcut = normalizedKey === 'f' && event.ctrlKey;
      const hasPlainControlModifier = event.ctrlKey && !event.metaKey && !event.altKey;
      const isSettingsShortcut = event.key === ',' && hasPlainControlModifier;
      const isInspectorShortcut =
        normalizedKey === 'i' && hasPlainControlModifier && !isEditableTarget(event.target);
      const isSlashShortcut =
        event.key === '/' &&
        !event.ctrlKey &&
        !event.metaKey &&
        !event.altKey &&
        !isEditableTarget(event.target);
      if (
        !settingsOpen &&
        !updateDialogOpen &&
        !hasActiveModal &&
        !isEditableTarget(event.target) &&
        isSettingsShortcut
      ) {
        event.preventDefault();
        setSettingsMounted(true);
        setSettingsOpen(true);
        return;
      }
      if (!settingsOpen && !updateDialogOpen && !hasActiveModal && isInspectorShortcut) {
        event.preventDefault();
        toggleUtility();
        return;
      }
      if (!settingsOpen && (isFindShortcut || isSlashShortcut)) {
        event.preventDefault();
        searchInputRef.current?.focus();
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [settingsOpen, setSettingsOpen, toggleUtility, updateDialogOpen]);

  return (
    <TooltipProvider delay={350} closeDelay={80} timeout={450}>
      <BootstrapGate useStore={useStore}>
        {bootstrap?.gui_smoke === true && <PackagedSmokeTest bridge={bridge} useStore={useStore} />}
        {bootstrap && (
          <div
            data-app-shell
            className="grid h-dvh min-h-0 w-full min-w-0 grid-rows-[40px_minmax(0,1fr)_92px] overflow-hidden bg-background"
          >
            <AppTitleBar
              bootstrap={bootstrap}
              useStore={useStore}
              settingsTriggerRef={settingsTriggerRef}
              searchInputRef={searchInputRef}
              windowControls={windowControls}
              onSettingsOpen={() => setSettingsMounted(true)}
              onUpdateOpen={() => setUpdateMounted(true)}
            />
            <Workbench useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
            <PlayerBar useStore={useStore} utilityTriggerRef={utilityTriggerRef} />
            <Suspense fallback={null}>
              {(settingsOpen || settingsMounted) && (
                <SettingsPanel
                  bootstrap={bootstrap}
                  useStore={useStore}
                  onUpdateDialogOpen={() => {
                    returnToSettingsAfterUpdateRef.current = true;
                    setUpdateMounted(true);
                    setSettingsOpen(false);
                  }}
                />
              )}
              {(updateMounted || updateDialogOpen) && (
                <UpdateDialog
                  useStore={useStore}
                  onCloseComplete={() => {
                    if (returnToSettingsAfterUpdateRef.current) {
                      returnToSettingsAfterUpdateRef.current = false;
                      setSettingsOpen(true);
                    }
                  }}
                />
              )}
            </Suspense>
          </div>
        )}
      </BootstrapGate>
    </TooltipProvider>
  );
}
