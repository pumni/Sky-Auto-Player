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
  PlaybackStart,
  PlaybackSessionCommand,
  PlaybackSession,
  PreparedPlayback,
  SongDetail,
  UiEvent,
  Unsubscribe,
  ViewportRequest,
  ViewportResult,
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
    preparePlayback: (request: PlaybackPrepare) =>
      call<PreparedPlayback>('prepare_playback', request),
    startPlayback: (request: PlaybackStart) => call<PlaybackSession>('start_playback', request),
    stopPlayback: (request: PlaybackSessionCommand) =>
      call<Record<string, unknown>>('stop_playback', request),
    pausePlayback: (request: PlaybackSessionCommand) =>
      call<Record<string, unknown>>('pause_playback', request),
    resumePlayback: (request: PlaybackSessionCommand) =>
      call<Record<string, unknown>>('resume_playback', request),
    skipPlayback: (request: PlaybackSessionCommand) =>
      call<Record<string, unknown>>('skip_playback', request),
    subscribeUiEvents: async (listener): Promise<Unsubscribe> => {
      const channel = new Channel<CoreChannelEvent>();
      channel.onmessage = (event) => listener(normalizeEvent(event));
      await invoke('subscribe_ui_events', { channel });
      return () => {
        channel.onmessage = () => undefined;
      };
    },
    shutdown: async () => {
      await invoke('shutdown');
    },
  };
}
