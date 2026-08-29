import type {
  BootstrapDto,
  CatalogDetailRequest,
  CatalogRowDto,
  CatalogSearchDto,
  CatalogSearchRequest,
  CatalogViewportDto,
  CatalogViewportRequest,
  NativeBuildDto,
  PlaybackConfigDto,
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
} from './generated';

export type ThemeId = 'aurora' | 'minimalist' | 'slate' | 'cyberpunk' | 'classic';
export type RiskLevel = 'low' | 'medium' | 'high' | 'unknown';
export type NativeBuild = NativeBuildDto;
export type PlaybackDefaults = PlaybackDefaultsDto;
export type PlaybackOptionSets = PlaybackOptionSetsDto;
export type UpdatePreferences = Omit<UpdatePreferencesDto, 'channel'> & {
  channel: 'stable' | 'beta';
};
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
export type RiskSummary = Omit<RiskSummaryDto, 'level'> & { level: RiskLevel };
export type PlaybackRecommendation = PlaybackRecommendationDto;
export type SongDetail = Omit<SongDetailDto, 'risk'> & { risk: RiskSummary };
export type ViewportRequest = CatalogViewportRequest;
export type ViewportResult = CatalogViewportDto;
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
  Omit<GeneratedSettingsPatch, 'theme' | 'telemetryEnabled' | 'verboseHud' | 'playbackDefaults'> & {
    theme: ThemeId;
    telemetryEnabled: boolean;
    verboseHud: boolean;
    playbackDefaults: PlaybackPatch;
  }
>;
export type GeneratedCoreEvent = GeneratedUiEvent;

export type SongRow = Omit<CatalogRowDto, 'risk_level' | 'metadata_state'> & {
  risk_level: RiskLevel;
  metadata_state: 'pending' | 'ready' | 'error';
};

export type UiEvent = GeneratedUiEvent;
export type PlaybackConfig = PlaybackConfigDto;
export type PlaybackPrepare = PlaybackPrepareRequest;
export type PlaybackDecisionAcceptance = PlaybackDecisionAcceptanceDto;
export type PreparedPlayback = PreparedPlaybackDto;
export type PlaybackStart = PlaybackStartRequest;
export type PlaybackSession = PlaybackSessionDto;
export type PlaybackSessionCommand = PlaybackSessionCommandRequest;

export type Unsubscribe = () => void;

export interface DesktopBridge {
  bootstrap(): Promise<Bootstrap>;
  searchSongs(request: SearchRequest): Promise<SearchResult>;
  getSongDetail(request: DetailRequest): Promise<SongDetail>;
  reloadLibrary(): Promise<{ generation: number; total: number }>;
  setLibraryViewport(request: ViewportRequest): Promise<ViewportResult>;
  getSettings(): Promise<Settings>;
  patchSettings(patch: SettingsPatch): Promise<Settings>;
  preparePlayback(request: PlaybackPrepare): Promise<PreparedPlayback>;
  startPlayback(request: PlaybackStart): Promise<PlaybackSession>;
  stopPlayback(request: PlaybackSessionCommand): Promise<Record<string, unknown>>;
  pausePlayback(request: PlaybackSessionCommand): Promise<Record<string, unknown>>;
  resumePlayback(request: PlaybackSessionCommand): Promise<Record<string, unknown>>;
  skipPlayback(request: PlaybackSessionCommand): Promise<Record<string, unknown>>;
  subscribeUiEvents(listener: (event: UiEvent) => void): Promise<Unsubscribe>;
  shutdown(): Promise<void>;
}
