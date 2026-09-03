import type { PlaybackDecisionId } from '../../bridge/DesktopBridge';

export function formatPlayerDuration(value: number): string {
  const seconds = Math.max(0, Math.round(value / 1_000_000));
  return `${Math.floor(seconds / 60)}:${(seconds % 60).toString().padStart(2, '0')}`;
}

export function playerStateLabel(state: string): string {
  const label = state.replaceAll('_', ' ');
  return label.charAt(0).toUpperCase() + label.slice(1);
}

export function admissionDecisionLabel(decision: PlaybackDecisionId, label: string): string {
  return decision === 'dry_run' ? 'Test playback (no input)' : label;
}
