import type { DesktopStore } from '../../state/store';
import { playbackIssuePresentation, playbackSessionStatusConcern } from '../../state/store';

export type PlaybackSummaryState = Pick<
  DesktopStore['playback'],
  | 'error'
  | 'sessionId'
  | 'startRequestId'
  | 'state'
  | 'statusRetryPending'
  | 'transportOperation'
  | 'prepared'
>;

export function number(value: number | null | undefined, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : value.toFixed(digits);
}

export function measure(value: number | null | undefined, unit: string, digits = 2): string {
  return value === null || value === undefined ? 'Unavailable' : `${number(value, digits)} ${unit}`;
}

export function formatEventTime(timestamp: number): string {
  return new Date(timestamp).toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  });
}

export function playbackStatusLabel(playback: PlaybackSummaryState): string {
  const issue = playbackIssuePresentation(playback);
  const statusConcern = playbackSessionStatusConcern(playback);
  if (statusConcern === 'recoverable_status') {
    return 'Native playback status uncertain';
  }
  if (statusConcern === 'conflict') return 'Native playback session conflict';
  if (issue?.kind === 'target_failure' || issue?.kind === 'playback_failure') {
    return 'Playback failed';
  }
  if (playback.prepared?.admission === 'confirmation_required') return 'Confirmation required';

  switch (playback.transportOperation) {
    case 'preparing':
      return 'Preparing playback';
    case 'starting':
      return 'Starting playback';
    case 'pausing':
      return 'Pausing playback';
    case 'resuming':
      return 'Resuming playback';
    case 'stopping':
      return 'Stopping playback';
    case 'advancing':
      return 'Starting next song';
    case 'restarting':
      return 'Restarting song';
  }

  if (playback.startRequestId !== null) return 'Waiting for native playback confirmation';
  switch (playback.state) {
    case 'starting':
      return 'Starting playback';
    case 'playing':
      return 'Playing';
    case 'paused':
      return 'Paused';
    case 'stopping':
      return 'Stopping playback';
    case 'finished':
      return 'Playback finished';
    case 'failed':
      return 'Playback failed';
    default:
      return playback.prepared ? 'Prepared' : 'No active playback session';
  }
}

export function backendStatusLabel(
  status: DesktopStore['diagnostics']['samples'][number]['backend_status'],
): string {
  return status === 'healthy'
    ? 'Healthy'
    : status === 'degraded'
      ? 'Degraded'
      : status === 'error'
        ? 'Error'
        : 'Unavailable';
}
