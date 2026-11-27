import { StrictMode, createRef } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import type { SearchRequest, SearchResult } from '../../bridge/DesktopBridge';
import { createDesktopStore } from '../../state/store';
import { GlobalSearch } from './GlobalSearch';

type Store = ReturnType<typeof createDesktopStore>;
type Bridge = ReturnType<typeof createMockBridge>;

interface DeferredSearchResponse {
  request: SearchRequest;
  resolve: () => void;
}

async function createInitializedStore() {
  const bridge = createMockBridge();
  const useStore = createDesktopStore(bridge);
  await act(async () => useStore.getState().initialize());
  return { bridge, useStore };
}

function observeSearchActions(useStore: Store) {
  const search = vi.fn(useStore.getState().search);
  act(() => useStore.setState({ search }));
  return search;
}

function deferBridgeResponses(bridge: Bridge) {
  const requests: SearchRequest[] = [];
  const responses: DeferredSearchResponse[] = [];
  const searchSongs = bridge.searchSongs.bind(bridge);

  bridge.searchSongs = (request) => {
    requests.push(request);
    return searchSongs(request).then(
      (result) =>
        new Promise<SearchResult>((resolve) => {
          let resolved = false;
          responses.push({
            request,
            resolve: () => {
              if (resolved) return;
              resolved = true;
              resolve(result);
            },
          });
        }),
    );
  };

  return {
    requests,
    responses,
    resolve(query: string) {
      const response = responses.find((entry) => entry.request.query === query);
      if (!response) throw new Error(`No deferred response for query ${query}`);
      response.resolve();
    },
  };
}

async function flushPromises() {
  for (let turn = 0; turn < 5; turn += 1) await Promise.resolve();
}

