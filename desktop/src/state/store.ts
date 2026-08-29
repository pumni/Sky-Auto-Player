import { create } from 'zustand';
import type {
  Bootstrap,
  DesktopBridge,
  Settings,
  SettingsPatch,
  SongDetail,
  SongRow,
  UiEvent,
} from '../bridge/DesktopBridge';
import { initialEventState, reduceEvent } from './eventReducer';

type LoadState = 'idle' | 'loading' | 'ready' | 'fatal';

export interface DesktopStore {
  bootstrapState: LoadState;
  bootstrap: Bootstrap | null;
  fatal: string | null;
  library: {
    query: string;
    generation: number;
    total: number;
    rows: SongRow[];
    selectedSongId: string | null;
    visibleRange: { first: number; last: number };
    loading: boolean;
    searchRequestGeneration: number;
    error: string | null;
  };
  detail: { state: LoadState; value: SongDetail | null; error: string | null };
  settings: Settings | null;
  settingsState: LoadState;
  settingsOpen: boolean;
  initialize: () => Promise<void>;
  applyEvent: (event: UiEvent) => void;
  search: (query?: string) => Promise<void>;
  selectSong: (songId: string) => Promise<void>;
  setViewport: (first: number, last: number) => Promise<void>;
  reloadLibrary: () => Promise<void>;
  patchSettings: (patch: SettingsPatch) => Promise<void>;
  setSettingsOpen: (open: boolean) => void;
}

export function createDesktopStore(bridge: DesktopBridge) {
  let detailRequestToken = 0;

  return create<DesktopStore>((set, get) => ({
    bootstrapState: 'idle',
    bootstrap: null,
    fatal: null,
    library: {
      query: '',
      generation: 0,
      total: 0,
      rows: [],
      selectedSongId: null,
      visibleRange: { first: 0, last: 0 },
      loading: false,
      searchRequestGeneration: 0,
      error: null,
    },
    detail: { state: 'idle', value: null, error: null },
    settings: null,
    settingsState: 'idle',
    settingsOpen: false,

    async initialize() {
      if (get().bootstrapState === 'loading' || get().bootstrapState === 'ready') return;
      set({ bootstrapState: 'loading', fatal: null });
      try {
        await bridge.subscribeUiEvents((event) => get().applyEvent(event));
        const bootstrap = await bridge.bootstrap();
        const settings = await bridge.getSettings();
        set({
          bootstrap,
          bootstrapState: 'ready',
          settings,
          settingsState: 'ready',
          library: { ...get().library, generation: bootstrap.catalog_generation },
        });
        await get().search();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        set({ bootstrapState: 'fatal', fatal: message });
      }
    },

    applyEvent(event) {
      const eventState = reduceEvent(
        {
          ...initialEventState,
          catalogGeneration: get().library.generation,
          catalogTotal: get().library.total,
          fatal: get().fatal,
        },
        event,
      );
      if (event.name === 'core.fatal') {
        set({ fatal: eventState.fatal, bootstrapState: 'fatal' });
        return;
      }
      if (event.name === 'catalog.changed') {
        set({
          library: {
            ...get().library,
            generation: eventState.catalogGeneration,
            total: eventState.catalogTotal,
            rows: [],
            error: null,
          },
        });
        void get().search();
      }
    },

    async search(query) {
      const current = get().library;
      const nextQuery = query ?? current.query;
      const token = current.searchRequestGeneration + 1;
      const request = { query: nextQuery, offset: 0, limit: 200 } as {
        query: string;
        offset: number;
        limit: number;
        generation?: number;
      };
      if (current.generation > 0) request.generation = current.generation;
      set({
        library: {
          ...current,
          query: nextQuery,
          loading: true,
          error: null,
          searchRequestGeneration: token,
        },
      });
      try {
        const result = await bridge.searchSongs(request);
        if (get().library.searchRequestGeneration !== token) return;
        set({
          library: {
            ...get().library,
            rows: result.items,
            total: result.total,
            generation: result.generation,
            loading: false,
            error: null,
          },
        });
      } catch (error) {
        if (get().library.searchRequestGeneration !== token) return;
        const message = error instanceof Error ? error.message : String(error);
        set({ library: { ...get().library, loading: false, error: message } });
      }
    },

    async selectSong(songId) {
      detailRequestToken += 1;
      const token = detailRequestToken;
      set({
        library: { ...get().library, selectedSongId: songId },
        detail: { state: 'loading', value: null, error: null },
      });
      try {
        const request = { songId } as { songId: string; generation?: number };
        if (get().library.generation > 0) request.generation = get().library.generation;
        const value = await bridge.getSongDetail(request);
        if (token !== detailRequestToken) return;
        set({ detail: { state: 'ready', value, error: null } });
      } catch (error) {
        if (token !== detailRequestToken) return;
        const message = error instanceof Error ? error.message : String(error);
        set({ detail: { state: 'fatal', value: null, error: message } });
      }
    },

    async setViewport(first, last) {
      const library = get().library;
      set({ library: { ...library, visibleRange: { first, last } } });
      if (library.generation === 0 || first > last) return;
      try {
        await bridge.setLibraryViewport({
          generation: library.generation,
          firstIndex: first,
          lastIndex: last,
          selectedSongId: library.selectedSongId,
        });
      } catch {
        // Viewport hints are best effort. A catalog.changed event will trigger
        // a fresh search and establish the next generation.
      }
    },

    async reloadLibrary() {
      set({ library: { ...get().library, loading: true, error: null } });
      try {
        await bridge.reloadLibrary();
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        set({ library: { ...get().library, loading: false, error: message } });
      }
    },

    async patchSettings(patch) {
      set({ settingsState: 'loading' });
      try {
        const settings = await bridge.patchSettings(patch);
        set({ settings, settingsState: 'ready' });
        document.documentElement.dataset.theme = settings.theme;
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        set({ settingsState: 'fatal', fatal: message });
      }
    },

    setSettingsOpen(open) {
      set({ settingsOpen: open });
    },
  }));
}

export type DesktopStoreHook = ReturnType<typeof createDesktopStore>;
