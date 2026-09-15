import type { UiEvent } from '../bridge/DesktopBridge';
import { MAX_DIAGNOSTIC_LINE_LENGTH } from './types';

export function boundedText(value: string): string {
  // The NUL replacement is intentional: diagnostic lines must remain single-line text.
  // eslint-disable-next-line no-control-regex
  const normalized = value.replace(/[\u0000\r\n\t]/g, ' ');
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  if (encoder.encode(normalized).length <= MAX_DIAGNOSTIC_LINE_LENGTH) {
    return normalized;
  }
  let bounded = decoder.decode(encoder.encode(normalized).slice(0, MAX_DIAGNOSTIC_LINE_LENGTH));
  while (encoder.encode(bounded).length > MAX_DIAGNOSTIC_LINE_LENGTH) {
    bounded = bounded.slice(0, -1);
  }
  return bounded;
}

export function eventDetail(event: UiEvent): string {
  switch (event.name) {
    case 'core.ready':
      return `Protocol ${event.payload.protocol_version} ready`;
    case 'core.fatal':
      return `${event.payload.code}: ${event.payload.message}`;
    case 'catalog.changed':
      return `Generation ${event.payload.generation}, ${event.payload.total} songs`;
    case 'catalog.load_failed':
      return `Catalog load failed: ${event.payload.message}`;
    case 'diagnostics.snapshot':
      return `p95 ${event.payload.p95_ms === null ? 'unavailable' : `${event.payload.p95_ms.toFixed(2)} ms`}; max ${event.payload.max_lateness_us === null ? 'unavailable' : `${event.payload.max_lateness_us} μs`}`;
    case 'calibration.progress':
      return `${event.payload.phase}: ${event.payload.completed}/${event.payload.total}`;
    case 'calibration.finished':
      return `${event.payload.outcome}: ${event.payload.status}`;
    case 'update.available':
      return `Update ${event.payload.available_version} is available`;
    case 'update.result':
      return `Update check: ${event.payload.state}`;
    case 'update.progress':
      return `${event.payload.message}: ${event.payload.completed} bytes`;
    case 'playback.state_changed':
      return `${event.payload.song_id} → ${event.payload.state}`;
    case 'playback.snapshot':
      return `${event.payload.title}: ${event.payload.state}`;
    case 'playback.finished':
      return `${event.payload.song_id}: ${event.payload.outcome}`;
    case 'playback.failed':
      return `${event.payload.code}: ${event.payload.message}`;
  }
}
