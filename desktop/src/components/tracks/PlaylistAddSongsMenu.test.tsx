import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { PlaylistAddSongsMenu } from './PlaylistAddSongsMenu';

describe('PlaylistAddSongsMenu', () => {
  afterEach(() => cleanup());

  it('opens from the keyboard and dispatches Browse All Songs', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;

    render(<PlaylistAddSongsMenu playlistId={playlistId} useStore={store} />);
    const trigger = screen.getByRole('button', { name: 'Add songs' });
    trigger.focus();
    fireEvent.keyDown(trigger, { key: 'Enter' });

    expect(await screen.findByRole('menu', { name: 'Add songs' })).toBeVisible();
    expect(screen.getByRole('menuitem', { name: 'Browse All Songs…' })).toBeVisible();
    expect(screen.getByRole('menuitem', { name: 'Import files…' })).toBeVisible();
    expect(screen.getByRole('menuitem', { name: 'Import folder…' })).toBeVisible();

    await act(async () => {
      fireEvent.click(screen.getByRole('menuitem', { name: 'Browse All Songs…' }));
    });
    await waitFor(() => {
      expect(store.getState().library.playlistAddMode).toEqual({ playlistId });
    });
    expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'all' });
  });
});
