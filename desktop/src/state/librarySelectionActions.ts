import type {
  DesktopBridge,
  LibrarySource,
  SheetConversionMode,
  SheetPitch,
} from '../bridge/DesktopBridge';
import { cacheDetail, sourceKey } from './libraryHelpers';
import { playbackContextMatchesLibrary, selectSongById } from './selectors';
import type { DetailEntry, DesktopStore } from './types';
import type { LibraryPlaybackCoordinator } from './libraryPlaybackCoordinator';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

function cachedDetailMatchesSelection(
  entry: DetailEntry | undefined,
  conversionMode: SheetConversionMode,
  targetPitch: SheetPitch | null,
): entry is DetailEntry {
  if (entry?.state !== 'ready' || !entry.value) return false;
  const sheet = entry.value.sheet_analysis;
  if (!sheet || sheet.schema_version === null) return true;
  if (sheet.readiness === 'unsupported' || sheet.readiness === 'malformed') {
    return conversionMode.kind === 'merge_playable' && targetPitch === null;
  }
  return (
    JSON.stringify(sheet.conversion_mode) === JSON.stringify(conversionMode) &&
    (targetPitch === null
      ? sheet.target_pitch_assumed_from_source
      : !sheet.target_pitch_assumed_from_source && sheet.target_pitch === targetPitch)
  );
}

export interface LibrarySelectionActionsContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  playback: LibraryPlaybackCoordinator;
  nextDetailRequestToken: () => number;
  isCurrentDetailRequest: (token: number) => boolean;
  navigateBuiltinPage: (page: number, focusIndex?: number) => Promise<void>;
  pageSize: number;
}

export type LibrarySelectionActions = Pick<
  DesktopStore,
  | 'selectSong'
  | 'setSheetConversionMode'
  | 'setSheetTargetPitch'
  | 'showNowPlaying'
  | 'selectLibrarySource'
  | 'openPlaylistAdd'
  | 'exitPlaylistAdd'
>;

