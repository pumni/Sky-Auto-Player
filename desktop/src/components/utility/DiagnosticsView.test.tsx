import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { DiagnosticsSnapshot } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { DiagnosticsView } from './DiagnosticsView';

function snapshot(overrides: Partial<DiagnosticsSnapshot> = {}): DiagnosticsSnapshot {
  return {
    seq: 1,
    physical_session: true,
    player_attached: true,
    sender_sample_count: 1,
    max_lateness_us: 850,
    p50_ms: 0.4,
    p95_ms: 1.1,
    sigma_onset_ms: 0.2,
    late_2ms: 4,
    late_5ms: 2,
    late_10ms: 1,
    max_sendinput_pre_call_lateness_us: 320,
    pre_call_late_2ms: 3,
    pre_call_late_5ms: 2,
    pre_call_late_10ms: 1,
    down_late_grace_us: 500,
    fps: 60,
    frame_us: 16_667,
    hold_frames: 1,
    frame_base_hold_us: 16_667,
    timing_margin_us: 800,
    min_hold_us: 17_467,
    min_release_gap_us: 17_467,
    timing_margin_recommendation: {
      recommended_timing_margin_us: 800,
      qualified: false,
      source: 'default_fallback',
    },
    pre_call_lt_250us: 0,
    pre_call_250_500us: 0,
    pre_call_500_750us: 1,
    pre_call_750_1000us: 0,
    pre_call_1000_1500us: 0,
    pre_call_1500_2000us: 0,
    pre_call_ge_2000us: 0,
    active_keys: 3,
    stuck_keys: 1,
    keys_dropped: 5,
    chord_split_events: 2,
    missed_down_boundaries: 0,
    missed_down_keys: 0,
    missed_backlog_boundaries: 0,
    missed_hard_late_boundaries: 0,
    final_gate_cutoff_misses: 0,
    final_gate_control_rejections: 0,
    final_gate_target_changes: 0,
    final_gate_focus_losses: 0,
    final_gate_lease_expirations: 0,
    sendinput_partial_events: 0,
    sendinput_zero_progress_failures: 0,
    backend_status: 'healthy',
    release_max_us: 420,
    release_late_2ms: 1,
    session_id: 'a'.repeat(32),
    last_error: null,
    ...overrides,
  };
}

