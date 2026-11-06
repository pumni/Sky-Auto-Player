import type { DesktopBridge } from '../bridge/DesktopBridge';
import type { DesktopStore } from './types';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface UpdateSliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
}

export type UpdateSliceActions = Pick<
  DesktopStore,
  'checkForUpdate' | 'setUpdateDialogOpen' | 'beginUpdateHandoff'
>;

export function createUpdateSlice({ bridge, get, set }: UpdateSliceContext): UpdateSliceActions {
  return {
    async checkForUpdate(origin: 'manual' | 'background' = 'manual') {
      const isManual = origin === 'manual';
      const requestRevision = get().update.lastNativeRevision;
      set({
        update: {
          ...get().update,
          checkRequestPending: true,
          transportError: null,
          transportErrorAction: null,
          dialogOpen: isManual ? true : get().update.dialogOpen,
        },
      });

      try {
        await bridge.checkForUpdate({ origin });
        set({
          update: {
            ...get().update,
            checkRequestPending: false,
          },
        });
      } catch (error) {
        const current = get().update;
        const hasNewerSnapshot = current.lastNativeRevision > requestRevision;
        set({
          update: {
            ...current,
            checkRequestPending: false,
            transportError: hasNewerSnapshot
              ? null
              : error instanceof Error
                ? error.message
                : String(error),
            transportErrorAction: hasNewerSnapshot ? null : 'check',
            dialogOpen: isManual ? true : current.dialogOpen,
          },
        });
        // A thrown bridge/IPC failure must not invent native timestamps.
      }
    },

    setUpdateDialogOpen(open) {
      set({ update: { ...get().update, dialogOpen: open } });
    },

    async beginUpdateHandoff() {
      if (get().update.installRequestPending) return;
      const targetVersion = get().update.availableVersion;
      if (!targetVersion) return;
      const requestRevision = get().update.lastNativeRevision;
      set({
        update: {
          ...get().update,
          installRequestPending: true,
          transportError: null,
          transportErrorAction: null,
        },
      });
      try {
        await bridge.beginUpdateHandoff(targetVersion);
        set({
          update: {
            ...get().update,
            installRequestPending: false,
          },
        });
      } catch (error) {
        const current = get().update;
        const hasNewerSnapshot = current.lastNativeRevision > requestRevision;
        set({
          update: {
            ...current,
            installRequestPending: false,
            transportError: hasNewerSnapshot
              ? null
              : error instanceof Error
                ? error.message
                : String(error),
            transportErrorAction: hasNewerSnapshot ? null : 'install',
          },
        });
      }
    },
  };
}
