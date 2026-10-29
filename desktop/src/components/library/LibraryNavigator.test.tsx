import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { LibraryPlaylistSummary } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { LibraryNavigator } from './LibraryNavigator';

describe('LibraryNavigator built-in source', () => {
  afterEach(cleanup);

  it('starts on Built-in Songs and exposes no All Songs or My Songs destination', async () => {
    const store = createDesktopStore(
      createMockBridge({
        builtinSongIds: Array.from({ length: 96 }, (_, index) =>
          index.toString(16).padStart(32, '0'),
        ),
      }),
    );
    await act(async () => store.getState().initialize());
    const view = render(
      <LibraryNavigator collapsed={false} onToggleCollapsed={() => undefined} useStore={store} />,
    );

    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'builtin' });
    const navigator = screen.getByRole('navigation', { name: 'Library' });
    const library = within(navigator);
    const builtin = library.getByRole('button', { name: /Built-in Songs/ });
    expect(builtin).toBeEnabled();
    expect(builtin).toHaveTextContent('96');
    expect(library.getByRole('button', { name: /Liked Songs/ })).toBeEnabled();
    expect(library.getByText('Playlists')).toBeInTheDocument();
    expect(library.queryByRole('button', { name: /All Songs/ })).not.toBeInTheDocument();
    expect(library.queryByRole('button', { name: /My Songs/ })).not.toBeInTheDocument();

    view.rerender(
      <LibraryNavigator collapsed onToggleCollapsed={() => undefined} useStore={store} />,
    );
    expect(screen.queryByRole('button', { name: 'Playlists' })).toBeNull();
    expect(screen.getByRole('group', { name: 'Playlists' })).toBeInTheDocument();
  });

  it('does not present the initial false/zero defaults as native unavailability', () => {
    const store = createDesktopStore(createMockBridge());
    render(
      <LibraryNavigator collapsed={false} onToggleCollapsed={() => undefined} useStore={store} />,
    );

    const builtin = screen.getByRole('button', { name: /Checking Built-in Songs/ });
    expect(builtin).toBeDisabled();
    expect(builtin).not.toHaveTextContent('0');
    expect(screen.getByRole('status')).toHaveTextContent('Checking Built-in Songs');
  });

  it('shows the native available count and explains a genuinely unavailable source', async () => {
    const store = createDesktopStore(
      createMockBridge({ builtinAvailable: false, builtinSongIds: [] }),
    );
    await act(async () => store.getState().initialize());
    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('ready'));
    render(
      <LibraryNavigator collapsed={false} onToggleCollapsed={() => undefined} useStore={store} />,
    );

    const builtin = screen.getByRole('button', { name: /Built-in Songs/ });
    expect(builtin).toBeDisabled();
    expect(builtin).not.toHaveTextContent('0');
    expect(screen.getByRole('status')).toHaveTextContent(/aren’t available in this installation/i);
    expect(screen.queryByRole('button', { name: /All Songs/ })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Liked Songs/ })).toBeEnabled();
  });

  it('keeps available empty catalogs distinct from unavailable catalogs', async () => {
    const store = createDesktopStore(
      createMockBridge({ builtinAvailable: true, builtinSongIds: [] }),
    );
    await act(async () => store.getState().initialize());
    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('ready'));
    render(
      <LibraryNavigator collapsed={false} onToggleCollapsed={() => undefined} useStore={store} />,
    );

    const builtin = screen.getByRole('button', { name: /Built-in Songs0/ });
    expect(builtin).toBeEnabled();
    expect(builtin).toHaveTextContent('0');
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('offers a working retry after navigation loading fails', async () => {
    const bridge = createMockBridge({
      builtinSongIds: Array.from({ length: 96 }, (_, index) =>
        index.toString(16).padStart(32, '0'),
      ),
    });
    const originalListNavigation = bridge.listLibraryNavigation.bind(bridge);
    const listNavigation = vi
      .spyOn(bridge, 'listLibraryNavigation')
      .mockRejectedValueOnce(new Error('navigation bridge unavailable'))
      .mockImplementation(originalListNavigation);
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('error'));
    render(
      <LibraryNavigator collapsed={false} onToggleCollapsed={() => undefined} useStore={store} />,
    );

    expect(screen.getByRole('alert')).toHaveTextContent('Couldn’t check Built-in Songs');
    const retry = screen.getByRole('button', { name: 'Retry library navigation' });
    await act(async () => retry.click());

    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('ready'));
    expect(listNavigation).toHaveBeenCalledTimes(2);
    const builtin = screen.getByRole('button', { name: /Built-in Songs/ });
    expect(builtin).toBeEnabled();
    expect(builtin).toHaveTextContent('96');
    await act(async () => builtin.click());
    expect(store.getState().library.source).toEqual({ kind: 'smart', id: 'builtin' });
  });

  it('keeps collapsed navigation available and retryable when playlist loading fails', async () => {
    const bridge = createMockBridge();
    const originalListNavigation = bridge.listLibraryNavigation.bind(bridge);
    const listNavigation = vi
      .spyOn(bridge, 'listLibraryNavigation')
      .mockRejectedValueOnce(new Error('navigation bridge unavailable'))
      .mockImplementation(originalListNavigation);
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('error'));
    render(<LibraryNavigator collapsed onToggleCollapsed={() => undefined} useStore={store} />);

    const group = screen.getByRole('group', { name: 'Playlists' });
    expect(within(group).queryAllByRole('button')).toHaveLength(1);
    expect(screen.getByRole('alert')).toHaveTextContent('Couldn’t load playlists');
    expect(screen.getByRole('button', { name: 'Retry library navigation' })).toBeVisible();
    expect(screen.queryByRole('button', { name: 'Playlists' })).toBeNull();

    await act(async () => screen.getByRole('button', { name: 'Retry library navigation' }).click());
    await waitFor(() => expect(store.getState().libraryNavigation.loadState).toBe('ready'));
    expect(listNavigation).toHaveBeenCalledTimes(2);
    expect(within(group).queryAllByRole('button')).toHaveLength(0);
  });

  it('keeps empty collapsed playlists truthful and leaves Create playlist fixed', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const toggle = vi.fn();
    render(<LibraryNavigator collapsed onToggleCollapsed={toggle} useStore={store} />);

    const group = screen.getByRole('group', { name: 'Playlists' });
    expect(within(group).queryAllByRole('button')).toHaveLength(0);
    expect(screen.getByRole('button', { name: 'Create playlist' })).toBeVisible();
    expect(screen.queryByRole('menu', { name: 'Playlists' })).toBeNull();
    expect(toggle).not.toHaveBeenCalled();
  });

  it.each([0, 1, 3, 20, 100, 500])(
    'renders %i collapsed playlists as direct, ordered, one-click destinations',
    async (count) => {
      const store = createDesktopStore(createMockBridge());
      await act(async () => store.getState().initialize());
      const playlists: LibraryPlaylistSummary[] = Array.from({ length: count }, (_, index) => ({
        id: `playlist-${index}`,
        name:
          index === 0
            ? 'Ánh sáng 🌅'
            : index === 1
              ? 'Ánh sáng của tôi'
              : index === 2
                ? '👨‍👩‍👧‍👦 Piano'
                : count >= 20 && index < 5
                  ? 'Shared playlist'
                  : `Playlist ${index + 1}`,
        song_count: index,
      }));
      store.setState({
        libraryNavigation: {
          ...store.getState().libraryNavigation,
          loadState: 'ready',
          playlistOrder: playlists.map((playlist) => playlist.id),
          playlistsById: new Map(playlists.map((playlist) => [playlist.id, playlist])),
        },
      });
      const toggle = vi.fn();
      render(<LibraryNavigator collapsed onToggleCollapsed={toggle} useStore={store} />);

      const group = screen.getByRole('group', { name: 'Playlists' });
      const tiles = within(group).queryAllByRole('button');
      const duplicateNames = new Map<string, number>();
      for (const playlist of playlists) {
        duplicateNames.set(playlist.name, (duplicateNames.get(playlist.name) ?? 0) + 1);
      }
      expect(tiles).toHaveLength(count);
      expect(tiles.map((tile) => tile.getAttribute('aria-label'))).toEqual(
        playlists.map((playlist, index) =>
          (duplicateNames.get(playlist.name) ?? 0) > 1
            ? `${playlist.name}, playlist ${index + 1}`
            : playlist.name,
        ),
      );
      if (count > 0) {
        expect(tiles[0]?.querySelector('.library-playlist-monogram')).toHaveTextContent('Á');
      }
      if (count > 2) {
        expect(tiles[2]?.querySelector('.library-playlist-monogram')).toHaveTextContent('👨‍👩‍👧‍👦');
      }
      expect(screen.queryByRole('button', { name: 'Playlists' })).toBeNull();
      expect(screen.queryByRole('menu', { name: 'Playlists' })).toBeNull();

      if (count > 0) {
        fireEvent.click(tiles[count - 1]!);
        await waitFor(() =>
          expect(store.getState().library.source).toEqual({
            kind: 'playlist',
            id: playlists[count - 1]!.id,
          }),
        );
      }
      expect(toggle).not.toHaveBeenCalled();
    },
  );

  it('uses the generic playlist glyph and accessible fallback for an empty name', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const playlist: LibraryPlaylistSummary = { id: 'empty-name', name: ' ', song_count: 0 };
    store.setState({
      libraryNavigation: {
        ...store.getState().libraryNavigation,
        loadState: 'ready',
        playlistOrder: [playlist.id],
        playlistsById: new Map([[playlist.id, playlist]]),
      },
    });
    render(<LibraryNavigator collapsed onToggleCollapsed={() => undefined} useStore={store} />);

    const tile = screen.getByRole('button', { name: 'Playlist 1' });
    expect(tile.querySelector('.lucide-list-music')).toBeInTheDocument();
  });
});
