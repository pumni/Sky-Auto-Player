import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { BuiltinPager } from './BuiltinPager';

describe('BuiltinPager', () => {
  afterEach(cleanup);

  it('validates page jumps, supports Enter and Escape, and announces the committed range', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(<BuiltinPager useStore={store} />);

    const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
    expect(pager).toHaveAttribute('aria-busy', 'false');
    expect(screen.getByRole('button', { name: 'Page 1' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Showing 1–100 of 500 songs. Page 1 of 5.',
    );

    fireEvent.click(screen.getByRole('button', { name: 'Go to page' }));
    const input = screen.getByRole('textbox', { name: 'Go to page' });
    fireEvent.change(input, { target: { value: '1.5' } });
    fireEvent.submit(input.closest('form')!);
    expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole page number from 1 to 5.');
    expect(input).toHaveAttribute('aria-invalid', 'true');
    expect(store.getState().library.builtinPage.committedPage).toBe(1);

    fireEvent.change(input, { target: { value: '5' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('textbox', { name: 'Go to page' })).toBeNull();
    expect(store.getState().library.builtinPage.committedPage).toBe(1);

    fireEvent.click(screen.getByRole('button', { name: 'Go to page' }));
    const reopenedInput = screen.getByRole('textbox', { name: 'Go to page' });
    fireEvent.change(reopenedInput, { target: { value: '5' } });
    await act(async () => fireEvent.submit(reopenedInput.closest('form')!));
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(5));
    expect(screen.getByRole('button', { name: 'Page 5' })).toHaveAttribute('aria-current', 'page');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Showing 401–500 of 500 songs. Page 5 of 5.',
    );
  });

  it.each([0, 1, 96, 99, 100])(
    'does not render page controls for %i built-in songs',
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
      const { container } = render(<BuiltinPager useStore={store} />);
      expect(container.querySelector('[aria-label="Built-in Songs pages"]')).toBeNull();
      expect(screen.queryByText(/Page 1 of 0/)).toBeNull();
    },
  );
});
