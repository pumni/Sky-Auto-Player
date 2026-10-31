import type { LibrarySource } from '../bridge/DesktopBridge';
import type { LibraryState, PlaybackSongIdentity, DesktopStore } from './types';
import type { PlaybackSlice } from './playbackSlice';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface LibraryPlaybackCoordinator {
  getLibrary: () => LibraryState;
  getPlayback: () => DesktopStore['playback'];
  currentPlaybackSong: (songId: string) => PlaybackSongIdentity | null;
  projectPlaybackSongLike: (
    songId: string,
    liked: boolean,
  ) => { song: PlaybackSongIdentity | null; playback: DesktopStore['playback'] };
  afterBrowserNavigation: () => DesktopStore['playback'];
  clearOperationFromEvent: () => void;
  invalidateMembership: (source: LibrarySource) => void;
  applyCatalogGeneration: (options: {
    generation: number;
    catalogTotal?: number;
    membershipRevision?: number;
    pageIntentId?: number;
  }) => boolean;
}

export function createLibraryPlaybackCoordinator(context: {
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  playback: PlaybackSlice;
}): LibraryPlaybackCoordinator {
  const { get, set, playback } = context;

  const playbackForCatalogChange = (): DesktopStore['playback'] => {
    const current = get().playback;
    const cancelPreparing = current.transportOperation === 'preparing';
    if (cancelPreparing) playback.clearOperationFromEvent();
    return {
      ...current,
      prepared: null,
      preparedIdentity: null,
      preparedContext: null,
      transportOperation: cancelPreparing ? null : current.transportOperation,
    };
  };

  return {
    getLibrary: () => get().library,
    getPlayback: () => get().playback,
    currentPlaybackSong: (songId) => {
      const song = get().playback.currentSong;
      return song?.songId === songId ? song : null;
    },
    projectPlaybackSongLike: (songId, liked) => {
      const current = get().playback;
      const song = current.currentSong?.songId === songId ? current.currentSong : null;
      return {
        song,
        playback: song ? { ...current, currentSong: { ...song, liked } } : current,
      };
    },
    afterBrowserNavigation: () => playback.playbackAfterBrowserNavigation(get().playback),
    clearOperationFromEvent: () => playback.clearOperationFromEvent(),
    invalidateMembership: (source) => playback.invalidatePlaybackMembershipContext(source),
    applyCatalogGeneration: ({ generation, catalogTotal, membershipRevision, pageIntentId }) => {
      const current = get().library;
      if (generation <= current.generation) return false;
      const intentId = pageIntentId ?? current.builtinPage.intentId + 1;
      set({
        library: {
          ...current,
          generation,
          ...(catalogTotal === undefined ? {} : { catalogTotal, resultTotal: catalogTotal }),
          builtinPage: {
            committedPage: 1,
            pendingPage: null,
            intentId,
            failedPage: null,
            focusNowPlaying: null,
          },
          pages: new Map(),
          indexById: new Map(),
          selectedSongId: null,
          selectedSongSummary: null,
          selectedSongIndex: null,
          conversionMode: { kind: 'merge_playable' },
          targetPitch: null,
          conversionConsentFingerprint: null,
          ...(membershipRevision === undefined ? {} : { membershipRevision }),
          indexMembershipRevision: -1,
          visibleRange: { first: 0, last: 0 },
          ...(catalogTotal === undefined ? { resultTotal: 0 } : {}),
          loading: true,
          error: null,
        },
        details: { bySongId: new Map() },
        playback: playbackForCatalogChange(),
      });
      return true;
    },
  };
}
