import { describe, expect, it } from 'vitest';
import {
  formatFpsFrameHint,
  formatHoldFramesHint,
  formatTempoHint,
} from './playbackOptionFormatting';

describe('playback option formatting', () => {
  it('formats hold-frame and fps millisecond hints accurately', () => {
    expect(formatHoldFramesHint(1, 60)).toBe('16.7 ms');
    expect(formatHoldFramesHint(1.25, 60)).toBe('20.8 ms');
    expect(formatHoldFramesHint(1.5, 60)).toBe('25.0 ms');
    expect(formatFpsFrameHint(60)).toBe('16.7 ms/f');
    expect(formatFpsFrameHint(120)).toBe('8.3 ms/f');
    expect(formatHoldFramesHint(1, 0)).toBe('');
    expect(formatFpsFrameHint(0)).toBe('');
  });

  it('marks the default tempo without adding a redundant hint', () => {
    expect(formatTempoHint(1)).toBe('Default');
    expect(formatTempoHint(0.95)).toBeUndefined();
  });
});
