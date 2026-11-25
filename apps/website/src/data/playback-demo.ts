export const PLAYBACK_DEMO_DURATION_MS = 2600;

export const PLAYBACK_DEMO_EVENTS = [
  { startMs: 0, endMs: 700, kind: 'chord', keys: ['A1', 'B2', 'C3'] },
  { startMs: 700, endMs: 1400, kind: 'note', keys: ['D2'] },
  { startMs: 1400, endMs: 2600, kind: 'hold', keys: ['E3'] },
] as const;

export const PLAYBACK_DEMO_KEYS = [
  'A1',
  'B1',
  'C1',
  'D1',
  'E1',
  'A2',
  'B2',
  'C2',
  'D2',
  'E2',
  'A3',
  'B3',
  'C3',
  'D3',
  'E3',
] as const;

export type PlaybackDemoEvent = (typeof PLAYBACK_DEMO_EVENTS)[number];

export function formatPlaybackTime(milliseconds: number): string {
  const elapsed = Math.max(0, Math.floor(milliseconds));
  const minutes = Math.floor(elapsed / 60_000);
  const seconds = Math.floor(elapsed / 1_000) % 60;
  const fraction = elapsed % 1_000;
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}.${String(fraction).padStart(3, '0')}`;
}
