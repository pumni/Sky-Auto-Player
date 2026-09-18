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

export type SettingsSliceActions = Pick<
  DesktopStore,
  | 'patchSettings'
  | 'checkForUpdate'
  | 'setUpdateDialogOpen'
  | 'beginUpdateHandoff'
  | 'setSettingsOpen'
>;

export function createSettingsSlice(context: SettingsSliceContext): SettingsSliceActions {
  const { bridge, get, set } = context;
  let settingsMutationTail: Promise<void> = Promise.resolve();

  return {
    async patchSettings(patch) {
      const mutation = settingsMutationTail.then(async () => {
        set({ settingsState: 'loading' });
        try {
          const settings = await bridge.patchSettings(patch);
          const playback = get().playback;
          if (patch.autoPlay !== undefined && patch.autoPlay !== get().settings?.auto_play) {
            context.onAutoPlayChanged();
          }
          const autoPlayOnly =
            patch.autoPlay !== undefined &&
            patch.theme === undefined &&
            patch.telemetryEnabled === undefined &&
            patch.verboseHud === undefined &&
            patch.playbackDefaults === undefined &&
            patch.updatePreferences === undefined;
          const cancelPreparing = !autoPlayOnly && playback.transportOperation === 'preparing';
          if (cancelPreparing) context.clearOperationFromEvent();
          set({
            settings,
            settingsState: 'ready',
            playback: {
              ...playback,
              prepared: autoPlayOnly ? playback.prepared : null,
              preparedIdentity: autoPlayOnly ? playback.preparedIdentity : null,
              preparedContext: autoPlayOnly ? playback.preparedContext : null,
              transportOperation: cancelPreparing ? null : playback.transportOperation,
            },
          });
          document.documentElement.dataset.theme = settings.theme;
          return settings;
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          set({ settingsState: 'fatal', fatal: message });
          return get().settings;
        }
      });
      // Keep the queue alive after an individual mutation fails. Later user
      // intent must still be applied in order.
      settingsMutationTail = mutation.then(
        () => undefined,
        () => undefined,
      );
      return mutation;
    },

    async checkForUpdate(origin: 'manual' | 'background' = 'manual') {
      const isManual = origin === 'manual';
      set({
        update: {
          ...get().update,
          checkRequestPending: true,
          transportError: null,
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
        try {
          const preferences = await bridge.getUpdatePreferences();
          const currentSettings = get().settings;
          if (currentSettings) {
            set({
              settings: {
                ...currentSettings,
                update_preferences: preferences,
              },
            });
          }
        } catch {
          // Native query failure; keep existing preferences
        }
      } catch (error) {
        set({
          update: {
            ...get().update,
            checkRequestPending: false,
            transportError: error instanceof Error ? error.message : String(error),
            dialogOpen: isManual ? true : get().update.dialogOpen,
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
      set({
        update: {
          ...get().update,
          installRequestPending: true,
          transportError: null,
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
        set({
          update: {
            ...get().update,
            installRequestPending: false,
            transportError: error instanceof Error ? error.message : String(error),
          },
        });
      }
    },

    setSettingsOpen(open) {
      set({ settingsOpen: open });
    },
  };
}
