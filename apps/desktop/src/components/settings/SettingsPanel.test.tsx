import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { SettingsPanel } from './SettingsPanel';

describe('SettingsPanel playback timing', () => {
  afterEach(() => cleanup());

  it('preserves the manual timing control and timing summary', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const bootstrap = store.getState().bootstrap;
    if (!bootstrap) throw new Error('bootstrap should be available');
    act(() => store.getState().setSettingsOpen(true));

    render(<SettingsPanel bootstrap={bootstrap} useStore={store} />);

    expect(screen.getByLabelText('Base Hold')).toBeInTheDocument();
    const settingsContent = document.querySelector<HTMLElement>('[data-settings-content]');
    expect(settingsContent).toBeInTheDocument();
    if (!settingsContent) throw new Error('settings content should be available');
    expect(settingsContent).toHaveAttribute('data-slot', 'scroll-area-viewport');
    const settingsScrollArea = settingsContent.closest<HTMLElement>('[data-slot="scroll-area"]');
    expect(settingsScrollArea).not.toBeNull();
    expect(settingsScrollArea).toContainElement(settingsContent);
    const behaviorHeading = screen.getByRole('heading', { name: 'Behavior' });
    const defaultsHeading = screen.getByRole('heading', { name: 'Playback defaults' });
    expect(
      behaviorHeading.compareDocumentPosition(defaultsHeading) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    const autoPlay = screen.getByRole('switch', { name: 'Auto Play' });
    const autoPlayRow = screen
      .getByText('Auto Play')
      .closest('[data-slot="auto-play-setting-row"]');
    expect(autoPlayRow).toContainElement(autoPlay);
    expect(autoPlayRow).toHaveTextContent(
      'Automatically continue to the next song after the current song finishes successfully.',
    );
    expect(autoPlay).toHaveAccessibleDescription(
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
    expect(screen.getByText('500 µs')).toBeInTheDocument();
    expect(screen.queryByText(/sender evidence, not proof/)).not.toBeInTheDocument();
  });

  it('shows Donate without claiming the page is a persisted native setting', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const bootstrap = store.getState().bootstrap;
    if (!bootstrap) throw new Error('bootstrap should be available');
    act(() => store.getState().setSettingsOpen(true));

    render(<SettingsPanel bootstrap={bootstrap} useStore={store} />);
    const nativeNote = 'Settings are validated and persisted by the native runtime.';
    expect(screen.getByText(nativeNote)).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Donate' }));
    expect(screen.getByRole('region', { name: 'Support Sky Auto Player' })).toBeInTheDocument();
    expect(screen.queryByText(nativeNote)).not.toBeInTheDocument();
    expect(screen.getByText('0857288363')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Playback' }));
    expect(screen.getByText(nativeNote)).toBeInTheDocument();
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
    const palette = screen.getByLabelText('Dark palette');

    await act(async () => {
      fireEvent.click(palette);
    });
    await act(async () => {
      fireEvent.keyDown(screen.getByRole('option', { name: 'Slate' }), { key: 'Enter' });
    });
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent('Could not save settings.'),
    );
    expect(screen.getByRole('dialog', { name: 'Settings' })).toBeInTheDocument();
    expect(store.getState().settings?.palette).toBe('graphite');
    expect(palette).toHaveTextContent('Graphite (Default)');

    await act(async () => {
      fireEvent.click(palette);
    });
    await act(async () => {
      fireEvent.keyDown(screen.getByRole('option', { name: 'Classic' }), { key: 'Enter' });
    });
    await waitFor(() => expect(store.getState().settings?.palette).toBe('classic'));
    expect(screen.queryByRole('alert')).toBeNull();
  });
});
