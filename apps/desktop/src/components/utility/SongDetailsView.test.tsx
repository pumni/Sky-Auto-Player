import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Settings } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { selectRowAtIndex, selectSelectedDetail, createDesktopStore } from '../../state/store';
import { SongDetailsView } from './SongDetailsView';

describe('SongDetailsView', () => {
  afterEach(() => cleanup());

  it('shows a centered song-selection cue when Inspector has no selected song', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const { container } = render(<SongDetailsView useStore={store} />);
    expect(screen.getByText('No song selected')).toBeInTheDocument();
    expect(screen.getByText('Select a song to view details.')).toBeInTheDocument();
    expect(container.querySelector('svg[aria-hidden="true"]')).toHaveClass(
      'lucide-mouse-pointer-2',
    );
    expect(screen.getByLabelText('Song details')).toHaveAttribute('data-scroll-region', '');
  });

  async function selectFixture(index: number) {
    const songId = index.toString(16).padStart(32, '0');
    const bridge = createMockBridge({ builtinSongIds: [songId] });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectSong(songId));
    return { bridge, store, songId };
  }

  it('keeps the detail skeleton and summary stable while a new song loads', async () => {
    const bridge = createMockBridge();
    const originalDetail = bridge.getSongDetail;
    let releaseDetail: (() => void) | undefined;
    bridge.getSongDetail = async (request) => {
      if (request.songId === '1'.repeat(32)) {
        await new Promise<void>((resolve) => {
          releaseDetail = resolve;
        });
      }
      return originalDetail(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    const second = selectRowAtIndex(store.getState().library, 1);
    if (!first || !second) throw new Error('mock library is too small');
    const pendingSecond = {
      ...second,
      metadata_state: 'pending' as const,
      duration_us: null,
      note_count: null,
    };
    const pages = new Map(store.getState().library.pages);
    pages.set(0, [first, pendingSecond, ...(pages.get(0) ?? []).slice(2)]);
    store.setState({ library: { ...store.getState().library, pages } });

    await act(async () => store.getState().selectSong(first.song_id));
    render(<SongDetailsView useStore={store} />);
    expect(screen.getByRole('heading', { name: first.title })).toBeInTheDocument();
    expect(screen.getByText('Selected song')).toBeInTheDocument();
    expect(screen.getByText('Timing risk')).toBeInTheDocument();
    expect(screen.queryByText(/Test playback \(no input\).*dry-run playback/i)).toBeNull();

    const pending = store.getState().selectSong(second.song_id);
    await act(async () => Promise.resolve());

    expect(screen.getByRole('heading', { name: second.title })).toBeInTheDocument();
    expect(screen.queryByText('Medium timing risk')).toBeNull();
    expect(screen.getByText('Timing rationale and settings')).toBeInTheDocument();
    expect(screen.getByText('TXT · … · … notes')).toBeInTheDocument();

    releaseDetail?.();
    await act(async () => pending);
    expect(screen.getByText('TXT · … · … notes')).toBeInTheDocument();
  });

  it('keeps the selected song visible after its browser page is evicted', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const selected = selectRowAtIndex(store.getState().library, 0);
    if (!selected) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(selected.song_id));
    store.setState({
      library: {
        ...store.getState().library,
        pages: new Map(),
        indexById: new Map(),
      },
    });

    render(<SongDetailsView useStore={store} />);

    expect(store.getState().library.selectedSongSummary?.song_id).toBe(selected.song_id);
    expect(screen.getByRole('heading', { name: selected.title })).toBeInTheDocument();
  });

  it('shows supported versioned track and pitch controls and refreshes diagnostics on selection', async () => {
    const { bridge, store } = await selectFixture(496);
    const getDetail = vi.spyOn(bridge, 'getSongDetail');
    render(<SongDetailsView useStore={store} />);

    expect(screen.getByText('Composed v5 · schema 5 · requires confirmation')).toBeInTheDocument();
    expect(screen.getByText('Technical conversion counts')).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Conversion mode' })).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Target Sky Piano pitch' })).toBeInTheDocument();
    expect(screen.getByText(/assumed from the source song/i)).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole('combobox', { name: 'Conversion mode' }));
    });
    const firstTrackOption = screen
      .getAllByRole('option')
      .find((option) => option.getAttribute('data-value') === 'track:0');
    if (!firstTrackOption) throw new Error('expected track:0 option');
    await act(async () => {
      fireEvent.keyDown(firstTrackOption, { key: 'Enter' });
    });
    expect(getDetail).toHaveBeenLastCalledWith(
      expect.objectContaining({ conversionMode: { kind: 'single_track', track_id: 0 } }),
    );
    expect(screen.getByText('Events from unselected tracks are omitted.')).toBeInTheDocument();
  });

  it('presents conversion loss consent separately from preparing the exact plan', async () => {
    const { bridge, store } = await selectFixture(496);
    const prepare = vi.spyOn(bridge, 'preparePlayback');
    render(<SongDetailsView useStore={store} />);

    await act(async () => store.getState().prepareSelectedPlayback());
    expect(store.getState().playback.prepared?.prepared_id).toBeNull();
    expect(store.getState().playback.error).toBeNull();
    expect(
      screen.getByRole('button', { name: 'Accept conversion losses and prepare' }),
    ).toBeInTheDocument();
    const token = store.getState().playback.prepared?.song.sheet_analysis?.confirmation_fingerprint;
    expect(token).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Accept conversion losses and prepare' }));
    await waitFor(() => expect(prepare).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(store.getState().playback.prepared?.prepared_id).not.toBeNull());
    expect(prepare.mock.calls[1]?.[0].conversionConfirmationFingerprint).toBe(token);
    expect(store.getState().playback.prepared?.song.sheet_analysis?.confirmation_received).toBe(
      true,
    );
    expect(screen.getByText('Conversion losses accepted for this plan.')).toBeInTheDocument();
  });

  it('keeps unsupported schema details visible and blocks conversion controls', async () => {
    const { store } = await selectFixture(497);
    render(<SongDetailsView useStore={store} />);

    expect(screen.getByText('Composed v99 · schema 99 · unsupported')).toBeInTheDocument();
    expect(screen.getByText(/Unsupported composed sheet version 99/)).toBeInTheDocument();
    expect(screen.queryByRole('combobox', { name: 'Conversion mode' })).toBeNull();
    expect(screen.queryByRole('combobox', { name: 'Target Sky Piano pitch' })).toBeNull();
  });

  it('explains when a versioned sheet uses its assumed source pitch', async () => {
    const { store } = await selectFixture(498);
    render(<SongDetailsView useStore={store} />);

    expect(screen.getByText('Composed v4 · schema 4 · ready')).toBeInTheDocument();
    expect(screen.getByText(/Sky Piano tuned to Ab/)).toBeInTheDocument();
    expect(screen.getByText(/assumed from the source song/i)).toBeInTheDocument();
  });

  it('applies recommended hold and tempo defaults from Song Details', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<SongDetailsView useStore={store} />);
    const applyButton = screen.getByRole('button', { name: 'Apply recommendation' });
    expect(applyButton).toBeEnabled();

    fireEvent.click(applyButton);
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Recommended settings applied' })).toBeDisabled(),
    );
    const recommendation = selectSelectedDetail(store.getState()).value?.recommendation;
    expect(store.getState().settings?.playback_defaults.hold_frames).toBe(
      recommendation?.recommended_hold_frames,
    );
    expect(store.getState().settings?.playback_defaults.tempo_scale).toBe(
      recommendation?.recommended_tempo_scale,
    );
  });

  it('displays inline error with role="alert" when patchSettings returns null, keeps retry enabled, and succeeds when retried', async () => {
    const bridge = createMockBridge();
    let shouldFailPatch = true;
    const originalPatch = bridge.patchSettings.bind(bridge);
    bridge.patchSettings = async (patch) => {
      if (shouldFailPatch) {
        throw new Error('Failed to apply recommendation. Please retry.');
      }
      return originalPatch(patch);
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<SongDetailsView useStore={store} />);
    const applyButton = screen.getByRole('button', { name: 'Apply recommendation' });
    expect(applyButton).toBeEnabled();

    // Click apply, which fails
    fireEvent.click(applyButton);
    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(/failed to apply recommendation/i);
    });
    const retryButton = screen.getByRole('button', { name: 'Retry recommendation' });
    expect(retryButton).toBeEnabled();

    // Now retry after backend succeeds
    shouldFailPatch = false;
    fireEvent.click(retryButton);
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Recommended settings applied' })).toBeDisabled();
    });
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('clears recommendation apply error when a different song is selected', async () => {
    const bridge = createMockBridge();
    bridge.patchSettings = async () => {
      throw new Error('Failed to apply recommendation.');
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    const second = selectRowAtIndex(store.getState().library, 5);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<SongDetailsView useStore={store} />);
    fireEvent.click(screen.getByRole('button', { name: 'Apply recommendation' }));
    await waitFor(() => {
      expect(screen.getByRole('alert')).toBeInTheDocument();
    });

    // Select different song
    await act(async () => store.getState().selectSong(second.song_id));
    await waitFor(() => {
      expect(screen.queryByRole('alert')).toBeNull();
    });
  });

  it('ignores in-flight recommendation failure or success when song selection changes before response arrives', async () => {
    const bridge = createMockBridge();
    let rejectPatch!: (reason: unknown) => void;
    bridge.patchSettings = () =>
      new Promise((_, rej) => {
        rejectPatch = rej;
      });

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    const second = selectRowAtIndex(store.getState().library, 5);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<SongDetailsView useStore={store} />);
    const applyButton = screen.getByRole('button', { name: 'Apply recommendation' });
    fireEvent.click(applyButton);
    expect(screen.getByRole('button', { name: 'Applying…' })).toBeDisabled();

    // Switch to second song while request is in-flight
    await act(async () => store.getState().selectSong(second.song_id));

    // Song 2's button must be immediately enabled and actionable, NOT stuck in 'Applying…'
    expect(screen.getByRole('button', { name: 'Apply recommendation' })).toBeEnabled();

    // Now reject the pending request for song 1
    await act(async () => {
      rejectPatch(new Error('Song 1 failed'));
    });

    // Song 2 should NOT have role="alert" or show Song 1's error
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('button', { name: 'Apply recommendation' })).toBeEnabled();
  });

  it('allows applying recommendation on Song B while Song A apply is still pending, applying Song B successfully', async () => {
    const bridge = createMockBridge();
    let resolvePatchSong1!: (val: Settings) => void;
    const originalPatch = bridge.patchSettings.bind(bridge);
    let patchCount = 0;

    bridge.patchSettings = (patch) => {
      patchCount += 1;
      if (patchCount === 1) {
        return new Promise((res) => {
          resolvePatchSong1 = res;
        });
      }
      return originalPatch(patch);
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    const second = selectRowAtIndex(store.getState().library, 5);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<SongDetailsView useStore={store} />);
    // Apply on song A
    fireEvent.click(screen.getByRole('button', { name: 'Apply recommendation' }));
    expect(screen.getByRole('button', { name: 'Applying…' })).toBeDisabled();

    // Switch to song B
    await act(async () => store.getState().selectSong(second.song_id));
    const buttonB = screen.getByRole('button', { name: 'Apply recommendation' });
    expect(buttonB).toBeEnabled();

    // Apply on song B
    fireEvent.click(buttonB);
    expect(screen.getByRole('button', { name: 'Applying…' })).toBeDisabled();

    // Now resolve song A's pending patch
    await act(async () => {
      resolvePatchSong1(null as unknown as Settings);
    });

    // Once song A completes, song B's queued mutation executes and succeeds
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Recommended settings applied' })).toBeDisabled();
    });

    // Song B view remains clean
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('does not clobber fresh Song A state when switching A -> B -> A and old in-flight request arrives', async () => {
    const bridge = createMockBridge();
    let resolvePatchSongA1!: (val: Settings) => void;
    let patchCount = 0;

    bridge.patchSettings = () => {
      patchCount += 1;
      return new Promise((res) => {
        if (patchCount === 1) resolvePatchSongA1 = res;
      });
    };

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    const second = selectRowAtIndex(store.getState().library, 5);
    if (!first || !second) throw new Error('mock library is too small');
    await act(async () => store.getState().selectSong(first.song_id));

    render(<SongDetailsView useStore={store} />);
    // Apply request A1
    fireEvent.click(screen.getByRole('button', { name: 'Apply recommendation' }));
    expect(screen.getByRole('button', { name: 'Applying…' })).toBeDisabled();

    // Switch to song B
    await act(async () => store.getState().selectSong(second.song_id));
    expect(screen.getByRole('button', { name: 'Apply recommendation' })).toBeEnabled();

    // Switch back to song A
    await act(async () => store.getState().selectSong(first.song_id));
    // Song A is fresh: enabled and NOT stuck in 'Applying…'
    expect(screen.getByRole('button', { name: 'Apply recommendation' })).toBeEnabled();

    // Now resolve old request A1 with failure (null)
    await act(async () => {
      resolvePatchSongA1(null as unknown as Settings);
    });

    // Fresh Song A must NOT display error from old A1
    expect(screen.queryByRole('alert')).toBeNull();
    expect(screen.getByRole('button', { name: 'Apply recommendation' })).toBeEnabled();
  });

  it('safely handles unmount while recommendation apply request is in-flight', async () => {
    const bridge = createMockBridge();
    let resolvePatch!: (val: Settings) => void;
    const patchCalled = new Promise<void>((done) => {
      bridge.patchSettings = () =>
        new Promise((res) => {
          resolvePatch = res;
          done();
        });
    });

    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = selectRowAtIndex(store.getState().library, 0);
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));

    const { unmount } = render(<SongDetailsView useStore={store} />);
    fireEvent.click(screen.getByRole('button', { name: 'Apply recommendation' }));
    await patchCalled;
    expect(screen.getByRole('button', { name: 'Applying…' })).toBeDisabled();

    unmount();

    // Resolving after unmount should not throw
    await act(async () => {
      resolvePatch(store.getState().settings as Settings);
    });
  });
});
