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
    store.setState({
      diagnostics: {
        ...store.getState().diagnostics,
        enabled: true,
        samples: [snapshot()],
      },
    });

    render(<DiagnosticsView useStore={store} />);

    expect(screen.getByText('Completion p50')).toBeInTheDocument();
    expect(screen.getByText('Completion p95')).toBeInTheDocument();
    expect(screen.getByText('Completion jitter σ')).toBeInTheDocument();
    expect(screen.getByText('Session max')).toBeInTheDocument();
    expect(screen.getByText('Dropped keys')).toBeInTheDocument();
    expect(screen.getByText('Stuck keys')).toBeInTheDocument();
    expect(screen.getByText('Healthy')).toBeInTheDocument();
    expect(screen.queryByText('P50')).toBeNull();
    expect(screen.queryByText('Sigma')).toBeNull();

    fireEvent.click(screen.getByRole('tab', { name: 'Timing' }));
    expect(screen.getByRole('img', { name: /Completion p95 lateness/ })).toBeVisible();
    expect(screen.getByText(/Latest completion p95 1\.10 ms/)).toBeInTheDocument();
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
