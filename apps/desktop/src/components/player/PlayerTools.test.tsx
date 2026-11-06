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

  it('explains and starts the existing dry-run action without physical input', async () => {
    const bridge = createMockBridge({ playbackDurationMs: 120_000, startDelayMs: 1 });
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const row = store.getState().library.pages.get(0)?.[1];
    if (!row) throw new Error('mock Library is too small');
    await act(async () => store.getState().selectSong(row.song_id));
    render(<PlayerTools useStore={store} />);

    await act(async () =>
      screen.getByRole('button', { name: 'Configure playback profile' }).click(),
    );
    const dialog = screen.getByRole('dialog', { name: 'Playback profile' });
    expect(
      within(dialog).getByText(
        'Tests the current profile with dry-run playback; no physical keyboard input is sent.',
      ),
    ).toBeInTheDocument();
    await act(async () =>
      within(dialog).getByRole('button', { name: 'Test playback (no input)' }).click(),
    );

    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    expect(store.getState().playback.context?.dryRun).toBe(true);
  });
});
