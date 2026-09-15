import { describe, expect, it } from 'vitest';
import type { UiEvent } from './DesktopBridge';
import { assertMissReasonAccounting } from './diagnosticsTestUtils';
import { createMockBridge } from './mockBridge';

describe('mock bridge timing contracts', () => {
  it('emits diagnostics with mutually exclusive miss-reason accounting', async () => {
    const bridge = createMockBridge();
    const events: UiEvent[] = [];
    const unsubscribe = await bridge.subscribeUiEvents((event) => events.push(event));

    await bridge.setDiagnosticsEnabled({ enabled: true });

    const event = events.find((candidate) => candidate.name === 'diagnostics.snapshot');
    expect(event).toBeDefined();
    if (!event || event.name !== 'diagnostics.snapshot') throw new Error('missing diagnostics');
    assertMissReasonAccounting(event.payload);
    expect(event.payload.missed_down_boundaries).toBe(2);
    await bridge.setDiagnosticsEnabled({ enabled: false });
    unsubscribe();
  });

  it('rejects impossible Down-key accounting', () => {
    expect(() =>
      assertMissReasonAccounting({
        missed_down_boundaries: 2,
        missed_down_keys: 1,
        missed_unobserved_backlog_boundaries: 0,
        missed_physical_window_boundaries: 1,
        final_sender_window_expirations: 1,
      }),
    ).toThrow('missed Down key accounting drift');
  });

  it('exports the current sender-trace envelope and timing-policy schemas', async () => {
    const bridge = createMockBridge();
    const trace = JSON.parse(await bridge.exportSenderTrace()) as {
      export_schema_version: number;
      session_id: string;
      song_id: string;
      timing_policy: {
        fps: number;
        frame_period_us: number;
        hold_frames: number;
        frame_base_hold_us: number;
        timing_margin_us: number;
        target_hold_us: number;
        release_gap_us: number;
      };
      telemetry: { schema_version: number };
    };

    expect(trace.export_schema_version).toBe(2);
    expect(trace).toMatchObject({
      session_id: 'a'.repeat(32),
      song_id: 'fixture-song',
      timing_policy: {
        fps: 60,
        frame_period_us: 16_667,
        hold_frames: 1,
        frame_base_hold_us: 16_667,
        timing_margin_us: 500,
        target_hold_us: 17_167,
        release_gap_us: 17_167,
      },
    });
    expect(trace.telemetry.schema_version).toBe(16);
  });
});
