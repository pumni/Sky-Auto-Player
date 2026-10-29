import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { BuiltinPager } from './BuiltinPager';
import { TrackBrowser } from './TrackBrowser';

describe('BuiltinPager', () => {
  afterEach(cleanup);

  it.each([
    { total: 106, pages: 2, target: 2 },
    { total: 500, pages: 5, target: 5 },
  ])(
    'opens an inline page editor for $total songs and navigates to page $target',
    async ({ total, pages, target }) => {
      const store = createDesktopStore(createMockBridge({ catalogCount: total }));
      await act(async () => store.getState().initialize());
      await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
      render(<BuiltinPager useStore={store} />);

      const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
      const pageOne = screen.getByRole('button', {
        name: `1 / ${pages}, Page 1 of ${pages}. Jump to page`,
      });
      expect(pageOne).toHaveAttribute('aria-current', 'page');
      expect(pageOne).toHaveAttribute('title', 'Jump to page');
      expect(pager.querySelectorAll('button')).toHaveLength(3);
      fireEvent.click(pageOne);
      const input = screen.getByRole('textbox', { name: `Page number, from 1 to ${pages}` });
      await waitFor(() => expect(input).toHaveFocus());
      expect(input).toHaveValue('1');
      expect(screen.queryByRole('dialog')).toBeNull();
      expect(screen.queryByText(/go to page/i)).toBeNull();
      expect(screen.queryByRole('button', { name: /^go$/i })).toBeNull();
      fireEvent.change(input, { target: { value: String(target) } });
      fireEvent.keyDown(input, { key: 'Enter' });
      await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(target));
      const selectedPage = screen.getByRole('button', {
        name: `${target} / ${pages}, Page ${target} of ${pages}. Jump to page`,
      });
      await waitFor(() => expect(selectedPage).toHaveFocus());
      expect(
        screen.getByText(`${(target - 1) * 100 + 1}–${total} of ${total} songs`),
      ).toBeVisible();

      if (total === 106) {
        fireEvent.click(selectedPage);
        const returnInput = screen.getByRole('textbox', { name: 'Page number, from 1 to 2' });
        fireEvent.change(returnInput, { target: { value: '1' } });
        fireEvent.click(screen.getByRole('button', { name: 'Confirm page change' }));
        await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(1));
        await waitFor(() =>
          expect(
            screen.getByRole('button', { name: '1 / 2, Page 1 of 2. Jump to page' }),
          ).toHaveFocus(),
        );
      }
    },
  );

  it.each([
    { total: 501, pages: 6 },
    { total: 2438, pages: 25 },
    { total: 10_000, pages: 100 },
  ])('validates and commits an inline page edit for $total songs', async ({ total, pages }) => {
    const store = createDesktopStore(createMockBridge({ catalogCount: total }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(<BuiltinPager useStore={store} />);

    const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
    const pageOne = screen.getByRole('button', {
      name: `1 / ${pages}, Page 1 of ${pages}. Jump to page`,
    });
    expect(pageOne).toHaveAttribute('aria-current', 'page');
    expect(pageOne).toHaveAttribute('title', 'Jump to page');
    fireEvent.click(pageOne);

    const input = screen.getByRole('textbox', { name: `Page number, from 1 to ${pages}` });
    await waitFor(() => expect(input).toHaveFocus());
    expect(input).toHaveValue('1');
    expect((input as HTMLInputElement).selectionStart).toBe(0);
    expect((input as HTMLInputElement).selectionEnd).toBe(1);
    expect(pager.querySelectorAll('button')).toHaveLength(3);
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(screen.queryByText(/go to page/i)).toBeNull();
    expect(screen.queryByRole('button', { name: /^go$/i })).toBeNull();

    for (const value of ['', 'abc', '1.5', '0', '-1', String(pages + 1), '9007199254740992']) {
      fireEvent.change(input, { target: { value } });
      fireEvent.keyDown(input, { key: 'Enter' });
      expect(screen.getByRole('alert')).toHaveTextContent(
        `Enter a whole page number from 1 to ${pages}.`,
      );
      expect(input).toHaveAttribute('aria-invalid', 'true');
      expect(input.getAttribute('aria-describedby')).toMatch(/.+/);
      expect(store.getState().library.builtinPage.committedPage).toBe(1);
    }

    fireEvent.change(input, { target: { value: String(pages) } });
    fireEvent.keyDown(input, { key: 'Enter' });
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(pages));
    const lastPage = screen.getByRole('button', {
      name: `${pages} / ${pages}, Page ${pages} of ${pages}. Jump to page`,
    });
    await waitFor(() => expect(lastPage).toHaveFocus());
    expect(screen.getByText(`${(pages - 1) * 100 + 1}–${total} of ${total} songs`)).toBeVisible();
  });

  it('commits with the inline confirmation button, cancels with Escape and blur, and restores focus', async () => {
    const store = createDesktopStore(createMockBridge({ catalogCount: 501 }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(
      <>
        <BuiltinPager useStore={store} />
        <button type="button">Outside pager</button>
      </>,
    );

    let trigger = screen.getByRole('button', {
      name: '1 / 6, Page 1 of 6. Jump to page',
    });
    fireEvent.click(trigger);
    let input = screen.getByRole('textbox', { name: 'Page number, from 1 to 6' });
    fireEvent.change(input, { target: { value: '3' } });
    fireEvent.click(screen.getByRole('button', { name: 'Confirm page change' }));
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(3));
    trigger = screen.getByRole('button', { name: '3 / 6, Page 3 of 6. Jump to page' });
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: '3 / 6, Page 3 of 6. Jump to page' }),
      ).toHaveFocus(),
    );

    fireEvent.click(trigger);
    input = screen.getByRole('textbox', { name: 'Page number, from 1 to 6' });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('textbox', { name: /page number/i })).toBeNull();
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: '3 / 6, Page 3 of 6. Jump to page' }),
      ).toHaveFocus(),
    );
    expect(store.getState().library.builtinPage.committedPage).toBe(3);

    trigger = screen.getByRole('button', { name: '3 / 6, Page 3 of 6. Jump to page' });
    fireEvent.click(trigger);
    input = screen.getByRole('textbox', { name: 'Page number, from 1 to 6' });
    await waitFor(() => expect(input).toHaveFocus());
    const confirm = screen.getByRole('button', { name: 'Confirm page change' });
    const outside = screen.getByRole('button', { name: 'Outside pager' });
    fireEvent.blur(input, { relatedTarget: confirm });
    expect(input).toBeInTheDocument();
    fireEvent.blur(confirm, { relatedTarget: outside });
    await waitFor(() => expect(screen.queryByRole('textbox', { name: /page number/i })).toBeNull());
    outside.focus();
    expect(outside).toHaveFocus();
  });

  it('does not steal focus moved elsewhere while an inline page navigation is pending', async () => {
    const store = createDesktopStore(createMockBridge({ catalogCount: 500, searchDelayMs: 150 }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(
      <>
        <BuiltinPager useStore={store} />
        <button type="button">Outside pager</button>
      </>,
    );

    fireEvent.click(screen.getByRole('button', { name: '1 / 5, Page 1 of 5. Jump to page' }));
    const input = screen.getByRole('textbox', { name: 'Page number, from 1 to 5' });
    await waitFor(() => expect(input).toHaveFocus());
    fireEvent.change(input, { target: { value: '2' } });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(screen.getByRole('navigation', { name: 'Built-in Songs pages' })).toHaveAttribute(
      'aria-busy',
      'true',
    );

    const outside = screen.getByRole('button', { name: 'Outside pager' });
    outside.focus();
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(2));
    await waitFor(() =>
      expect(screen.getByRole('navigation', { name: 'Built-in Songs pages' })).toHaveAttribute(
        'aria-busy',
        'false',
      ),
    );
    expect(outside).toHaveFocus();
  });

  it('moves focus between the previous, page indicator and next controls with arrow keys', async () => {
    const store = createDesktopStore(createMockBridge({ catalogCount: 500 }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    await act(async () => store.getState().navigateBuiltinPage(2));
    render(<BuiltinPager useStore={store} />);

    const previous = screen.getByRole('button', { name: 'Previous page' });
    const indicator = screen.getByRole('button', {
      name: '2 / 5, Page 2 of 5. Jump to page',
    });
    const next = screen.getByRole('button', { name: 'Next page' });
    previous.focus();
    fireEvent.keyDown(previous, { key: 'ArrowRight' });
    expect(indicator).toHaveFocus();
    fireEvent.keyDown(indicator, { key: 'ArrowRight' });
    expect(next).toHaveFocus();
    fireEvent.keyDown(next, { key: 'ArrowLeft' });
    expect(indicator).toHaveFocus();
    fireEvent.keyDown(indicator, { key: 'ArrowLeft' });
    expect(previous).toHaveFocus();
  });

  it('keeps the committed range visible while a requested page is loading', async () => {
    const store = createDesktopStore(createMockBridge({ searchDelayMs: 150 }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(<BuiltinPager useStore={store} />);

    const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
    fireEvent.click(screen.getByRole('button', { name: 'Next page' }));
    expect(pager).toHaveAttribute('aria-busy', 'true');
    expect(screen.getByText('1–100 of 500 songs')).toBeVisible();
    expect(screen.getByRole('status')).toHaveTextContent('Loading page 2 of 5.');

    await waitFor(() => expect(screen.getByText('2 / 5')).toBeVisible());
    expect(pager).toHaveAttribute('aria-busy', 'false');
    expect(screen.getByText('101–200 of 500 songs')).toBeVisible();
  });

  it('retries a failed page and focuses the page indicator after navigation', async () => {
    const store = createDesktopStore(createMockBridge({ catalogCount: 500 }));
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    const current = store.getState();
    act(() =>
      store.setState({
        library: {
          ...current.library,
          builtinPage: { ...current.library.builtinPage, failedPage: 2 },
        },
      }),
    );
    render(<TrackBrowser useStore={store} />);

    expect(screen.getByRole('alert')).toHaveTextContent('Couldn’t load page 2.');
    await act(async () => screen.getByRole('button', { name: 'Retry page 2' }).click());
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(2));
    await waitFor(() =>
      expect(
        screen.getByRole('button', { name: '2 / 5, Page 2 of 5. Jump to page' }),
      ).toHaveFocus(),
    );
    expect(screen.getByText('2 / 5')).toBeVisible();
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it.each([0, 1, 96, 100, 101, 106, 500, 501, 2438, 10_000])(
    'uses the right interaction model for %i built-in songs',
    async (count) => {
      const store = createDesktopStore(createMockBridge({ catalogCount: count }));
      await act(async () => store.getState().initialize());
      await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
      const { container } = render(<TrackBrowser useStore={store} />);

      if (count === 0) {
        expect(screen.getByText('The built-in song catalog is empty.')).toBeInTheDocument();
        expect(container.querySelector('.builtin-pager-range')).toBeNull();
        expect(container.querySelector('[aria-label="Built-in Songs pages"]')).toBeNull();
        expect(screen.queryByText(/Page 1 of 0/)).toBeNull();
        return;
      }

      const lastRow = Math.min(100, count);
      expect(screen.getByText(`1–${lastRow} of ${count} songs`)).toBeVisible();
      expect(screen.getByRole('status')).toHaveTextContent(
        `Showing 1–${lastRow} of ${count} songs.`,
      );
      if (count <= 100) {
        expect(container.querySelector('[aria-label="Built-in Songs pages"]')).toBeNull();
        expect(container.querySelectorAll('.builtin-pager-control')).toHaveLength(0);
        return;
      }

      const pageCount = Math.ceil(count / 100);
      const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
      expect(pager.querySelectorAll('button')).toHaveLength(3);
      expect(screen.getByRole('button', { name: 'Previous page' })).toBeDisabled();
      expect(screen.getByRole('button', { name: 'Next page' })).toBeEnabled();
      expect(pager.querySelector('.builtin-pager-static-indicator')).toBeNull();
      expect(
        screen.getByRole('button', {
          name: `1 / ${pageCount}, Page 1 of ${pageCount}. Jump to page`,
        }),
      ).toHaveAttribute('aria-current', 'page');
    },
  );

  it('keeps the unavailable empty state truthful without displaying a zero page', async () => {
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
