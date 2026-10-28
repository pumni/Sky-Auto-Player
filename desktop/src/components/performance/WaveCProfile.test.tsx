import { Profiler } from 'react';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SongRow, UiEvent } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
import { DiagnosticsView } from '../utility/DiagnosticsView';
import { PlayerTrackInfo } from '../player/PlayerTrackInfo';
import { PlayerTransport } from '../player/PlayerTransport';
import { TrackTable } from '../tracks/TrackTable';

const sessionId = 'c'.repeat(32);

function rows(count: number): SongRow[] {
  return Array.from({ length: count }, (_, index) => ({
    song_id: index.toString(16).padStart(32, '0'),
    title: `Profile Song ${index}`,
    format_label: 'JSON',
    duration_us: 30_000_000,
    note_count: 100,
    risk_level: 'low',
    metadata_state: 'ready',
    liked: false,
  }));
}

type ProfileScenario = {
  name: string;
  librarySize: number;
  playing: boolean;
  diagnostics: boolean;
  selectedIndex: number;
};

const scenarios: ProfileScenario[] = [
  {
    name: 'normal-idle-diagnostics-off-visible-selection',
    librarySize: 24,
    playing: false,
    diagnostics: false,
    selectedIndex: 0,
  },
  {
    name: 'normal-playing-diagnostics-off-visible-selection',
    librarySize: 24,
    playing: true,
    diagnostics: false,
    selectedIndex: 0,
  },
  {
    name: 'large-playing-diagnostics-on-offscreen-selection',
    librarySize: 500,
    playing: true,
    diagnostics: true,
    selectedIndex: 250,
  },
];

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('Wave C controlled UI profile', () => {
  it.each(scenarios)('$name', async (scenario) => {
    const bridge = createMockBridge({
      catalogRows: rows(scenario.librarySize),
      emitSnapshots: false,
    });
    let receiveEvent: ((event: UiEvent) => void) | null = null;
    bridge.subscribeUiEvents = async (listener) => {
      receiveEvent = listener;
      return () => {
        receiveEvent = null;
      };
    };
    const searchCalls = vi.spyOn(bridge, 'searchSongs');
    const viewportCalls = vi.spyOn(bridge, 'setLibraryViewport');
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const currentLibrary = store.getState().library;
    const playingRow = selectRowAtIndex(currentLibrary, 1);
    const selectedRow = selectRowAtIndex(currentLibrary, scenario.selectedIndex);
    if (!playingRow) {
      throw new Error('profile fixture failed to load the requested rows');
    }
    const currentSong = {
      songId: playingRow.song_id,
      title: playingRow.title,
      liked: playingRow.liked,
      durationUs: playingRow.duration_us,
      formatLabel: playingRow.format_label,
      noteCount: playingRow.note_count,
      riskLevel: playingRow.risk_level,
      generation: currentLibrary.generation,
    };
    const context = {
      source: currentLibrary.searchSource,
      query: currentLibrary.query,
      generation: currentLibrary.generation,
      total: currentLibrary.resultTotal,
      currentIndex: 1,
      currentSongId: playingRow.song_id,
      shuffleTraversal: null,
      dryRun: false,
      membershipRevision: 0,
      valid: true,
    };
    act(() =>
      store.setState({
        library: {
          ...currentLibrary,
          selectedSongId:
            selectedRow?.song_id ?? scenario.selectedIndex.toString(16).padStart(32, '0'),
        },
        diagnostics: { ...store.getState().diagnostics, enabled: scenario.diagnostics },
        playback: {
          ...store.getState().playback,
          state: scenario.playing ? 'playing' : 'idle',
          sessionId: scenario.playing ? sessionId : null,
          currentSong: scenario.playing ? currentSong : null,
          context: scenario.playing ? context : null,
          snapshot: null,
        },
      }),
    );

    const commits = new Map<string, number>();
    const profile = (id: string) => (_id: string, _phase: string) => {
      commits.set(id, (commits.get(id) ?? 0) + 1);
    };
    const { container } = render(
      <>
        <Profiler id="transport" onRender={profile('transport')}>
          <PlayerTransport useStore={store} />
        </Profiler>
        <Profiler id="track-info" onRender={profile('track-info')}>
          <PlayerTrackInfo useStore={store} />
        </Profiler>
        <Profiler id="diagnostics" onRender={profile('diagnostics')}>
          <DiagnosticsView useStore={store} />
        </Profiler>
        <Profiler id="track-table" onRender={profile('track-table')}>
          <TrackTable useStore={store} />
        </Profiler>
      </>,
    );
    await waitFor(() => expect(viewportCalls).toHaveBeenCalled());
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
    const setupBridgeRequests = {
      searchSongs: searchCalls.mock.calls.length,
      setLibraryViewport: viewportCalls.mock.calls.length,
    };

    const activeDescendantQueries = vi.spyOn(document, 'getElementById');
    const mutationCounts = { childList: 0, attributes: 0, characterData: 0 };
    const observer = new MutationObserver((records) => {
      for (const record of records) mutationCounts[record.type] += 1;
    });
    observer.observe(container, {
      attributes: true,
      characterData: true,
      childList: true,
      subtree: true,
    });
    activeDescendantQueries.mockClear();
    searchCalls.mockClear();
    viewportCalls.mockClear();
    commits.clear();

    let incomingSnapshots = 0;
    if (scenario.playing) {
      for (let seq = 1; seq <= 10; seq += 1) {
        const event: UiEvent = {
          v: 1,
          name: 'playback.snapshot',
          payload: {
            session_id: sessionId,
            seq,
            state: 'playing',
            song_id: currentSong.songId,
            title: currentSong.title,
            current_us: seq * 100_000,
            total_us: 30_000_000,
            pre_roll_remaining_us: 0,
            focus_state: 'focused',
            health: 'healthy',
            input_path_degraded: false,
            message: null,
          },
        };
        await act(async () => {
          receiveEvent?.(event);
          incomingSnapshots += 1;
        });
      }
    }
    await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
    observer.disconnect();

    const title = container.querySelector('.player-track-info strong')?.textContent ?? null;
    expect(title).toBe(scenario.playing ? playingRow.title : selectedRow?.title);
    expect(activeDescendantQueries).not.toHaveBeenCalled();
    expect(commits.get('track-info') ?? 0).toBe(0);
    expect(commits.get('diagnostics') ?? 0).toBe(0);
    expect(commits.get('track-table') ?? 0).toBe(0);
    expect(searchCalls).not.toHaveBeenCalled();
    expect(viewportCalls).not.toHaveBeenCalled();
    expect(container.querySelectorAll('.player-controls-row button')).toHaveLength(5);
    const grid = container.querySelector('[role="grid"]');
    if (scenario.playing && scenario.selectedIndex === 250) {
      expect(grid).not.toHaveAttribute('aria-activedescendant');
    } else {
      expect(grid).toHaveAttribute(
        'aria-activedescendant',
        `song-row-${store.getState().library.selectedSongId}`,
      );
    }

    console.info(
      '[wave-c-profile]',
      JSON.stringify({
        scenario: scenario.name,
        incomingPlaybackSnapshots: incomingSnapshots,
        componentCommits: Object.fromEntries(commits),
        domMutationRecords: mutationCounts,
        activeDescendantElementQueries: activeDescendantQueries.mock.calls.length,
        setupBridgeRequests,
        bridgeRequests: {
          searchSongs: searchCalls.mock.calls.length,
          setLibraryViewport: viewportCalls.mock.calls.length,
        },
        semanticOutput: {
          playerTitle: title,
          progress: container.querySelector('progress')?.getAttribute('value'),
          transportButtons: Array.from(
            container.querySelectorAll<HTMLButtonElement>('.player-controls-row button'),
          ).map((button) => ({
            label: button.getAttribute('aria-label'),
            disabled: button.disabled,
          })),
          selectedSongId: store.getState().library.selectedSongId,
          playingSongId: store.getState().playback.currentSong?.songId ?? null,
          activeDescendant: grid?.getAttribute('aria-activedescendant') ?? null,
        },
      }),
    );
  });
});
