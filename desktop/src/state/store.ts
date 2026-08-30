import { create } from 'zustand';
import type {
  Bootstrap,
  CalibrationFinished,
  CalibrationModeId,
  CalibrationProgress,
  CalibrationStart,
  DesktopBridge,
  DiagnosticsSnapshot,
  SearchRequest,
  Settings,
  SettingsPatch,
  SearchResult,
  SongDetail,
  SongRow,
  UiEvent,
  PlaybackConfig,
  PlaybackDecisionId,
  PlaybackDecisionAcceptance,
  PlaybackPrepare,
  PreparedPlayback,
  UpdateCheck,
  UpdateChannelId,
} from '../bridge/DesktopBridge';
import { initialEventState, reduceEvent } from './eventReducer';

type LoadState = 'idle' | 'loading' | 'ready' | 'fatal';
type PlaybackUiState =
  'idle' | 'starting' | 'playing' | 'paused' | 'stopping' | 'finished' | 'failed';
type CalibrationUiState =
  'idle' | 'starting' | 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled';

export const MAX_DIAGNOSTIC_SAMPLES = 600;
export const MAX_DIAGNOSTIC_EVENTS = 500;
export const MAX_DIAGNOSTIC_LOGS = 200;
export const MAX_DIAGNOSTIC_LINE_LENGTH = 4096;

export interface DiagnosticsEventLine {
  seq: number;
  name: string;
  detail: string;
}

export interface DiagnosticsLogLine {
  seq: number;
  level: 'info' | 'warning' | 'error';
  message: string;
}

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
  diagnostics: {
    open: boolean;
    enabled: boolean;
    samples: DiagnosticsSnapshot[];
    events: DiagnosticsEventLine[];
    logs: DiagnosticsLogLine[];
    error: string | null;
  };
  calibration: {
    open: boolean;
    operationId: string | null;
    state: CalibrationUiState;
    phase: string;
    completed: number;
    total: number;
    message: string;
    result: CalibrationFinished | null;
    error: string | null;
  };
  update: {
    state: UpdateCheck['state'];
    dialogOpen: boolean;
    currentVersion: string | null;
    availableVersion: string | null;
    channel: UpdateChannelId;
    releaseNotes: string | null;
    publishedAt: string | null;
    error: string | null;
    handoffId: string | null;
  };
  playback: {
    state: PlaybackUiState;
    sessionId: string | null;
    songTitle: string | null;
    pendingCommand: 'pause' | 'resume' | null;
    prepared: PreparedPlayback | null;
    snapshot: Extract<UiEvent, { name: 'playback.snapshot' }>['payload'] | null;
    error: string | null;
  };
  initialize: () => Promise<void>;
  applyEvent: (event: UiEvent) => void;
  search: (query?: string) => Promise<void>;
  selectSong: (songId: string) => Promise<void>;
  setViewport: (first: number, last: number) => Promise<void>;
  reloadLibrary: () => Promise<void>;
  patchSettings: (patch: SettingsPatch) => Promise<void>;
  checkForUpdate: () => Promise<void>;
  setUpdateDialogOpen: (open: boolean) => void;
  beginUpdateHandoff: () => Promise<void>;
  prepareSelectedPlayback: (overrides?: Partial<PlaybackConfig>) => Promise<void>;
  startPreparedPlayback: (decision?: PlaybackDecisionId) => Promise<void>;
  stopPlayback: () => Promise<void>;
  pausePlayback: () => Promise<void>;
  resumePlayback: () => Promise<void>;
  skipPlayback: () => Promise<void>;
  setSettingsOpen: (open: boolean) => void;
  setDiagnosticsOpen: (open: boolean) => void;
  setDiagnosticsEnabled: (enabled: boolean) => Promise<void>;
  startCalibration: (mode?: CalibrationModeId) => Promise<void>;
  cancelCalibration: () => Promise<void>;
  setCalibrationOpen: (open: boolean) => void;
}