export function createLibrarySelectionActions(
  context: LibrarySelectionActionsContext,
): LibrarySelectionActions {
  const {
    bridge,
    get,
    set,
    playback,
    nextDetailRequestToken,
    isCurrentDetailRequest,
    navigateBuiltinPage,
    pageSize,
  } = context;
  const errorMessage = (error: unknown): string =>
    error instanceof Error ? error.message : String(error);

  const refreshSelectedSheetDetail = async (
    conversionMode: SheetConversionMode,
    targetPitch: SheetPitch | null,
  ) => {
    const current = get();
    const songId = current.library.selectedSongId;
    if (!songId) {
      set({
        library: {
          ...current.library,
          conversionMode,
          targetPitch,
          conversionConsentFingerprint: null,
        },
      });
      return;
    }
    const token = nextDetailRequestToken();
    const generation = current.library.generation;
    const currentPlayback = playback.getPlayback();
    if (currentPlayback.transportOperation === 'preparing') playback.clearOperationFromEvent();
    set({
      library: {
        ...current.library,
        conversionMode,
        targetPitch,
        conversionConsentFingerprint: null,
      },
      details: {
        bySongId: cacheDetail(current.details.bySongId, songId, {
          state: 'loading',
          value: null,
          error: null,
        }),
      },
      playback: {
        ...currentPlayback,
        prepared: null,
        preparedIdentity: null,
        preparedContext: null,
        transportOperation:
          currentPlayback.transportOperation === 'preparing'
            ? null
            : currentPlayback.transportOperation,
        error: null,
      },
    });
    try {
      const request = {
        songId,
        ...(generation > 0 ? { generation } : {}),
        conversionMode,
        ...(targetPitch ? { targetPitch } : {}),
      };
      const value = await bridge.getSongDetail(request);
      if (!isCurrentDetailRequest(token)) return;
      const latest = get();
      if (
        latest.library.selectedSongId !== songId ||
        latest.library.generation !== generation ||
        JSON.stringify(latest.library.conversionMode) !== JSON.stringify(conversionMode) ||
        latest.library.targetPitch !== targetPitch
      ) {
        return;
      }
      set({
        details: {
          bySongId: cacheDetail(latest.details.bySongId, songId, {
            state: 'ready',
            value,
            error: null,
          }),
        },
      });
    } catch (error) {
      if (!isCurrentDetailRequest(token)) return;
      const latest = get();
      if (latest.library.selectedSongId !== songId || latest.library.generation !== generation) {
        return;
      }
      set({
        details: {
          bySongId: cacheDetail(latest.details.bySongId, songId, {
            state: 'fatal',
            value: null,
            error: errorMessage(error),
          }),
        },
      });
    }
  };

  return {
    async selectSong(songId) {
      const current = get();
      const sameSelection = current.library.selectedSongId === songId;
      const conversionMode = sameSelection
        ? current.library.conversionMode
        : ({ kind: 'merge_playable' } satisfies SheetConversionMode);
      const targetPitch = sameSelection ? current.library.targetPitch : null;
      const cachedCandidate = current.details.bySongId.get(songId);
      const cached =
        (sameSelection && cachedCandidate?.state === 'loading') ||
        cachedDetailMatchesSelection(cachedCandidate, conversionMode, targetPitch)
          ? cachedCandidate
          : undefined;
      const currentPlayback = playback.getPlayback();
      const selectingDifferentSongAfterRetirement =
        currentPlayback.sessionId === null &&
        currentPlayback.transportOperation === null &&
        currentPlayback.currentSong !== null &&
        currentPlayback.currentSong.songId !== songId;
      if (
        current.library.selectedSongId === songId &&
        !selectingDifferentSongAfterRetirement &&
        cached &&
        (cached.state === 'ready' || cached.state === 'loading')
      ) {
        return;
      }
      const token = nextDetailRequestToken();
      const requestGeneration = current.library.generation;
      const cancelPreparing = currentPlayback.transportOperation === 'preparing';
      if (cancelPreparing) playback.clearOperationFromEvent();
      const preservePlaybackIdentity =
        currentPlayback.sessionId !== null ||
        (!cancelPreparing && currentPlayback.transportOperation !== null);
      const detail =
        cached ?? ({ state: 'loading', value: null, error: null } satisfies DetailEntry);
      const selectedSummary = selectSongById(current.library, songId);
      const indexIsCurrent =
        current.library.indexMembershipRevision === current.library.membershipRevision;
      const selectedIndex = indexIsCurrent
        ? (current.library.indexById.get(songId) ??
          (current.library.selectedSongId === songId ? current.library.selectedSongIndex : null))
        : null;
      set({
        library: {
          ...current.library,
          selectedSongId: songId,
          selectedSongSummary: selectedSummary ?? null,
          selectedSongIndex: selectedIndex,
          conversionMode,
          targetPitch,
          conversionConsentFingerprint: null,
        },
        details: {
          bySongId: cached
            ? cacheDetail(current.details.bySongId, songId, cached)
            : cacheDetail(current.details.bySongId, songId, detail),
        },
        playback: {
          ...currentPlayback,
          prepared: null,
          preparedIdentity: null,
          preparedContext: null,
          currentSong: preservePlaybackIdentity ? currentPlayback.currentSong : null,
          context: preservePlaybackIdentity ? currentPlayback.context : null,
          state: preservePlaybackIdentity ? currentPlayback.state : 'idle',
          sessionId: preservePlaybackIdentity ? currentPlayback.sessionId : null,
          snapshot: preservePlaybackIdentity ? currentPlayback.snapshot : null,
          transportOperation: cancelPreparing ? null : currentPlayback.transportOperation,
          error: null,
        },
      });
      if (cached && cached.state !== 'loading') return;
      try {
        const request = {
          songId,
          ...(requestGeneration > 0 ? { generation: requestGeneration } : {}),
          conversionMode,
          ...(targetPitch ? { targetPitch } : {}),
        };
        const value = await bridge.getSongDetail(request);
        if (!isCurrentDetailRequest(token)) return;
        if (get().library.generation !== requestGeneration) return;
        const nextDetail = { state: 'ready' as const, value, error: null };
        set({
          details: {
            bySongId: cacheDetail(get().details.bySongId, songId, nextDetail),
          },
        });
      } catch (error) {
        if (!isCurrentDetailRequest(token)) return;
        const nextDetail = { state: 'fatal' as const, value: null, error: errorMessage(error) };
        set({
          details: {
            bySongId: cacheDetail(get().details.bySongId, songId, nextDetail),
          },
        });
      }
    },

    async setSheetConversionMode(mode) {
      const current = get().library.conversionMode;
      if (JSON.stringify(current) === JSON.stringify(mode)) return;
      await refreshSelectedSheetDetail(mode, get().library.targetPitch);
    },

    async setSheetTargetPitch(pitch) {
      if (get().library.targetPitch === pitch) return;
      await refreshSelectedSheetDetail(get().library.conversionMode, pitch);
    },

    async showNowPlaying() {
      const library = playback.getLibrary();
      const currentPlayback = playback.getPlayback();
      const playbackContext = currentPlayback.context;
      const currentSong = currentPlayback.currentSong;
      if (
        !playbackContextMatchesLibrary(playbackContext, library) ||
        library.source.kind !== 'smart' ||
        library.source.id !== 'builtin' ||
        library.searchSource.kind !== 'smart' ||
        library.searchSource.id !== 'builtin' ||
        !currentSong ||
        playbackContext.currentSongId !== currentSong.songId ||
        playbackContext.total !== library.resultTotal ||
        !Number.isSafeInteger(playbackContext.currentIndex) ||
        playbackContext.currentIndex < 0 ||
        playbackContext.currentIndex >= library.resultTotal
      ) {
        return;
      }
      const page = Math.floor(playbackContext.currentIndex / pageSize) + 1;
      await navigateBuiltinPage(page, playbackContext.currentIndex);
    },

    async selectLibrarySource(source) {
      const current = get().library;
      if (sourceKey(current.source) !== sourceKey(source) || current.playlistAddMode !== null) {
        nextDetailRequestToken();
        set({
          library: {
            ...get().library,
            selectedSongId: null,
            selectedSongSummary: null,
            selectedSongIndex: null,
            conversionMode: { kind: 'merge_playable' },
            targetPitch: null,
            conversionConsentFingerprint: null,
            playlistAddMode: null,
          },
          details: { bySongId: new Map() },
          playback: playback.afterBrowserNavigation(),
        });
      }
      await get().search('', source);
    },

    async openPlaylistAdd(playlistId) {
      const playlistSource: LibrarySource = { kind: 'playlist', id: playlistId };
      nextDetailRequestToken();
      const current = get().library;
      set({
        library: {
          ...current,
          source: playlistSource,
          playlistAddMode: { playlistId },
          selectedSongId: null,
          selectedSongSummary: null,
          selectedSongIndex: null,
          conversionMode: { kind: 'merge_playable' },
          targetPitch: null,
          conversionConsentFingerprint: null,
        },
        details: { bySongId: new Map() },
        playback: playback.afterBrowserNavigation(),
      });
      await get().search('', playlistSource);
    },

    async exitPlaylistAdd() {
      const current = get().library;
      const playlistId = current.playlistAddMode?.playlistId;
      if (!playlistId) return;
      nextDetailRequestToken();
      const playlistSource: LibrarySource = { kind: 'playlist', id: playlistId };
      set({
        library: {
          ...current,
          source: playlistSource,
          playlistAddMode: null,
          selectedSongId: null,
          selectedSongSummary: null,
          selectedSongIndex: null,
          conversionMode: { kind: 'merge_playable' },
          targetPitch: null,
          conversionConsentFingerprint: null,
        },
        details: { bySongId: new Map() },
        playback: playback.afterBrowserNavigation(),
      });
      await get().search('', playlistSource);
    },
  };
}
