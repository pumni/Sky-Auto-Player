import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { LibraryNavigator } from './LibraryNavigator';

describe('LibraryNavigator built-in source', () => {
  afterEach(cleanup);

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
    expect(screen.getByRole('button', { name: /All Songs/ })).toBeEnabled();
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
});
