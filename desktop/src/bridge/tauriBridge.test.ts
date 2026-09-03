import { beforeEach, describe, expect, it, vi } from 'vitest';

const invokeMock = vi.hoisted(() => vi.fn().mockResolvedValue({}));

vi.mock('@tauri-apps/api/core', () => ({
  Channel: class {
    onmessage: ((event: unknown) => void) | null = null;
  },
  invoke: invokeMock,
}));

import { createTauriBridge, encodeCommandArgs } from './tauriBridge';

describe('Tauri command bridge', () => {
  beforeEach(() => invokeMock.mockClear());

  it('uses the Rust command parameter name for every payload command', async () => {
    const bridge = createTauriBridge();
    const search = {
      query: 'Aurora',
      offset: 0,
      limit: 200,
      source: { kind: 'smart', id: 'all' },
    } as const;
    const detail = { songId: 'song-1' } as const;
    const viewport = {
      generation: 4,
      firstIndex: 10,
      lastIndex: 30,
      selectedSongId: null,
      songIds: [] as string[],
    } as const;
    const patch = { theme: 'slate' as const };

    await bridge.searchSongs(search);
    await bridge.getSongDetail(detail);
    await bridge.setLibraryViewport(viewport);
    await bridge.patchSettings(patch);

    expect(invokeMock.mock.calls).toEqual([
      ['search_songs', { params: search }],
      ['get_song_detail', { params: detail }],
      ['set_library_viewport', { params: viewport }],
      ['patch_settings', { params: patch }],
    ]);
  });

  it('does not send an argument object for no-parameter commands', async () => {
    const bridge = createTauriBridge();
    await bridge.bootstrap();
    await bridge.reloadLibrary();
    await bridge.getSettings();

    expect(invokeMock.mock.calls).toEqual([
      ['bootstrap', undefined],
      ['reload_library', undefined],
      ['get_settings', undefined],
    ]);
  });

  it('keeps command argument encoding independently testable', () => {
    expect(encodeCommandArgs({ value: 1 })).toEqual({ params: { value: 1 } });
    expect(encodeCommandArgs()).toBeUndefined();
  });
});
