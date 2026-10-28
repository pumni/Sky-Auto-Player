import type { DesktopBridge } from '../bridge/DesktopBridge';
import type { DesktopStore } from './types';
import { applyAppearance } from '../appearance/applyAppearance';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface SettingsSliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  clearOperationFromEvent: () => void;
  onAutoPlayChanged: () => void;
}

export type SettingsSliceActions = Pick<
  DesktopStore,
  | 'patchSettings'
  | 'checkForUpdate'
  | 'setUpdateDialogOpen'
  | 'beginUpdateHandoff'
  | 'setSettingsOpen'
>;

type SettingsSliceWithRefresh = SettingsSliceActions & {
  refreshSettings: () => Promise<void>;
};

export function createSettingsSlice(context: SettingsSliceContext): SettingsSliceWithRefresh {
  const { bridge, get, set } = context;
  let settingsMutationTail: Promise<void> = Promise.resolve();
  function enqueueSettingsOperation<T>(operation: () => Promise<T>): Promise<T> {
    const queued = settingsMutationTail.then(operation);
    // Keep later user intent runnable after any individual settings operation fails.
    settingsMutationTail = queued.then(
      () => undefined,
      () => undefined,
    );
    return queued;
  }

  return {
    async patchSettings(patch) {
      return enqueueSettingsOperation(async () => {
        set({ settingsState: 'loading', settingsError: null });
        try {
          const settings = await bridge.patchSettings(patch);
          const playback = get().playback;
          if (patch.autoPlay !== undefined && patch.autoPlay !== get().settings?.auto_play) {
            context.onAutoPlayChanged();
          }
          const patchFields = Object.entries(patch)
            .filter(([, value]) => value !== undefined && value !== null)
            .map(([field]) => field);
          const preparationSafePatch =
            patchFields.length > 0 &&
            patchFields.every((field) => field === 'palette' || field === 'autoPlay');
          const cancelPreparing =
            !preparationSafePatch && playback.transportOperation === 'preparing';
          if (cancelPreparing) context.clearOperationFromEvent();
          set({
            settings,
            settingsState: 'ready',
            settingsError: null,
            playback: {
              ...playback,
              prepared: preparationSafePatch ? playback.prepared : null,
              preparedIdentity: preparationSafePatch ? playback.preparedIdentity : null,
              preparedContext: preparationSafePatch ? playback.preparedContext : null,
              transportOperation: cancelPreparing ? null : playback.transportOperation,
            },
          });
          applyAppearance(settings.palette);
          return settings;
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          set({ settingsState: 'ready', settingsError: message });
          return null;
        }
      });
    },

    refreshSettings() {
      return enqueueSettingsOperation(async () => {
        try {
          const settings = await bridge.getSettings();
          set({ settings, settingsState: 'ready' });
        } catch {
          // Keep the last confirmed snapshot when a best-effort refresh fails.
        }
      });
    },

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
        // Thrown bridge/IPC failure; do not invent timestamps
      }
    },

    setUpdateDialogOpen(open) {
      set({ update: { ...get().update, dialogOpen: open } });
    },

    async beginUpdateHandoff() {
      if (get().update.installRequestPending) {
        return;
      }
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

    setSettingsOpen(open) {
      set({ settingsOpen: open });
    },
  };
}
