import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { SettingsPanel } from './SettingsPanel';

describe('SettingsPanel playback timing', () => {
  afterEach(() => cleanup());

  it('keeps recommendation source subordinate and preserves the timing summary', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const bootstrap = store.getState().bootstrap;
    if (!bootstrap) throw new Error('bootstrap should be available');
    act(() => store.getState().setSettingsOpen(true));

    render(<SettingsPanel bootstrap={bootstrap} useStore={store} />);

    expect(screen.getByLabelText('Base Hold')).toBeInTheDocument();
    expect(document.querySelector('.settings-content')).toHaveClass('scroll-surface');
    const behaviorHeading = screen.getByRole('heading', { name: 'Behavior' });
    const defaultsHeading = screen.getByRole('heading', { name: 'Playback defaults' });
    expect(
      behaviorHeading.compareDocumentPosition(defaultsHeading) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    const autoPlay = screen.getByRole('switch', { name: 'Auto Play' });
    expect(autoPlay.parentElement).toBe(behaviorHeading.parentElement);
    expect(autoPlay).toHaveTextContent(
      'Automatically continue to the next song after the current song finishes successfully.',
    );
    expect(autoPlay).toHaveAttribute('aria-checked', 'true');
    await act(async () => autoPlay.click());
    await waitFor(() => expect(store.getState().settings?.auto_play).toBe(false));
    expect(screen.getByRole('switch', { name: 'Auto Play' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
    expect(screen.queryByText(/Late Down/)).toBeNull();
    expect(screen.getByRole('heading', { name: 'Timing' })).toBeInTheDocument();
    expect(
      screen.getByText('Source: Default fallback (no valid calibration cache)'),
    ).toBeInTheDocument();
    expect(screen.getByText('· rec. 500 µs')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Use recommended/ })).not.toBeInTheDocument();
    expect(screen.queryByText(/sender evidence, not proof/)).not.toBeInTheDocument();
  });

  it('keeps the dialog usable after a failed settings mutation', async () => {
    const bridge = createMockBridge();
    const originalPatch = bridge.patchSettings;
    let attempts = 0;
    bridge.patchSettings = async (patch) => {
      attempts += 1;
      if (attempts === 1) throw new Error('settings validation failed');
      return originalPatch(patch);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const bootstrap = store.getState().bootstrap;
    if (!bootstrap) throw new Error('bootstrap should be available');
    act(() => {
      store.getState().setSettingsOpen(true);
    });

    render(<SettingsPanel bootstrap={bootstrap} useStore={store} />);
    fireEvent.click(screen.getByRole('button', { name: 'Appearance' }));
    const palette = screen.getByLabelText('Color palette');

    await act(async () => {
      fireEvent.change(palette, { target: { value: 'slate' } });
    });
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent('Could not save settings.'),
    );
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(store.getState().settings?.palette).toBe('aurora');
    expect(palette).toHaveValue('aurora');

    await act(async () => {
      fireEvent.change(palette, { target: { value: 'classic' } });
    });
    await waitFor(() => expect(store.getState().settings?.palette).toBe('classic'));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
