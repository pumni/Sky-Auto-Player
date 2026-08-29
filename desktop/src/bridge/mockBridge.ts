import type {
  Bootstrap,
  DesktopBridge,
  DetailRequest,
  SearchRequest,
  SearchResult,
  Settings,
  SettingsPatch,
  SongDetail,
  ThemeId,
  UiEvent,
  Unsubscribe,
  ViewportRequest,
  ViewportResult,
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

function mockId(index: number): string {
  return index.toString(16).padStart(32, '0');
}

function row(index: number, title: string): SearchResult['items'][number] {
  return {
    song_id: mockId(index),
    title,
    duration_us: 120_000_000 + index * 5_000_000,
    note_count: 128 + index * 9,
    risk_level: index % 5 === 0 ? 'medium' : 'low',
    metadata_state: 'ready',
  };
}

const rows = Array.from({ length: 500 }, (_, index) =>
  row(index, titles[index] ?? `Song ${String(index + 1).padStart(3, '0')}`),
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
  const listeners = new Set<(event: UiEvent) => void>();
  const emit = (event: UiEvent) => listeners.forEach((listener) => listener(event));
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
      const filtered = query
        ? rows.filter((item) => item.title.toLocaleLowerCase().includes(query))
        : rows;
      return {
        items: filtered.slice(request.offset, request.offset + request.limit),
        offset: request.offset,
        limit: request.limit,
        total: filtered.length,
        generation,
      };
    },
    async getSongDetail(request: DetailRequest) {
      const found = rows.find((item) => item.song_id === request.songId);
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
            found.risk_level === 'low' ? [] : ['Dense note transitions may need a slower tempo.'],
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
      };
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
      };
      return settings;
    },
    async preparePlayback(request) {
      const found = rows.find((item) => item.song_id === request.songId);
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
            reasons: risk === 'low' ? [] : ['Dense note transitions may need a slower tempo.'],
            recommendations: ['Keep the selected settings.'],
          },
          recommendation: null,
        },
        config: request.config,
        admission: risk === 'low' ? 'ready' : 'confirmation_required',
        risk: {
          level: risk,
          headline: risk === 'low' ? 'Low timing risk' : 'Medium timing risk',
          reasons: risk === 'low' ? [] : ['Dense note transitions may need a slower tempo.'],
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
    async subscribeUiEvents(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    async shutdown() {},
  };
}
