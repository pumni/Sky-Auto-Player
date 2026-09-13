import { act, cleanup, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { PlayerTools } from './PlayerTools';

describe('PlayerTools playback profile', () => {
  afterEach(cleanup);

  it('places the authoritative Auto Play switch beside Base Hold', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    render(<PlayerTools useStore={store} />);
    await act(async () =>
      screen.getByRole('button', { name: 'Configure playback profile' }).click(),
    );

    const row = document.querySelector('.profile-base-row');
    expect(row).toBeInTheDocument();
    expect(row).toContainElement(screen.getByLabelText('Base Hold'));
    const autoPlay = screen.getByRole('switch', { name: 'Auto Play' });
    expect(row).toContainElement(autoPlay);
    expect(autoPlay).toHaveAttribute('aria-checked', 'true');

    await act(async () => autoPlay.click());
    await waitFor(() => expect(store.getState().settings?.auto_play).toBe(false));
  });
});
