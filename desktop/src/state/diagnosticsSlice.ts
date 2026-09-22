import type { CalibrationStart, DesktopBridge } from '../bridge/DesktopBridge';
import type { CalibrationUiState, DesktopStore, UtilityView } from './types';
import { updateWorkbenchPreferences } from './workbenchPreferences';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface DiagnosticsSliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
}

export type DiagnosticsSliceActions = Pick<
  DesktopStore,
  | 'setDiagnosticsEnabled'
  | 'exportSenderTrace'
  | 'openUtility'
  | 'closeUtility'
  | 'toggleUtility'
  | 'setUtilityView'
  | 'startCalibration'
  | 'cancelCalibration'
  | 'setCalibrationOpen'
>;

export function createDiagnosticsSlice(context: DiagnosticsSliceContext): DiagnosticsSliceActions {
  const { bridge, get, set } = context;
  let diagnosticsToggleEpoch = 0;

  return {
    async setDiagnosticsEnabled(enabled) {
      const epoch = ++diagnosticsToggleEpoch;
      const current = get().diagnostics;
      set({ diagnostics: { ...current, enabled: false, samples: [], error: null } });
      try {
        const result = await bridge.setDiagnosticsEnabled({ enabled });
        if (epoch !== diagnosticsToggleEpoch) return;
        set({
          diagnostics: {
            ...get().diagnostics,
            enabled: result.enabled,
            error: null,
          },
        });
      } catch (error) {
        if (epoch !== diagnosticsToggleEpoch) return;
        set({
          diagnostics: {
            ...get().diagnostics,
            enabled: false,
            error: error instanceof Error ? error.message : String(error),
          },
        });
      }
    },

    exportSenderTrace() {
      return bridge.exportSenderTrace();
    },

    openUtility(view) {
      set({ utility: { open: true, activeView: view } });
      updateWorkbenchPreferences({ utilityOpen: true });
      if (view === 'diagnostics' && !get().diagnostics.enabled) {
        void get().setDiagnosticsEnabled(true);
      } else if (view === 'details' && get().diagnostics.enabled) {
        void get().setDiagnosticsEnabled(false);
      }
    },

    closeUtility() {
      const activeView = get().utility.activeView;
      set({ utility: { ...get().utility, open: false } });
      updateWorkbenchPreferences({ utilityOpen: false });
      if (activeView === 'diagnostics' && get().diagnostics.enabled) {
        void get().setDiagnosticsEnabled(false);
      }
    },

    toggleUtility() {
      const utility = get().utility;
      if (utility.open) {
        get().closeUtility();
        return;
      }
      get().openUtility(utility.activeView);
    },

    setUtilityView(view: UtilityView) {
      const current = get().utility;
      if (current.activeView === view) return;
      set({ utility: { ...current, activeView: view } });
      if (view === 'diagnostics' && !get().diagnostics.enabled) {
        void get().setDiagnosticsEnabled(true);
      } else if (view === 'details' && get().diagnostics.enabled) {
        void get().setDiagnosticsEnabled(false);
      }
    },

    async startCalibration(mode = 'quick') {
      if (['starting', 'running', 'cancelling'].includes(get().calibration.state)) return;
      set({
        calibration: {
          ...get().calibration,
          open: true,
          operationId: null,
          state: 'starting',
          phase: mode,
          completed: 0,
          total: 0,
          message: 'Starting calibration…',
          result: null,
          error: null,
        },
      });
      try {
        const ack = await bridge.startCalibration({
          mode,
          className: null,
          polyphony: null,
          samples: null,
          timeoutSeconds: null,
        } satisfies CalibrationStart);
        const current = get().calibration;
        if (current.operationId && current.operationId !== ack.operation_id) return;
        if (['succeeded', 'failed', 'cancelled'].includes(current.state)) return;
        set({
          calibration: {
            ...current,
            operationId: ack.operation_id,
            state: ack.state as CalibrationUiState,
            error: null,
          },
        });
      } catch (error) {
        set({
          calibration: {
            ...get().calibration,
            state: 'failed',
            error: error instanceof Error ? error.message : String(error),
          },
        });
      }
    },

    async cancelCalibration() {
      const operationId = get().calibration.operationId;
      if (!operationId || ['succeeded', 'failed', 'cancelled'].includes(get().calibration.state)) {
        return;
      }
      set({ calibration: { ...get().calibration, state: 'cancelling' } });
      try {
        const ack = await bridge.cancelCalibration({ operationId });
        set({ calibration: { ...get().calibration, state: ack.state as CalibrationUiState } });
      } catch (error) {
        set({
          calibration: {
            ...get().calibration,
            state: 'failed',
            error: error instanceof Error ? error.message : String(error),
          },
        });
      }
    },

    setCalibrationOpen(open) {
      const current = get().calibration;
      if (!open && ['starting', 'running', 'cancelling'].includes(current.state)) return;
      set({ calibration: { ...current, open } });
    },
  };
}
