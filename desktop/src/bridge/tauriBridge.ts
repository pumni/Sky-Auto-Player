import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  Bootstrap,
  DesktopBridge,
  DetailRequest,
  SearchRequest,
  SearchResult,
  Settings,
  SettingsPatch,
  PlaybackPrepare,
  PlaybackCommandAck,
  PlaybackStart,
  PlaybackSessionCommand,
  PlaybackSession,
  PreparedPlayback,
  SongDetail,
  CalibrationCancel,
  CalibrationCancelAck,
  CalibrationStart,
  CalibrationStartAck,
  DiagnosticsEnabled,
  DiagnosticsSetEnabled,
  UiEvent,
  Unsubscribe,
  ViewportRequest,
  ViewportResult,
  UpdateCheck,
  UpdateHandoff,
  UpdatePatch,
  UpdatePreferences,
} from './DesktopBridge';

type CoreChannelEvent = UiEvent;

function normalizeEvent(event: CoreChannelEvent): UiEvent {
  return event;
}

export function encodeCommandArgs(request?: unknown): Record<string, unknown> | undefined {
  return request === undefined ? undefined : { params: request };
}

function call<T>(command: string, request?: unknown): Promise<T> {
  return invoke<T>(command, encodeCommandArgs(request));
}

export function createTauriBridge(): DesktopBridge {
  return {
    bootstrap: () => call<Bootstrap>('bootstrap'),
    searchSongs: (request: SearchRequest) => call<SearchResult>('search_songs', request),
    getSongDetail: (request: DetailRequest) => call<SongDetail>('get_song_detail', request),
    reloadLibrary: () => call<{ generation: number; total: number }>('reload_library'),
    setLibraryViewport: (request: ViewportRequest) =>
      call<ViewportResult>('set_library_viewport', request),
    getSettings: () => call<Settings>('get_settings'),
    patchSettings: (patch: SettingsPatch) => call<Settings>('patch_settings', patch),
    checkForUpdate: () => call<UpdateCheck>('check_for_update'),
    getUpdatePreferences: () => call<UpdatePreferences>('get_update_preferences'),
    patchUpdatePreferences: (patch: UpdatePatch) =>
      call<UpdatePreferences>('patch_update_preferences', patch),
    beginUpdateHandoff: (targetVersion: string) =>
      call<UpdateHandoff>('begin_update_handoff', { targetVersion }),
    preparePlayback: (request: PlaybackPrepare) =>
      call<PreparedPlayback>('prepare_playback', request),
    startPlayback: (request: PlaybackStart) => call<PlaybackSession>('start_playback', request),
    stopPlayback: (request: PlaybackSessionCommand) =>
      call<PlaybackCommandAck>('stop_playback', request),
    pausePlayback: (request: PlaybackSessionCommand) =>
      call<PlaybackCommandAck>('pause_playback', request),
    resumePlayback: (request: PlaybackSessionCommand) =>
      call<PlaybackCommandAck>('resume_playback', request),
    skipPlayback: (request: PlaybackSessionCommand) =>
      call<PlaybackCommandAck>('skip_playback', request),
    setDiagnosticsEnabled: (request: DiagnosticsSetEnabled) =>
      call<DiagnosticsEnabled>('set_diagnostics_enabled', request),
    startCalibration: (request: CalibrationStart) =>
      call<CalibrationStartAck>('start_calibration', request),
    cancelCalibration: (request: CalibrationCancel) =>
      call<CalibrationCancelAck>('cancel_calibration', request),
    subscribeUiEvents: async (listener): Promise<Unsubscribe> => {
      const channel = new Channel<CoreChannelEvent>();
      channel.onmessage = (event) => listener(normalizeEvent(event));
      await invoke('subscribe_ui_events', { channel });
      return () => {
        channel.onmessage = () => undefined;
      };
    },
    shutdown: async (failed = false) => {
      if (failed) {
        await invoke('shutdown', { params: { failed: true } });
        return;
      }
      await invoke('shutdown');
    },
  };
}
