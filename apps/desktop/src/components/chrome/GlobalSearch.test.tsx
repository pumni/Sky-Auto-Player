import { createRef } from 'react';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { GlobalSearch } from './GlobalSearch';

describe('GlobalSearch', () => {
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it('retains focus after the debounced query is acknowledged', async () => {
    const useStore = createDesktopStore(createMockBridge());
    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');

    input.focus();
    fireEvent.change(input, { target: { value: 'Moonlit' } });

    await waitFor(() => expect(useStore.getState().library.query).toBe('Moonlit'));
    expect(input).toHaveFocus();
    expect(input).toHaveValue('Moonlit');
  });

  it('preserves a newer local draft when an earlier query is acknowledged', async () => {
    vi.useFakeTimers();
    const useStore = createDesktopStore(createMockBridge());
    let acknowledgeFirstQuery: (() => void) | undefined;
    const search = vi.fn(
      (query?: string) =>
        new Promise<void>((resolve) => {
          acknowledgeFirstQuery = () => {
            useStore.setState({
              library: { ...useStore.getState().library, query: query ?? '' },
            });
            resolve();
          };
        }),
    );
    act(() => useStore.setState({ search }));

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'first' } });
    act(() => vi.advanceTimersByTime(120));
    expect(search).toHaveBeenCalledWith('first');

    fireEvent.change(input, { target: { value: 'second' } });
    await act(async () => {
      acknowledgeFirstQuery?.();
      await Promise.resolve();
    });

    expect(input).toHaveValue('second');
  });

  it('keeps one request per settled typing burst and clears an uncommitted in-flight draft immediately', async () => {
    vi.useFakeTimers();
    const useStore = createDesktopStore(createMockBridge());
    const acknowledgements: Array<{ query: string; acknowledge: () => void }> = [];
    const search = vi.fn(
      (query?: string) =>
        new Promise<void>((resolve) => {
          const acknowledgedQuery = query ?? '';
          acknowledgements.push({
            query: acknowledgedQuery,
            acknowledge: () => {
              useStore.setState((state) => ({
                library: { ...state.library, query: acknowledgedQuery },
              }));
              resolve();
            },
          });
        }),
    );
    act(() => useStore.setState({ search }));

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'f' } });
    fireEvent.change(input, { target: { value: 'fi' } });
    fireEvent.change(input, { target: { value: 'first' } });

    act(() => vi.advanceTimersByTime(119));
    expect(search).not.toHaveBeenCalled();
    act(() => vi.advanceTimersByTime(1));
    expect(search).toHaveBeenCalledTimes(1);
    expect(search).toHaveBeenNthCalledWith(1, 'first');
    expect(useStore.getState().library.query).toBe('');
    expect(vi.getTimerCount()).toBe(0);

    const clearButton = screen.getByRole('button', { name: 'Clear search' });
    fireEvent.click(clearButton);
    expect(input).toHaveValue('');
    expect(search).toHaveBeenCalledTimes(2);
    expect(search).toHaveBeenNthCalledWith(2, '');
    act(() => vi.advanceTimersByTime(120));
    expect(search).toHaveBeenCalledTimes(2);
    expect(input).toHaveFocus();

    await act(async () => {
      acknowledgements[0]?.acknowledge();
      await Promise.resolve();
    });
    expect(useStore.getState().library.query).toBe('first');
    expect(input).toHaveValue('');

    await act(async () => {
      acknowledgements[1]?.acknowledge();
      await Promise.resolve();
    });
    expect(useStore.getState().library.query).toBe('');
    expect(input).toHaveValue('');
    expect(search).toHaveBeenCalledTimes(2);
  });

  it('restores focus after mouse clear with and without the optional ref', async () => {
    const useStore = createDesktopStore(createMockBridge());
    act(() => {
      useStore.setState({
        library: { ...useStore.getState().library, query: 'Blue' },
      });
    });
    const inputRef = createRef<HTMLInputElement>();
    render(<GlobalSearch inputRef={inputRef} useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    const clearButton = screen.getByRole('button', { name: 'Clear search' });
    expect(inputRef.current).toBe(input);

    clearButton.focus();
    fireEvent.click(clearButton);
    expect(input).toHaveValue('');
    expect(input).toHaveFocus();
    expect(screen.queryByRole('button', { name: 'Clear search' })).not.toBeInTheDocument();

    cleanup();
    const unreferencedStore = createDesktopStore(createMockBridge());
    act(() => {
      unreferencedStore.setState({
        library: { ...unreferencedStore.getState().library, query: 'Blue' },
      });
    });
    render(<GlobalSearch useStore={unreferencedStore} />);
    const unreferencedInput = screen.getByRole('searchbox');
    const keyboardClear = screen.getByRole('button', { name: 'Clear search' });
    keyboardClear.focus();
    fireEvent.click(keyboardClear);
    expect(unreferencedInput).toHaveValue('');
    expect(unreferencedInput).toHaveFocus();
  });

  it('drops a queued draft when the active library scope changes', async () => {
    vi.useFakeTimers();
    const useStore = createDesktopStore(createMockBridge());
    const search = vi.fn();
    act(() => useStore.setState({ search }));
    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'stale draft' } });
    act(() => vi.advanceTimersByTime(50));

    act(() => {
      const library = useStore.getState().library;
      useStore.setState({
        library: {
          ...library,
          source: { kind: 'smart', id: 'liked' },
          searchSource: { kind: 'smart', id: 'liked' },
          query: '',
        },
      });
    });
    expect(screen.getByRole('searchbox', { name: 'Search Liked Songs' })).toHaveValue('');

    act(() => vi.advanceTimersByTime(120));
    expect(search).not.toHaveBeenCalled();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('fences a late bridge response after clear restores the cached empty results', async () => {
    const bridge = createMockBridge();
    const useStore = createDesktopStore(bridge);
    await act(async () => useStore.getState().initialize());
    const initialResultTotal = useStore.getState().library.resultTotal;
    const searchSongs = bridge.searchSongs.bind(bridge);
    let acknowledgeOldSearch: (() => Promise<void>) | undefined;
    const bridgeSearch = vi.spyOn(bridge, 'searchSongs').mockImplementation(
      (request) =>
        new Promise((resolve) => {
          acknowledgeOldSearch = async () => resolve(await searchSongs(request));
        }),
    );
    vi.useFakeTimers();

    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'Blue' } });
    act(() => vi.advanceTimersByTime(120));
    expect(bridgeSearch).toHaveBeenCalledTimes(1);
    expect(bridgeSearch.mock.calls[0]?.[0].query).toBe('Blue');
    expect(useStore.getState().library.query).toBe('Blue');

    fireEvent.click(screen.getByRole('button', { name: 'Clear search' }));
    expect(input).toHaveValue('');
    expect(useStore.getState().library.query).toBe('');
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(input).toHaveFocus();
    expect(useStore.getState().library.loading).toBe(false);
    expect(useStore.getState().library.resultTotal).toBe(initialResultTotal);

    await act(async () => {
      await acknowledgeOldSearch?.();
    });
    expect(useStore.getState().library.query).toBe('');
    expect(useStore.getState().library.resultTotal).toBe(initialResultTotal);
    expect(input).toHaveValue('');
    expect(bridgeSearch).toHaveBeenCalledTimes(1);
  });

  it('synchronizes external query changes into the visible input', () => {
    vi.useFakeTimers();
    const useStore = createDesktopStore(createMockBridge());
    const search = vi.fn(() => new Promise<void>(() => {}));
    act(() => useStore.setState({ search }));
    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');
    fireEvent.change(input, { target: { value: 'queued draft' } });
    act(() => vi.advanceTimersByTime(120));
    expect(search).toHaveBeenCalledTimes(1);

    act(() => {
      useStore.setState({
        library: { ...useStore.getState().library, query: 'Moonlit' },
      });
    });
    expect(input).toHaveValue('Moonlit');
    act(() => vi.advanceTimersByTime(120));
    expect(search).toHaveBeenCalledTimes(1);

    act(() => {
      useStore.setState({
        library: { ...useStore.getState().library, query: '' },
      });
    });
    expect(input).toHaveValue('');
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