export function createDesktopStore(bridge: DesktopBridge) {
  let detailRequestToken = 0;
  let prepareRequestEpoch = 0;
  let startRequestEpoch = 0;
  let pendingStart: {
    epoch: number;
    preparedId: string;
    songId: string;
    songTitle: string;
  } | null = null;
  let settingsMutationTail: Promise<void> = Promise.resolve();
  let diagnosticsToggleEpoch = 0;
  const retiredSessionIds = new Set<string>();
  const pageSize = 200;
  const pageCache = new Map<string, Map<number, SearchResult>>();
  const pageRequests = new Map<string, Promise<SearchResult>>();
  let diagnosticsEventSeq = 0;
  let diagnosticsLogSeq = 0;

  const cacheKey = (query: string, generation: number) => `${generation}\u0000${query}`;

  const boundedText = (value: string): string => {
    const normalized = value.replace(/[\u0000\r\n\t]/g, ' ');
    const encoder = new TextEncoder();
    const decoder = new TextDecoder();
    if (encoder.encode(normalized).length <= MAX_DIAGNOSTIC_LINE_LENGTH) {
      return normalized;
    }
    let bounded = decoder.decode(encoder.encode(normalized).slice(0, MAX_DIAGNOSTIC_LINE_LENGTH));
    while (encoder.encode(bounded).length > MAX_DIAGNOSTIC_LINE_LENGTH) {
      bounded = bounded.slice(0, -1);
    }
    return bounded;
  };

  const eventDetail = (event: UiEvent): string => {
    switch (event.name) {
      case 'core.ready':
        return `Protocol ${event.payload.protocol_version} ready`;
      case 'core.fatal':
        return `${event.payload.code}: ${event.payload.message}`;
      case 'catalog.changed':
        return `Generation ${event.payload.generation}, ${event.payload.total} songs`;
      case 'diagnostics.snapshot':
        return `p95 ${event.payload.p95_ms.toFixed(2)} ms; max ${event.payload.max_lateness_us} μs`;
      case 'calibration.progress':
        return `${event.payload.phase}: ${event.payload.completed}/${event.payload.total}`;
      case 'calibration.finished':
        return `${event.payload.outcome}: ${event.payload.status}`;
      case 'update.available':
        return `Update ${event.payload.available_version} is available`;
      case 'update.result':
        return `Update check: ${event.payload.state}`;
      case 'update.handoff_ready':
        return `Update handoff ready for ${event.payload.target_version}`;
      case 'playback.state_changed':
        return `${event.payload.song_id} → ${event.payload.state}`;
      case 'playback.snapshot':
        return `${event.payload.title}: ${event.payload.state}`;
      case 'playback.finished':
        return `${event.payload.song_id}: ${event.payload.outcome}`;
      case 'playback.failed':
        return `${event.payload.code}: ${event.payload.message}`;
    }
  };

  return create<DesktopStore>((set, get) => {
    const acceptsSessionEvent = (sessionId: string, songId: string): boolean => {
      const current = get().playback;
      if (retiredSessionIds.has(sessionId)) return false;
      if (current.sessionId === sessionId) return true;
      if (
        pendingStart &&
        pendingStart.songId === songId &&
        (current.state === 'idle' || current.state === 'finished' || current.state === 'failed')
      ) {
        // The Core may publish the first state/terminal event before the
        // start promise continuation runs. Bind that event to the pending
        // start, while still rejecting events from an unrelated song/session.
        return true;
      }
      return !current.sessionId && pendingStart?.songId === songId;
    };

    const bindSessionTitle = (sessionId: string, songId: string): string | null => {
      const current = get().playback;
      if (current.sessionId === sessionId && current.songTitle) return current.songTitle;
      if (pendingStart?.songId === songId) return pendingStart.songTitle;
      return current.songTitle;
    };

    const mergePage = (result: SearchResult, token: number): boolean => {
      const current = get().library;
      if (current.searchRequestGeneration !== token) return false;
      const rows =
        current.rows.length === result.total
          ? [...current.rows]
          : (new Array<SongRow | undefined>(result.total) as SongRow[]);
      result.items.forEach((row, index) => {
        const target = result.offset + index;
        if (target < rows.length) rows[target] = row;
      });
      set({
        library: {
          ...current,
          rows,
          total: result.total,
          generation: result.generation,
        },
      });
      return true;
    };

    const loadPage = async (
      query: string,
      offset: number,
      generation: number,
      token: number,
    ): Promise<SearchResult> => {
      const key = cacheKey(query, generation);
      const cached = pageCache.get(key)?.get(offset);
      if (cached) {
        mergePage(cached, token);
        return cached;
      }
      const inFlight = pageRequests.get(`${key}\u0000${offset}`);
      if (inFlight) return inFlight;

      const request: SearchRequest = {
        query,
        offset,
        limit: pageSize,
        ...(generation > 0 ? { generation } : {}),
      };
      const requestPromise = bridge.searchSongs(request);
      const requestKey = `${key}\u0000${offset}`;
      pageRequests.set(requestKey, requestPromise);
      try {
        const result = await requestPromise;
        const generationPages = pageCache.get(cacheKey(query, result.generation)) ?? new Map();
        generationPages.set(result.offset, result);
        pageCache.set(cacheKey(query, result.generation), generationPages);
        mergePage(result, token);
        return result;
      } finally {
        if (pageRequests.get(requestKey) === requestPromise) pageRequests.delete(requestKey);
      }
    };

    const ensureRange = async (first: number, last: number, token: number): Promise<void> => {
      if (last < first) return;
      const current = get().library;
      if (current.total === 0) return;
      const offsets: number[] = [];
      const firstPage = Math.floor(Math.max(0, first) / pageSize) * pageSize;
      const lastPage = Math.floor(Math.max(0, last) / pageSize) * pageSize;
      for (let offset = firstPage; offset <= lastPage; offset += pageSize) offsets.push(offset);
      try {
        await Promise.all(
          offsets.map((offset) => loadPage(current.query, offset, current.generation, token)),
        );
      } catch (error) {
        if (get().library.searchRequestGeneration !== token) return;
        const message = error instanceof Error ? error.message : String(error);
        set({ library: { ...get().library, loading: false, error: message } });
      }
    };

    return {
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
      diagnostics: {
        open: false,
        enabled: false,
        samples: [],
        events: [],
        logs: [],
        error: null,
      },
      calibration: {
        open: false,
        operationId: null,
        state: 'idle',
        phase: '',
        completed: 0,
        total: 0,
        message: '',
        result: null,
        error: null,
      },
      update: {
        state: 'idle',
        dialogOpen: false,
        currentVersion: null,
        availableVersion: null,
        channel: 'stable',
        releaseNotes: null,
        publishedAt: null,
        error: null,
        handoffId: null,
      },
      playback: {
        state: 'idle',
        sessionId: null,
        songTitle: null,
        pendingCommand: null,
        prepared: null,
        snapshot: null,
        error: null,
      },

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
          if (bootstrap.update_preferences.auto_check) void get().checkForUpdate();
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          set({ bootstrapState: 'fatal', fatal: message });
        }
      },

      applyEvent(event) {
        diagnosticsEventSeq += 1;
        const detail = boundedText(eventDetail(event));
        const level: DiagnosticsLogLine['level'] =
          event.name === 'core.fatal' || event.name === 'playback.failed' ? 'error' : 'info';
        const diagnostics = get().diagnostics;
        const eventLine: DiagnosticsEventLine = {
          seq: diagnosticsEventSeq,
          name: event.name,
          detail,
        };
        diagnosticsLogSeq += 1;
        const logLine: DiagnosticsLogLine = { seq: diagnosticsLogSeq, level, message: detail };
        set({
          diagnostics: {
            ...diagnostics,
            events: [...diagnostics.events, eventLine].slice(-MAX_DIAGNOSTIC_EVENTS),
            logs: [...diagnostics.logs, logLine].slice(-MAX_DIAGNOSTIC_LOGS),
          },
        });
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
          detailRequestToken += 1;
          set({
            fatal: eventState.fatal,
            bootstrapState: 'fatal',
            playback: { ...get().playback, state: 'failed', error: eventState.fatal },
            calibration: {
              ...get().calibration,
              state:
                get().calibration.state === 'running' || get().calibration.state === 'starting'
                  ? 'failed'
                  : get().calibration.state,
              error: eventState.fatal,
            },
          });
          return;
        }
        if (event.name === 'catalog.changed') {
          if (eventState.catalogGeneration <= get().library.generation) return;
          detailRequestToken += 1;
          prepareRequestEpoch += 1;
          set({
            library: {
              ...get().library,
              generation: eventState.catalogGeneration,
              total: eventState.catalogTotal,
              rows: [],
              selectedSongId: null,
              error: null,
              loading: true,
            },
            playback: { ...get().playback, prepared: null },
          });
          set({ detail: { state: 'idle', value: null, error: null } });
          void get().search();
        }
        if (event.name === 'playback.state_changed') {
          const current = get().playback;
          if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return;
          const songTitle = bindSessionTitle(event.payload.session_id, event.payload.song_id);
          if (pendingStart?.songId === event.payload.song_id) pendingStart = null;
          set({
            playback: {
              ...current,
              sessionId: event.payload.session_id,
              songTitle,
              pendingCommand:
                (current.pendingCommand === 'pause' && event.payload.state === 'paused') ||
                (current.pendingCommand === 'resume' && event.payload.state === 'playing')
                  ? null
                  : current.pendingCommand,
              snapshot: current.sessionId === event.payload.session_id ? current.snapshot : null,
              state:
                event.payload.state === 'failed'
                  ? 'failed'
                  : (event.payload.state as PlaybackUiState),
              error: event.payload.message,
            },
          });
        } else if (event.name === 'playback.snapshot') {
          const current = get().playback;
          if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return;
          if (pendingStart?.songId === event.payload.song_id) pendingStart = null;
          set({
            playback: {
              ...current,
              sessionId: event.payload.session_id,
              songTitle: event.payload.title,
              pendingCommand:
                (current.pendingCommand === 'pause' && event.payload.state === 'paused') ||
                (current.pendingCommand === 'resume' && event.payload.state === 'playing')
                  ? null
                  : current.pendingCommand,
              state: event.payload.state as PlaybackUiState,
              snapshot: event.payload,
              error: event.payload.message,
            },
          });
        } else if (event.name === 'playback.finished') {
          const current = get().playback;
          if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return;
          retiredSessionIds.add(event.payload.session_id);
          const songTitle = bindSessionTitle(event.payload.session_id, event.payload.song_id);
          if (pendingStart?.songId === event.payload.song_id) pendingStart = null;
          set({
            playback: {
              ...current,
              sessionId: event.payload.session_id,
              songTitle,
              pendingCommand: null,
              state: 'finished',
              error: null,
            },
          });
        } else if (event.name === 'playback.failed') {
          const current = get().playback;
          if (!acceptsSessionEvent(event.payload.session_id, event.payload.song_id)) return;
          retiredSessionIds.add(event.payload.session_id);
          const songTitle = bindSessionTitle(event.payload.session_id, event.payload.song_id);
          if (pendingStart?.songId === event.payload.song_id) pendingStart = null;
          set({
            playback: {
              ...current,
              sessionId: event.payload.session_id,
              songTitle,
              pendingCommand: null,
              state: 'failed',
              error: `${event.payload.code}: ${event.payload.message}`,
            },
          });
        } else if (event.name === 'diagnostics.snapshot') {
          const current = get().diagnostics;
          if (!current.enabled || !current.open) return;
          set({
            diagnostics: {
              ...current,
              samples: [...current.samples, event.payload].slice(-MAX_DIAGNOSTIC_SAMPLES),
            },
          });
        } else if (event.name === 'calibration.progress') {
          const current = get().calibration;
          if (current.operationId && current.operationId !== event.payload.operation_id) return;
          set({
            calibration: {
              ...current,
              operationId: current.operationId ?? event.payload.operation_id,
              state: event.payload.state as CalibrationUiState,
              phase: boundedText(event.payload.phase),
              completed: event.payload.completed,
              total: event.payload.total,
              message: boundedText(event.payload.message),
              error: null,
            },
          });
        } else if (event.name === 'calibration.finished') {
          const current = get().calibration;
          if (current.operationId && current.operationId !== event.payload.operation_id) return;
          if (['succeeded', 'failed', 'cancelled'].includes(current.state)) return;
          const state: CalibrationUiState =
            event.payload.outcome === 'succeeded'
              ? 'succeeded'
              : event.payload.outcome === 'cancelled'
                ? 'cancelled'
                : 'failed';
          set({
            calibration: {
              ...current,
              operationId: current.operationId ?? event.payload.operation_id,
              state,
              message: boundedText(event.payload.message),
              result: event.payload,
              error: state === 'failed' ? boundedText(event.payload.message) : null,
            },
          });
          if (state === 'succeeded') {
            void bridge.getSettings().then((settings) => {
              set({ settings, settingsState: 'ready' });
              prepareRequestEpoch += 1;
            });
          }
        } else if (event.name === 'update.available') {
          set({
            update: {
              ...get().update,
              state: 'available',
              currentVersion: event.payload.current_version,
              availableVersion: event.payload.available_version,
              channel: event.payload.channel,
              releaseNotes: event.payload.release_notes,
              publishedAt: event.payload.published_at,
              error: null,
              dialogOpen: get().update.dialogOpen,
            },
          });
        } else if (event.name === 'update.result') {
          set({
            update: {
              ...get().update,
              state: event.payload.state,
              currentVersion: event.payload.current_version,
              availableVersion: event.payload.available_version,
              channel: event.payload.channel,
              error: event.payload.error,
            },
          });
        } else if (event.name === 'update.handoff_ready') {
          set({
            update: {
              ...get().update,
              state: 'handoff_ready',
              handoffId: event.payload.handoff_id,
              availableVersion: event.payload.target_version,
              error: null,
            },
          });
        }
      },

      async search(query) {
        const current = get().library;
        const nextQuery = query ?? current.query;
        const token = current.searchRequestGeneration + 1;
        set({
          library: {
            ...current,
            query: nextQuery,
            rows: [],
            total: 0,
            loading: true,
            error: null,
            searchRequestGeneration: token,
          },
        });
        try {
          const result = await loadPage(nextQuery, 0, current.generation, token);
          if (get().library.searchRequestGeneration !== token) return;
          set({
            library: {
              ...get().library,
              rows: get().library.rows,
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
        prepareRequestEpoch += 1;
        const token = detailRequestToken;
        const requestGeneration = get().library.generation;
        const currentPlayback = get().playback;
        const sessionIsActive = ['starting', 'playing', 'paused', 'stopping'].includes(
          currentPlayback.state,
        );
        set({
          library: { ...get().library, selectedSongId: songId },
          detail: { state: 'loading', value: null, error: null },
          playback: {
            ...currentPlayback,
            prepared: null,
            songTitle: sessionIsActive ? currentPlayback.songTitle : null,
            error: null,
          },
        });
        try {
          const request = { songId } as { songId: string; generation?: number };
          if (requestGeneration > 0) request.generation = requestGeneration;
          const value = await bridge.getSongDetail(request);
          if (token !== detailRequestToken) return;
          if (get().library.generation !== requestGeneration) return;
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
        void ensureRange(first, last, library.searchRequestGeneration);

        // The Core viewport contract is expressed in full-catalog indices. A
        // filtered result has its own index space, so sending those indices as
        // metadata-priority hints would select unrelated songs. Paging still
        // loads the filtered window above; only an unfiltered view sends the
        // full-catalog hint.
        if (library.query.trim() !== '') return;
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
        const mutation = settingsMutationTail.then(async () => {
          set({ settingsState: 'loading' });
          try {
            const settings = await bridge.patchSettings(patch);
            set({
              settings,
              settingsState: 'ready',
              playback: { ...get().playback, prepared: null },
            });
            prepareRequestEpoch += 1;
            document.documentElement.dataset.theme = settings.theme;
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            set({ settingsState: 'fatal', fatal: message });
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
        set({ update: { ...get().update, state: 'handoff_in_progress', error: null } });
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
          // The Core has completed its authoritative handoff. Shutdown keeps
          // the existing cleanup lifecycle in charge before the shell exits.
          await bridge.shutdown();
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

      async prepareSelectedPlayback(overrides) {
        const selectedSongId = get().library.selectedSongId;
        const settings = get().settings;
        if (!selectedSongId || !settings) {
          set({
            playback: { ...get().playback, error: 'Select a song before preparing playback.' },
          });
          return;
        }
        const config: PlaybackConfig = {
          hold_frames: overrides?.hold_frames ?? settings.playback_defaults.hold_frames,
          tempo_scale: overrides?.tempo_scale ?? settings.playback_defaults.tempo_scale,
          fps: overrides?.fps ?? settings.playback_defaults.fps,
          dry_run: overrides?.dry_run ?? false,
        };
        const request: PlaybackPrepare = {
          songId: selectedSongId,
          generation: get().library.generation,
          config,
        };
        const requestEpoch = prepareRequestEpoch;
        const requestGeneration = request.generation;
        try {
          const prepared = await bridge.preparePlayback(request);
          if (
            requestEpoch !== prepareRequestEpoch ||
            get().library.selectedSongId !== selectedSongId ||
            get().library.generation !== requestGeneration
          ) {
            return;
          }
          set({ playback: { ...get().playback, prepared, error: prepared.error_message } });
        } catch (error) {
          if (
            requestEpoch !== prepareRequestEpoch ||
            get().library.selectedSongId !== selectedSongId ||
            get().library.generation !== requestGeneration
          ) {
            return;
          }
          set({
            playback: {
              ...get().playback,
              prepared: null,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      async startPreparedPlayback(decision) {
        const prepared = get().playback.prepared;
        if (!prepared?.prepared_id) {
          set({ playback: { ...get().playback, error: 'Prepare playback before starting.' } });
          return;
        }
        const decisions: PlaybackDecisionAcceptance[] = decision
          ? [{ decision, accepted: true }]
          : [];
        const startEpoch = ++startRequestEpoch;
        const preparedTitle = prepared.song.title;
        pendingStart = {
          epoch: startEpoch,
          preparedId: prepared.prepared_id,
          songId: prepared.song.song_id,
          songTitle: preparedTitle,
        };
        try {
          const session = await bridge.startPlayback({
            preparedId: prepared.prepared_id,
            decisions,
          });
          if (startEpoch !== startRequestEpoch) return;
          pendingStart = null;
          const current = get().playback;
          const boundToEarlyEvent = current.sessionId === session.session_id;
          set({
            playback: {
              ...current,
              prepared: null,
              sessionId: session.session_id,
              songTitle: current.songTitle ?? preparedTitle,
              state: boundToEarlyEvent ? current.state : 'starting',
              snapshot: boundToEarlyEvent ? current.snapshot : null,
              error: null,
            },
          });
        } catch (error) {
          if (startEpoch === startRequestEpoch) pendingStart = null;
          set({
            playback: {
              ...get().playback,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      async stopPlayback() {
        const sessionId = get().playback.sessionId;
        if (!sessionId) return;
        try {
          await bridge.stopPlayback({ sessionId });
          set({ playback: { ...get().playback, pendingCommand: null } });
        } catch (error) {
          set({
            playback: {
              ...get().playback,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      async pausePlayback() {
        const sessionId = get().playback.sessionId;
        if (!sessionId) return;
        try {
          const ack = await bridge.pausePlayback({ sessionId });
          set({ playback: { ...get().playback, pendingCommand: ack.pending_command } });
        } catch (error) {
          set({
            playback: {
              ...get().playback,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      async resumePlayback() {
        const sessionId = get().playback.sessionId;
        if (!sessionId) return;
        try {
          const ack = await bridge.resumePlayback({ sessionId });
          set({ playback: { ...get().playback, pendingCommand: ack.pending_command } });
        } catch (error) {
          set({
            playback: {
              ...get().playback,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      async skipPlayback() {
        const sessionId = get().playback.sessionId;
        if (!sessionId) return;
        try {
          await bridge.skipPlayback({ sessionId });
          set({ playback: { ...get().playback, pendingCommand: null } });
        } catch (error) {
          set({
            playback: {
              ...get().playback,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      setSettingsOpen(open) {
        set({ settingsOpen: open });
      },

      async setDiagnosticsEnabled(enabled) {
        const epoch = ++diagnosticsToggleEpoch;
        const current = get().diagnostics;
        set({ diagnostics: { ...current, enabled: false, error: null } });
        try {
          const result = await bridge.setDiagnosticsEnabled({ enabled });
          if (epoch !== diagnosticsToggleEpoch) return;
          set({
            diagnostics: {
              ...get().diagnostics,
              enabled: result.enabled,
              error: null,
            },
          });
        } catch (error) {
          if (epoch !== diagnosticsToggleEpoch) return;
          set({
            diagnostics: {
              ...get().diagnostics,
              enabled: false,
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      setDiagnosticsOpen(open) {
        if (get().diagnostics.open === open) return;
        set({ diagnostics: { ...get().diagnostics, open } });
        void get().setDiagnosticsEnabled(open);
      },

      async startCalibration(mode = 'quick') {
        if (['starting', 'running', 'cancelling'].includes(get().calibration.state)) return;
        set({
          calibration: {
            ...get().calibration,
            open: true,
            operationId: null,
            state: 'starting',
            phase: mode,
            completed: 0,
            total: 0,
            message: 'Starting calibration…',
            result: null,
            error: null,
          },
        });
        try {
          const ack = await bridge.startCalibration({
            mode,
            className: null,
            polyphony: null,
            samples: null,
            timeoutSeconds: null,
          } satisfies CalibrationStart);
          const current = get().calibration;
          if (current.operationId && current.operationId !== ack.operation_id) return;
          if (['succeeded', 'failed', 'cancelled'].includes(current.state)) return;
          set({
            calibration: {
              ...current,
              operationId: ack.operation_id,
              state: ack.state as CalibrationUiState,
              error: null,
            },
          });
        } catch (error) {
          set({
            calibration: {
              ...get().calibration,
              state: 'failed',
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      async cancelCalibration() {
        const operationId = get().calibration.operationId;
        if (
          !operationId ||
          ['succeeded', 'failed', 'cancelled'].includes(get().calibration.state)
        ) {
          return;
        }
        set({ calibration: { ...get().calibration, state: 'cancelling' } });
        try {
          const ack = await bridge.cancelCalibration({ operationId });
          set({ calibration: { ...get().calibration, state: ack.state as CalibrationUiState } });
        } catch (error) {
          set({
            calibration: {
              ...get().calibration,
              state: 'failed',
              error: error instanceof Error ? error.message : String(error),
            },
          });
        }
      },

      setCalibrationOpen(open) {
        const current = get().calibration;
        if (!open && ['starting', 'running', 'cancelling'].includes(current.state)) return;
        set({ calibration: { ...current, open } });
      },
    };
  });
}

export type DesktopStoreHook = ReturnType<typeof createDesktopStore>;
