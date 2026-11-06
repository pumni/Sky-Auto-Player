import { afterEach, describe, expect, it } from 'vitest';
import type { UiEvent } from './DesktopBridge';
import { assertMissReasonAccounting } from './diagnosticsTestUtils';
import { createMockBridge } from './mockBridge';

const initialMockBridgeTestUrl = window.location.href;

afterEach(() => {
  window.history.replaceState(null, '', initialMockBridgeTestUrl);
});

describe('mock bridge timing contracts', () => {
  it('defaults fresh mock settings to System and keeps the dark palette independent', async () => {
    const bridge = createMockBridge();
    expect((await bridge.getSettings()).appearance_mode).toBe('system');

    await bridge.patchSettings({ appearanceMode: 'light' });
    await bridge.patchSettings({ palette: 'slate' });
    expect(await bridge.getSettings()).toMatchObject({
      appearance_mode: 'light',
      palette: 'slate',
    });

    const migratedLegacy = createMockBridge({ appearanceMode: 'dark' });
    expect((await migratedLegacy.getSettings()).appearance_mode).toBe('dark');
  });

  it('loads URL release-note fixtures through the browser mock', async () => {
    window.history.replaceState(null, '', '/?mockReleaseNotes=maximum');
    const bridge = createMockBridge();
    const events: UiEvent[] = [];
    const unsubscribe = await bridge.subscribeUiEvents((event) => events.push(event));

    await bridge.checkForUpdate({ origin: 'manual' });

    const update = events.find((event) => event.name === 'update.changed');
    expect(update?.name).toBe('update.changed');
    if (update?.name !== 'update.changed') throw new Error('missing update fixture event');
    expect(update.payload.state).toBe('available');
    expect(update.payload.release_notes).toHaveLength(16_384);
    expect(update.payload.release_notes).toContain('FINAL RELEASE NOTE LINE — Hoàn tất cập nhật.');
    unsubscribe();
  });

  it('prefers an explicit update snapshot over a URL release-note fixture', async () => {
    window.history.replaceState(null, '', '/?mockReleaseNotes=maximum');
    const bridge = createMockBridge({
      updateSnapshot: {
        state: 'current',
        current_version: '4.8.0-mock',
        available_version: null,
        release_notes: 'Explicit caller fixture.',
      },
    });
    const events: UiEvent[] = [];
    const unsubscribe = await bridge.subscribeUiEvents((event) => events.push(event));

    await bridge.checkForUpdate({ origin: 'manual' });

    const update = events.find((event) => event.name === 'update.changed');
    expect(update?.name).toBe('update.changed');
    if (update?.name !== 'update.changed') throw new Error('missing explicit update event');
    expect(update.payload).toMatchObject({
      state: 'current',
      current_version: '4.8.0-mock',
      release_notes: 'Explicit caller fixture.',
    });
    unsubscribe();
  });

  it('prefers an explicit transport error over a URL error fixture', async () => {
    window.history.replaceState(null, '', '/?mockUpdateState=error-long');
    const bridge = createMockBridge({
      updateCheckTransportError: 'Explicit mock transport error.',
    });

    await expect(bridge.checkForUpdate({ origin: 'manual' })).rejects.toThrow(
      'Explicit mock transport error.',
    );
  });

  it('keeps the default update fixture with no fixture query', async () => {
    window.history.replaceState(null, '', '/');
    const bridge = createMockBridge();
    const events: UiEvent[] = [];
    const unsubscribe = await bridge.subscribeUiEvents((event) => events.push(event));

    await bridge.checkForUpdate({ origin: 'manual' });

    const update = events.find((event) => event.name === 'update.changed');
    expect(update?.name).toBe('update.changed');
    if (update?.name !== 'update.changed') throw new Error('missing default update event');
    expect(update.payload.release_notes).toBe('A deterministic update fixture for the desktop UI.');
    unsubscribe();
  });

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
    expect(trace.telemetry.schema_version).toBe(19);
  });
});
