import type {
  BootstrapDto,
  CalibrationCancelAckDto,
  CalibrationCancelRequest,
  CalibrationFinishedPayload,
  CalibrationMode,
  CalibrationOutcome,
  CalibrationProgressPayload,
  CalibrationStartAckDto,
  CalibrationStartRequest,
  CalibrationState,
  CatalogDetailRequest,
  CatalogSetLikedDto,
  CatalogSetLikedRequest,
  CatalogRowDto,
  CatalogSearchDto,
  CatalogSearchRequest,
  CatalogViewportDto,
  CatalogViewportRequest,
  LibraryCollectionDto,
  LibraryCollectionsDto,
  LibraryImportedSourceDto,
  LibraryImportDto,
  LibrarySource as GeneratedLibrarySource,
  DiagnosticsBackendStatus,
  DiagnosticsEnabledDto,
  DiagnosticsSetEnabledRequest,
  DiagnosticsSnapshotDto,
  NativeBuildDto,
  PlaybackConfigDto,
  PlaybackCommandAckDto,
  PlaybackDecision,
  PlaybackDecisionAcceptanceDto,
  PlaybackPrepareRequest,
  PlaybackSessionCommandRequest,
  PlaybackSessionDto,
  PreparedPlaybackDto,
  PlaybackStartRequest,
  PlaybackDefaultsDto,
  PlaybackOptionSetsDto,
  PlaybackPatch as GeneratedPlaybackPatch,
  PlaybackRecommendationDto,
  RiskSummaryDto,
  SettingsDto,
  SettingsPatch as GeneratedSettingsPatch,
  SongDetailDto,
  UiEvent as GeneratedUiEvent,
  UpdatePreferencesDto,
  UpdateChannel as GeneratedUpdateChannel,
  UpdateState as GeneratedUpdateState,
  UpdateCheckDto,
  UpdateHandoffDto,
  UpdateAvailablePayload,
  UpdateResultPayload,
  UpdateHandoffReadyPayload,
} from './generated';

export type ThemeId = 'aurora' | 'minimalist' | 'slate' | 'cyberpunk' | 'classic';
export type RiskLevel = 'low' | 'medium' | 'high' | 'unknown';
export type NativeBuild = NativeBuildDto;
export type PlaybackDefaults = PlaybackDefaultsDto;
export type PlaybackOptionSets = PlaybackOptionSetsDto;
export type UpdatePreferences = Omit<UpdatePreferencesDto, 'channel'> & {
  channel: GeneratedUpdateChannel;
};
export type UpdateChannelId = GeneratedUpdateChannel;
export type UpdateStateId = GeneratedUpdateState;
export type UpdateCheck = UpdateCheckDto;
export type UpdateHandoff = UpdateHandoffDto;
export type UpdatePatch = Partial<{
  autoCheck: boolean;
  channel: GeneratedUpdateChannel;
  skipVersion: string;
}>;
export type Bootstrap = Omit<BootstrapDto, 'theme' | 'update_preferences'> & {
  theme: ThemeId;
  update_preferences: UpdatePreferences;
};
export type SearchRequest = Omit<CatalogSearchRequest, 'generation'> & {
  generation?: number;
};
export type SearchResult = Omit<CatalogSearchDto, 'items'> & { items: SongRow[] };
export type DetailRequest = Omit<CatalogDetailRequest, 'generation'> & {
  generation?: number;
};
export type SetSongLikedRequest = Omit<CatalogSetLikedRequest, 'generation'> & {
  generation?: number;
};
export type RiskSummary = Omit<RiskSummaryDto, 'level'> & { level: RiskLevel };
export type PlaybackRecommendation = PlaybackRecommendationDto;
export type SongDetail = Omit<SongDetailDto, 'risk'> & { risk: RiskSummary };
export type ViewportRequest = CatalogViewportRequest;
export type ViewportResult = Omit<CatalogViewportDto, 'items'> & { items: SongRow[] };
export type LibrarySource = GeneratedLibrarySource;
export type LibraryCollection = LibraryCollectionDto;
export type LibraryCollections = LibraryCollectionsDto;
export type LibraryImportedSource = LibraryImportedSourceDto;
export type LibraryImport = LibraryImportDto;
export type Settings = Omit<SettingsDto, 'theme' | 'update_preferences'> & {
  theme: ThemeId;
  update_preferences: UpdatePreferences;
};
export type PlaybackPatch = Partial<
  Omit<GeneratedPlaybackPatch, 'holdFrames' | 'tempoScale' | 'fps'> & {
    holdFrames: number;
    tempoScale: number;
    fps: number;
  }
