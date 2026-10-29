import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { BuiltinPager } from './BuiltinPager';
import { TrackBrowser } from './TrackBrowser';

describe('BuiltinPager', () => {
  afterEach(cleanup);

  it('validates page jumps, supports Enter and Escape, and returns focus to the current page indicator', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(<BuiltinPager useStore={store} />);

    const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
    expect(pager).toHaveAttribute('aria-busy', 'false');
    const pageOne = screen.getByRole('button', { name: /Page 1 of 5/ });
    expect(pageOne).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Showing 1–100 of 500 songs. Page 1 of 5.',
    );

    fireEvent.click(pageOne);
    expect(pageOne).toHaveAttribute('aria-expanded', 'true');
    const input = screen.getByRole('textbox', { name: 'Go to page' });
    await waitFor(() => expect(input).toHaveFocus());
    for (const value of ['1.5', '0', '6', '9007199254740992']) {
      fireEvent.change(input, { target: { value } });
      fireEvent.submit(input.closest('form')!);
      expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole page number from 1 to 5.');
      expect(input).toHaveAttribute('aria-invalid', 'true');
      expect(store.getState().library.builtinPage.committedPage).toBe(1);
    }

    fireEvent.change(input, { target: { value: '5' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('textbox', { name: 'Go to page' })).toBeNull();
    await waitFor(() => expect(pageOne).toHaveFocus());
    expect(store.getState().library.builtinPage.committedPage).toBe(1);

    fireEvent.click(pageOne);
    const reopenedInput = screen.getByRole('textbox', { name: 'Go to page' });
    fireEvent.change(reopenedInput, { target: { value: '5' } });
    await act(async () => fireEvent.submit(reopenedInput.closest('form')!));
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(5));
    const pageFive = screen.getByRole('button', { name: /Page 5 of 5/ });
    expect(pageFive).toHaveAttribute('aria-current', 'page');
    await waitFor(() => expect(pageFive).toHaveFocus());
    expect(screen.getByRole('status')).toHaveTextContent(
      'Showing 401–500 of 500 songs. Page 5 of 5.',
    );
  });

  it.each([0, 1, 96, 99, 100, 101])(
    'keeps the range visible and hides one-page controls for %i built-in songs',
    async (count) => {
      const rows = Array.from({ length: count }, (_, index) => ({
        song_id: index.toString(16).padStart(32, '0'),
        title: `Pager fixture ${index + 1}`,
        format_label: 'TXT',
        duration_us: 1_000_000,
        note_count: 1,
        risk_level: 'low' as const,
        metadata_state: 'ready' as const,
        liked: false,
      }));
      const store = createDesktopStore(
        createMockBridge({
          catalogRows: rows,
          builtinSongIds: rows.map((row) => row.song_id),
        }),
      );
      await act(async () => store.getState().initialize());
      await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
      const { container } = render(<TrackBrowser useStore={store} />);
      if (count === 0) {
        expect(screen.getByText('The built-in song catalog is empty.')).toBeInTheDocument();
        expect(container.querySelector('.builtin-pager-status')).toBeNull();
        expect(container.querySelector('[aria-label="Built-in Songs pages"]')).toBeNull();
        expect(screen.queryByText(/Page 1 of 0/)).toBeNull();
        return;
      }
      expect(screen.getByRole('status')).toHaveTextContent(
        `Showing 1–${Math.min(100, count)} of ${count} songs.`,
      );
      if (count <= 100) {
        expect(container.querySelector('[aria-label="Built-in Songs pages"]')).toBeNull();
        expect(container.querySelectorAll('.builtin-pager-control')).toHaveLength(0);
        expect(screen.queryByRole('button', { name: /Page .* of/ })).toBeNull();
      } else {
        expect(
          screen.getByRole('navigation', { name: 'Built-in Songs pages' }),
        ).toBeInTheDocument();
        expect(screen.getByRole('button', { name: /Page 1 of 2/ })).toHaveAttribute(
          'aria-current',
          'page',
        );
      }
    },
  );

  it('retains the explicit unavailable empty state without displaying a zero page', async () => {
    const store = createDesktopStore(
      createMockBridge({ builtinAvailable: false, builtinSongIds: [], catalogRows: [] }),
    );
    await act(async () => store.getState().initialize());
    const current = store.getState();
    const builtin = { kind: 'smart' as const, id: 'builtin' as const };
    act(() =>
      store.setState({
        library: {
          ...current.library,
          source: builtin,
          searchSource: builtin,
          resultTotal: 0,
        },
      }),
    );

    const { container } = render(<TrackBrowser useStore={store} />);
    expect(
      screen.getByText(
        'Built-in songs are unavailable in this installation. Create a playlist to import local sheets.',
      ),
    ).toBeInTheDocument();
    expect(container.querySelector('[aria-label="Built-in Songs pages"]')).toBeNull();
    expect(screen.queryByText(/Page 1 of 0/)).toBeNull();
  });
});
