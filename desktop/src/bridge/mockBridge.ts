import type {
  Bootstrap,
  CalibrationCancel,
  CalibrationCancelAck,
  CalibrationFinished,
  CalibrationProgress,
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
  SongDetail,
  ThemeId,
  UiEvent,
  Unsubscribe,
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
    playback_defaults: { hold_frames: 2, tempo_scale: 1, fps: 60, dry_run: false },
    telemetry_enabled: true,
    verbose_hud: false,
    update_preferences: { auto_check: true, channel: 'stable', skip_version: '' },
  };
}

export function createMockBridge(): DesktopBridge {
  let generation = 1;
  let settings = initialSettings();
  let activeSession: { sessionId: string; songId: string } | null = null;
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
    emit({
      v: 1,
      name: 'diagnostics.snapshot',
      payload: {
        seq: diagnosticsSeq,
        max_lateness_us: diagnosticsSeq * 10,
        p50_ms: 0.4,
        p95_ms: 1.1,
        sigma_onset_ms: 0.2,
        late_2ms: 0,
        late_5ms: 0,
        late_10ms: 0,
        active_keys: 0,
        stuck_keys: 0,
        keys_dropped: 0,
        chord_split_events: 0,
        backend_status: 'healthy',
        release_max_us: null,
        release_late_2ms: null,
        session_id: activeSession?.sessionId ?? null,
      },
    });
  };
  const emitCalibrationFinished = (operationId: string, outcome: 'succeeded' | 'cancelled') => {
    emit({
      v: 1,
      name: 'calibration.finished',
      payload: {
        operation_id: operationId,
        outcome,
        status: outcome === 'succeeded' ? 'ready' : 'cancelled',
        margin_us: outcome === 'succeeded' ? 850 : null,
        sample_count: outcome === 'succeeded' ? 24 : 0,
        source: 'mock',
        message: outcome === 'succeeded' ? 'Calibration completed.' : 'Calibration cancelled.',
        applied: outcome === 'succeeded',
      },
    });
  };
  const emitPlaybackState = (
    session: { sessionId: string; songId: string },
    state: 'playing' | 'paused' | 'stopping' | 'finished',
  ) => {
    emit({
      v: 1,
      name: 'playback.state_changed',
      payload: {
        session_id: session.sessionId,
        song_id: session.songId,
        state,
        physical: false,
        message: null,
        outcome: state === 'finished' ? 'finished' : null,
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
        option_sets: {
          hold_frames: [1, 2, 3, 4],
          tempo_scales: [0.75, 0.9, 1, 1.1],
          fps: [30, 60, 120],
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
      return {
        prepared_id: `prepared-${found.song_id}`,
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
      const session = {
        sessionId: 'b'.repeat(32),
        songId: request.preparedId.replace('prepared-', ''),
      };
      activeSession = session;
      setTimeout(() => emitPlaybackState(session, 'playing'), 0);
      return {
        session_id: session.sessionId,
        prepared_id: request.preparedId,
        song_id: session.songId,
        state: 'starting',
        config: { hold_frames: 2, tempo_scale: 1, fps: 60, dry_run: false },
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
      activeSession = null;
      setTimeout(() => {
        emitPlaybackState(session, 'stopping');
        emitPlaybackState(session, 'finished');
      }, 0);
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
      setTimeout(() => emitPlaybackState(session, 'paused'), 0);
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
      setTimeout(() => emitPlaybackState(session, 'playing'), 0);
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
      activeSession = null;
      setTimeout(() => {
        emitPlaybackState(session, 'stopping');
        emitPlaybackState(session, 'finished');
      }, 0);
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
      if (calibrationTimer !== null) clearTimeout(calibrationTimer);
      diagnosticsTimer = null;
      calibrationTimer = null;
      listeners.clear();
    },
  };
}
