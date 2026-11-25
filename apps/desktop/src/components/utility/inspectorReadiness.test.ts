import { describe, expect, it } from 'vitest';
import { resolveInspectorReadiness } from './inspectorReadiness';

const ready = {
  pending: false,
  detailUnavailable: false,
  sheetReadiness: 'ready' as const,
  conversionConfirmed: false,
  riskLevel: 'low',
};

describe('Inspector readiness summary', () => {
  it('keeps unknown and pending analysis from claiming the sheet is ready', () => {
    expect(resolveInspectorReadiness({ ...ready, pending: true }).state).toBe('pending');
    expect(resolveInspectorReadiness({ ...ready, sheetReadiness: null }).state).toBe('unavailable');
    expect(resolveInspectorReadiness({ ...ready, detailUnavailable: true }).state).toBe(
      'unavailable',
    );
    expect(resolveInspectorReadiness({ ...ready, riskLevel: null }).state).toBe('unavailable');
    expect(resolveInspectorReadiness({ ...ready, riskLevel: 'unknown' }).state).toBe('unavailable');
    expect(resolveInspectorReadiness({ ...ready, riskLevel: 'unexpected' }).state).toBe(
      'unavailable',
    );
  });

  it('gives a live pending check precedence over an unavailable detail error', () => {
    expect(
      resolveInspectorReadiness({ ...ready, pending: true, detailUnavailable: true }),
    ).toMatchObject({ state: 'pending', label: 'Checking' });
    expect(resolveInspectorReadiness({ ...ready, detailUnavailable: true })).toMatchObject({
      state: 'unavailable',
      label: 'Readiness unavailable',
    });
  });

  it.each(['malformed', 'unsupported', 'unplayable'] as const)(
    'reports %s sheets as blocked',
    (sheetReadiness) => {
      expect(resolveInspectorReadiness({ ...ready, sheetReadiness }).state).toBe('blocked');
    },
  );

  it('keeps conversion consent and timing review as distinct review reasons', () => {
    const conversion = resolveInspectorReadiness({
      ...ready,
      sheetReadiness: 'requires_confirmation',
    });
    expect(conversion.state).toBe('review');
    expect(conversion.detail).toMatch(/sheet conversion/);

    const timing = resolveInspectorReadiness({
      ...ready,
      sheetReadiness: 'physical_risk',
      riskLevel: 'high',
    });
    expect(timing.state).toBe('review');
    expect(timing.detail).toMatch(/Check the timing risk guidance below/);
    expect(timing.detail).not.toMatch(/conversion|focus/i);
  });

  it('clears conversion review only after the source reports consent', () => {
    expect(
      resolveInspectorReadiness({
        ...ready,
        sheetReadiness: 'requires_confirmation',
        conversionConfirmed: true,
      }).state,
    ).toBe('ready');
  });

  it('does not imply native target or focus qualification', () => {
    const result = resolveInspectorReadiness(ready);
    expect(result.state).toBe('ready');
    expect(result.detail).toMatch(/Native target and focus gates are checked/);
  });
});
