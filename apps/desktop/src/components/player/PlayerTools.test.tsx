import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { PlayerTools } from './PlayerTools';

describe('PlayerTools playback modes and profile', () => {
  afterEach(cleanup);

  it('keeps playback tools compact with an accessible profile menu', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    render(<PlayerTools useStore={store} />);

    expect(screen.queryByRole('button', { name: 'Test without keyboard input' })).toBeNull();
    const autoPlay = screen.getByRole('button', { name: 'Auto Play' });
    const profile = screen.getByRole('button', { name: 'Configure playback profile' });
    const utility = screen.getByRole('button', { name: 'Open utility panel' });
    expect(autoPlay).toHaveAttribute('aria-pressed', 'true');
    expect(autoPlay).not.toHaveAttribute('title');
    expect(autoPlay.querySelector('svg')).toHaveClass('lucide-list-video');
    expect(autoPlay.querySelector('svg')).not.toHaveClass('lucide-repeat');
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
    expect(screen.getByRole('button', { name: 'Auto Play' })).not.toHaveAttribute('title');

    await act(async () => screen.getByRole('button', { name: 'Auto Play' }).click());
    await waitFor(() => expect(store.getState().settings?.auto_play).toBe(true));
    expect(screen.getByRole('button', { name: 'Auto Play' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );

    await act(async () => profile.click());

    const dialog = screen.getByRole('dialog', { name: 'Playback profile' });
    expect(within(dialog).getByRole('heading', { name: 'Playback profile' })).toBeVisible();
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

    const trigger = screen.getByRole('button', { name: 'Configure playback profile' });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    await act(async () => trigger.click());
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    const dialog = screen.getByRole('dialog', { name: 'Playback profile' });
    expect(within(dialog).getByText(row.title)).toBeInTheDocument();
    expect(
      within(dialog).getByText('Try these settings without sending any physical keyboard input.'),
    ).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Test playback (no input)' }));

    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    expect(store.getState().playback.context?.dryRun).toBe(true);
    await act(async () =>
      screen.getByRole('button', { name: 'Configure playback profile' }).click(),
    );
    const runningDialog = screen.getByRole('dialog', { name: 'Playback profile' });
    expect(
      within(runningDialog).getByRole('button', { name: 'Test playback (no input)' }),
    ).toBeDisabled();
  });

  it('does not expose raw profile parameters or a Test button in the player dock', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    render(<PlayerTools useStore={store} />);

    expect(screen.queryByRole('button', { name: 'Test playback (no input)' })).toBeNull();
    expect(screen.queryByLabelText('Base Hold')).toBeNull();
    await act(async () =>
      screen.getByRole('button', { name: 'Configure playback profile' }).click(),
    );
    const dialog = screen.getByRole('dialog', { name: 'Playback profile' });
    const advancedTiming = within(dialog).getByText('Advanced timing');
    expect(advancedTiming.closest('details')).not.toHaveAttribute('open');
    expect(within(dialog).getByLabelText('FPS')).not.toBeVisible();
    await act(async () => advancedTiming.closest('summary')?.click());
    expect(within(dialog).getByLabelText('FPS')).toBeVisible();
    expect(within(dialog).getByText('Calculated target hold')).toBeVisible();
    expect(within(dialog).getByText('Safe preview')).toBeVisible();
    expect(
      within(dialog).getByText(
        'Select a song to enable a dry-run preview. No keyboard input will be sent.',
      ),
    ).toBeVisible();
    expect(within(dialog).getByRole('button', { name: 'Test playback (no input)' })).toBeVisible();
  });
});
