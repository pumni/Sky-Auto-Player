import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore, selectRowAtIndex } from '../../state/store';
import { PlaybackAdmission } from './PlaybackAdmission';
import { PlayerTransport } from './PlayerTransport';

async function setupPlaying(emitSnapshots = true) {
  const bridge = createMockBridge({
    playbackDurationMs: 60_000,
    startDelayMs: 5,
    emitSnapshots,
  });
  const store = createDesktopStore(bridge);
  await act(async () => store.getState().initialize());
  const row = selectRowAtIndex(store.getState().library, 0);
  if (!row) throw new Error('mock Library is empty');
  await act(async () => store.getState().selectSong(row.song_id));
  await act(async () => store.getState().prepareSelectedPlayback());
  await act(async () => store.getState().startPreparedPlayback('proceed'));
  await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
  return { bridge, store };
}

describe('PlaybackAdmission recovery actions', () => {
  afterEach(cleanup);

  it('shows the confirmation warning with nominal slack advisory, Proceed, and Dry Run while preserving settings', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    const { container } = render(<PlaybackAdmission useStore={store} />);
    const prepared = store.getState().playback.prepared;
    if (!prepared) throw new Error('expected a prepared playback');
    const reason = container.querySelector('#playback-confirmation-reason');
    const consequence = container.querySelector('#playback-confirmation-consequence');
    expect(reason).toHaveTextContent(prepared.risk.reasons.join(' '));
    expect(consequence).toHaveTextContent(
      'This warning estimates how accurately the game may register notes. Choose an option below or cancel without starting.',
    );
    expect(screen.getByText('Uses your current timing settings unchanged.')).toHaveAttribute(
      'id',
      'playback-decision-description-proceed',
    );
    expect(screen.getByText('Runs in Test mode without sending keyboard input.')).toHaveAttribute(
      'id',
      'playback-decision-description-dry_run',
    );
    expect(screen.getByRole('button', { name: 'Not now' })).toBeEnabled();
    const estimates = screen.getAllByText('Timing estimate (min slack 0 µs)', {
      selector: 'summary',
    });
    expect(estimates).toHaveLength(2);
    estimates.forEach((summary) => expect(summary).toBeVisible());
    expect(
      container.querySelector('[data-plan-fingerprint="mock-plan"]')?.getAttribute('data-status'),
    ).toBe('analyzed');
    expect(container.querySelector('[data-plan-fingerprint="mock-plan"]')).toHaveTextContent(
      'Same-key repeats: 2 · min nominal slack (I - H): 0 µs · zero-slack (I = H): 1 · minimum hold H: 17467 µs',
    );
    expect(container.querySelector('[data-plan-fingerprint="mock-plan"]')).toHaveTextContent(
      'Bounds: 50000 generations, 3000000 work units, and 64 MiB working memory.',
    );
    expect(container.querySelector('[data-plan-fingerprint="mock-plan"]')).toHaveTextContent(
      'independent component maxima:',
    );
    expect(screen.getAllByRole('button')).toHaveLength(3);
    const proceedButton = screen.getByRole('button', { name: 'Proceed with current settings' });
    const dryRunButton = screen.getByRole('button', { name: 'Test playback (no input)' });
    expect(proceedButton).toBeEnabled();
    expect(proceedButton).toHaveAttribute(
      'aria-describedby',
      'playback-decision-description-proceed',
    );
    expect(screen.queryByRole('button', { name: /Use recommended/i })).not.toBeInTheDocument();
    expect(dryRunButton).toBeEnabled();
    expect(dryRunButton).toHaveAttribute(
      'aria-describedby',
      'playback-decision-description-dry_run',
    );
  });

  it('keeps Proceed and Dry Run decisions compact and lets Not now dismiss them', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const playback = store.getState().playback;
    const prepared = playback.prepared;
    if (prepared?.admission !== 'confirmation_required') {
      throw new Error('expected a prepared playback requiring confirmation');
    }
    expect(prepared.decisions.map((decision) => decision.decision)).toEqual(['proceed', 'dry_run']);
    expect(prepared.variants.map((variant) => variant.decision)).toEqual(['proceed', 'dry_run']);

    render(<PlaybackAdmission useStore={store} />);
    expect(
      screen.getAllByRole('button', {
        name: /^(Proceed with current settings|Test playback \(no input\))$/,
      }),
    ).toHaveLength(2);
    expect(screen.getByText(prepared.risk.headline)).toBeVisible();
    expect(screen.getByText(prepared.risk.reasons[0]!)).toBeVisible();
    expect(screen.getByRole('button', { name: 'Proceed with current settings' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Test playback (no input)' })).toBeEnabled();
    const estimates = screen.getAllByText('Timing estimate (min slack 0 µs)', {
      selector: 'summary',
    });
    expect(estimates).toHaveLength(2);
    for (const summary of estimates) {
      const collapse = summary.closest('details');
      expect(collapse).not.toBeNull();
      expect(collapse).not.toHaveAttribute('open');
    }
    fireEvent.click(screen.getByRole('button', { name: 'Not now' }));
    expect(store.getState().playback.prepared).toBeNull();
    expect(screen.queryByRole('group', { name: 'Playback confirmation' })).toBeNull();
  });

  it('keeps an independent timing recommendation available for its separate risk reason', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const prepared = store.getState().playback.prepared;
    if (!prepared) throw new Error('expected a prepared playback');

    const proceedVariant = prepared.variants.find((variant) => variant.decision === 'proceed');
    if (!proceedVariant) throw new Error('expected a Proceed plan variant');
    const timingAdvisory = proceedVariant.timing_advisory;
    if (!timingAdvisory) throw new Error('expected a Proceed timing advisory');
    const independentRiskVariant = {
      ...proceedVariant,
      timing_advisory: {
        ...timingAdvisory,
        status: 'analyzed' as const,
        explanation: '2 same-key repeat(s); minimum nominal scheduling slack (I - H) is 500 µs.',
        min_same_key_slack_us: 500,
        zero_slack_repeat_count: 0,
      },
    };
    store.setState((state) => ({
      ...state,
      playback: {
        ...state.playback,
        prepared: {
          ...prepared,
          risk: {
            ...prepared.risk,
            reasons: ['Dense transitions may need a slower tempo.'],
            recommendations: ['Try 0.9× tempo.'],
            min_same_key_slack_us: 500,
            same_key_repeat_count: 2,
            zero_slack_repeat_count: 0,
          },
          decisions: [
            { decision: 'proceed', label: 'Proceed with current settings' },
            { decision: 'use_recommended', label: 'Use recommended settings' },
          ],
          variants: [
            independentRiskVariant,
            {
              ...independentRiskVariant,
              decision: 'use_recommended',
              plan_fingerprint: 'mock-recommended',
            },
          ],
        },
      },
    }));

    render(<PlaybackAdmission useStore={store} />);
    expect(screen.getByText('Dense transitions may need a slower tempo.')).toBeVisible();
    const recommendedButton = screen.getByRole('button', { name: 'Use recommended settings' });
    await waitFor(() => expect(recommendedButton).toHaveFocus());
    expect(recommendedButton).toHaveAttribute(
      'aria-describedby',
      'playback-decision-description-use_recommended',
    );
    expect(
      screen.getByText('Applies the timing settings recommended for this session.'),
    ).toHaveAttribute('id', 'playback-decision-description-use_recommended');
    expect(screen.getByRole('button', { name: 'Proceed with current settings' })).toBeEnabled();
    expect(
      screen.queryByRole('button', { name: 'Test playback (no input)' }),
    ).not.toBeInTheDocument();
  });

  it('preserves native 3-option order, recommended primary/focus, descriptions, and command dispatch', async () => {
    const bridge = createMockBridge({ startDelayMs: 5 });
    const startedDecisions: string[] = [];
    const originalStart = bridge.startPlayback;
    bridge.startPlayback = async (request) => {
      startedDecisions.push(...request.decisions.map((item) => item.decision));
      return originalStart(request);
    };
    const store = createDesktopStore(bridge);
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());
    const prepared = store.getState().playback.prepared;
    if (!prepared) throw new Error('expected a prepared playback');
    const proceedVariant = prepared.variants.find((variant) => variant.decision === 'proceed');
    const dryRunVariant = prepared.variants.find((variant) => variant.decision === 'dry_run');
    if (!proceedVariant || !dryRunVariant) throw new Error('expected proceed and dry_run variants');

    store.setState((state) => ({
      ...state,
      playback: {
        ...state.playback,
        prepared: {
          ...prepared,
          decisions: [
            { decision: 'proceed', label: 'Proceed with current settings' },
            { decision: 'use_recommended', label: 'Use recommended settings' },
            { decision: 'dry_run', label: 'Run a dry-run first' },
          ],
          variants: [
            proceedVariant,
            {
              ...proceedVariant,
              decision: 'use_recommended',
              plan_fingerprint: 'mock-recommended-physical',
            },
            dryRunVariant,
          ],
        },
      },
    }));

    const { container } = render(<PlaybackAdmission useStore={store} />);
    const optionButtons = Array.from(
      container.querySelectorAll<HTMLButtonElement>('[data-slot="admission-option"] button'),
    );
    expect(optionButtons.map((button) => button.textContent)).toEqual([
      'Proceed with current settings',
      'Use recommended settings',
      'Test playback (no input)',
    ]);
    await waitFor(() => expect(optionButtons[1]).toHaveFocus());
    expect(optionButtons[0]).toHaveAttribute(
      'aria-describedby',
      'playback-decision-description-proceed',
    );
    expect(optionButtons[1]).toHaveAttribute(
      'aria-describedby',
      'playback-decision-description-use_recommended',
    );
    expect(optionButtons[2]).toHaveAttribute(
      'aria-describedby',
      'playback-decision-description-dry_run',
    );

    fireEvent.click(optionButtons[1]!);
    await waitFor(() => expect(startedDecisions).toEqual(['use_recommended']));
  });

  it('keeps ready-plan timing data without flashing an advisory before Play', async () => {
    const store = createDesktopStore(createMockBridge({ readyPlanAdvisoryStatus: 'analyzed' }));
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 1);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    await act(async () => store.getState().prepareSelectedPlayback());

    expect(store.getState().playback.prepared?.admission).toBe('ready');
    render(
      <>
        <PlayerTransport useStore={store} />
        <PlaybackAdmission useStore={store} />
      </>,
    );
    const readyAdvisory = store
      .getState()
      .playback.prepared?.variants.find(
        (variant) => variant.decision === 'proceed',
      )?.timing_advisory;
    expect(readyAdvisory?.status).toBe('analyzed');
    expect(readyAdvisory?.plan_fingerprint).toBe('mock-plan');
    expect(screen.queryByLabelText('Prepared playback timing advisory')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Play' })).toBeEnabled();

    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Play' }));
    });
    await waitFor(() => expect(store.getState().playback.state).toBe('playing'));
    expect(screen.queryByLabelText('Prepared playback timing advisory')).not.toBeInTheDocument();
    expect(screen.queryByRole('group', { name: 'Playback confirmation' })).not.toBeInTheDocument();
  });

  it('reconciles a recoverable status issue and restores the normal Pause control', async () => {
    const { store } = await setupPlaying();
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Mock playback status query unavailable.',
        },
      }),
    );

    render(
      <>
        <PlayerTransport useStore={store} />
        <PlaybackAdmission useStore={store} />
      </>,
    );
    expect(screen.getByRole('alert')).toHaveTextContent('Playback status is unavailable');
    expect(screen.getByRole('button', { name: 'Retry status' })).toBeEnabled();
    expect(screen.getByRole('button', { name: 'Stop playback' })).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Retry status' }));

    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
    expect(screen.getByRole('button', { name: 'Pause' })).toBeEnabled();
    expect(store.getState().playback.error).toBeNull();
  });

  it('keeps a known session owned after status query failure and offers Stop playback', async () => {
    const { bridge, store } = await setupPlaying(false);
    bridge.getPlaybackStatus = async () => {
      throw new Error('Mock IPC disconnected.');
    };
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          error: 'Playback status is unavailable: Previous status query failed.',
        },
      }),
    );
    render(<PlaybackAdmission useStore={store} />);

    fireEvent.click(screen.getByRole('button', { name: 'Retry status' }));
    await waitFor(() => expect(store.getState().playback.statusRetryPending).toBe(false));
    expect(store.getState().playback.sessionId).not.toBeNull();
    expect(store.getState().playback.error).toContain('Mock IPC disconnected');

    fireEvent.click(screen.getByRole('button', { name: 'Stop playback' }));
    await waitFor(() => expect(store.getState().playback.sessionId).toBeNull());
  });

  it('offers status retry without Stop or Try again when native ownership is unknown', async () => {
    const store = createDesktopStore(createMockBridge());
    await act(async () => store.getState().initialize());
    const row = selectRowAtIndex(store.getState().library, 0);
    if (!row) throw new Error('mock Library is empty');
    await act(async () => store.getState().selectSong(row.song_id));
    act(() =>
      store.setState({
        playback: {
          ...store.getState().playback,
          startRequestId: 42,
          transportOperation: null,
          error: 'Playback status is unavailable: Mock IPC disconnected.',
        },
      }),
    );

    render(<PlaybackAdmission useStore={store} />);
    expect(screen.getByRole('button', { name: 'Retry status' })).toBeEnabled();
    expect(screen.queryByRole('button', { name: 'Stop playback' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Try again' })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Retry status' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled());
    expect(store.getState().playback.startRequestId).toBeNull();
    expect(store.getState().playback.error).toContain('No active playback session was created');
  });

  it.each([
    [
      'target_not_found',
      'Sky window was not found',
      'Open Sky and make sure its window is visible, then try again.',
    ],
    [
      'target_integrity_mismatch',
      'Sky is running with different permissions',
      'Run both applications at the same permission level.',
    ],
    ['target_focus_failed', 'Sky could not be focused', 'Focus the Sky window and try again.'],
  ])(
    'keeps %s actionable without showing a misleading Stop action',
    async (code, title, message) => {
      const store = createDesktopStore(createMockBridge({ startFailure: { code, message } }));
      await act(async () => store.getState().initialize());
      const row = selectRowAtIndex(store.getState().library, 1);
      if (!row) throw new Error('mock Library is empty');
      await act(async () => store.getState().selectSong(row.song_id));
      await act(async () => store.getState().prepareSelectedPlayback());
      await act(async () => store.getState().startPreparedPlayback());

      render(<PlaybackAdmission useStore={store} />);
      expect(screen.getByRole('alert')).toHaveTextContent(title);
      expect(screen.getByText(message)).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Try again' })).toBeEnabled();
      expect(screen.queryByRole('button', { name: 'Stop playback' })).not.toBeInTheDocument();
    },
  );
});
