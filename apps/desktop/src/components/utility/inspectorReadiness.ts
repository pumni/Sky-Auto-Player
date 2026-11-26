import type { SheetReadinessDto } from '../../bridge/generated/SheetReadinessDto';

export type InspectorReadinessState = 'pending' | 'unavailable' | 'ready' | 'review' | 'blocked';

export interface InspectorReadiness {
  state: InspectorReadinessState;
  label: string;
  detail: string;
}

interface InspectorReadinessInput {
  pending: boolean;
  detailUnavailable: boolean;
  sheetReadiness: SheetReadinessDto | null;
  conversionConfirmed: boolean;
  riskLevel: string | null;
}

export function resolveInspectorReadiness({
  pending,
  detailUnavailable,
  sheetReadiness,
  conversionConfirmed,
  riskLevel,
}: InspectorReadinessInput): InspectorReadiness {
  if (pending || sheetReadiness === 'loading') {
    return {
      state: 'pending',
      label: 'Checking',
      detail: 'Checking sheet compatibility and timing risk.',
    };
  }

  if (detailUnavailable || sheetReadiness === null) {
    return {
      state: 'unavailable',
      label: 'Readiness unavailable',
      detail: 'No compatibility result is available, so readiness is not inferred.',
    };
  }

  if (
    sheetReadiness === 'malformed' ||
    sheetReadiness === 'unsupported' ||
    sheetReadiness === 'unplayable'
  ) {
    return {
      state: 'blocked',
      label: 'Playback blocked',
      detail: 'This sheet cannot be prepared with its current format.',
    };
  }

  if (sheetReadiness === 'requires_confirmation' && !conversionConfirmed) {
    return {
      state: 'review',
      label: 'Review required',
      detail: 'Review and accept the proposed sheet conversion before preparation.',
    };
  }

  if (sheetReadiness === 'physical_risk' || riskLevel === 'medium' || riskLevel === 'high') {
    return {
      state: 'review',
      label: 'Review required',
      detail: 'Check the timing risk guidance below before starting playback.',
    };
  }

  const sheetReady =
    sheetReadiness === 'ready' ||
    (sheetReadiness === 'requires_confirmation' && conversionConfirmed);
  if (!sheetReady || riskLevel !== 'low') {
    return {
      state: 'unavailable',
      label: 'Readiness unavailable',
      detail: 'The current sheet and timing results are incomplete.',
    };
  }

  return {
    state: 'ready',
    label: 'Sheet ready',
    detail:
      'Sheet checks are clear. Native target and focus gates are checked when playback starts.',
  };
}
