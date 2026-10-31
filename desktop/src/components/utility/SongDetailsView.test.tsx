import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { selectRowAtIndex, createDesktopStore } from '../../state/store';
import { SongDetailsView } from './SongDetailsView';

describe('SongDetailsView', () => {
  afterEach(() => cleanup());

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
    const { container } = render(<SongDetailsView useStore={store} />);
    expect(screen.getByRole('heading', { name: first.title })).toBeInTheDocument();
    expect(container.querySelector('.song-details-context')).toBeNull();
    expect(
      screen.getByText(/Test playback \(no input\).*dry-run playback.*physical keyboard input/i),
    ).toBeInTheDocument();
    const sectionCount = container.querySelectorAll('.utility-section').length;

    const pending = store.getState().selectSong(second.song_id);
    await act(async () => Promise.resolve());

    expect(screen.getByRole('heading', { name: second.title })).toBeInTheDocument();
    expect(screen.queryByText('Medium timing risk')).toBeNull();
    expect(container.querySelectorAll('.utility-section')).toHaveLength(sectionCount);
    expect(container.querySelector('.song-details-heading p')).toHaveTextContent(
      'TXT · … · … notes',
    );

    releaseDetail?.();
    await act(async () => pending);
    expect(container.querySelector('.song-details-heading p')).toHaveTextContent(
      'TXT · … · … notes',
    );
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
    expect(screen.getByRole('combobox', { name: 'Conversion mode' })).toBeInTheDocument();
    expect(screen.getByRole('combobox', { name: 'Target Sky Piano pitch' })).toBeInTheDocument();
    expect(screen.getByText(/assumed from the source song/i)).toBeInTheDocument();

    await act(async () => {
      fireEvent.change(screen.getByRole('combobox', { name: 'Conversion mode' }), {
        target: { value: 'track:0' },
      });
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
});
