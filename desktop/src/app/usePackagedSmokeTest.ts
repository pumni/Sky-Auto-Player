import { useEffect, useRef } from 'react';
import type { CalibrationFinished, DesktopBridge } from '../bridge/DesktopBridge';
import type { DesktopStoreHook } from '../state/store';

interface PackagedSmokeTestProps {
  bridge: DesktopBridge;
  useStore: DesktopStoreHook;
}

interface SmokeWindow extends Window {
  __SKY_DESKTOP_GUI_SMOKE__?: boolean;
}

export function usePackagedSmokeTest({ bridge, useStore }: PackagedSmokeTestProps): void {
  const smokeStarted = useRef(false);

  useEffect(() => {
    const runPackagedSmoke = async () => {
      if (smokeStarted.current) return;
      smokeStarted.current = true;
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
      await waitForStore(() => {
        const current = useStore.getState();
        return !current.library.loading && current.library.error === null;
      }, 'library');
      const generation = useStore.getState().library.generation;
      const search = await bridge.searchSongs({
        query: '',
        offset: 0,
        limit: 200,
        source: { kind: 'smart', id: 'all' },
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
      const expectedPalette = state.settings?.palette ?? 'graphite';
      const smokePalette = expectedPalette === 'aurora' ? 'slate' : 'aurora';
      const patched = await bridge.patchSettings({ palette: smokePalette });
      if (patched.palette !== smokePalette) {
        throw new Error('packaged GUI settings mutation postcondition failed');
      }
      const reread = await bridge.getSettings();
      if (reread.palette !== smokePalette) {
        throw new Error('packaged GUI settings round-trip postcondition failed');
      }
      const restored = await bridge.patchSettings({ palette: expectedPalette });
      if (restored.palette !== expectedPalette) {
        throw new Error('packaged GUI settings restore postcondition failed');
      }
      const restoredRead = await bridge.getSettings();
      if (restoredRead.palette !== expectedPalette) {
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

      const previousCalibrationResult = useStore.getState().calibration.result;
      let lastObservedCalibrationResult = previousCalibrationResult;
      let expectedOperationId: string | null = null;
      let calibrationFinished: CalibrationFinished;
      const earlyCalibrationResults: Array<{
        operationId: string | null;
        result: CalibrationFinished;
      }> = [];
      let resolveCalibration!: (result: CalibrationFinished) => void;
      let rejectCalibration!: (error: Error) => void;
      const calibrationDone = new Promise<CalibrationFinished>((resolve, reject) => {
        resolveCalibration = resolve;
        rejectCalibration = reject;
      });
      const handleCalibrationResult = (operationId: string | null, result: CalibrationFinished) => {
        if (expectedOperationId === null) {
          earlyCalibrationResults.push({ operationId, result });
          return;
        }
        if (operationId !== expectedOperationId || result.operation_id !== expectedOperationId) {
          rejectCalibration(new Error('packaged GUI calibration operation ID mismatch'));
          return;
        }
        if (!['succeeded', 'cancelled'].includes(result.outcome)) {
          rejectCalibration(new Error('packaged GUI calibration terminal postcondition failed'));
          return;
        }
        resolveCalibration(result);
      };
      const unsubscribe = useStore.subscribe((state) => {
        const result = state.calibration.result;
        if (!result || result === lastObservedCalibrationResult) return;
        lastObservedCalibrationResult = result;
        handleCalibrationResult(state.calibration.operationId, result);
      });
      let calibrationTimeout: number | null = null;
      try {
        const calibration = await bridge.startCalibration({
          mode: 'quick',
          className: null,
          polyphony: null,
          samples: null,
          timeoutSeconds: null,
        });
        if (calibration.state !== 'running' || !calibration.operation_id) {
          throw new Error('packaged GUI calibration did not start');
        }
        expectedOperationId = calibration.operation_id;
        const calibrationResult = Promise.race([
          calibrationDone,
          new Promise<never>(
            (_, reject) =>
              (calibrationTimeout = window.setTimeout(
                () => reject(new Error('packaged GUI calibration timed out')),
                10_000,
              )),
          ),
        ]);
        const pendingResults = earlyCalibrationResults.splice(0);
        const currentCalibration = useStore.getState().calibration;
        if (
          currentCalibration.result &&
          currentCalibration.result !== previousCalibrationResult &&
          !pendingResults.some(({ result }) => result === currentCalibration.result)
        ) {
          pendingResults.push({
            operationId: currentCalibration.operationId,
            result: currentCalibration.result,
          });
        }
        for (const pending of pendingResults) {
          handleCalibrationResult(pending.operationId, pending.result);
        }
        calibrationFinished = await calibrationResult;
      } finally {
        unsubscribe();
        if (calibrationTimeout !== null) window.clearTimeout(calibrationTimeout);
      }
      if (!['succeeded', 'cancelled'].includes(calibrationFinished.outcome)) {
        throw new Error('packaged GUI calibration terminal postcondition failed');
      }
      // The controlled-close command destroys this WebView after starting
      // bounded native cleanup. Do not wait for an invoke response from a
      // window that is intentionally being destroyed.
      void bridge.shutdown();
    };

    const onSmokeFailure = (error: unknown) => {
      console.error('packaged GUI smoke failed', error);
      void bridge.shutdown(true).catch((shutdownError: unknown) => {
        console.error('packaged GUI failure shutdown failed', shutdownError);
      });
    };
    const onSmokeEvent = () => {
      void runPackagedSmoke().catch(onSmokeFailure);
    };
    window.addEventListener('sky-desktop-gui-smoke', onSmokeEvent);
    if ((window as SmokeWindow).__SKY_DESKTOP_GUI_SMOKE__) {
      void runPackagedSmoke().catch(onSmokeFailure);
    }
    return () => window.removeEventListener('sky-desktop-gui-smoke', onSmokeEvent);
  }, [bridge, useStore]);
}
