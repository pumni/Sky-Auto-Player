import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { createMockBridge } from '../bridge/mockBridge';
import { App } from './App';

describe('desktop application shell', () => {
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

    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(screen.getByLabelText('Theme')).toHaveValue('aurora');
  });
});
