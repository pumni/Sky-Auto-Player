import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import type { DiagnosticsSnapshot } from '../../bridge/DesktopBridge';
import { createMockBridge } from '../../bridge/mockBridge';
import { createDesktopStore } from '../../state/store';
import { DiagnosticsView } from './DiagnosticsView';

function snapshot(overrides: Partial<DiagnosticsSnapshot> = {}): DiagnosticsSnapshot {
  return {
    seq: 1,
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
    active_keys: 3,
    stuck_keys: 1,
    keys_dropped: 5,
    chord_split_events: 2,
    backend_status: 'healthy',
    release_max_us: 420,
    release_late_2ms: 1,
    session_id: 'a'.repeat(32),
    ...overrides,
  };
}

describe('DiagnosticsView', () => {
  afterEach(() => cleanup());

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

  it('uses boundary-accurate labels and a recent completion metric for timing', () => {
    const store = createDesktopStore(createMockBridge());
    const sessionId = 'a'.repeat(32);
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [snapshot({ session_id: sessionId, release_late_2ms: 0 })],
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
    expect(screen.getByText('Dropped keys')).toBeInTheDocument();
    expect(screen.getByText('Stuck keys')).toBeInTheDocument();
    expect(screen.getByText('Healthy')).toBeInTheDocument();
    expect(screen.getByText('Release > 2 ms').parentElement).toHaveTextContent('0');
    expect(screen.queryByText('P50')).toBeNull();
    expect(screen.queryByText('Sigma')).toBeNull();

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByRole('img', { name: /Completion p95 residual/ })).toBeVisible();
    expect(screen.getByText(/Latest completion p95 residual 1\.10 ms/)).toBeInTheDocument();
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
            p50_ms: null,
            p95_ms: null,
            sigma_onset_ms: null,
            backend_status: 'unavailable',
          }),
        ],
      },
      playback: { ...store.getState().playback, sessionId, state: 'playing' },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getAllByText('Unavailable')).not.toHaveLength(0);
    expect(screen.queryByText('0.00 ms')).toBeNull();
    expect(document.querySelector('.diagnostics-backend-status.is-unavailable')).toHaveTextContent(
      'Unavailable',
    );
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
    expect(screen.getByText('Max pre-call lateness').parentElement).toHaveTextContent('327 μs');
    expect(screen.getByText('Pre-call > 2 ms').parentElement).toHaveTextContent('4');
    expect(screen.getByText('Pre-call > 5 ms').parentElement).toHaveTextContent('2');
    expect(screen.getByText('Pre-call > 10 ms').parentElement).toHaveTextContent('1');
    expect(screen.getByText('Session max').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Completion > 2 ms').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Max release lateness').parentElement).toHaveTextContent('Unavailable');
    expect(screen.getByText('Release > 2 ms').parentElement).toHaveTextContent('Unavailable');
    expect(screen.queryByText('0 μs')).toBeNull();
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

  it('keeps signed completion residuals and shows a zero reference line', () => {
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

    expect(screen.getByText(/Latest completion p95 residual -1\.50 ms/)).toBeInTheDocument();
    expect(document.querySelector('.plot-zero-axis')).not.toBeNull();
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
