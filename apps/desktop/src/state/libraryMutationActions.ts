import type {
  DesktopBridge,
  LibraryPlaylistImportResult,
  LibraryNavigation,
  LibrarySource,
} from '../bridge/DesktopBridge';
import {
  navigationFromDto,
  sourceKey,
  updateLoadedRows,
  updatePlaylistSummary,
} from './libraryHelpers';
import { selectSongById } from './selectors';
import type { DesktopStore } from './types';
import type { LibraryPlaybackCoordinator } from './libraryPlaybackCoordinator';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface LibraryMutationState {
  pendingLikeWrites: Set<Promise<unknown>>;
  pendingLikeStateBySongId: Map<string, { liked: boolean; mutationSequence: number }>;
  likeWriteTails: Map<string, Promise<unknown>>;
  latestLikeMutationBySongId: Map<string, number>;
  likeMutationRevision: number;
  likeWriteRevision: number;
  likeMutationSequence: number;
}

export function createLibraryMutationState(): LibraryMutationState {
  return {
    pendingLikeWrites: new Set(),
    pendingLikeStateBySongId: new Map(),
    likeWriteTails: new Map(),
    latestLikeMutationBySongId: new Map(),
    likeMutationRevision: 0,
    likeWriteRevision: 0,
    likeMutationSequence: 0,
  };
}

export interface LibraryMutationActionsContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  set: DesktopStoreSetter;
  playback: LibraryPlaybackCoordinator;
  state: LibraryMutationState;
  nextDetailRequestToken: () => number;
  syncCatalogGeneration: (generation: number) => Promise<void>;
  invalidateSourceCache: (source: LibrarySource) => void;
  beginMembershipMutation: (source: LibrarySource) => void;
  finishMembershipMutation: (source: LibrarySource) => boolean;
  refreshMembershipSource: (source: LibrarySource) => Promise<void>;
  patchCachedLike: (songId: string, liked: boolean, likedTotal: number) => void;
  patchCachedLikedTotal: (likedTotal: number) => void;
}

export type LibraryMutationActions = Pick<
  DesktopStore,
  | 'loadLibraryNavigation'
  | 'createPlaylist'
  | 'renamePlaylist'
  | 'deletePlaylist'
  | 'importLocalFilesToPlaylist'
  | 'importLocalFolderToPlaylist'
  | 'addSongToPlaylist'
  | 'removeSongFromPlaylist'
  | 'setSongLiked'
  | 'reloadLibrary'
>;

