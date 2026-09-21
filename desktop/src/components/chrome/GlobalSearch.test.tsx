import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
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

  it('synchronizes external query changes into the visible input', () => {
    const useStore = createDesktopStore(createMockBridge());
    render(<GlobalSearch useStore={useStore} />);
    const input = screen.getByRole('searchbox');

    act(() => {
      useStore.setState({
        library: { ...useStore.getState().library, query: 'Moonlit' },
      });
    });
    expect(input).toHaveValue('Moonlit');

    act(() => {
      useStore.setState({
        library: { ...useStore.getState().library, query: '' },
      });
    });
    expect(input).toHaveValue('');
  });
});
