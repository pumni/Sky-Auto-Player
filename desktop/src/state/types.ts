import type {
  Bootstrap,
  CalibrationFinished,
  CalibrationModeId,
  CalibrationStart,
  DesktopBridge,
  DiagnosticsSnapshot,
  LibraryPlaylistSummary,
  LibrarySource,
  PlaybackConfig,
  PlaybackDecisionAcceptance,
  PlaybackDecisionId,
  PreparedPlayback,
  SearchRequest,
  Settings,
  SettingsPatch,
  SongDetail,
  SongRow,
  SheetConversionMode,
  SheetPitch,
  UiEvent,
  UpdateChannelId,
  UpdateErrorCode,
  UpdateRetryAction,
  UpdateState,
} from '../bridge/DesktopBridge';

export type LoadState = 'idle' | 'loading' | 'ready' | 'fatal';

export type PlaybackUiState =
  'idle' | 'starting' | 'playing' | 'paused' | 'stopping' | 'finished' | 'failed';

export type TransportOperation =
  | null
  | 'preparing'
  | 'starting'
  | 'pausing'
  | 'resuming'
  | 'stopping'
  | 'advancing'
  | 'restarting';

export interface PlaybackSongIdentity {
  songId: string;
  title: string;
  liked: boolean;
  durationUs: number | null;
  formatLabel: string;
  noteCount: number | null;
  riskLevel: SongRow['risk_level'];
  generation: number;
}

export interface PlaybackContext {
  source: LibrarySource;
  query: string;
  generation: number;
  total: number;
  currentIndex: number;
  currentSongId: string;
  shuffleTraversal: ShuffleTraversal | null;
  dryRun: boolean;
  membershipRevision: number;
  valid: boolean;
}

export interface ShuffleTraversal {
  originIndex: number;
  step: number;
  position: number;
}

export type CalibrationUiState =
  'idle' | 'starting' | 'running' | 'cancelling' | 'succeeded' | 'failed' | 'cancelled';

export type UtilityView = 'details' | 'diagnostics';

export interface LibraryState {
  source: LibrarySource;
  searchSource: LibrarySource;
  playlistAddMode: { playlistId: string } | null;
  query: string;
  generation: number;
  catalogTotal: number;
  builtinPage: {
    committedPage: number;
    pendingPage: number | null;
    intentId: number;
    failedPage: number | null;
    focusNowPlaying: { id: number; index: number } | null;
  };
  likedTotal: number;
  resultTotal: number;
  pages: Map<number, readonly SongRow[]>;
  indexById: Map<string, number>;
  selectedSongId: string | null;
  selectedSongSummary: SongRow | null;
  selectedSongIndex: number | null;
  conversionMode: SheetConversionMode;
  targetPitch: SheetPitch | null;
  conversionConsentFingerprint: string | null;
  membershipRevision: number;
  indexMembershipRevision: number;
  visibleRange: { first: number; last: number };
  loading: boolean;
  searchRequestGeneration: number;
  error: string | null;
}

export interface LibraryNavigationState {
  loadState: 'idle' | 'loading' | 'ready' | 'error';
  builtinAvailable: boolean;
  builtinSongCount: number;
  playlistOrder: string[];
  playlistsById: Map<string, LibraryPlaylistSummary>;
  pendingMutations: Set<string>;
  lastError: string | null;
}

export interface DetailEntry {
  state: LoadState;
  value: SongDetail | null;
  error: string | null;
}

export interface DiagnosticsEventLine {
  seq: number;
  name: string;
  detail: string;
  timestamp: number;
}