export function createLibraryMutationActions(
  context: LibraryMutationActionsContext,
): LibraryMutationActions {
  const {
    bridge,
    get,
    set,
    playback,
    state,
    nextDetailRequestToken,
    syncCatalogGeneration,
    invalidateSourceCache,
    beginMembershipMutation,
    finishMembershipMutation,
    refreshMembershipSource,
    patchCachedLike,
    patchCachedLikedTotal,
  } = context;
  let libraryNavigationRequestId = 0;
  let libraryNavigationMutationRevision = 0;
  let refreshNavigationAfterMutation = false;

  const errorMessage = (error: unknown): string =>
    error instanceof Error ? error.message : String(error);

  const importedSongCount = (result: LibraryPlaylistImportResult | null | undefined): number => {
    const count = result?.imported_song_count;
    if (typeof count !== 'number' || !Number.isSafeInteger(count) || count < 0) {
      throw new Error('The import did not return a valid result.');
    }
    return count;
  };

  const setLibraryError = (message: string | null) => {
    set({ libraryNavigation: { ...get().libraryNavigation, lastError: message } });
  };

  const setLibraryMutationPending = (key: string, pending: boolean): boolean => {
    const current = get().libraryNavigation;
    if (pending && current.pendingMutations.has(key)) return false;
    const nextPending = new Set(current.pendingMutations);
    if (pending) nextPending.add(key);
    else nextPending.delete(key);
    libraryNavigationMutationRevision += 1;
    set({
      libraryNavigation: {
        ...current,
        pendingMutations: nextPending,
        lastError: current.lastError,
      },
    });
    if (!pending && nextPending.size === 0 && refreshNavigationAfterMutation) {
      refreshNavigationAfterMutation = false;
      void get().loadLibraryNavigation();
    }
    return true;
  };

  const updateNavigation = (navigation: LibraryNavigation) => {
    const pendingMutations = get().libraryNavigation.pendingMutations;
    set({
      libraryNavigation: {
        ...navigationFromDto(navigation),
        pendingMutations: new Set(pendingMutations),
      },
    });
  };

  const canApplyNavigationResponse = (
    requestId: number,
    generation: number,
    mutationRevision: number,
  ): boolean => {
    if (requestId !== libraryNavigationRequestId) return false;
    if (generation !== get().library.generation) {
      void get().loadLibraryNavigation();
      return false;
    }
    const pendingMutations = get().libraryNavigation.pendingMutations;
    if (mutationRevision !== libraryNavigationMutationRevision || pendingMutations.size > 0) {
      refreshNavigationAfterMutation = true;
      if (pendingMutations.size === 0) {
        refreshNavigationAfterMutation = false;
        void get().loadLibraryNavigation();
      }
      return false;
    }
    return true;
  };

  return {
    async loadLibraryNavigation() {
      const requestId = ++libraryNavigationRequestId;
      const generation = get().library.generation;
      const mutationRevision = libraryNavigationMutationRevision;
      const current = get().libraryNavigation;
      set({ libraryNavigation: { ...current, loadState: 'loading', lastError: null } });
      try {
        const navigation = await bridge.listLibraryNavigation();
        if (!canApplyNavigationResponse(requestId, generation, mutationRevision)) return;
        updateNavigation(navigation);
      } catch (error) {
        if (!canApplyNavigationResponse(requestId, generation, mutationRevision)) return;
        set({
          libraryNavigation: {
            ...get().libraryNavigation,
            loadState: 'error',
            lastError: errorMessage(error),
          },
        });
      }
    },

    async createPlaylist(name) {
      const key = 'playlist:create';
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      try {
        const summary = await bridge.createPlaylist(name.trim());
        set({ libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary) });
        await get().selectLibrarySource({ kind: 'playlist', id: summary.id });
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async renamePlaylist(playlistId, name) {
      const key = `playlist:${playlistId}:rename`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      try {
        const summary = await bridge.renamePlaylist(playlistId, name.trim());
        set({ libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary) });
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async deletePlaylist(playlistId) {
      const key = `playlist:${playlistId}:delete`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      const active = get().library.source;
      playback.invalidateMembership({ kind: 'playlist', id: playlistId });
      try {
        const removed = await bridge.deletePlaylist(playlistId);
        if (removed) {
          invalidateSourceCache({ kind: 'playlist', id: playlistId });
          const current = get().libraryNavigation;
          const playlistsById = new Map(current.playlistsById);
          playlistsById.delete(playlistId);
          set({
            libraryNavigation: {
              ...current,
              playlistsById,
              playlistOrder: current.playlistOrder.filter((id) => id !== playlistId),
            },
          });
          if (active.kind === 'playlist' && active.id === playlistId) {
            await get().selectLibrarySource({ kind: 'smart', id: 'builtin' });
          }
        }
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async importLocalFilesToPlaylist(playlistId) {
      const key = `playlist:${playlistId}:import:file`;
      if (!setLibraryMutationPending(key, true)) return null;
      setLibraryError(null);
      playback.invalidateMembership({ kind: 'playlist', id: playlistId });
      try {
        const result = await bridge.importLocalFilesToPlaylist(playlistId);
        const count = importedSongCount(result);
        set({ libraryNavigation: updatePlaylistSummary(get().libraryNavigation, result.playlist) });
        if (count > 0) await syncCatalogGeneration(result.catalog_generation);
        return count;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async importLocalFolderToPlaylist(playlistId) {
      const key = `playlist:${playlistId}:import:folder`;
      if (!setLibraryMutationPending(key, true)) return null;
      setLibraryError(null);
      playback.invalidateMembership({ kind: 'playlist', id: playlistId });
      try {
        const result = await bridge.importLocalFolderToPlaylist(playlistId);
        const count = importedSongCount(result);
        set({ libraryNavigation: updatePlaylistSummary(get().libraryNavigation, result.playlist) });
        if (count > 0) await syncCatalogGeneration(result.catalog_generation);
        return count;
      } finally {
        setLibraryMutationPending(key, false);
      }
    },

    async addSongToPlaylist(playlistId, songId) {
      const key = `playlist:${playlistId}:song:${songId}:add`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      const source: LibrarySource = { kind: 'playlist', id: playlistId };
      playback.invalidateMembership(source);
      beginMembershipMutation(source);
      try {
        const summary = await bridge.addSongsToPlaylist(playlistId, [songId]);
        set({ libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary) });
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
        if (finishMembershipMutation(source)) await refreshMembershipSource(source);
      }
    },

    async removeSongFromPlaylist(playlistId, songId) {
      const key = `playlist:${playlistId}:song:${songId}:remove`;
      if (!setLibraryMutationPending(key, true)) return;
      setLibraryError(null);
      const source: LibrarySource = { kind: 'playlist', id: playlistId };
      playback.invalidateMembership(source);
      beginMembershipMutation(source);
      try {
        const summary = await bridge.removeSongsFromPlaylist(playlistId, [songId]);
        set({ libraryNavigation: updatePlaylistSummary(get().libraryNavigation, summary) });
        const active = get().library.source;
        if (active.kind === 'playlist' && active.id === playlistId) {
          if (get().library.selectedSongId === songId) {
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
              },
              details: { bySongId: new Map() },
            });
          }
        }
      } catch (error) {
        setLibraryError(errorMessage(error));
        throw error;
      } finally {
        setLibraryMutationPending(key, false);
        if (finishMembershipMutation(source)) await refreshMembershipSource(source);
      }
    },

    async setSongLiked(songId, liked) {
      const current = get().library;
      const previous = selectSongById(current, songId);
      const currentPlaybackSong = playback.currentPlaybackSong(songId);
      if (!previous && !currentPlaybackSong) return;
      const previousLiked = previous?.liked ?? currentPlaybackSong?.liked ?? false;
      if (previousLiked === liked) return;
      const mutationSequence = ++state.likeMutationSequence;
      const mutationRevision = ++state.likeMutationRevision;
      const likedSource: LibrarySource = { kind: 'smart', id: 'liked' };
      state.latestLikeMutationBySongId.set(songId, mutationSequence);
      state.pendingLikeStateBySongId.set(songId, { liked, mutationSequence });
      playback.invalidateMembership(likedSource);
      const optimistic = previous ? { ...previous, liked } : null;
      const likedDelta = liked ? 1 : -1;
      const optimisticLikedTotal = Math.max(0, current.likedTotal + likedDelta);
      patchCachedLike(songId, liked, optimisticLikedTotal);
      const playbackProjection = playback.projectPlaybackSongLike(songId, liked);
      set({
        library: {
          ...(optimistic ? updateLoadedRows(current, [optimistic]) : current),
          likedTotal: optimisticLikedTotal,
        },
        playback: playbackProjection.playback,
      });
      beginMembershipMutation(likedSource);
      const targetGeneration = currentPlaybackSong?.generation ?? current.generation;
      const generation = targetGeneration > 0 ? targetGeneration : undefined;
      const request = {
        songId,
        liked,
        ...(generation === undefined ? {} : { generation }),
      };
      const previousWrite = state.likeWriteTails.get(songId);
      const write = (previousWrite ?? Promise.resolve())
        .catch(() => undefined)
        .then(() => bridge.setSongLiked(request));
      state.likeWriteTails.set(songId, write);
      state.pendingLikeWrites.add(write);

      const clearWrite = () => {
        state.pendingLikeWrites.delete(write);
        state.likeWriteRevision += 1;
        if (state.likeWriteTails.get(songId) === write) state.likeWriteTails.delete(songId);
      };

      try {
        const result = await write;
        clearWrite();
        const membershipRefresh = finishMembershipMutation(likedSource);
        if (state.latestLikeMutationBySongId.get(songId) !== mutationSequence) {
          if (membershipRefresh) await refreshMembershipSource(likedSource);
          return;
        }
        state.latestLikeMutationBySongId.delete(songId);
        if (state.pendingLikeStateBySongId.get(songId)?.mutationSequence === mutationSequence) {
          state.pendingLikeStateBySongId.delete(songId);
        }
        if (mutationRevision === state.likeMutationRevision && state.pendingLikeWrites.size === 0) {
          set({ library: { ...get().library, likedTotal: result.total } });
          patchCachedLikedTotal(result.total);
        }
        const library = get().library;
        if (sourceKey(library.searchSource) === sourceKey(likedSource)) {
          if (!liked && get().library.selectedSongId === songId) {
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
              },
              details: { bySongId: new Map() },
            });
          }
        }
        if (membershipRefresh) await refreshMembershipSource(likedSource);
      } catch (error) {
        clearWrite();
        const membershipRefresh = finishMembershipMutation(likedSource);
        if (state.latestLikeMutationBySongId.get(songId) !== mutationSequence) {
          if (membershipRefresh) await refreshMembershipSource(likedSource);
          return;
        }
        state.latestLikeMutationBySongId.delete(songId);
        if (state.pendingLikeStateBySongId.get(songId)?.mutationSequence === mutationSequence) {
          state.pendingLikeStateBySongId.delete(songId);
        }
        const message = errorMessage(error);
        const latest = get().library;
        const row = selectSongById(latest, songId);
        const latestPlayback = playback.getPlayback();
        const playbackSong = playback.currentPlaybackSong(songId);
        const rollbackPlayback = playback.projectPlaybackSongLike(songId, previousLiked);
        const shouldRollback = (row?.liked ?? playbackSong?.liked) === liked;
        const rollbackTotal = shouldRollback
          ? Math.max(0, latest.likedTotal - likedDelta)
          : latest.likedTotal;
        const rollback = shouldRollback
          ? row
            ? updateLoadedRows(latest, [{ ...row, liked: previousLiked }])
            : latest
          : latest;
        if (shouldRollback) patchCachedLike(songId, previousLiked, rollbackTotal);
        set({
          library: { ...rollback, likedTotal: rollbackTotal },
          playback: shouldRollback && playbackSong ? rollbackPlayback.playback : latestPlayback,
        });
        if (membershipRefresh) await refreshMembershipSource(likedSource);
        set({ library: { ...get().library, error: message } });
      }
    },

    async reloadLibrary() {
      set({ library: { ...get().library, loading: true, error: null } });
      try {
        await bridge.reloadLibrary();
        await get().loadLibraryNavigation();
      } catch (error) {
        set({ library: { ...get().library, loading: false, error: errorMessage(error) } });
      }
    },
  };
}
