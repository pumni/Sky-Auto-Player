import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import type { LibraryPlaylistImportResult } from '../../bridge/DesktopBridge';
import { createDesktopStore } from '../../state/store';
import { TrackBrowser } from './TrackBrowser';

describe('TrackBrowser playlist import feedback', () => {
  afterEach(() => cleanup());

  async function createPlaylistFixture() {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().createPlaylist('Practice'));
    return { bridge, store, playlistId: store.getState().library.source.id };
  }

  async function chooseImport(action: 'Import files…' | 'Import folder…') {
    const trigger = screen.getByRole('button', { name: 'Add songs' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });
    await screen.findByRole('menu', { name: 'Add songs' });
    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: action }));
    });
  }

  it('keeps the source row and accessible loading status while data is pending', async () => {
    const { store } = await createPlaylistFixture();
    await act(async () => {
      store.setState({
        library: {
          ...store.getState().library,
          loading: true,
          resultTotal: 0,
        },
      });
    });
    render(<TrackBrowser useStore={store} />);

    const heading = screen.getByRole('heading', { name: 'Practice' });
    const header = heading.closest('header');
    expect(header).not.toBeNull();
    if (!header) throw new Error('Track Browser header is missing');
    expect(within(header).getByRole('heading', { name: 'Practice' })).toBeInTheDocument();
    expect(within(header).getByText('Updating…')).toBeInTheDocument();
    expect(header.querySelector('[role="tablist"]')).toBeNull();
    expect(screen.getByRole('status')).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByText('Loading library…')).toBeInTheDocument();
  });

  it('names the empty playlist search scope and lets the user clear it', async () => {
    const { store } = await createPlaylistFixture();
    await act(async () => store.getState().search('no matching sheets'));
    render(<TrackBrowser useStore={store} />);

    expect(screen.getByText('No songs in Practice match your search')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Clear search' })).toBeInTheDocument();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    });

    expect(screen.getByText('This playlist is empty')).toBeInTheDocument();
    expect(store.getState().library.query).toBe('');
  });

  it('announces successful import and shows the imported playlist row', async () => {
    const { store, playlistId } = await createPlaylistFixture();
    render(<TrackBrowser useStore={store} />);

    await chooseImport('Import files…');

    expect(await screen.findByRole('status')).toHaveTextContent(
      'Import complete: 1 song found in Practice.',
    );
    expect(await screen.findByRole('row', { name: /Local Song B/ })).toBeInTheDocument();
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.song_count).toBe(1);
    expect(store.getState().library.loading).toBe(false);
  });

  it('explains the native picker zero-result state without claiming success', async () => {
    const { bridge, store, playlistId } = await createPlaylistFixture();
    let finishImport!: (result: LibraryPlaylistImportResult) => void;
    bridge.importLocalFolderToPlaylist = async () =>
      new Promise((resolve) => {
        finishImport = resolve;
      });
    render(<TrackBrowser useStore={store} />);

    await chooseImport('Import folder…');

    expect(screen.getByRole('status')).toHaveTextContent('Opening the local folder picker…');
    expect(
      store
        .getState()
        .libraryNavigation.pendingMutations.has(`playlist:${playlistId}:import:folder`),
    ).toBe(true);
    await act(async () => {
      finishImport({
        playlist: { id: playlistId, name: 'Practice', song_count: 0 },
        imported_song_count: 0,
        catalog_generation: 1,
      });
    });

    expect(await screen.findByRole('status')).toHaveTextContent(
      'No songs were imported. The picker may have been canceled, or the selection contained no playable sheets.',
    );
    expect(screen.queryByText(/Import complete/)).toBeNull();
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.song_count).toBe(0);
    expect(store.getState().libraryNavigation.pendingMutations.size).toBe(0);
  });

  it('reports an import failure at the playlist action that produced it', async () => {
    const { bridge, store } = await createPlaylistFixture();
    bridge.importLocalFilesToPlaylist = async () => {
      throw new Error('private native detail');
    };
    render(<TrackBrowser useStore={store} />);

    await chooseImport('Import files…');

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Import failed. Try again with supported Sky sheet files or a folder that contains them.',
    );
    expect(screen.queryByText('private native detail')).toBeNull();
    expect(store.getState().libraryNavigation.pendingMutations.size).toBe(0);
  });

  it('treats a missing import result as a failure and leaves playlist state intact', async () => {
    const { bridge, store, playlistId } = await createPlaylistFixture();
    bridge.importLocalFilesToPlaylist = async () => undefined as never;
    render(<TrackBrowser useStore={store} />);

    await chooseImport('Import files…');

    expect(await screen.findByRole('alert')).toHaveTextContent('Import failed.');
    expect(store.getState().libraryNavigation.playlistsById.get(playlistId)?.song_count).toBe(0);
    expect(store.getState().libraryNavigation.pendingMutations.size).toBe(0);
  });
});
