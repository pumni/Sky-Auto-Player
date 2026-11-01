import type {
  DesktopBridge,
  LibrarySource,
  PlaybackConfig,
  SearchResult,
  SongRow,
} from '../bridge/DesktopBridge';
import { createShuffleTraversal, selectRowAtIndex } from './selectors';
import { LIBRARY_PAGE_SIZE } from './types';
import type { DesktopStore, LibraryState, PlaybackContext, PlaybackSongIdentity } from './types';
import { sourceKey } from './libraryHelpers';

type DesktopStoreSetter = (partial: Partial<DesktopStore>) => void;

export interface PlaybackPreparationContext {
  bridge: DesktopBridge;
  get: () => DesktopStore;
  getLibrary: () => LibraryState;
  set: DesktopStoreSetter;
  loadPage: (
    query: string,
    source: LibrarySource,
    offset: number,
    generation: number,
    token: number,
    mergeIntoLibrary?: boolean,
  ) => Promise<SearchResult>;
  playbackContextIsCurrent: (context: PlaybackContext) => boolean;
  membershipRevisionFor: (source: LibrarySource) => number;
  nextShuffleTraversalSequence: () => number;
  operationIsCurrent: (epoch: number) => boolean;
}

export interface PlaybackPreparationController {
  contextForSong: (library: LibraryState, songId: string) => PlaybackContext | null;
  resolveContextRow: (context: PlaybackContext, index: number) => Promise<SongRow>;
  prepareForOperation: (
    identity: PlaybackSongIdentity,
    context: PlaybackContext,
    overrides: Partial<PlaybackConfig> | undefined,
    operationEpoch: number,
  ) => Promise<NonNullable<DesktopStore['playback']['prepared']> | null>;
}

export function createPlaybackPreparationController(
  context: PlaybackPreparationContext,
): PlaybackPreparationController {
  const {
    bridge,
    get,
    getLibrary,
    set,
    loadPage,
    playbackContextIsCurrent,
    membershipRevisionFor,
    nextShuffleTraversalSequence,
    operationIsCurrent,
  } = context;

  const contextForSong = (library: LibraryState, songId: string): PlaybackContext | null => {
    if (library.indexMembershipRevision !== library.membershipRevision) return null;
    const currentIndex =
      library.indexById.get(songId) ??
      (library.selectedSongId === songId ? (library.selectedSongIndex ?? undefined) : undefined);
    if (currentIndex === undefined) return null;
    const shuffleEnabled = get().playback.shuffleEnabled;
    return {
      source: library.searchSource,
      query: library.query,
      generation: library.generation,
      total: library.resultTotal,
      currentIndex,
      currentSongId: songId,
      shuffleTraversal: shuffleEnabled
        ? createShuffleTraversal(
            library.resultTotal,
            currentIndex,
            `${library.generation}:${sourceKey(library.searchSource)}:${library.query}:${songId}:${nextShuffleTraversalSequence()}`,
          )
        : null,
      dryRun: false,
      membershipRevision: membershipRevisionFor(library.searchSource),
      valid: true,
    };
  };

  const resolveContextRow = async (playbackContext: PlaybackContext, index: number) => {
    const library = getLibrary();
    if (
      playbackContext.generation !== library.generation ||
      !playbackContextIsCurrent(playbackContext)
    ) {
      throw new Error(
        'The Library context changed after playback started. Select a song to create a new playback context.',
      );
    }
    if (!Number.isInteger(index) || index < 0 || index >= playbackContext.total) {
      throw new Error('There is no song at that playback-context position.');
    }
    const offset = Math.floor(index / LIBRARY_PAGE_SIZE) * LIBRARY_PAGE_SIZE;
    const visibleContext =
      library.generation === playbackContext.generation &&
      library.query === playbackContext.query &&
      sourceKey(library.searchSource) === sourceKey(playbackContext.source);
    const visibleRow = visibleContext ? selectRowAtIndex(library, index) : undefined;
    if (visibleRow) return visibleRow;
    const result = await loadPage(
      playbackContext.query,
      playbackContext.source,
      offset,
      playbackContext.generation,
      library.searchRequestGeneration,
      false,
    );
    if (
      getLibrary().generation !== playbackContext.generation ||
      !playbackContextIsCurrent(playbackContext)
    ) {
      throw new Error('The Library changed while the adjacent song was being resolved.');
    }
    const row = result.items[index - result.offset];
    if (!row) throw new Error('The adjacent song is no longer available in this Library context.');
    return row;
  };

  const makePlaybackConfig = (
    overrides?: Partial<PlaybackConfig>,
    inheritedDryRun = false,
  ): PlaybackConfig => {
    const settings = get().settings;
    if (!settings) throw new Error('Playback settings are not ready.');
    return {
      hold_frames: overrides?.hold_frames ?? settings.playback_defaults.hold_frames,
      timing_margin_us: overrides?.timing_margin_us ?? settings.playback_defaults.timing_margin_us,
      tempo_scale: overrides?.tempo_scale ?? settings.playback_defaults.tempo_scale,
      fps: overrides?.fps ?? settings.playback_defaults.fps,
      dry_run: overrides?.dry_run ?? inheritedDryRun,
    };
  };

  const prepareForOperation = async (
    identity: PlaybackSongIdentity,
    playbackContext: PlaybackContext,
    overrides: Partial<PlaybackConfig> | undefined,
    operationEpoch: number,
  ): Promise<NonNullable<DesktopStore['playback']['prepared']> | null> => {
    if (!playbackContextIsCurrent(playbackContext)) {
      throw new Error(
        'The Library source changed. Select a song to create a new playback context.',
      );
    }
    const config = makePlaybackConfig(overrides, playbackContext.dryRun);
    const library = getLibrary();
    const selectedSongOptions = library.selectedSongId === identity.songId;
    const prepared = await bridge.preparePlayback({
      songId: identity.songId,
      generation: playbackContext.generation,
      config,
      conversionMode: selectedSongOptions ? library.conversionMode : { kind: 'merge_playable' },
      ...(selectedSongOptions && library.targetPitch ? { targetPitch: library.targetPitch } : {}),
      ...(selectedSongOptions && library.conversionConsentFingerprint
        ? { conversionConfirmationFingerprint: library.conversionConsentFingerprint }
        : {}),
    });
    if (
      playbackContext.generation !== getLibrary().generation ||
      !playbackContextIsCurrent(playbackContext)
    ) {
      throw new Error(
        'The Library context changed while playback was being prepared. Select a song to create a new context.',
      );
    }
    if (!operationIsCurrent(operationEpoch)) return null;
    const latestLibrary = getLibrary();
    const nextConsent =
      prepared.error_code === 'conversion_confirmation_required' &&
      latestLibrary.conversionConsentFingerprint !==
        prepared.song.sheet_analysis?.confirmation_fingerprint
        ? null
        : latestLibrary.conversionConsentFingerprint;
    const preparedContext = { ...playbackContext, dryRun: config.dry_run };
    set({
      ...(nextConsent === latestLibrary.conversionConsentFingerprint
        ? {}
        : {
            library: {
              ...latestLibrary,
              conversionConsentFingerprint: nextConsent,
            },
          }),
      playback: {
        ...get().playback,
        prepared,
        preparedIdentity: identity,
        preparedContext,
        error:
          prepared.error_code === 'conversion_confirmation_required'
            ? null
            : prepared.error_message,
      },
    });
    return prepared;
  };

  return { contextForSong, resolveContextRow, prepareForOperation };
}
