import { Channel, invoke } from '@tauri-apps/api/core';
import type {
  Bootstrap,
  CatalogChangedEvent,
  CoreFatalEvent,
  DesktopBridge,
  DetailRequest,
  SearchRequest,
  SearchResult,
  Settings,
  SettingsPatch,
  SongDetail,
  UiEvent,
  Unsubscribe,
  ViewportRequest,
  ViewportResult,
} from './DesktopBridge';

type CoreChannelEvent = {
  v: number;
  name: string;
  payload: Record<string, unknown>;
};

function normalizeEvent(event: CoreChannelEvent): UiEvent {
  if (event.name === 'catalog.changed') {
    return {
      v: event.v,
      name: event.name,
      payload: event.payload as unknown as CatalogChangedEvent,
    };
  }
  if (event.name === 'core.fatal') {
    return { v: event.v, name: event.name, payload: event.payload as unknown as CoreFatalEvent };
  }
  return event;
}

function call<T>(command: string, request?: unknown): Promise<T> {
  return invoke<T>(command, request === undefined ? undefined : { request });
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
