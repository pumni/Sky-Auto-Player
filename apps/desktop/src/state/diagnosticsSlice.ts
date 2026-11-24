import type { DesktopBridge } from '../bridge/DesktopBridge';
import type { DesktopStore, UtilityView } from './types';
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
  };
}