>;
export type SettingsPatch = Partial<
  Omit<
    GeneratedSettingsPatch,
    'theme' | 'telemetryEnabled' | 'verboseHud' | 'playbackDefaults' | 'updatePreferences'
  > & {
    theme: ThemeId;
    telemetryEnabled: boolean;
    verboseHud: boolean;
    playbackDefaults: PlaybackPatch;
    updatePreferences: UpdatePatch;
  }
>;
export type GeneratedCoreEvent = GeneratedUiEvent;

export type SongRow = Omit<CatalogRowDto, 'risk_level' | 'metadata_state'> & {
  risk_level: RiskLevel;
  metadata_state: 'pending' | 'ready' | 'error';
};

export type UiEvent = GeneratedUiEvent;
export type UpdateAvailable = UpdateAvailablePayload;
export type UpdateResult = UpdateResultPayload;
export type UpdateHandoffReady = UpdateHandoffReadyPayload;
export type PlaybackConfig = PlaybackConfigDto;
export type PlaybackCommandAck = PlaybackCommandAckDto;
export type PlaybackPrepare = PlaybackPrepareRequest;
export type PlaybackDecisionAcceptance = PlaybackDecisionAcceptanceDto;
export type PlaybackDecisionId = PlaybackDecision;
export type PreparedPlayback = PreparedPlaybackDto;
export type PlaybackStart = PlaybackStartRequest;
export type PlaybackSession = PlaybackSessionDto;
export type PlaybackSessionCommand = PlaybackSessionCommandRequest;
export type DiagnosticsSnapshot = DiagnosticsSnapshotDto;
export type DiagnosticsBackend = DiagnosticsBackendStatus;
export type DiagnosticsEnabled = DiagnosticsEnabledDto;
export type DiagnosticsSetEnabled = DiagnosticsSetEnabledRequest;
export type CalibrationStart = CalibrationStartRequest;
export type CalibrationStartAck = CalibrationStartAckDto;
export type CalibrationCancel = CalibrationCancelRequest;
export type CalibrationCancelAck = CalibrationCancelAckDto;
export type CalibrationProgress = CalibrationProgressPayload;
export type CalibrationFinished = CalibrationFinishedPayload;
export type CalibrationModeId = CalibrationMode;
export type CalibrationStateId = CalibrationState;
export type CalibrationOutcomeId = CalibrationOutcome;

export type Unsubscribe = () => void;

export interface DesktopBridge {
  bootstrap(): Promise<Bootstrap>;
  searchSongs(request: SearchRequest): Promise<SearchResult>;
  getSongDetail(request: DetailRequest): Promise<SongDetail>;
  reloadLibrary(): Promise<{ generation: number; total: number }>;
  setLibraryViewport(request: ViewportRequest): Promise<ViewportResult>;
  setSongLiked(request: SetSongLikedRequest): Promise<CatalogSetLikedDto>;
  listCollections(): Promise<LibraryCollections>;
  createCollection(name: string): Promise<LibraryCollection>;
  renameCollection(collectionId: string, name: string): Promise<LibraryCollection>;
  deleteCollection(collectionId: string): Promise<boolean>;
  addSongs(collectionId: string, songIds: string[]): Promise<LibraryCollection>;
  removeSongs(collectionId: string, songIds: string[]): Promise<LibraryCollection>;
  importLocalFiles(): Promise<LibraryImport>;
  importLocalFolder(): Promise<LibraryImport>;
  removeImport(sourceId: string): Promise<LibraryImport>;
  getSettings(): Promise<Settings>;
  patchSettings(patch: SettingsPatch): Promise<Settings>;
  checkForUpdate(): Promise<UpdateCheck>;
  getUpdatePreferences(): Promise<UpdatePreferences>;
  patchUpdatePreferences(patch: UpdatePatch): Promise<UpdatePreferences>;
  beginUpdateHandoff(targetVersion: string): Promise<UpdateHandoff>;
  preparePlayback(request: PlaybackPrepare): Promise<PreparedPlayback>;
  startPlayback(request: PlaybackStart): Promise<PlaybackSession>;
  stopPlayback(request: PlaybackSessionCommand): Promise<PlaybackCommandAckDto>;
  pausePlayback(request: PlaybackSessionCommand): Promise<PlaybackCommandAckDto>;
  resumePlayback(request: PlaybackSessionCommand): Promise<PlaybackCommandAckDto>;
  skipPlayback(request: PlaybackSessionCommand): Promise<PlaybackCommandAckDto>;
  setDiagnosticsEnabled(request: DiagnosticsSetEnabled): Promise<DiagnosticsEnabled>;
  startCalibration(request: CalibrationStart): Promise<CalibrationStartAck>;
  cancelCalibration(request: CalibrationCancel): Promise<CalibrationCancelAck>;
  subscribeUiEvents(listener: (event: UiEvent) => void): Promise<Unsubscribe>;
  shutdown(failed?: boolean): Promise<void>;
}
