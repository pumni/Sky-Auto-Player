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
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
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

  it('keeps the window available when native startup fails', async () => {
    const bridge = createMockBridge();
    bridge.subscribeUiEvents = async () => {
      throw new Error('Native application is unavailable');
    };
    render(<App bridge={bridge} />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Native application is unavailable');
    expect(screen.queryByRole('button', { name: /Try again/i })).toBeNull();
  });

  it('opens bounded diagnostics and the safe calibration dialog', async () => {
    render(<App bridge={createMockBridge()} />);
    await screen.findByRole('option', { name: /Aurora Landing/ });

    fireEvent.click(screen.getByRole('button', { name: 'Open diagnostics' }));
    expect(await screen.findByRole('dialog', { name: 'Diagnostics' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Performance' })).toBeInTheDocument();
    fireEvent.click(
      screen
        .getByRole('dialog', { name: 'Diagnostics' })
        .querySelector('button[aria-label="Close diagnostics"]') as HTMLButtonElement,
    );
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Diagnostics' })).toBeNull());

    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    fireEvent.click(screen.getByRole('button', { name: 'Advanced' }));
    fireEvent.click(screen.getByRole('button', { name: 'Open calibration' }));
    expect(await screen.findByRole('dialog', { name: 'Timing calibration' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Start quick calibration' }));
    expect(await screen.findByText('Calibration complete')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: 'Timing calibration' })).toBeNull(),
    );
  });
});
