import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import { App } from './App';

describe('desktop application shell', () => {
  afterEach(() => cleanup());

  it('renders the navigator, track browser, utility details, and settings', async () => {
    render(<App bridge={createMockBridge()} />);
    expect(screen.getByText('Opening your library…')).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByRole('row', { name: /Aurora Landing/ })).toBeInTheDocument(),
    );
    expect(screen.getByRole('navigation', { name: 'Library' })).toHaveTextContent('All Songs');
    expect(screen.getByRole('grid', { name: 'Songs' })).toBeInTheDocument();

    const firstSong = screen.getByRole('row', { name: /Aurora Landing/ });
    fireEvent.click(firstSong);
    await waitFor(() => expect(firstSong).toHaveAttribute('aria-selected', 'true'));
    expect(screen.queryByText('Medium timing risk')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open utility panel' }));
    await waitFor(() =>
      expect(screen.getByRole('region', { name: 'Utility: Song Details' })).toBeInTheDocument(),
    );
    expect(screen.getByText('Medium timing risk')).toBeInTheDocument();

    const settingsButton = screen.getByRole('button', { name: 'Open settings' });
    fireEvent.click(settingsButton);
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
    expect(screen.getByLabelText('Theme')).toHaveValue('aurora');
    fireEvent.click(screen.getByRole('button', { name: 'About' }));
    expect(screen.getByRole('dialog', { name: 'Settings' })).toHaveTextContent('3.5.0-mock');
    expect(screen.getByRole('dialog', { name: 'Settings' })).toHaveTextContent('Native ABI');
    expect(screen.getByRole('dialog', { name: 'Settings' })).toContainElement(
      document.activeElement as HTMLElement,
    );
    fireEvent.keyDown(screen.getByRole('dialog', { name: 'Settings' }), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Settings' })).toBeNull());
    expect(document.activeElement).toBe(settingsButton);
  });

  it('supports keyboard navigation in the virtualized track table', async () => {
    render(<App bridge={createMockBridge()} />);
    const grid = await screen.findByRole('grid', { name: 'Songs' });
    grid.focus();
    fireEvent.keyDown(grid, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Blue Bird/ })
          .some((row) => row.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
    fireEvent.keyDown(grid, { key: 'Home' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Aurora Landing/ })
          .some((row) => row.getAttribute('aria-selected') === 'true'),
      ).toBe(true),
    );
  });

  it('loads and selects a keyboard destination on an unloaded virtualized page', async () => {
    render(<App bridge={createMockBridge()} />);
    const grid = await screen.findByRole('grid', { name: 'Songs' });
    grid.focus();
    fireEvent.keyDown(grid, { key: 'End' });
    await waitFor(() =>
      expect(
        screen
          .getAllByRole('row', { name: /Song 500/ })
          .some((row) => row.getAttribute('aria-selected') === 'true'),
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

  it('opens utility diagnostics as a bounded overlay and closes with Escape', async () => {
    render(<App bridge={createMockBridge()} />);
    await screen.findByRole('row', { name: /Aurora Landing/ });

    const trigger = screen.getByRole('button', { name: 'Open utility panel' });
    fireEvent.click(trigger);
    const utility = await screen.findByRole('region', { name: 'Utility: Song Details' });
    fireEvent.click(screen.getByRole('tab', { name: 'Runtime' }));
    const diagnostics = await screen.findByRole('region', { name: 'Utility: Diagnostics' });
    expect(utility).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Performance' })).toBeInTheDocument();
    fireEvent.keyDown(diagnostics, { key: 'Escape' });
    await waitFor(() =>
      expect(screen.queryByRole('region', { name: 'Utility: Diagnostics' })).toBeNull(),
    );
    expect(document.activeElement).toBe(trigger);

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
