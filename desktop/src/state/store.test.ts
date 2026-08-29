import { act } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import { createDesktopStore } from './store';

describe('desktop store', () => {
  it('boots, searches, selects a song, and applies a catalog event', async () => {
    const bridge = createMockBridge();
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    expect(store.getState().bootstrapState).toBe('ready');
    expect(store.getState().library.rows.length).toBeGreaterThan(0);

    const first = store.getState().library.rows[0];
    if (!first) throw new Error('mock library is empty');
    await act(async () => store.getState().selectSong(first.song_id));
    expect(store.getState().detail.value?.song_id).toBe(first.song_id);

    await act(async () => store.getState().reloadLibrary());
    expect(store.getState().library.generation).toBe(2);
    expect(store.getState().library.rows).toHaveLength(12);

    await act(async () => store.getState().patchSettings({ theme: 'slate', verboseHud: true }));
    expect(store.getState().settings?.theme).toBe('slate');
    expect(store.getState().settings?.verbose_hud).toBe(true);
  });

  it('discards an older search response', async () => {
    let releaseSlow: (() => void) | undefined;
    const bridge = createMockBridge();
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      if (request.query === 'slow') {
        await new Promise<void>((resolve) => {
          releaseSlow = resolve;
        });
      }
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());

    const slow = store.getState().search('slow');
    const fast = store.getState().search('Aurora');
    await fast;
    releaseSlow?.();
    await slow;
    expect(store.getState().library.query).toBe('Aurora');
    expect(store.getState().library.rows[0]?.title).toBe('Aurora Landing');
  });
});
