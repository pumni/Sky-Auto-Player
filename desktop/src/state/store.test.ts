import { act, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import type { SettingsPatch } from '../bridge/DesktopBridge';
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
    expect(store.getState().library.rows).toHaveLength(500);

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

  it('loads and selects a song beyond the Core page limit', async () => {
    const bridge = createMockBridge();
    const requestedOffsets: number[] = [];
    const originalSearch = bridge.searchSongs;
    bridge.searchSongs = async (request) => {
      requestedOffsets.push(request.offset);
      return originalSearch(request);
    };
    const store = createDesktopStore(bridge);

    await act(async () => store.getState().initialize());
    await act(async () => store.getState().setViewport(390, 410));
    await waitFor(() => expect(store.getState().library.rows[400]?.title).toBe('Song 401'));

    await act(async () => store.getState().selectSong(store.getState().library.rows[400]!.song_id));
    expect(store.getState().detail.value?.title).toBe('Song 401');
    expect(requestedOffsets).toContain(200);
    expect(requestedOffsets).toContain(400);
  });

  it('clears selected detail and rejects an older detail response after catalog.changed', async () => {
    let releaseDetail: (() => void) | undefined;
    const bridge = createMockBridge();
    const originalDetail = bridge.getSongDetail;
    bridge.getSongDetail = async (request) => {
      await new Promise<void>((resolve) => {
        releaseDetail = resolve;
      });
      return originalDetail(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const first = store.getState().library.rows[0];
    if (!first) throw new Error('mock library is empty');

    const detail = store.getState().selectSong(first.song_id);
    await act(async () => store.getState().reloadLibrary());
    expect(store.getState().library.selectedSongId).toBeNull();
    expect(store.getState().detail.value).toBeNull();
    releaseDetail?.();
    await detail;
    expect(store.getState().detail.value).toBeNull();
  });

  it('does not send filtered-result indices as full-catalog viewport hints', async () => {
    const bridge = createMockBridge();
    let viewportCalls = 0;
    const originalViewport = bridge.setLibraryViewport;
    bridge.setLibraryViewport = async (request) => {
      viewportCalls += 1;
      return originalViewport(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().search('Song'));
    await act(async () => store.getState().setViewport(200, 220));
    expect(viewportCalls).toBe(0);
    expect(store.getState().library.rows[200]?.title).toBe('Song 212');
  });

  it('serializes settings mutations in user-intent order and preserves fields', async () => {
    const bridge = createMockBridge();
    const originalPatch = bridge.patchSettings;
    const calls: SettingsPatch[] = [];
    let releaseFirst: (() => void) | undefined;
    bridge.patchSettings = async (patch) => {
      calls.push(patch);
      if (calls.length === 1) {
        await new Promise<void>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return originalPatch(patch);
    };
    const store = createDesktopStore(bridge);

    const first = store.getState().patchSettings({ theme: 'slate' });
    const second = store.getState().patchSettings({ verboseHud: true });
    await waitFor(() => expect(calls).toHaveLength(1));
    expect(calls[0]).toEqual({ theme: 'slate' });

    releaseFirst?.();
    await Promise.all([first, second]);
    expect(calls).toEqual([{ theme: 'slate' }, { verboseHud: true }]);
    expect(store.getState().settings?.theme).toBe('slate');
    expect(store.getState().settings?.verbose_hud).toBe(true);
  });
});
