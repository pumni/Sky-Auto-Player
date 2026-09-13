import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
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
    const autoPlay = screen.getByRole('switch', { name: 'Auto Play' });
    expect(autoPlay).toHaveAttribute('aria-checked', 'true');
    await act(async () => autoPlay.click());
    await waitFor(() => expect(store.getState().settings?.auto_play).toBe(false));
    expect(screen.getByRole('switch', { name: 'Auto Play' })).toHaveAttribute(
      'aria-checked',
      'false',
    );
    expect(screen.getByText('Late Down tolerance').parentElement).toHaveTextContent('2000 µs');
    expect(screen.getByRole('heading', { name: 'Timing' })).toBeInTheDocument();
    expect(
      screen.getByText('Source: Default fallback (no valid calibration cache)'),
    ).toBeInTheDocument();
    expect(screen.getByText('· rec. 500 µs')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Use recommended/ })).not.toBeInTheDocument();
    expect(screen.queryByText(/sender evidence, not proof/)).not.toBeInTheDocument();
  });
});
