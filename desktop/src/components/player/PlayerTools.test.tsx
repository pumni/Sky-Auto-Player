import { act, cleanup, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { PlayerTools } from './PlayerTools';

describe('PlayerTools playback modes and profile', () => {
  afterEach(cleanup);

  it('places the persisted Auto Play button before Profile and Utility and removes it from the profile', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    render(<PlayerTools useStore={store} />);

    const autoPlay = screen.getByRole('button', { name: 'Auto Play' });
    const profile = screen.getByRole('button', { name: 'Configure playback profile' });
    const utility = screen.getByRole('button', { name: 'Open utility panel' });
    expect(autoPlay).toHaveAttribute('aria-pressed', 'true');
    expect(autoPlay).toHaveAttribute('title', 'Auto Play on');
    expect(autoPlay.querySelector('svg')).toHaveClass('lucide-list-music');
    expect(autoPlay.querySelector('svg')).not.toHaveClass('lucide-list-end');
    expect(
      autoPlay.compareDocumentPosition(profile) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      profile.compareDocumentPosition(utility) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();

    await act(async () => autoPlay.click());
    await waitFor(() => expect(store.getState().settings?.auto_play).toBe(false));
    expect(screen.getByRole('button', { name: 'Auto Play' })).toHaveAttribute(
      'aria-pressed',
      'false',
    );
    expect(screen.getByRole('button', { name: 'Auto Play' })).toHaveAttribute(
      'title',
      'Auto Play off',
    );

    await act(async () => screen.getByRole('button', { name: 'Auto Play' }).click());
    await waitFor(() => expect(store.getState().settings?.auto_play).toBe(true));
    expect(screen.getByRole('button', { name: 'Auto Play' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );

    await act(async () => profile.click());

    const dialog = screen.getByRole('dialog', { name: 'Playback profile' });
    expect(within(dialog).getByLabelText('Base Hold')).toBeInTheDocument();
    expect(within(dialog).queryByRole('switch', { name: 'Auto Play' })).not.toBeInTheDocument();
    expect(within(dialog).queryByRole('button', { name: 'Auto Play' })).not.toBeInTheDocument();
  });
});
