import type { DesktopBridge } from '../bridge/DesktopBridge';
import type { DesktopStore } from './types';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface SettingsSliceContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  clearOperationFromEvent: () => void;
  onAutoPlayChanged: () => void;
}

export type SettingsSliceActions = Pick<DesktopStore, 'patchSettings' | 'setSettingsOpen'>;

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
            patchFields.every(
              (field) => field === 'appearanceMode' || field === 'palette' || field === 'autoPlay',
            );
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

    setSettingsOpen(open) {
      set({ settingsOpen: open });
    },
  };
}
