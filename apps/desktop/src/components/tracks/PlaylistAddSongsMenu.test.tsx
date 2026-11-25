import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { PlaylistAddSongsMenu } from './PlaylistAddSongsMenu';

describe('PlaylistAddSongsMenu', () => {
  afterEach(() => cleanup());

  it('supports keyboard navigation and activation within the menu', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;

    render(
      <PlaylistAddSongsMenu playlistId={playlistId} useStore={store} onImportFeedback={vi.fn()} />,
    );
    const trigger = screen.getByRole('button', { name: 'Add songs' });
    trigger.focus();
    fireEvent.mouseDown(trigger, { button: 0 });

    expect(await screen.findByRole('menu', { name: 'Add songs' })).toBeVisible();
    const browseItem = screen.getByRole('menuitem', { name: 'Browse Built-in Songs…' });
    expect(browseItem).toBeVisible();
    expect(screen.getByRole('menuitem', { name: 'Import files…' })).toBeVisible();
    expect(screen.getByRole('menuitem', { name: 'Import folder…' })).toBeVisible();

    await act(async () => {
      fireEvent.keyDown(browseItem, { key: 'Enter' });
    });
    await waitFor(() => {
      expect(store.getState().library.playlistAddMode).toEqual({ playlistId });
    });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'builtin' });
  });
});
