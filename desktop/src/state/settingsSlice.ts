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

    async checkForUpdate() {
      set({ update: { ...get().update, state: 'checking', error: null } });
      try {
        const result = await bridge.checkForUpdate();
        set({
          update: {
            ...get().update,
            state: result.state,
            currentVersion: result.current_version,
            availableVersion: result.available_version,
            channel: result.channel,
            releaseNotes: result.release_notes,
            publishedAt: result.published_at,
            error: result.error,
            dialogOpen: get().update.dialogOpen,
          },
        });
      } catch (error) {
        set({
          update: {
            ...get().update,
            state: 'error',
            error: error instanceof Error ? error.message : String(error),
          },
        });
      }
    },

    setUpdateDialogOpen(open) {
      set({ update: { ...get().update, dialogOpen: open } });
    },

    async beginUpdateHandoff() {
      const targetVersion = get().update.availableVersion;
      if (!targetVersion) return;
      set({ update: { ...get().update, state: 'downloading', error: null } });
      try {
        const handoff = await bridge.beginUpdateHandoff(targetVersion);
        set({
          update: {
            ...get().update,
            state: handoff.state,
            handoffId: handoff.handoff_id,
            error: null,
          },
        });
      } catch (error) {
        set({
          update: {
            ...get().update,
            state: 'error',
            error: error instanceof Error ? error.message : String(error),
          },
        });
      }
    },

    setSettingsOpen(open) {
      set({ settingsOpen: open });
    },
  };
}
