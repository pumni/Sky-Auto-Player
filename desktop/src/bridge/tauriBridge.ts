import { Channel, invoke } from '@tauri-apps/api/core';
import {
  COMMANDS,
  UI_EVENTS_COMMAND,
  type CommandRequestMap,
  type CommandResponseMap,
  type DesktopCommandName,
} from './generated/commands';
import type {
  DesktopBridge,
  DetailRequest,
  SearchRequest,
  SettingsPatch,
  PlaybackPrepare,
  PlaybackStart,
  PlaybackSessionCommand,
  CalibrationCancel,
  CalibrationStart,
  DiagnosticsSetEnabled,
  UiEvent,
  Unsubscribe,
  ViewportRequest,
  UpdatePatch,
} from './DesktopBridge';

type UiChannelEvent = UiEvent;

function normalizeEvent(event: UiChannelEvent): UiEvent {
  return event;
}

export function encodeCommandArgs(request?: unknown): Record<string, unknown> | undefined {
  return request === undefined ? undefined : { params: request };
}

function call<K extends DesktopCommandName>(
  command: K,
  request?: CommandRequestMap[K],
): Promise<CommandResponseMap[K]> {
  return invoke<CommandResponseMap[K]>(command, encodeCommandArgs(request));
}

export function createTauriBridge(): DesktopBridge {
  return {
    bootstrap: () => call(COMMANDS.bootstrap),
    searchSongs: (request: SearchRequest) => call(COMMANDS.searchSongs, request),
    getSongDetail: (request: DetailRequest) => call(COMMANDS.getSongDetail, request),
    reloadLibrary: () => call(COMMANDS.reloadLibrary),
    setLibraryViewport: (request: ViewportRequest) => call(COMMANDS.setLibraryViewport, request),
    setSongLiked: (request) => call(COMMANDS.setSongLiked, request),
    listLibraryNavigation: () => call(COMMANDS.libraryListCollections),
    createCollection: (name) => call(COMMANDS.libraryCreateCollection, { name }),
    renameCollection: (collectionId, name) =>
      call(COMMANDS.libraryRenameCollection, { collectionId, name }),
    deleteCollection: (collectionId) => call(COMMANDS.libraryDeleteCollection, { collectionId }),
    addSongs: (collectionId, songIds) => call(COMMANDS.libraryAddSongs, { collectionId, songIds }),
    removeSongs: (collectionId, songIds) =>
      call(COMMANDS.libraryRemoveSongs, { collectionId, songIds }),
    importLocalFiles: () => call(COMMANDS.libraryImportLocalFiles),
    importLocalFolder: () => call(COMMANDS.libraryImportLocalFolder),
    removeImport: (sourceId) => call(COMMANDS.libraryRemoveImport, { sourceId }),
    getSettings: () => call(COMMANDS.getSettings),
    patchSettings: (patch: SettingsPatch) => call(COMMANDS.patchSettings, patch),
    checkForUpdate: () => call(COMMANDS.checkForUpdate),
    getUpdatePreferences: () => call(COMMANDS.getUpdatePreferences),
    patchUpdatePreferences: (patch: UpdatePatch) => call(COMMANDS.patchUpdatePreferences, patch),
    beginUpdateHandoff: (targetVersion: string) =>
      call(COMMANDS.beginUpdateHandoff, { targetVersion }),
    preparePlayback: (request: PlaybackPrepare) => call(COMMANDS.preparePlayback, request),
    startPlayback: (request: PlaybackStart) => call(COMMANDS.startPlayback, request),
    stopPlayback: (request: PlaybackSessionCommand) => call(COMMANDS.stopPlayback, request),
    pausePlayback: (request: PlaybackSessionCommand) => call(COMMANDS.pausePlayback, request),
    resumePlayback: (request: PlaybackSessionCommand) => call(COMMANDS.resumePlayback, request),
    skipPlayback: (request: PlaybackSessionCommand) => call(COMMANDS.skipPlayback, request),
    setDiagnosticsEnabled: (request: DiagnosticsSetEnabled) =>
      call(COMMANDS.setDiagnosticsEnabled, request),
    startCalibration: (request: CalibrationStart) => call(COMMANDS.startCalibration, request),
    cancelCalibration: (request: CalibrationCancel) => call(COMMANDS.cancelCalibration, request),
    subscribeUiEvents: async (listener): Promise<Unsubscribe> => {
      const channel = new Channel<UiChannelEvent>();
      channel.onmessage = (event) => listener(normalizeEvent(event));
      await invoke(UI_EVENTS_COMMAND, { channel });
      return () => {
        channel.onmessage = () => undefined;
      };
    },
    shutdown: async (failed = false) => {
      if (failed) {
        await call(COMMANDS.shutdown, { failed: true });
        return;
      }
      await call(COMMANDS.shutdown);
    },
  };
}