export interface DesktopStore {
  bootstrapState: LoadState;
  bootstrap: Bootstrap | null;
  fatal: string | null;
  library: LibraryState;
  libraryNavigation: LibraryNavigationState;
  details: { bySongId: Map<string, DetailEntry> };
  settings: Settings | null;
  settingsState: LoadState;
  settingsError: string | null;
  settingsOpen: boolean;
  utility: {
    open: boolean;
    activeView: UtilityView;
  };
  diagnostics: {
    enabled: boolean;
    samples: DiagnosticsSnapshot[];
    events: DiagnosticsEventLine[];
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
    dialogOpen: boolean;
    checkRequestPending: boolean;
    installRequestPending: boolean;
    transportError: string | null;
    transportErrorAction: 'check' | 'install' | null;
    lastNativeRevision: number;

    state: UpdateState;
    currentVersion: string | null;
    availableVersion: string | null;
    channel: UpdateChannelId;
    releaseNotes: string | null;
    publishedAt: string | null;
    errorCode: UpdateErrorCode | null;
    errorDetail: string | null;
    retryAction: UpdateRetryAction;
    handoffId: string | null;
    progress: { completed: number; total: number | null; message: string } | null;
  };
  playback: {
    shuffleEnabled: boolean;
    state: PlaybackUiState;
    sessionId: string | null;
    currentSong: PlaybackSongIdentity | null;
    context: PlaybackContext | null;
    preparedIdentity: PlaybackSongIdentity | null;
    preparedContext: PlaybackContext | null;
    startRequestId: number | null;
    transportOperation: TransportOperation;
    prepared: PreparedPlayback | null;
    snapshot: Extract<UiEvent, { name: 'playback.snapshot' }>['payload'] | null;
    error: string | null;
    statusRetryPending: boolean;
  };
  initialize: () => Promise<void>;
  applyEvent: (event: UiEvent) => void;
  search: (query?: string, source?: LibrarySource) => Promise<void>;
  selectSong: (songId: string) => Promise<void>;
  setSheetConversionMode: (mode: SheetConversionMode) => Promise<void>;
  setSheetTargetPitch: (pitch: SheetPitch | null) => Promise<void>;
  acknowledgeSheetLoss: () => Promise<void>;
  setViewport: (first: number, last: number) => Promise<void>;
  navigateBuiltinPage: (page: number) => Promise<void>;
  retryBuiltinPage: () => Promise<void>;
  showNowPlaying: () => Promise<void>;
  selectLibrarySource: (source: LibrarySource) => Promise<void>;
  openPlaylistAdd: (playlistId: string) => Promise<void>;
  exitPlaylistAdd: () => Promise<void>;
  loadLibraryNavigation: () => Promise<void>;
  createPlaylist: (name: string) => Promise<void>;
  renamePlaylist: (playlistId: string, name: string) => Promise<void>;
  deletePlaylist: (playlistId: string) => Promise<void>;
  importLocalFilesToPlaylist: (playlistId: string) => Promise<number | null>;
  importLocalFolderToPlaylist: (playlistId: string) => Promise<number | null>;
  addSongToPlaylist: (playlistId: string, songId: string) => Promise<void>;
  removeSongFromPlaylist: (playlistId: string, songId: string) => Promise<void>;
  setSongLiked: (songId: string, liked: boolean) => Promise<void>;
  reloadLibrary: () => Promise<void>;
  patchSettings: (patch: SettingsPatch) => Promise<Settings | null>;
  checkForUpdate: (origin?: 'manual' | 'background') => Promise<void>;
  setUpdateDialogOpen: (open: boolean) => void;
  beginUpdateHandoff: () => Promise<void>;
  prepareSelectedPlayback: (overrides?: Partial<PlaybackConfig>) => Promise<void>;
  startPreparedPlayback: (decision?: PlaybackDecisionId) => Promise<void>;
  cancelPreparedPlayback: () => void;
  stopPlayback: () => Promise<void>;
  retryPlaybackStatus: () => Promise<void>;
  pausePlayback: () => Promise<void>;
  resumePlayback: () => Promise<void>;
  previousPlayback: () => Promise<void>;
  nextPlayback: () => Promise<void>;
  setShuffleEnabled: (enabled: boolean) => void;
  setSettingsOpen: (open: boolean) => void;
  setDiagnosticsEnabled: (enabled: boolean) => Promise<void>;
  exportSenderTrace: () => Promise<string>;
  openUtility: (view: UtilityView) => void;
  closeUtility: () => void;
  toggleUtility: () => void;
  setUtilityView: (view: UtilityView) => void;
  startCalibration: (mode?: CalibrationModeId) => Promise<void>;
  cancelCalibration: () => Promise<void>;
  setCalibrationOpen: (open: boolean) => void;
}

export const MAX_DIAGNOSTIC_SAMPLES = 600;
export const MAX_DIAGNOSTIC_EVENTS = 500;
export const MAX_DIAGNOSTIC_LINE_LENGTH = 4096;
export const LIBRARY_PAGE_SIZE = 100;
export const MAX_ACTIVE_LIBRARY_PAGES = 3;
export const DETAIL_CACHE_LIMIT = 64;
export const AUTO_PLAY_HANDOFF_MS = 450;

export type PlaybackIssueKind =
  'recoverable_status' | 'playback_failure' | 'target_failure' | 'conflict';

export interface PlaybackIssuePresentation {
  kind: PlaybackIssueKind;
  title: string;
  message: string;
}

export type { CalibrationStart, DesktopBridge, PlaybackDecisionAcceptance, SearchRequest };
