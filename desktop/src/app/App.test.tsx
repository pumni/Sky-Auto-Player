import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import { App } from './App';

describe('desktop application shell', () => {
  afterEach(() => cleanup());

  it('renders library, inspector, and settings through the bridge', async () => {
    render(<App bridge={createMockBridge()} />);
    expect(screen.getByText('Opening your library…')).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole('option', { name: /Aurora Landing/ })).toBeInTheDocument(),
    );

    const firstSong = screen.getByRole('option', { name: /Aurora Landing/ });
    fireEvent.click(firstSong);
    await waitFor(() => expect(firstSong).toHaveAttribute('aria-selected', 'true'));
    await waitFor(() => expect(screen.getByText('Medium timing risk')).toBeInTheDocument());

    const settingsButton = screen.getByRole('button', { name: 'Open settings' });
    fireEvent.click(settingsButton);
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByLabelText('Theme')).toHaveValue('aurora');
    expect(screen.getByRole('dialog', { name: 'Settings' })).toContainElement(
      document.activeElement as HTMLElement,
    );
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Settings' }), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull());
    expect(document.activeElement).toBe(settingsButton);
  });

  it('supports keyboard navigation in the virtualized library', async () => {
    render(<App bridge={createMockBridge()} />);
    const list = await screen.findByRole('listbox', { name: 'Songs' });
    list.focus();
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('option', { name: /Blue Bird/ })
          .some((option) => option.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
    fireEvent.keyDown(list, { key: 'Home' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('option', { name: /Aurora Landing/ })
          .some((option) => option.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
  });

  it('loads and selects a keyboard destination on an unloaded virtualized page', async () => {
    render(<App bridge={createMockBridge()} />);
    const list = await screen.findByRole('listbox', { name: 'Songs' });
    list.focus();

    fireEvent.keyDown(list, { key: 'End' });

    await waitFor(() =>
      expect(
        screen
          .getAllByRole('option', { name: /Song 500/ })
          .some((option) => option.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
  });

  it('keeps the window available when Core startup fails', async () => {
    const bridge = createMockBridge();
    bridge.subscribeUiEvents = async () => {
      throw new Error('Core executable is missing');
    };
    render(<App bridge={bridge} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Core executable is missing');
    expect(screen.queryByRole('button', { name: /Try again/i })).toBeNull();
  });
});