describe('DiagnosticsView', () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it('distinguishes enabled diagnostics with no active session', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      diagnostics: { ...store.getState().diagnostics, enabled: true },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('No active playback session')).toBeInTheDocument();
    expect(
      screen.getByText('Runtime timing and input-health metrics appear when playback starts.'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Completion p50')).toBeNull();
  });

  it('uses boundary-accurate labels and sender-side timing metrics', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'a'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            release_late_2ms: 0,
            missed_down_boundaries: 2,
            missed_down_keys: 3,
            missed_backlog_boundaries: 1,
            missed_hard_late_boundaries: 2,
            final_gate_cutoff_misses: 2,
            final_gate_control_rejections: 1,
            final_gate_target_changes: 1,
            final_gate_focus_losses: 1,
            final_gate_lease_expirations: 1,
            sendinput_partial_events: 1,
            sendinput_zero_progress_failures: 0,
          }),
        ],
      },
      playback: {
        ...store.getState().playback,
        sessionId,
        state: 'playing',
      },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Completion p50')).toBeInTheDocument();
    expect(screen.getByText('Completion p95')).toBeInTheDocument();
    expect(screen.getByText('Completion jitter σ')).toBeInTheDocument();
    expect(screen.getByText('Session max')).toBeInTheDocument();
    expect(screen.getByText('Max pre-call lateness')).toBeInTheDocument();
    expect(screen.getByText('Pre-call > 10 ms')).toBeInTheDocument();
    expect(screen.getByText('Pre-call 500–750 μs')).toBeInTheDocument();
    expect(screen.getByText('Late Down tolerance')).toBeInTheDocument();
    expect(screen.getByText('Configured Timing Margin').parentElement).toHaveTextContent('800 µs');
    expect(screen.getByText('Target hold').parentElement).toHaveTextContent('17.467 ms');
    expect(screen.getByText('Release gap').parentElement).toHaveTextContent('17.467 ms');
    expect(screen.getByText('Hard-late Down boundaries')).toBeInTheDocument();
    expect(screen.getByText('Missed Down keys')).toBeInTheDocument();
    expect(screen.getByText('Backlog misses')).toBeInTheDocument();
    expect(screen.getByText('Focus gate rejections')).toBeInTheDocument();
    expect(screen.getByText('Target changes')).toBeInTheDocument();
    expect(screen.getByText('Lease expirations')).toBeInTheDocument();
    expect(screen.getByText('Control rejections')).toBeInTheDocument();
    expect(screen.getByText('SendInput zero-progress failures')).toBeInTheDocument();
    expect(screen.getByText('SendInput partial events')).toBeInTheDocument();
    expect(screen.getByText('Dropped keys')).toBeInTheDocument();
    expect(screen.getByText('Stuck keys')).toBeInTheDocument();
    expect(screen.getByText(/Sender-side status: Attention/)).toBeInTheDocument();
    expect(screen.getByText('Release > 2 ms').parentElement).toHaveTextContent('0');
    expect(screen.queryByText('P50')).toBeNull();
    expect(screen.queryByText('Sigma')).toBeNull();

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByRole('img', { name: /SendInput pre-call lateness/ })).toBeVisible();
    expect(
      screen.getByText(
        /Session max pre-call lateness observed at the latest diagnostics snapshot: 320 μs/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/This cumulative value does not decrease after recovery/),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText(/fixed Down late cutoff applies only to Down-bearing sends/),
    ).toHaveLength(2);
    expect(screen.getByText('Late Down tolerance 500 μs')).toBeInTheDocument();
  });

  it('shows the frozen user margin separately from the Late Down tolerance', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'd'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            timing_margin_us: 0,
            min_hold_us: 16_667,
            min_release_gap_us: 16_667,
            down_late_grace_us: 500,
            timing_margin_recommendation: {
              recommended_timing_margin_us: 800,
              qualified: false,
              source: 'default_fallback',
            },
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Configured Timing Margin').parentElement).toHaveTextContent('0 µs');
    expect(screen.getByText('Target hold').parentElement).toHaveTextContent('16.667 ms');
    expect(screen.getByText('Release gap').parentElement).toHaveTextContent('16.667 ms');
    expect(screen.getByText('Late Down tolerance').parentElement).toHaveTextContent('500 µs');
    expect(screen.getByText('Recommended sender margin').parentElement).toHaveTextContent('800 µs');
    expect(screen.getByText('Recommendation source').parentElement).toHaveTextContent(
      'Default fallback (no valid calibration cache)',
    );
  });

  it('does not present unavailable distribution metrics as zero', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'b'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            max_lateness_us: null,
            p50_ms: null,
            p95_ms: null,
            sigma_onset_ms: null,
            late_2ms: null,
            late_5ms: null,
            late_10ms: null,
            physical_session: false,
            player_attached: false,
            sender_sample_count: 0,
            max_sendinput_pre_call_lateness_us: null,
            pre_call_late_2ms: 0,
            pre_call_late_5ms: 0,
            pre_call_late_10ms: 0,
            release_max_us: null,
            release_late_2ms: null,
            backend_status: 'unavailable',
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getAllByText('Unavailable')).not.toHaveLength(0);
    expect(screen.queryByText('0.00 ms')).toBeNull();
    expect(screen.getByText('Max pre-call lateness').parentElement).toHaveTextContent(
      'Unavailable',
    );
    expect(screen.getByText('Pre-call > 2 ms').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Dropped keys').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Sender backend').parentElement).toHaveTextContent('Unavailable');
    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByText('Timing unavailable')).toBeInTheDocument();
    expect(screen.queryByRole('img', { name: /SendInput pre-call lateness/ })).toBeNull();
    expect(screen.queryByText('No session')).toBeNull();
  });

  it('keeps production pre-call metrics separate from unavailable observer metrics', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'p'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            max_lateness_us: null,
            p50_ms: null,
            p95_ms: null,
            sigma_onset_ms: null,
            late_2ms: null,
            late_5ms: null,
            late_10ms: null,
            max_sendinput_pre_call_lateness_us: 327,
            pre_call_late_2ms: 4,
            pre_call_late_5ms: 2,
            pre_call_late_10ms: 1,
            release_max_us: null,
            release_late_2ms: null,
            backend_status: 'healthy',
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Healthy')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Deadline admission' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Input transport' })).toBeInTheDocument();
    expect(screen.getByText('Max pre-call lateness').parentElement).toHaveTextContent('327 μs');
    expect(screen.getByText('Pre-call > 2 ms').parentElement).toHaveTextContent('4');
    expect(screen.getByText('Pre-call > 5 ms').parentElement).toHaveTextContent('2');
    expect(screen.getByText('Pre-call > 10 ms').parentElement).toHaveTextContent('1');
    expect(screen.getByText('Session max').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Completion > 2 ms').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Max release lateness').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Release > 2 ms').parentElement).toHaveTextContent('Unavailable');
    expect(screen.queryByText('0 μs')).toBeNull();

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByRole('img', { name: /SendInput pre-call lateness/ })).toBeVisible();
    expect(
      screen.getByText(
        /Session max pre-call lateness observed at the latest diagnostics snapshot: 327 μs/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText('Late Down tolerance 500 μs')).toBeInTheDocument();
  });

  it('distinguishes an attached physical player with no sender samples yet', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'n'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            sender_sample_count: 0,
            max_sendinput_pre_call_lateness_us: null,
            pre_call_lt_250us: 0,
            pre_call_500_750us: 0,
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Sender-side status: Waiting')).toBeInTheDocument();
    expect(screen.getByText('Physical session').parentElement).toHaveTextContent('Yes');
    expect(screen.getByText('Player attached').parentElement).toHaveTextContent('Yes');
    expect(screen.getByText('Sender samples').parentElement).toHaveTextContent('No samples');
    expect(screen.getByText('Max pre-call lateness').parentElement).toHaveTextContent('No samples');
    expect(screen.getByText('Pre-call < 250 μs').parentElement).toHaveTextContent('No samples');

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByText('No sender samples yet')).toBeInTheDocument();
    expect(screen.queryByRole('img', { name: /SendInput pre-call lateness/ })).toBeNull();
  });

  it('keeps backend-unavailable distinct from an attached player with no samples', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'u'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            physical_session: true,
            player_attached: true,
            sender_sample_count: 0,
            max_sendinput_pre_call_lateness_us: null,
            backend_status: 'unavailable',
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Sender-side status: Unavailable')).toBeInTheDocument();
    expect(screen.getByText('Sender backend').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Sender samples').parentElement).toHaveTextContent('Unavailable');

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByText('Timing unavailable')).toBeInTheDocument();
    expect(screen.queryByText('No sender samples yet')).toBeNull();
    expect(screen.queryByRole('img', { name: /SendInput pre-call lateness/ })).toBeNull();
  });

  it('reports a sampled zero-microsecond sender lateness as a real measurement', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'z'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            sender_sample_count: 1,
            max_sendinput_pre_call_lateness_us: 0,
            pre_call_lt_250us: 1,
            pre_call_500_750us: 0,
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Sender samples').parentElement).toHaveTextContent('1');
    expect(screen.getByText('Max pre-call lateness').parentElement).toHaveTextContent('0 μs');
    expect(screen.getByText('Pre-call < 250 μs').parentElement).toHaveTextContent('1');

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByRole('img', { name: /SendInput pre-call lateness/ })).toBeVisible();
    expect(
      screen.getByText(
        /Session max pre-call lateness observed at the latest diagnostics snapshot: 0 μs/,
      ),
    ).toBeInTheDocument();
  });

  it('reports a physical session without an attached player as an error', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'x'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            physical_session: true,
            player_attached: false,
            sender_sample_count: 0,
            max_sendinput_pre_call_lateness_us: null,
            backend_status: 'error',
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Sender-side status: Error')).toBeInTheDocument();
    expect(
      screen.getByText('The physical session is active, but no native player is attached.'),
    ).toBeInTheDocument();
    expect(screen.getByText('Physical session').parentElement).toHaveTextContent('Yes');
    expect(screen.getByText('Player attached').parentElement).toHaveTextContent('No');
    expect(screen.getByText('Sender backend').parentElement).toHaveTextContent('Error');
    for (const label of [
      'Missed Down boundaries',
      'Final cutoff misses',
      'SendInput partial events',
      'Dropped keys',
      'Active keys',
      'Release > 2 ms',
    ]) {
      expect(screen.getByText(label).nextElementSibling).toHaveTextContent('Unavailable');
    }
  });

  it('labels an exported trace with its song and owning session', async () => {
    vi.stubGlobal('URL', {
      createObjectURL: vi.fn(() => 'blob:sender-trace'),
      revokeObjectURL: vi.fn(),
    });
    vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {});
    const store = createDesktopStore(createMockBridge());
    render(<DiagnosticsView useStore={store} />);

    fireEvent.click(screen.getByRole('button', { name: 'Export last sender trace' }));

    expect(
      await screen.findByText(`Trace available: Song Fixture Song · Session ${'a'.repeat(32)}`),
    ).toBeInTheDocument();
    await act(async () => {
      store.setState({
        playback: { ...store.getState().playback, sessionId: 'b'.repeat(32), state: 'playing' },
      });
    });
    expect(
      screen.getByText('Available after a completed physical playback session.'),
    ).toBeInTheDocument();
  });

  it('uses playback lifecycle to hide a completed session', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'c'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [snapshot({ session_id: sessionId })],
      },
      playback: { ...store.getState().playback, sessionId, state: 'finished' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('No active playback session')).toBeInTheDocument();
    expect(screen.queryByText('Completion p50')).toBeNull();
  });

  it('keeps optional completion residuals and shows a zero reference line', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'd'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [snapshot({ session_id: sessionId, p95_ms: -1.5 })],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));

    expect(
      screen.getByText(
        /Session max pre-call lateness observed at the latest diagnostics snapshot: 320 μs/,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/Completion p95 observer value -1\.50 ms/)).toBeInTheDocument();
    expect(document.querySelector('.plot-zero-axis')).not.toBeNull();
  });

  it('uses backend severity precedence and renders the last error', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'e'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [
          snapshot({
            session_id: sessionId,
            backend_status: 'error',
            last_error: 'authored Down send integrity failure',
            missed_down_boundaries: 0,
            sendinput_partial_events: 0,
            sendinput_zero_progress_failures: 0,
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Sender-side status: Error')).toBeInTheDocument();
    expect(
      screen.getByText('Last error: authored Down send integrity failure'),
    ).toBeInTheDocument();
    expect(screen.queryByText('Sender-side status: Healthy')).toBeNull();
    expect(screen.getByText('Missed Down boundaries').nextElementSibling).toHaveTextContent('0');
  });

  it('does not report degraded backend health as healthy when counters are zero', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'f'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [snapshot({ session_id: sessionId, backend_status: 'degraded' })],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Sender-side status: Attention')).toBeInTheDocument();
    expect(screen.getByText('Sender-side backend reported degraded health.')).toBeInTheDocument();
    expect(screen.queryByText('Sender-side status: Healthy')).toBeNull();
  });

  it('removes the fabricated Logs view and timestamps human events', () => {
    const store = createDesktopStore(createMockBridge());
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        events: [
          {
            seq: 1,
            name: 'playback.state_changed',
            detail: 'playing',
            timestamp: Date.UTC(2026, 0, 1, 12, 34, 56),
          },
        ],
      },
    });

    render(<DiagnosticsView useStore={store} />);
    fireEvent.click(screen.getByRole('tab', { name: 'Events' }));

    expect(screen.queryByRole('tab', { name: 'Logs' })).toBeNull();
    expect(screen.getByText('playback.state_changed')).toBeInTheDocument();
    const time = screen.getByText('playback.state_changed').closest('li')?.querySelector('time');
    expect(time).not.toBeNull();
    expect(time).toHaveAttribute('datetime', '2026-01-01T12:34:56.000Z');
  });
});