async function advanceSearchTimer(milliseconds: number) {
  await act(async () => {
    vi.advanceTimersByTime(milliseconds);
    await flushPromises();
  });
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('GlobalSearch', () => {
  it('publishes the committed query at the 120 ms debounce boundary and retains focus', async () => {
    const { bridge, useStore } = await createInitializedStore();
    const search = observeSearchActions(useStore);
    const bridgeSearch = vi.spyOn(bridge, 'searchSongs');
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    input.focus();
    fireEvent.change(input, { target: { value: 'Moonlit' } });

    expect(useStore.getState().library.query).toBe('');
    await advanceSearchTimer(119);
    expect(search).not.toHaveBeenCalled();
    expect(bridgeSearch).not.toHaveBeenCalled();

    await advanceSearchTimer(1);
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith('Moonlit');
    expect(useStore.getState().library.query).toBe('Moonlit');
    expect(bridgeSearch).toHaveBeenCalledTimes(1);
    expect(bridgeSearch.mock.calls[0]?.[0].query).toBe('Moonlit');
    expect(vi.getTimerCount()).toBe(0);

    await act(async () => {
      await flushPromises();
    });
    expect(input).toHaveValue('Moonlit');
    expect(input).toHaveFocus();
    expect(useStore.getState().library.loading).toBe(false);
  });

  it('clears an uncommitted draft before debounce without dispatching a search', async () => {
    const { bridge, useStore } = await createInitializedStore();
    const search = observeSearchActions(useStore);
    const bridgeSearch = vi.spyOn(bridge, 'searchSongs');
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    input.focus();
    fireEvent.change(input, { target: { value: 'Blue' } });
    await advanceSearchTimer(50);

    expect(input).toHaveValue('Blue');
    expect(useStore.getState().library.query).toBe('');
    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));

    expect(input).toHaveValue('');
    expect(input).toHaveFocus();
    expect(screen.queryByRole('button', { name: 'Clear search' })).not.toBeInTheDocument();
    await advanceSearchTimer(120);
    expect(search).not.toHaveBeenCalled();
    expect(bridgeSearch).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('commits clear immediately in flight and rejects the older bridge result', async () => {
    const { bridge, useStore } = await createInitializedStore();
    const initialResultTotal = useStore.getState().library.resultTotal;
    const deferred = deferBridgeResponses(bridge);
    const search = observeSearchActions(useStore);
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'Blue' } });
    await advanceSearchTimer(120);

    expect(useStore.getState().library.query).toBe('Blue');
    expect(deferred.requests.map((request) => request.query)).toEqual(['Blue']);
    expect(input).toHaveValue('Blue');

    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    await act(async () => flushPromises());
    expect(search.mock.calls.map(([query]) => query)).toEqual(['Blue', '']);
    expect(useStore.getState().library.query).toBe('');
    expect(useStore.getState().library.loading).toBe(false);
    expect(useStore.getState().library.resultTotal).toBe(initialResultTotal);
    expect(deferred.requests.map((request) => request.query)).toEqual(['Blue']);
    expect(input).toHaveValue('');
    expect(input).toHaveFocus();

    await act(async () => {
      deferred.resolve('Blue');
      await flushPromises();
    });
    expect(useStore.getState().library.query).toBe('');
    expect(useStore.getState().library.resultTotal).toBe(initialResultTotal);
    expect(useStore.getState().library.loading).toBe(false);
    expect(input).toHaveValue('');
    expect(search).toHaveBeenCalledTimes(2);
    expect(deferred.requests).toHaveLength(1);
  });

  it('keeps a newer queued draft when an older response arrives', async () => {
    const { bridge, useStore } = await createInitializedStore();
    const deferred = deferBridgeResponses(bridge);
    const search = observeSearchActions(useStore);
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'Blue' } });
    await advanceSearchTimer(120);
    expect(useStore.getState().library.query).toBe('Blue');
    expect(deferred.requests.map((request) => request.query)).toEqual(['Blue']);

    fireEvent.change(input, { target: { value: 'Moonlit' } });
    await advanceSearchTimer(50);
    await act(async () => {
      deferred.resolve('Blue');
      await flushPromises();
    });
    expect(useStore.getState().library.query).toBe('Blue');
    expect(input).toHaveValue('Moonlit');
    expect(search).toHaveBeenCalledTimes(1);

    await advanceSearchTimer(69);
    expect(search).toHaveBeenCalledTimes(1);
    await advanceSearchTimer(1);
    expect(useStore.getState().library.query).toBe('Moonlit');
    expect(input).toHaveValue('Moonlit');
    expect(search).toHaveBeenCalledTimes(2);
    expect(deferred.requests.map((request) => request.query)).toEqual(['Blue', 'Moonlit']);

    await act(async () => {
      deferred.resolve('Moonlit');
      await flushPromises();
    });
    expect(useStore.getState().library.query).toBe('Moonlit');
    expect(input).toHaveValue('Moonlit');
  });

  it('keeps the latest results through an A-B-A response race', async () => {
    const { bridge, useStore } = await createInitializedStore();
    const initialResultTotal = useStore.getState().library.resultTotal;
    const deferred = deferBridgeResponses(bridge);
    const search = observeSearchActions(useStore);
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');

    for (const query of ['Aurora', 'Moonlit', 'Aurora']) {
      fireEvent.change(input, { target: { value: query } });
      await advanceSearchTimer(120);
      expect(useStore.getState().library.query).toBe(query);
    }
    expect(search).toHaveBeenCalledTimes(3);
    expect(deferred.requests.map((request) => request.query)).toEqual(['Aurora', 'Moonlit']);

    await act(async () => {
      deferred.resolve('Aurora');
      await flushPromises();
    });
    expect(useStore.getState().library.query).toBe('Aurora');
    const latestResultTotal = useStore.getState().library.resultTotal;
    expect(latestResultTotal).toBeGreaterThan(0);
    expect(latestResultTotal).toBeLessThan(initialResultTotal);

    await act(async () => {
      deferred.resolve('Moonlit');
      await flushPromises();
    });
    expect(useStore.getState().library.query).toBe('Aurora');
    expect(useStore.getState().library.resultTotal).toBe(latestResultTotal);
    expect(input).toHaveValue('Aurora');
  });

  it('cancels a repeated current query instead of scheduling a duplicate request', async () => {
    const { bridge, useStore } = await createInitializedStore();
    const search = observeSearchActions(useStore);
    const bridgeSearch = vi.spyOn(bridge, 'searchSongs');
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'Moonlit' } });
    await advanceSearchTimer(120);
    expect(useStore.getState().library.query).toBe('Moonlit');

    fireEvent.change(input, { target: { value: 'Blue' } });
    await advanceSearchTimer(50);
    fireEvent.change(input, { target: { value: 'Moonlit' } });
    expect(input).toHaveValue('Moonlit');
    await advanceSearchTimer(120);

    expect(search).toHaveBeenCalledTimes(1);
    expect(bridgeSearch).toHaveBeenCalledTimes(1);
    expect(useStore.getState().library.query).toBe('Moonlit');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('cancels queued typing on external query, source, and playlist-add scope changes', async () => {
    const { useStore } = await createInitializedStore();
    const search = observeSearchActions(useStore);
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    let input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'queued draft' } });
    await advanceSearchTimer(50);
    await act(async () => useStore.getState().search('Moonlit'));
    input = screen.getByRole('searchbox');
    expect(input).toHaveValue('Moonlit');
    await advanceSearchTimer(120);
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenCalledWith('Moonlit');

    fireEvent.change(input, { target: { value: 'source draft' } });
    await advanceSearchTimer(50);
    await act(async () => useStore.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    input = screen.getByRole('searchbox', { name: 'Search Liked Songs' });
    expect(input).toHaveValue('');
    await advanceSearchTimer(120);
    expect(useStore.getState().library.source).toEqual({ kind: 'smart', id: 'liked' });

    await act(async () => useStore.getState().createPlaylist('Practice'));
    const playlistId = useStore.getState().library.source.id;
    fireEvent.change(input, { target: { value: 'playlist draft' } });
    await advanceSearchTimer(50);
    await act(async () => useStore.getState().openPlaylistAdd(playlistId));
    input = screen.getByRole('searchbox', {
      name: 'Search Built-in Songs to add to Practice',
    });
    expect(input).toHaveValue('');
    await advanceSearchTimer(120);
    expect(search).not.toHaveBeenCalledWith('queued draft');
    expect(search).not.toHaveBeenCalledWith('source draft');
    expect(search).not.toHaveBeenCalledWith('playlist draft');
    expect(vi.getTimerCount()).toBe(0);
  });

  it('cleans up a queued timer under StrictMode and on unmount', async () => {
    const { useStore } = await createInitializedStore();
    const search = observeSearchActions(useStore);
    vi.useFakeTimers();

    const rendered = render(
      <StrictMode>
        <GlobalSearch useStore={useStore} />
      </StrictMode>,
    );
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: 'unmounted' } });
    rendered.unmount();
    await advanceSearchTimer(120);

    expect(search).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('restores focus after mouse clear with and without the optional ref', async () => {
    const { useStore } = await createInitializedStore();
    await act(async () => useStore.getState().search('Blue'));
    const inputRef = createRef<HTMLInputElement>();
    render(<GlobalSearch inputRef={inputRef} useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    const clearButton = screen.getByRole('button', { name: 'Clear search' });
    expect(inputRef.current).toBe(input);

    clearButton.focus();
    fireEvent.click(clearButton);
    expect(useStore.getState().library.query).toBe('');
    expect(input).toHaveValue('');
    expect(input).toHaveFocus();
    expect(screen.queryByRole('button', { name: 'Clear search' })).not.toBeInTheDocument();

    cleanup();
    const { useStore: unreferencedStore } = await createInitializedStore();
    await act(async () => unreferencedStore.getState().search('Blue'));
    render(<GlobalSearch useStore={unreferencedStore} />);
    const unreferencedInput = screen.getByRole('searchbox');
    const keyboardClear = screen.getByRole('button', { name: 'Clear search' });
    keyboardClear.focus();
    fireEvent.click(keyboardClear);
    expect(unreferencedStore.getState().library.query).toBe('');
    expect(unreferencedInput).toHaveValue('');
    expect(unreferencedInput).toHaveFocus();
  });

  it('synchronizes external query changes into the visible input', async () => {
    const { useStore } = await createInitializedStore();
    const search = observeSearchActions(useStore);
    vi.useFakeTimers();
    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'queued draft' } });
    await advanceSearchTimer(50);

    await act(async () => useStore.getState().search('Moonlit'));
    expect(input).toHaveValue('Moonlit');
    await advanceSearchTimer(120);
    expect(search).toHaveBeenCalledTimes(1);

    await act(async () => useStore.getState().search(''));
    expect(input).toHaveValue('');
    expect(useStore.getState().library.query).toBe('');
  });

  it('names the active search scope for catalog, playlist, and add-songs mode', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    render(<GlobalSearch useStore={store} />);

    expect(screen.getByRole('searchbox', { name: 'Search Built-in Songs' })).toHaveAttribute(
      'placeholder',
      'Search current source…',
    );
    expect(within(screen.getByRole('search')).getByText('Built-in')).toBeInTheDocument();

    await act(async () => store.getState().createPlaylist('Practice'));
    const playlistId = store.getState().library.source.id;
    expect(screen.getByRole('searchbox', { name: 'Search Practice' })).toHaveAttribute(
      'placeholder',
      'Search current source…',
    );
    expect(within(screen.getByRole('search')).getByText('Practice')).toBeInTheDocument();

    await act(async () => store.getState().openPlaylistAdd(playlistId));
    const addSearch = screen.getByRole('searchbox', {
      name: 'Search Built-in Songs to add to Practice',
    });
    expect(addSearch).toHaveAttribute('placeholder', 'Search songs…');
    expect(within(screen.getByRole('search')).getByText('Built-in')).toBeInTheDocument();
    fireEvent.change(addSearch, { target: { value: 'Moonlit' } });
    await waitFor(() => {
      expect(store.getState().library.query).toBe('Moonlit');
      expect(store.getState().library.searchSource).toEqual({ kind: 'smart', id: 'builtin' });
    });
    expect(addSearch).toHaveValue('Moonlit');

    await act(async () => store.getState().exitPlaylistAdd());
    expect(screen.getByRole('searchbox', { name: 'Search Practice' })).toHaveAttribute(
      'placeholder',
      'Search current source…',
    );

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'liked' }));
    expect(screen.getByRole('searchbox', { name: 'Search Liked Songs' })).toHaveAttribute(
      'placeholder',
      'Search current source…',
    );
    expect(within(screen.getByRole('search')).getByText('Liked')).toBeInTheDocument();

    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    expect(screen.getByRole('searchbox', { name: 'Search Built-in Songs' })).toHaveAttribute(
      'placeholder',
      'Search current source…',
    );
  });
});
