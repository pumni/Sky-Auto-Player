import { describe, expect, it } from 'vitest';
import { timingMarginRecommendationSourceLabel } from './timingMarginSource';

describe('timing margin recommendation source labels', () => {
  it.each([
    ['qualified_calibration', 'Qualified calibration'],
    ['default_fallback', 'Default fallback (no valid calibration cache)'],
    ['invalid_cache_fallback', 'Invalid calibration cache fallback'],
    ['incompatible_calibration_fallback', 'Incompatible calibration fallback'],
    ['out_of_envelope_fallback', 'Calibration outside trusted range'],
  ])('shows the reason for source %s', (source, label) => {
    expect(timingMarginRecommendationSourceLabel(source)).toBe(label);
  });

  it('keeps unknown sources visible', () => {
    expect(timingMarginRecommendationSourceLabel('future_source')).toBe(
      'Unrecognized source (future_source)',
    );
  });
});
