export function timingMarginRecommendationSourceLabel(source: string): string {
  switch (source) {
    case 'qualified_calibration':
      return 'Qualified calibration';
    case 'insufficient_headroom':
      return 'Required margin exceeds the supported maximum';
    case 'default_fallback':
      return 'Default fallback (no valid calibration cache)';
    case 'invalid_cache_fallback':
      return 'Invalid calibration cache fallback';
    case 'incompatible_calibration_fallback':
      return 'Incompatible calibration fallback';
    case 'out_of_envelope_fallback':
      return 'Calibration outside trusted range';
    default:
      return `Unrecognized source (${source})`;
  }
}
