import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { BuiltinPager } from './BuiltinPager';
import { TrackBrowser } from './TrackBrowser';

describe('BuiltinPager', () => {
  afterEach(cleanup);

  it('validates jumps, preselects the current page, supports Enter and Go, and restores focus', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    render(<BuiltinPager useStore={store} />);

    const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
    expect(pager).toHaveAttribute('aria-busy', 'false');
    expect(pager.querySelectorAll('button')).toHaveLength(3);
    const pageOne = screen.getByRole('button', {
      name: '1 / 5, Page 1 of 5. Go to page',
    });
    const previous = screen.getByRole('button', { name: 'Previous page' });
    const next = screen.getByRole('button', { name: 'Next page' });
    expect(previous).toHaveAttribute('title', 'Previous page');
    expect(next).toHaveAttribute('title', 'Next page');
    expect(previous).not.toHaveTextContent('Previous');
    expect(next).not.toHaveTextContent('Next');
    expect(previous.querySelector('svg')).toHaveAttribute('width', '16');
    expect(next.querySelector('svg')).toHaveAttribute('width', '16');
    expect(pageOne).toHaveAttribute('aria-current', 'page');
    expect(pageOne).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Showing 1–100 of 500 songs. Page 1 of 5.',
    );
    expect(screen.getByText('1–100 of 500 songs')).toBeVisible();

    fireEvent.click(pageOne);
    expect(pageOne).toHaveAttribute('aria-expanded', 'true');
    expect(
      document.getElementById(pageOne.getAttribute('aria-controls') ?? ''),
    ).toBeInTheDocument();
    const input = screen.getByRole('textbox', { name: 'Page number' });
    await waitFor(() => expect(input).toHaveFocus());
    expect(input).toHaveValue('1');
    expect((input as HTMLInputElement).selectionStart).toBe(0);
    expect((input as HTMLInputElement).selectionEnd).toBe(1);
    const dialog = screen.getByRole('dialog', { name: 'Go to page' });
    expect(within(dialog).getByText('of 5')).toBeVisible();
    expect(within(dialog).getAllByRole('button')).toHaveLength(1);
    expect(within(dialog).getByRole('button', { name: 'Go' })).toBeVisible();
    expect(within(dialog).queryByRole('button', { name: 'Cancel' })).toBeNull();
    for (const value of ['', 'abc', '1.5', '0', '-1', '6', '9007199254740992']) {
      fireEvent.change(input, { target: { value } });
      fireEvent.submit(input.closest('form')!);
      expect(screen.getByRole('alert')).toHaveTextContent('Enter a whole page number from 1 to 5.');
      expect(input).toHaveAttribute('aria-invalid', 'true');
      expect(store.getState().library.builtinPage.committedPage).toBe(1);
    }

    fireEvent.change(input, { target: { value: '5' } });
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(screen.queryByRole('textbox', { name: 'Page number' })).toBeNull();
    await waitFor(() => expect(pageOne).toHaveFocus());
    expect(store.getState().library.builtinPage.committedPage).toBe(1);

    fireEvent.click(pageOne);
    const reopenedInput = screen.getByRole('textbox', { name: 'Page number' });
    fireEvent.change(reopenedInput, { target: { value: '5' } });
    await act(async () => fireEvent.submit(reopenedInput.closest('form')!));
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(5));
    const pageFive = screen.getByRole('button', {
      name: '5 / 5, Page 5 of 5. Go to page',
    });
    expect(pageFive).toHaveAttribute('aria-current', 'page');
    await waitFor(() => expect(pageFive).toHaveFocus());
    expect(screen.getByRole('status')).toHaveTextContent(
      'Showing 401–500 of 500 songs. Page 5 of 5.',
    );

    fireEvent.click(pageFive);
    const goInput = screen.getByRole('textbox', { name: 'Page number' });
    fireEvent.change(goInput, { target: { value: '3' } });
    fireEvent.click(screen.getByRole('button', { name: 'Go' }));
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(3));
    const pageThree = screen.getByRole('button', {
      name: '3 / 5, Page 3 of 5. Go to page',
    });
    await waitFor(() => expect(pageThree).toHaveFocus());

    fireEvent.click(pageThree);
    fireEvent.keyDown(screen.getByRole('textbox', { name: 'Page number' }), { key: 'Escape' });
    await waitFor(() => expect(pageThree).toHaveFocus());
  });

  it('moves focus across the enabled Previous, page-jump, and Next controls with arrow keys', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    await act(async () => store.getState().selectLibrarySource({ kind: 'smart', id: 'builtin' }));
    await act(async () => store.getState().navigateBuiltinPage(2));
    render(<BuiltinPager useStore={store} />);

    const previous = screen.getByRole('button', { name: 'Previous page' });
    const indicator = screen.getByRole('button', {
      name: '2 / 5, Page 2 of 5. Go to page',
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
    expect(screen.getByRole('button', { name: '1 / 5, Page 1 of 5. Go to page' })).toBeVisible();
    expect(screen.getByText('1–100 of 500 songs')).toBeVisible();
    expect(screen.getByRole('status')).toHaveTextContent('Loading page 2 of 5.');

    await waitFor(() =>
      expect(screen.getByRole('button', { name: '2 / 5, Page 2 of 5. Go to page' })).toBeVisible(),
    );
    expect(pager).toHaveAttribute('aria-busy', 'false');
    expect(screen.getByText('101–200 of 500 songs')).toBeVisible();
  });

  it.each([0, 1, 96, 100, 101, 106, 500, 2438, 10_000])(
    'keeps the range and expected three-control pager for %i built-in songs',
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
        expect(screen.queryByRole('button', { name: /Go to page/ })).toBeNull();
      } else {
        const pageCount = Math.ceil(count / 100);
        const pager = screen.getByRole('navigation', { name: 'Built-in Songs pages' });
        expect(pager.querySelectorAll('button')).toHaveLength(3);
        expect(screen.getByRole('button', { name: 'Previous page' })).toBeDisabled();
        expect(
          screen.getByRole('button', {
            name: `1 / ${pageCount}, Page 1 of ${pageCount}. Go to page`,
          }),
        ).toHaveAttribute('aria-current', 'page');
        expect(screen.getByRole('button', { name: 'Next page' })).toBeEnabled();
        expect(screen.queryByRole('button', { name: /First page|Last page|Page \d+$/ })).toBeNull();
      }
    },
  );

  it('shows failed-page retry outside the pager row and restores focus after retry', async () => {
    const store = createDesktopStore(createMockBridge());
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
    const retry = screen.getByRole('button', { name: 'Retry page 2' });
    await act(async () => fireEvent.click(retry));
    await waitFor(() => expect(store.getState().library.builtinPage.committedPage).toBe(2));
    const pageTwo = screen.getByRole('button', {
      name: '2 / 5, Page 2 of 5. Go to page',
    });
    await waitFor(() => expect(pageTwo).toHaveFocus());
    expect(screen.queryByRole('alert')).toBeNull();
  });

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
