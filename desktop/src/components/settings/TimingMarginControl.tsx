import type { PlaybackOptionSets, TimingMarginRecommendation } from '../../bridge/DesktopBridge';

interface TimingMarginControlProps {
  value: number;
  options: PlaybackOptionSets;
  recommendation: TimingMarginRecommendation;
  onChange: (value: number) => void;
}

export function TimingMarginControl({
  value,
  options,
  recommendation,
  onChange,
}: TimingMarginControlProps) {
  const min = options.timing_margin_min_us;
  const max = options.timing_margin_max_us;
  const step = options.timing_margin_step_us;
  return (
    <div className="timing-margin-control" aria-label="Timing Margin">
      <span className="timing-margin-label">Timing Margin</span>
      <div className="timing-margin-stepper">
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Decrease Timing Margin"
          disabled={value <= min}
          onClick={() => onChange(Math.max(min, value - step))}
        >
          −
        </button>
        <output aria-live="polite">{value} µs</output>
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Increase Timing Margin"
          disabled={value >= max}
          onClick={() => onChange(Math.min(max, value + step))}
        >
          +
        </button>
      </div>
      {value !== recommendation.recommended_timing_margin_us && (
        <button
          className="button timing-margin-recommendation"
          type="button"
          onClick={() => onChange(recommendation.recommended_timing_margin_us)}
        >
          Use recommended ({recommendation.recommended_timing_margin_us} µs)
        </button>
      )}
      <span className="settings-note">
        Applies to Hold and Release Gap. Changes take effect in the next prepared session.
      </span>
    </div>
  );
}
