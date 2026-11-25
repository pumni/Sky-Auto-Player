import type { PlaybackDecisionId } from '../../bridge/DesktopBridge';

export const TEST_MODE_LABEL = 'Test mode';
export const TEST_MODE_ACCESSIBLE_DESCRIPTION = 'Test mode: no keyboard input is sent.';
export const TEST_MODE_PREPARED_DESCRIPTION =
  'Prepared for Test mode: no keyboard input will be sent.';
export const TEST_MODE_DIAGNOSTICS_LABEL = 'Test mode (dry run) · no physical input sent';
export const TEST_MODE_TIMING_UNMEASURED_TITLE = 'Test mode (dry run) · sender timing not measured';

export function formatPlayerDuration(value: number): string {
  const seconds = Math.max(0, Math.round(value / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

export function playerStateLabel(state: string): string {
  if (state === 'starting') return 'Starting…';
  if (state === 'stopping') return 'Stopping…';
  const label = state.replaceAll('_', ' ');
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function admissionDecisionLabel(decision: PlaybackDecisionId, label: string): string {
  return decision === 'dry_run' ? 'Test playback (no input)' : label;
}

export function admissionDecisionDescription(decision: PlaybackDecisionId): string {
  switch (decision) {
    case 'dry_run':
      return 'Runs in Test mode without sending keyboard input.';
    case 'use_recommended':
      return 'Applies the timing settings recommended for this session.';
    case 'proceed':
      return 'Uses your current timing settings unchanged.';
  }
}
