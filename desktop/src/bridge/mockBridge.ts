import type {
  Bootstrap,
  PlaybackConfig,
  CalibrationCancel,
  CalibrationCancelAck,
  CalibrationStart,
  CalibrationStartAck,
  CalibrationStateId,
  DesktopBridge,
  DetailRequest,
  SearchRequest,
  SearchResult,
  Settings,
  SettingsPatch,
  DiagnosticsEnabled,
  DiagnosticsSetEnabled,
  LibraryPlaylistSummary,
  LibraryPlaylistImportResult,
  LibraryNavigation,
  ThemeId,
  UiEvent,
  ViewportRequest,
  ViewportResult,
  UpdateCheck,
  UpdateHandoff,
  UpdatePatch,
  UpdatePreferences,
} from './DesktopBridge';

const MOCK_NATIVE: Bootstrap['native_build'] = {
  native_build_commit: 'mock-native-build',
  native_version: '3.5.0-mock',
  schema_version: 10,
  native_abi: 'mock',
  rustc_version: 'mock',
  win32_backend: true,
};

const titles = [
  'Aurora Landing',
  'Blue Bird',
  'Candle Run',
  'Dawn Chorus',
  'Elder Song',
  'First Flight',
  'Home',
  'Isle of Dawn',
  'Jellyfish Waltz',
  'Kite Dance',
  'Light Manta',
  'Moonlit Village',
];

const LONG_CONTENT_INDEX = 495;
const LONG_CONTENT_TITLE = 'A sheet with an intentionally long title for layout verification';
const LONG_CONTENT_REASON =
  'This intentionally long timing-risk explanation verifies wrapping and prevents horizontal overflow in narrow layouts.';

function mockId(index: number): string {
  return index.toString(16).padStart(32, '0');
}

function row(index: number, title: string): SearchResult['items'][number] {
  return {
    song_id: mockId(index),
    title,
    format_label: 'TXT',
    duration_us: 120_000_000 + index * 5_000_000,
    note_count: 128 + index * 9,
    risk_level: index % 5 === 0 ? 'medium' : 'low',
    metadata_state: 'ready',
    liked: false,
  };
}

const rows = Array.from({ length: 500 }, (_, index) =>
  row(
    index,
    index === LONG_CONTENT_INDEX
      ? LONG_CONTENT_TITLE
      : (titles[index] ?? `Song ${String(index + 1).padStart(3, '0')}`),
  ),
);

function initialSettings(): Settings {
  return {
    theme: 'aurora',
    ui_background_mode: 'opaque',
    playback_defaults: {
      hold_frames: 1,
      timing_margin_us: 500,
      down_late_grace_us: 2_000,
      tempo_scale: 1,
      fps: 60,
      dry_run: false,
    },
    timing_margin_recommendation: {
      recommended_timing_margin_us: 500,
      qualified: false,
      source: 'default_fallback',
    },
    telemetry_enabled: true,
    verbose_hud: false,
    update_preferences: { auto_check: true, channel: 'stable', skip_version: '' },
  };
}

export interface MockBridgeOptions {
  playbackDurationMs?: number;
  startDelayMs?: number;
}

