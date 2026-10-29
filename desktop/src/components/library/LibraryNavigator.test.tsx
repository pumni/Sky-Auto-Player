import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
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
    expect(screen.getByRole('button', { name: 'Playlists' })).toBeEnabled();
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

  it('opens a playlist picker from the collapsed rail and preserves selection and focus', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().createPlaylist('Morning Practice'));
    const morningId = store.getState().library.source.id;
    await act(async () => store.getState().createPlaylist('Evening Practice'));
    const toggle = vi.fn();
    render(<LibraryNavigator collapsed onToggleCollapsed={toggle} useStore={store} />);

    const launcher = screen.getByRole('button', { name: 'Playlists' });
    fireEvent.click(launcher);
    const menu = screen.getByRole('menu', { name: 'Playlists' });
    expect(menu).toBeVisible();
    const current = within(menu).getByRole('menuitem', {
      name: 'Evening Practice, current playlist',
    });
    expect(current).toHaveClass('is-current');
    await act(async () =>
      fireEvent.click(within(menu).getByRole('menuitem', { name: /Morning Practice/ })),
    );
    await waitFor(() =>
      expect(store.getState().library.source).toEqual({ kind: 'playlist', id: morningId }),
    );
    expect(screen.queryByRole('menu', { name: 'Playlists' })).toBeNull();
    expect(toggle).not.toHaveBeenCalled();
    await waitFor(() => expect(launcher).toHaveFocus());
  });

  it('offers a truthful empty state and Create playlist action from the collapsed picker', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const toggle = vi.fn();
    render(<LibraryNavigator collapsed onToggleCollapsed={toggle} useStore={store} />);

    fireEvent.click(screen.getByRole('button', { name: 'Playlists' }));
    const menu = screen.getByRole('menu', { name: 'Playlists' });
    expect(within(menu).getByRole('menuitem', { name: 'No playlists yet' })).toHaveAttribute(
      'aria-disabled',
      'true',
    );
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Create playlist' }));
    expect(screen.getByRole('dialog', { name: 'New playlist' })).toBeVisible();
    expect(toggle).not.toHaveBeenCalled();
  });
});
