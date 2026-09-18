import { create } from 'zustand';
import type { DesktopBridge } from '../bridge/DesktopBridge';
import { initialEventState, reduceEvent } from './eventReducer';
import { boundedText, eventDetail } from './eventHelpers';
import { createLibrarySlice } from './librarySlice';
import { createSettingsSlice } from './settingsSlice';
import { createDiagnosticsSlice } from './diagnosticsSlice';
import { createPlaybackSlice } from './playbackSlice';
import { MAX_DIAGNOSTIC_EVENTS, MAX_DIAGNOSTIC_SAMPLES } from './types';
import type { CalibrationUiState, DesktopStore, DiagnosticsEventLine } from './types';
import type { LibrarySlice } from './librarySlice';

export {
  createShuffleTraversal,
  nextPlaybackPosition,
  playbackContextMatchesLibrary,
  playbackIssuePresentation,
  playbackContextIndexAt,
  previousPlaybackPosition,
  selectNowPlayingSongId,
  selectRowAtIndex,
  selectSelectedDetail,
  selectSongById,
} from './selectors';
export {
  AUTO_PLAY_HANDOFF_MS,
  DETAIL_CACHE_LIMIT,
  LIBRARY_PAGE_SIZE,
  MAX_DIAGNOSTIC_EVENTS,
  MAX_DIAGNOSTIC_LINE_LENGTH,
  MAX_DIAGNOSTIC_SAMPLES,
} from './types';
export type {
  CalibrationUiState,
  DesktopStore,
  DetailEntry,
  DiagnosticsEventLine,
  LibraryNavigationState,
  LibraryState,
  LoadState,
  PlaybackContext,
  PlaybackIssueKind,
  PlaybackIssuePresentation,
  PlaybackSongIdentity,
  PlaybackUiState,
  ShuffleTraversal,
  TransportOperation,
  UtilityView,
} from './types';
export function createDesktopStore(bridge: DesktopBridge) {
  let detailRequestToken = 0;
  let diagnosticsEventSeq = 0;
  let cancelStaleAutoAdvanceHandoff = (_state: DesktopStore): void => undefined;
  const store = create<DesktopStore>((set, get) => {
    let loadPage: LibrarySlice['loadPage'] = async () => {
      throw new Error('Library slice is not initialized.');
    };
    const playbackSlice = createPlaybackSlice({
      bridge,
      get,
      set,
      loadPage: (...args) => loadPage(...args),
    });
    cancelStaleAutoAdvanceHandoff = playbackSlice.cancelStaleAutoAdvanceHandoff;
    const librarySlice = createLibrarySlice({
      bridge,
      get,
      set,
      clearOperationFromEvent: playbackSlice.clearOperationFromEvent,
      invalidatePlaybackMembershipContext: playbackSlice.invalidatePlaybackMembershipContext,
      playbackAfterBrowserNavigation: playbackSlice.playbackAfterBrowserNavigation,
      nextDetailRequestToken: () => {
        detailRequestToken += 1;
        return detailRequestToken;
      },
      isCurrentDetailRequest: (token) => token === detailRequestToken,
    });
    loadPage = librarySlice.loadPage;
    const settingsSlice = createSettingsSlice({
      bridge,
      get,
      set,
      clearOperationFromEvent: playbackSlice.clearOperationFromEvent,
      onAutoPlayChanged: playbackSlice.onAutoPlayChanged,
    });
    const diagnosticsSlice = createDiagnosticsSlice({ bridge, get, set });

    return {
      bootstrapState: 'idle',
      bootstrap: null,
      fatal: null,
      library: {
        source: { kind: 'smart', id: 'all' },
        searchSource: { kind: 'smart', id: 'all' },
        playlistAddMode: null,
        query: '',
        generation: 0,
        catalogTotal: 0,
        likedTotal: 0,
        resultTotal: 0,
        pages: new Map(),
        indexById: new Map(),

        selectedSongId: null,
        visibleRange: { first: 0, last: 0 },
        loading: false,
        searchRequestGeneration: 0,
        error: null,
      },
      libraryNavigation: {
        loadState: 'idle',
        playlistOrder: [],
        playlistsById: new Map(),
        pendingMutations: new Set(),
        lastError: null,
      },
      details: { bySongId: new Map() },
      settings: null,
      settingsState: 'idle',
      settingsOpen: false,
      utility: {
        open: false,
        activeView: 'details',
      },
      diagnostics: {
        enabled: false,
        samples: [],
        events: [],
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
        dialogOpen: false,
        checkRequestPending: false,
        installRequestPending: false,
        transportError: null,
        lastNativeRevision: 0,

        state: 'idle',
        currentVersion: null,
        availableVersion: null,
        channel: 'stable',
        releaseNotes: null,
        publishedAt: null,
        errorCode: null,
        errorDetail: null,
        retryAction: 'none',
        handoffId: null,
        progress: null,
      },
      playback: {
        shuffleEnabled: false,
        state: 'idle',
        sessionId: null,
        currentSong: null,
        context: null,
        preparedIdentity: null,
        preparedContext: null,
        startRequestId: null,
        transportOperation: null,
        prepared: null,
        snapshot: null,
        error: null,
        statusRetryPending: false,
      },
      ...librarySlice.actions,
      ...settingsSlice,
      ...diagnosticsSlice,
      ...playbackSlice.actions,

      async initialize() {
        if (get().bootstrapState === 'loading' || get().bootstrapState === 'ready') return;
        set({ bootstrapState: 'loading', fatal: null });
        try {
          await bridge.subscribeUiEvents((event) => get().applyEvent(event));
          const bootstrap = await bridge.bootstrap();
          const settings = bootstrap.settings;
          const currentLibrary = get().library;
          const catalogFailed =
            currentLibrary.error !== null || bootstrap.catalog_state === 'failed';
          set({
            bootstrap,
            bootstrapState: 'ready',
            settings,
            settingsState: 'ready',
            library: {
              ...currentLibrary,
              generation: Math.max(currentLibrary.generation, bootstrap.catalog_generation ?? 0),
              loading: catalogFailed
                ? false
                : bootstrap.catalog_state !== 'ready' || currentLibrary.loading,
              error: catalogFailed
                ? (currentLibrary.error ?? 'Catalog load failed. Retry the library.')
                : currentLibrary.error,
            },
          });
          if (bootstrap.catalog_state === 'ready' || get().library.generation > 0) {
            await librarySlice.reconcileCatalog();
          }
          void get().checkForUpdate('background');
        } catch (error) {
          const message = error instanceof Error ? error.message : String(error);
          set({ bootstrapState: 'fatal', fatal: message });
        }
      },

      applyEvent(event) {
        if (event.name === 'diagnostics.snapshot') {
          const current = get().diagnostics;
          if (!current.enabled) return;
          const playback = get().playback;
          const sessionId = event.payload.session_id;
          if (
            !sessionId ||
            playback.sessionId !== sessionId ||
            playbackSlice.isSessionRetired(sessionId) ||
            !['starting', 'playing', 'paused', 'stopping'].includes(playback.state)
          ) {
            return;
          }
          const previous = current.samples.at(-1);
          const samples =
            previous && previous.session_id !== event.payload.session_id
              ? [event.payload]
              : [...current.samples, event.payload].slice(-MAX_DIAGNOSTIC_SAMPLES);
          set({ diagnostics: { ...current, samples } });
          return;
        }

        diagnosticsEventSeq += 1;
        const shouldRecordEvent = !['playback.snapshot', 'calibration.progress'].includes(
          event.name,
        );
        if (shouldRecordEvent) {
          const detail = boundedText(eventDetail(event));
          const diagnostics = get().diagnostics;
          const eventLine: DiagnosticsEventLine = {
            seq: diagnosticsEventSeq,
            name: event.name,
            detail,
            timestamp: Date.now(),
          };
          set({
            diagnostics: {
              ...diagnostics,
              events: [...diagnostics.events, eventLine].slice(-MAX_DIAGNOSTIC_EVENTS),
            },
          });
        }
        const eventState = reduceEvent(
          {
            ...initialEventState,
            catalogGeneration: get().library.generation,
            catalogTotal: get().library.catalogTotal,
            fatal: get().fatal,
          },
          event,
        );
        if (event.name === 'core.fatal') {
          detailRequestToken += 1;
          playbackSlice.clearOperationFromEvent();
          set({
            fatal: eventState.fatal,
            bootstrapState: 'fatal',
            playback: {
              ...get().playback,
              state: 'failed',
              startRequestId: null,
              transportOperation: null,
              error: eventState.fatal,
            },
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
          const playback = get().playback;
          const cancelPreparing = playback.transportOperation === 'preparing';
          if (cancelPreparing) playbackSlice.clearOperationFromEvent();
          set({
            library: {
              ...get().library,
              generation: eventState.catalogGeneration,
              catalogTotal: eventState.catalogTotal,
              resultTotal: eventState.catalogTotal,
              pages: new Map(),
              indexById: new Map(),
              selectedSongId: null,
              error: null,
              loading: true,
            },
            playback: {
              ...playback,
              prepared: null,
              preparedIdentity: null,
              preparedContext: null,
              transportOperation: cancelPreparing ? null : playback.transportOperation,
            },
          });
          set({ details: { bySongId: new Map() } });
          void librarySlice.reconcileCatalog();
          return;
        }
        if (event.name === 'catalog.load_failed') {
          set({
            library: {
              ...get().library,
              loading: false,
              error: event.payload.message,
            },
          });
          return;
        }
        if (playbackSlice.handleEvent(event)) return;
        if (event.name === 'calibration.progress') {
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
            });
          }
        } else if (event.name === 'update.changed') {
          const current = get().update;
          if (event.payload.revision <= current.lastNativeRevision) return;
          set({
            update: {
              ...current,
              dialogOpen: current.dialogOpen,
              checkRequestPending: current.checkRequestPending,
              installRequestPending: current.installRequestPending,
              transportError: null,
              lastNativeRevision: event.payload.revision,

              state: event.payload.state,
              currentVersion: event.payload.current_version,
              availableVersion: event.payload.available_version,
              channel: event.payload.channel,
              releaseNotes: event.payload.release_notes,
              publishedAt: event.payload.published_at,
              errorCode: event.payload.error_code,
              errorDetail: event.payload.error_detail,
              retryAction: event.payload.retry_action,
              handoffId: event.payload.operation_id,
              progress: event.payload.progress
                ? {
                    completed: event.payload.progress.completed,
                    total: event.payload.progress.total,
                    message: event.payload.progress.message,
                  }
                : null,
            },
          });
        }
      },
    };
  });
  store.subscribe((state) => {
    cancelStaleAutoAdvanceHandoff(state);
  });
  return store;
}

export type DesktopStoreHook = ReturnType<typeof createDesktopStore>;
