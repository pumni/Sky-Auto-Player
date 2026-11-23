import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { DiagnosticsTimingPlot } from './DiagnosticsTimingPlot';

afterEach(cleanup);

describe('DiagnosticsTimingPlot', () => {
  it('reports dry-run timing as not measured instead of fabricating sender samples', () => {
    render(<DiagnosticsTimingPlot samples={[]} hasActiveSession dryRun />);

    expect(screen.getByRole('status')).toHaveTextContent('No physical timing samples in dry run');
    expect(screen.getByRole('status')).toHaveTextContent(
      'Dry-run playback sends no physical input, so sender timing is not measured.',
    );
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('keeps the timing plot unavailable until native session data is available', () => {
    render(<DiagnosticsTimingPlot samples={[]} hasActiveSession={false} dryRun={false} />);

    expect(screen.getByRole('status')).toHaveTextContent('No active playback session');
    expect(screen.queryByRole('img')).toBeNull();
  });

  it('renders Y-axis scale labels, gridlines, and a latest-sample marker when samples exist', () => {
    const sample = {
      seq: 1,
      physical_session: true,
      player_attached: true,
      sender_sample_count: 5,
      max_lateness_us: 600,
      p50_ms: 0.4,
      p95_ms: 1.1,
      sigma_onset_ms: 0.2,
      late_2ms: 0,
      late_5ms: 0,
      late_10ms: 0,
      max_sendinput_pre_call_lateness_us: 360,
      pre_call_late_2ms: 0,
      pre_call_late_5ms: 0,
      pre_call_late_10ms: 0,
      fps: 60,
      frame_us: 16_667,
      hold_frames: 1,
      frame_base_hold_us: 16_667,
      timing_margin_us: 800,
      min_hold_us: 17_467,
      timing_margin_recommendation: {
        recommended_timing_margin_us: 800,
        qualified: false,
        source: 'default_fallback' as const,
      },
      pre_call_lt_250us: 2,
      pre_call_250_500us: 3,
      pre_call_500_750us: 0,
      pre_call_750_1000us: 0,
      pre_call_1000_1500us: 0,
      pre_call_1500_2000us: 0,
      pre_call_ge_2000us: 0,
      active_keys: 0,
      stuck_keys: 0,
      keys_dropped: 0,
      chord_split_events: 0,
      missed_down_boundaries: 0,
      missed_down_keys: 0,
      missed_unobserved_backlog_boundaries: 0,
      missed_physical_window_boundaries: 0,
      final_sender_window_expirations: 0,
      hold_floor_delay_boundaries: 0,
      max_hold_floor_delay_us: 0,
      final_gate_control_rejections: 0,
      final_gate_target_changes: 0,
      final_gate_focus_losses: 0,
      final_gate_lease_expirations: 0,
      sendinput_partial_events: 0,
      sendinput_zero_progress_failures: 0,
      backend_status: 'healthy' as const,
      release_max_us: 180,
      release_late_2ms: 0,
      session_id: 'a'.repeat(32),
      last_error: null,
      power_request_created: true,
      power_request_active: true,
      power_request_create_failures: 0,
      power_request_set_failures: 0,
      power_request_clear_failures: 0,
      power_request_close_failures: 0,
      suspend_resume_registered: true,
      suspend_resume_registration_failures: 0,
      suspend_resume_unregistration_failures: 0,
      system_suspend_active: false,
      system_suspend_notifications: 0,
      system_resume_notifications: 0,
      duplicate_system_power_notifications: 0,
    };
    const { container } = render(
      <DiagnosticsTimingPlot
        samples={[sample, { ...sample, seq: 2, max_sendinput_pre_call_lateness_us: 480 }]}
        hasActiveSession
        dryRun={false}
      />,
    );

    expect(screen.getByRole('img')).toBeInTheDocument();
    expect(container.querySelectorAll('.plot-grid-line')).toHaveLength(2);
    expect(container.querySelector('.plot-zero-axis')).not.toBeNull();
    expect(container.querySelector('.plot-latest-dot')).not.toBeNull();
    expect(screen.getByText('500 µs')).toBeInTheDocument();
    expect(screen.getByText('250 µs')).toBeInTheDocument();
  });
});