export function createMockBridge(options: MockBridgeOptions = {}): DesktopBridge {
  const playbackDurationMs = Math.max(50, options.playbackDurationMs ?? 15_000);
  const startDelayMs = Math.max(0, options.startDelayMs ?? 30);
  let generation = 1;
  let settings = initialSettings();
  let playbackSessionSequence = 0;
  let activeSession: {
    sessionId: string;
    songId: string;
    title: string;
    config: PlaybackConfig;
    totalUs: number;
    startedAt: number;
    pausedAt: number | null;
    pausedTotalMs: number;
    state: 'starting' | 'playing' | 'paused' | 'stopping';
  } | null = null;
  let playbackTimer: ReturnType<typeof setInterval> | null = null;
  const preparedConfigs = new Map<string, PlaybackConfig>();
  let diagnosticsEnabled = false;
  let diagnosticsSeq = 0;
  let diagnosticsTimer: ReturnType<typeof setInterval> | null = null;
  let calibration: { operationId: string; state: CalibrationStateId } | null = null;
  const likedSongIds = new Set<string>();
  const playlistMembership = new Map<string, Set<string>>();
  const localRows = new Map<string, SearchResult['items'][number]>([
    [mockId(900), row(900, 'Local Song B')],
    [mockId(901), row(901, 'Local Song C')],
  ]);
  let playlists: LibraryPlaylistSummary[] = [];
  let playlistSequence = 0;
  let calibrationTimer: ReturnType<typeof setTimeout> | null = null;
  const listeners = new Set<(event: UiEvent) => void>();
  const emit = (event: UiEvent) => listeners.forEach((listener) => listener(event));
  const emitDiagnostics = () => {
    if (!diagnosticsEnabled) return;
    diagnosticsSeq += 1;
    const config = activeSession?.config ?? {
      ...settings.playback_defaults,
      dry_run: false,
    };
    const frameUs = Math.ceil(1_000_000 / config.fps);
    const frameBaseHoldUs = Math.ceil(config.hold_frames * frameUs);
    emit({
      v: 1,
      name: 'diagnostics.snapshot',
      payload: {
        seq: diagnosticsSeq,
        physical_session: true,
        player_attached: true,
        sender_sample_count: diagnosticsSeq,
        max_lateness_us: diagnosticsSeq * 10,
        p50_ms: 0.4,
        p95_ms: 1.1,
        sigma_onset_ms: 0.2,
        late_2ms: 0,
        late_5ms: 0,
        late_10ms: 0,
        max_sendinput_pre_call_lateness_us: diagnosticsSeq * 8,
        pre_call_late_2ms: 0,
        pre_call_late_5ms: 0,
        pre_call_late_10ms: 0,
        fps: config.fps,
        frame_us: frameUs,
        hold_frames: config.hold_frames,
        frame_base_hold_us: frameBaseHoldUs,
        timing_margin_us: config.timing_margin_us,
        min_hold_us: frameBaseHoldUs + config.timing_margin_us,
        min_release_gap_us: frameUs + config.timing_margin_us,
        down_late_grace_us: config.down_late_grace_us,
        timing_margin_recommendation: settings.timing_margin_recommendation,
        pre_call_lt_250us: 0,
        pre_call_250_500us: 0,
        pre_call_500_750us: 1,
        pre_call_750_1000us: 0,
        pre_call_1000_1500us: 0,
        pre_call_1500_2000us: 0,
        pre_call_ge_2000us: 0,
        active_keys: 0,
        stuck_keys: 0,
        keys_dropped: 0,
        chord_split_events: 0,
        missed_down_boundaries: 1,
        missed_down_keys: 1,
        missed_backlog_boundaries: 0,
        missed_hard_late_boundaries: 1,
        final_gate_cutoff_misses: 1,
        final_gate_control_rejections: 0,
        final_gate_target_changes: 1,
        final_gate_focus_losses: 1,
        final_gate_lease_expirations: 1,
        sendinput_partial_events: 1,
        sendinput_zero_progress_failures: 1,
        backend_status: 'healthy',
        release_max_us: 0,
        release_late_2ms: 0,
        session_id: activeSession?.sessionId ?? null,
        last_error: null,
      },
    });
  };
  const emitCalibrationFinished = (operationId: string, outcome: 'succeeded' | 'cancelled') => {
    const recommendedTimingMarginUs =
      outcome === 'succeeded'
        ? Math.ceil((settings.playback_defaults.down_late_grace_us + 300) / 100) * 100
        : null;
    if (outcome === 'succeeded') {
      settings = {
        ...settings,
        timing_margin_recommendation: {
          recommended_timing_margin_us: recommendedTimingMarginUs!,
          qualified: true,
          source: 'qualified_calibration',
        },
      };
    }
    emit({
      v: 1,
      name: 'calibration.finished',
      payload: {
        operation_id: operationId,
        outcome,
        status: outcome === 'succeeded' ? 'ready' : 'cancelled',
        recommended_timing_margin_us: recommendedTimingMarginUs,
        recommendation_qualified: outcome === 'succeeded',
        sample_count: outcome === 'succeeded' ? 24 : 0,
        source: outcome === 'succeeded' ? 'qualified_calibration' : 'unavailable',
        message: outcome === 'succeeded' ? 'Calibration completed.' : 'Calibration cancelled.',
      },
    });
  };
  const emitPlaybackState = (
    session: { sessionId: string; songId: string },
    state: 'starting' | 'playing' | 'paused' | 'stopping' | 'finished',
    message: string | null = null,
  ) => {
    emit({
      v: 1,
      name: 'playback.state_changed',
      payload: {
        session_id: session.sessionId,
        song_id: session.songId,
        state,
        physical: false,
        message,
        outcome: state === 'finished' ? 'finished' : null,
      },
    });
  };
  const stopPlaybackTimer = () => {
    if (playbackTimer === null) return;
    clearInterval(playbackTimer);
    playbackTimer = null;
  };
  const playbackElapsedMs = (session: NonNullable<typeof activeSession>) =>
    Math.max(
      0,
      (session.state === 'paused' ? (session.pausedAt ?? Date.now()) : Date.now()) -
        session.startedAt -
        session.pausedTotalMs -
        (session.state === 'starting' ? startDelayMs : 0),
    );
  const emitPlaybackSnapshot = (session: NonNullable<typeof activeSession>) => {
    const elapsedMs = Math.min(playbackDurationMs, playbackElapsedMs(session));
    const currentUs = Math.min(
      session.totalUs,
      Math.floor((elapsedMs / playbackDurationMs) * session.totalUs),
    );
    emit({
      v: 1,
      name: 'playback.snapshot',
      payload: {
        session_id: session.sessionId,
        seq: Math.max(1, Math.floor(elapsedMs / 40) + 1),
        state:
          session.state === 'starting'
            ? 'starting'
            : session.state === 'paused'
              ? 'paused'
              : 'playing',
        song_id: session.songId,
        title: session.title,
        current_us: currentUs,
        total_us: session.totalUs,
        pre_roll_remaining_us:
          session.state === 'starting'
            ? Math.max(0, startDelayMs - (Date.now() - session.startedAt)) * 1_000
            : 0,
        focus_state: session.state === 'starting' ? 'waiting' : 'focused',
        health: 'healthy',
        input_path_degraded: false,
        message: null,
      },
    });
  };
  const retirePlaybackSession = (
    session: NonNullable<typeof activeSession>,
    outcome: string,
    message: string,
  ) => {
    if (activeSession !== session) return;
    emitPlaybackState(session, 'finished', message);
    activeSession = null;
    stopPlaybackTimer();
    emit({
      v: 1,
      name: 'playback.finished',
      payload: {
        session_id: session.sessionId,
        song_id: session.songId,
        outcome,
        total_us: session.totalUs,
        message,
      },
    });
  };
  const allRows = () => [...rows, ...localRows.values()];
  const updatePlaylist = (playlistId: string, songIds: string[]) => {
    const playlist = playlists.find((item) => item.id === playlistId);
    if (!playlist) throw new Error('playlist was not found');
    const membership = playlistMembership.get(playlistId) ?? new Set<string>();
    songIds.forEach((songId) => membership.add(songId));
    playlistMembership.set(playlistId, membership);
    const next = { ...playlist, song_count: membership.size };
    playlists = playlists.map((item) => (item.id === playlistId ? next : item));
    return next;
  };
  const createMockImport = (
    playlistId: string,
    kind: 'file' | 'folder',
  ): LibraryPlaylistImportResult => {
    const importedSongIds = [...localRows.keys()].slice(0, kind === 'folder' ? 2 : 1);
    const playlist = updatePlaylist(playlistId, importedSongIds);
    generation += 1;
    emit({ v: 1, name: 'catalog.changed', payload: { generation, total: rows.length } });
    return {
      playlist,
      imported_song_count: importedSongIds.length,
      catalog_generation: generation,
    };
  };

  return {
    async bootstrap() {
      return {
        app_version: '3.5.0-mock',
        protocol_version: 1,
        native_build: MOCK_NATIVE,
        playback_defaults: settings.playback_defaults,
        timing_margin_recommendation: settings.timing_margin_recommendation,
        option_sets: {
          hold_frames: [1, 1.25, 1.5],
          tempo_scales: [0.75, 0.9, 1, 1.1],
          fps: [30, 60, 120],
          timing_margin_min_us: 0,
          timing_margin_max_us: 3_000,
          timing_margin_step_us: 100,
          down_late_grace_min_us: 0,
          down_late_grace_max_us: 5_000,
          down_late_grace_step_us: 100,
        },
        theme: settings.theme,
        telemetry_enabled: settings.telemetry_enabled,
        update_preferences: settings.update_preferences,
        catalog_generation: generation,
      };
    },
    async searchSongs(request: SearchRequest) {
      const query = request.query.trim().toLocaleLowerCase();
      const sourceRows =
        request.source.kind === 'smart' && request.source.id === 'liked'
          ? allRows().filter((item) => likedSongIds.has(item.song_id))
          : request.source.kind === 'playlist'
            ? allRows().filter((item) =>
                playlistMembership.get(request.source.id)?.has(item.song_id),
              )
            : rows;
      const filtered = query
        ? sourceRows.filter((item) => item.title.toLocaleLowerCase().includes(query))
        : sourceRows;
      return {
        items: filtered
          .slice(request.offset, request.offset + request.limit)
          .map((item) => ({ ...item, liked: likedSongIds.has(item.song_id) })),
        offset: request.offset,
        limit: request.limit,
        total: filtered.length,
        liked_total: likedSongIds.size,
        generation,
      };
    },
    async getSongDetail(request: DetailRequest) {
      const found = allRows().find((item) => item.song_id === request.songId);
      if (!found) throw new Error('song was not found');
      return {
        song_id: found.song_id,
        title: found.title,
        duration_us: found.duration_us ?? 0,
        note_count: found.note_count ?? 0,
        format_label: 'TXT',
        risk: {
          level: found.risk_level,
          headline: found.risk_level === 'low' ? 'Low timing risk' : 'Medium timing risk',
          reasons:
            found.risk_level === 'low'
              ? []
              : [
                  found.song_id === mockId(LONG_CONTENT_INDEX)
                    ? LONG_CONTENT_REASON
                    : 'Dense note transitions may need a slower tempo.',
                ],
          recommendations:
            found.risk_level === 'low' ? ['Keep the selected settings.'] : ['Try 0.9× tempo.'],
        },
        recommendation: {
          recommended_hold_frames: 2,
          recommended_tempo_scale: found.risk_level === 'low' ? 1 : 0.9,
          summary:
            found.risk_level === 'low' ? 'Keep the selected settings.' : 'Try a slower tempo.',
        },
      };
    },
    async reloadLibrary() {
      generation += 1;
      emit({ v: 1, name: 'catalog.changed', payload: { generation, total: rows.length } });
      return { generation, total: rows.length };
    },
    async setLibraryViewport(request: ViewportRequest): Promise<ViewportResult> {
      if (request.generation !== generation) throw new Error('catalog generation is stale');
      return {
        accepted: true,
        generation,
        first_index: request.firstIndex,
        last_index: request.lastIndex,
        selected_song_id: request.selectedSongId,
        items: (request.songIds.length > 0
          ? request.songIds
              .map((songId) => allRows().find((item) => item.song_id === songId))
              .filter((item): item is (typeof rows)[number] => item !== undefined)
          : rows.slice(request.firstIndex, request.lastIndex + 1)
        ).map((item) => ({ ...item, liked: likedSongIds.has(item.song_id) })),
      };
    },
    async setSongLiked(request) {
      if (!allRows().some((item) => item.song_id === request.songId)) {
        throw new Error('song was not found');
      }
      if (request.liked) likedSongIds.add(request.songId);
      else likedSongIds.delete(request.songId);
      return { song_id: request.songId, liked: request.liked, total: likedSongIds.size };
    },
    async listLibraryNavigation(): Promise<LibraryNavigation> {
      return {
        playlists: playlists.map((playlist) => ({ ...playlist })),
      };
    },
    async createPlaylist(name: string): Promise<LibraryPlaylistSummary> {
      playlistSequence += 1;
      const playlist: LibraryPlaylistSummary = {
        id: `mock-playlist-${playlistSequence}`,
        name: name.trim(),
        song_count: 0,
      };
      playlists = [...playlists, playlist];
      playlistMembership.set(playlist.id, new Set());
      return playlist;
    },
    async renamePlaylist(playlistId: string, name: string): Promise<LibraryPlaylistSummary> {
      const index = playlists.findIndex((playlist) => playlist.id === playlistId);
      const current = playlists[index];
      if (!current) throw new Error('playlist was not found');
      const playlist: LibraryPlaylistSummary = { ...current, name: name.trim() };
      playlists = playlists.map((item, itemIndex) => (itemIndex === index ? playlist : item));
      return playlist;
    },
    async deletePlaylist(playlistId: string): Promise<boolean> {
      const next = playlists.filter((playlist) => playlist.id !== playlistId);
      const removed = next.length !== playlists.length;
      playlists = next;
      if (removed) playlistMembership.delete(playlistId);
      return removed;
    },
    async addSongsToPlaylist(
      playlistId: string,
      songIds: string[],
    ): Promise<LibraryPlaylistSummary> {
      return updatePlaylist(playlistId, songIds);
    },
    async removeSongsFromPlaylist(
      playlistId: string,
      songIds: string[],
    ): Promise<LibraryPlaylistSummary> {
      const playlist = playlists.find((item) => item.id === playlistId);
      if (!playlist) throw new Error('playlist was not found');
      const membership = playlistMembership.get(playlistId) ?? new Set<string>();
      songIds.forEach((songId) => membership.delete(songId));
      playlistMembership.set(playlistId, membership);
      const next = { ...playlist, song_count: membership.size };
      playlists = playlists.map((item) => (item.id === playlistId ? next : item));
      return next;
    },
    async importLocalFilesToPlaylist(playlistId: string): Promise<LibraryPlaylistImportResult> {
      return createMockImport(playlistId, 'file');
    },
    async importLocalFolderToPlaylist(playlistId: string): Promise<LibraryPlaylistImportResult> {
      return createMockImport(playlistId, 'folder');
    },
    async getSettings() {
      return settings;
    },
    async patchSettings(patch: SettingsPatch) {
      const playback = patch.playbackDefaults;
      settings = {
        ...settings,
        ...(patch.theme === undefined ? {} : { theme: patch.theme as ThemeId }),
        ...(patch.telemetryEnabled === undefined
          ? {}
          : { telemetry_enabled: patch.telemetryEnabled }),
        ...(patch.verboseHud === undefined ? {} : { verbose_hud: patch.verboseHud }),
        ...(playback === undefined
          ? {}
          : {
              playback_defaults: {
                ...settings.playback_defaults,
                ...(playback.holdFrames === undefined ? {} : { hold_frames: playback.holdFrames }),
                ...(playback.timingMarginUs === undefined
                  ? {}
                  : { timing_margin_us: playback.timingMarginUs }),
                ...(playback.downLateGraceUs === undefined
                  ? {}
                  : { down_late_grace_us: playback.downLateGraceUs }),
                ...(playback.tempoScale === undefined ? {} : { tempo_scale: playback.tempoScale }),
                ...(playback.fps === undefined ? {} : { fps: playback.fps }),
              },
            }),
        ...(patch.updatePreferences === undefined
          ? {}
          : {
              update_preferences: {
                ...settings.update_preferences,
                ...(patch.updatePreferences.autoCheck === undefined
                  ? {}
                  : { auto_check: patch.updatePreferences.autoCheck }),
                ...(patch.updatePreferences.channel === undefined
                  ? {}
                  : { channel: patch.updatePreferences.channel }),
                ...(patch.updatePreferences.skipVersion === undefined
                  ? {}
                  : { skip_version: patch.updatePreferences.skipVersion }),
              },
            }),
      };
      if (playback?.downLateGraceUs !== undefined) {
        settings = {
          ...settings,
          timing_margin_recommendation: {
            ...settings.timing_margin_recommendation,
            recommended_timing_margin_us: settings.timing_margin_recommendation.qualified
              ? Math.ceil((playback.downLateGraceUs + 300) / 100) * 100
              : 500,
          },
        };
      }
      return settings;
    },
    async checkForUpdate(): Promise<UpdateCheck> {
      const result: UpdateCheck = {
        state: 'available',
        current_version: '4.0.0-alpha.1-mock',
        available_version: '4.0.0-alpha.2-mock',
        channel: settings.update_preferences.channel,
        release_notes: 'A deterministic update fixture for the desktop UI.',
        published_at: '2026-08-30T00:00:00Z',
        error: null,
      };
      emit({
        v: 1,
        name: 'update.available',
        payload: {
          current_version: result.current_version,
          available_version: result.available_version!,
          channel: result.channel,
          release_notes: result.release_notes,
          published_at: result.published_at,
        },
      });
      emit({
        v: 1,
        name: 'update.result',
        payload: {
          state: result.state,
          current_version: result.current_version,
          available_version: result.available_version,
          channel: result.channel,
          error: result.error,
        },
      });
      return result;
    },
    async getUpdatePreferences(): Promise<UpdatePreferences> {
      return settings.update_preferences;
    },
    async patchUpdatePreferences(patch: UpdatePatch): Promise<UpdatePreferences> {
      settings = {
        ...settings,
        update_preferences: {
          ...settings.update_preferences,
          ...(patch.autoCheck === undefined ? {} : { auto_check: patch.autoCheck }),
          ...(patch.channel === undefined ? {} : { channel: patch.channel }),
          ...(patch.skipVersion === undefined ? {} : { skip_version: patch.skipVersion }),
        },
      };
      return settings.update_preferences;
    },
    async beginUpdateHandoff(targetVersion: string): Promise<UpdateHandoff> {
      const handoff: UpdateHandoff = {
        handoff_id: `h${Date.now().toString(16).padStart(31, '0')}`.slice(-32),
        target_version: targetVersion,
        state: 'installing',
      };
      emit({
        v: 1,
        name: 'update.progress',
        payload: {
          operation_id: handoff.handoff_id,
          state: 'installing',
          available_version: targetVersion,
          completed: 1,
          total: 1,
          message: 'Installing update and restarting',
        },
      });
      return handoff;
    },
    async preparePlayback(request) {
      const found = allRows().find((item) => item.song_id === request.songId);
      if (!found) throw new Error('song was not found');
      const risk = found.risk_level === 'low' ? 'low' : 'medium';
      const preparedId = `prepared-${found.song_id}`;
      preparedConfigs.set(preparedId, request.config);
      return {
        prepared_id: preparedId,
        song: {
          song_id: found.song_id,
          title: found.title,
          duration_us: found.duration_us ?? 0,
          note_count: found.note_count ?? 0,
          format_label: 'TXT',
          risk: {
            level: risk,
            headline: risk === 'low' ? 'Low timing risk' : 'Medium timing risk',
            reasons:
              risk === 'low'
                ? []
                : [
                    found.song_id === mockId(LONG_CONTENT_INDEX)
                      ? LONG_CONTENT_REASON
                      : 'Dense note transitions may need a slower tempo.',
                  ],
            recommendations: ['Keep the selected settings.'],
          },
          recommendation: null,
        },
        config: request.config,
        admission: risk === 'low' ? 'ready' : 'confirmation_required',
        risk: {
          level: risk,
          headline: risk === 'low' ? 'Low timing risk' : 'Medium timing risk',
          reasons:
            risk === 'low'
              ? []
              : [
                  found.song_id === mockId(LONG_CONTENT_INDEX)
                    ? LONG_CONTENT_REASON
                    : 'Dense note transitions may need a slower tempo.',
                ],
          recommendations: ['Keep the selected settings.'],
        },
        decisions:
          risk === 'low'
            ? []
            : [
                { decision: 'proceed', label: 'Proceed with current settings' },
                { decision: 'use_recommended', label: 'Use recommended settings' },
                { decision: 'dry_run', label: 'Run a dry-run first' },
              ],
        plan_fingerprint: 'mock-plan',
        variants:
          risk === 'low'
            ? [
                {
                  decision: 'proceed',
                  config: request.config,
                  plan_fingerprint: 'mock-plan',
                },
              ]
            : [
                {
                  decision: 'proceed',
                  config: request.config,
                  plan_fingerprint: 'mock-plan',
                },
                {
                  decision: 'use_recommended',
                  config: request.config,
                  plan_fingerprint: 'mock-recommended-plan',
                },
                {
                  decision: 'dry_run',
                  config: { ...request.config, dry_run: true },
                  plan_fingerprint: 'mock-dry-run-plan',
                },
              ],
        error_code: null,
        error_message: null,
      };
    },
    async startPlayback(request) {
      if (activeSession) throw new Error('another playback session is active');
      const baseConfig = preparedConfigs.get(request.preparedId) ?? {
        ...settings.playback_defaults,
        dry_run: false,
      };
      const config = request.decisions.some((item) => item.decision === 'dry_run')
        ? { ...baseConfig, dry_run: true }
        : baseConfig;
      const songId = request.preparedId.replace('prepared-', '');
      const song = allRows().find((item) => item.song_id === songId);
      if (!song) throw new Error('song was not found');
      playbackSessionSequence += 1;
      const session = {
        sessionId: playbackSessionSequence.toString(16).padStart(32, '0'),
        songId,
        title: song.title,
        config,
        totalUs: song.duration_us ?? 1_000_000,
        startedAt: Date.now(),
        pausedAt: null,
        pausedTotalMs: 0,
        state: 'starting' as 'starting' | 'playing' | 'paused' | 'stopping',
      };
      activeSession = session;
      emitPlaybackState(session, 'starting');
      stopPlaybackTimer();
      playbackTimer = setInterval(() => {
        if (activeSession !== session) return;
        if (session.state === 'starting' && Date.now() - session.startedAt >= startDelayMs) {
          session.state = 'playing';
          emitPlaybackState(session, 'playing');
        }
        if (session.state === 'playing' || session.state === 'paused') {
          emitPlaybackSnapshot(session);
        }
        if (session.state === 'playing' && playbackElapsedMs(session) >= playbackDurationMs) {
          retirePlaybackSession(session, 'finished', 'Playback finished');
        }
      }, 40);
      return {
        session_id: session.sessionId,
        prepared_id: request.preparedId,
        song_id: session.songId,
        state: 'starting',
        config,
        plan_fingerprint: request.decisions.some((item) => item.decision === 'use_recommended')
          ? 'mock-recommended-plan'
          : request.decisions.some((item) => item.decision === 'dry_run')
            ? 'mock-dry-run-plan'
            : 'mock-plan',
      };
    },
    async stopPlayback(request) {
      if (activeSession?.sessionId !== request.sessionId) throw new Error('stale session');
      const session = activeSession;
      if (session.state !== 'stopping') {
        session.state = 'stopping';
        emitPlaybackState(session, 'stopping');
        setTimeout(() => retirePlaybackSession(session, 'quit', 'Playback stopped'), 0);
      }
      return {
        accepted: true,
        session_id: request.sessionId,
        state: 'stopping',
        pending_command: null,
        reason: null,
      };
    },
    async pausePlayback(request) {
      if (activeSession?.sessionId !== request.sessionId) throw new Error('stale session');
      const session = activeSession;
      if (session.state !== 'playing') throw new Error('pause requires a playing session');
      session.state = 'paused';
      session.pausedAt = Date.now();
      emitPlaybackState(session, 'paused');
      return {
        accepted: true,
        session_id: request.sessionId,
        state: 'playing',
        pending_command: 'pause',
        reason: null,
      };
    },
    async resumePlayback(request) {
      if (activeSession?.sessionId !== request.sessionId) throw new Error('stale session');
      const session = activeSession;
      if (session.state !== 'paused') throw new Error('resume requires a paused session');
      session.pausedTotalMs += Date.now() - (session.pausedAt ?? Date.now());
      session.pausedAt = null;
      session.state = 'playing';
      emitPlaybackState(session, 'playing');
      return {
        accepted: true,
        session_id: request.sessionId,
        state: 'paused',
        pending_command: 'resume',
        reason: null,
      };
    },
    async skipPlayback(request) {
      if (activeSession?.sessionId !== request.sessionId) throw new Error('stale session');
      const session = activeSession;
      if (session.state !== 'stopping') {
        session.state = 'stopping';
        emitPlaybackState(session, 'stopping');
        setTimeout(() => retirePlaybackSession(session, 'skipped', 'Playback skipped'), 0);
      }
      return {
        accepted: true,
        session_id: request.sessionId,
        state: 'stopping',
        pending_command: null,
        reason: null,
      };
    },
    async setDiagnosticsEnabled(request: DiagnosticsSetEnabled): Promise<DiagnosticsEnabled> {
      diagnosticsEnabled = request.enabled;
      if (diagnosticsTimer !== null) {
        clearInterval(diagnosticsTimer);
        diagnosticsTimer = null;
      }
      if (diagnosticsEnabled) {
        emitDiagnostics();
        diagnosticsTimer = setInterval(emitDiagnostics, 100);
      }
      return { enabled: diagnosticsEnabled };
    },
    async exportSenderTrace(): Promise<string> {
      return JSON.stringify({
        export_schema_version: 1,
        session_id: 'a'.repeat(32),
        song_id: 'fixture-song',
        song_title: 'Fixture Song',
        plan_fingerprint: 'b'.repeat(64),
        timing_policy: {
          fps: 60,
          frame_period_us: 16_667,
          frame_base_hold_us: 16_667,
          timing_margin_us: 500,
          target_hold_us: 17_167,
          release_gap_us: 17_167,
          late_down_tolerance_us: 2_000,
        },
        telemetry: {
          schema_version: 14,
          qpc_frequency_hz: 10_000_000,
          records: [],
          attempted: 0,
          accepted: 0,
          dropped: 0,
          observer_queue_dropped: 0,
          truncated: false,
        },
      });
    },
    async startCalibration(request: CalibrationStart): Promise<CalibrationStartAck> {
      if (activeSession) throw new Error('calibration conflicts with active playback');
      if (calibration && ['starting', 'running', 'cancelling'].includes(calibration.state)) {
        throw new Error('calibration is already running');
      }
      const operationId = `c${Date.now().toString(16).padStart(31, '0')}`.slice(-32);
      calibration = { operationId, state: 'running' };
      emit({
        v: 1,
        name: 'calibration.progress',
        payload: {
          operation_id: operationId,
          state: 'running',
          phase: request.mode,
          completed: 0,
          total: 3,
          message: 'Calibration is running.',
        },
      });
      calibrationTimer = setTimeout(() => {
        if (!calibration || calibration.operationId !== operationId) return;
        calibration = { operationId, state: 'succeeded' };
        emitCalibrationFinished(operationId, 'succeeded');
      }, 30);
      return { operation_id: operationId, state: 'running' };
    },
    async cancelCalibration(request: CalibrationCancel): Promise<CalibrationCancelAck> {
      if (!calibration || calibration.operationId !== request.operationId) {
        throw new Error('calibration operation is stale');
      }
      if (calibration.state === 'succeeded' || calibration.state === 'cancelled') {
        return { operation_id: request.operationId, state: calibration.state, accepted: false };
      }
      if (calibrationTimer !== null) {
        clearTimeout(calibrationTimer);
        calibrationTimer = null;
      }
      calibration = { operationId: request.operationId, state: 'cancelled' };
      emitCalibrationFinished(request.operationId, 'cancelled');
      return { operation_id: request.operationId, state: 'cancelled', accepted: true };
    },
    async subscribeUiEvents(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async shutdown(_failed = false) {
      if (diagnosticsTimer !== null) clearInterval(diagnosticsTimer);
      stopPlaybackTimer();
      if (calibrationTimer !== null) clearTimeout(calibrationTimer);
      diagnosticsTimer = null;
      calibrationTimer = null;
      activeSession = null;
      listeners.clear();
    },
  };
}
