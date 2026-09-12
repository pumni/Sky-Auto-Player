import { useEffect, useRef, useState } from 'react';
import type { PlaybackOptionSets, TimingMarginRecommendation } from '../../bridge/DesktopBridge';
import { timingMarginRecommendationSourceLabel } from '../timingMarginSource';

interface TimingMarginControlProps {
  value: number;
  options: PlaybackOptionSets;
  recommendation: TimingMarginRecommendation;
  onChange: (value: number) => Promise<number | null>;
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
  const recommendationWithinRange =
    recommendation.recommended_timing_margin_us >= min &&
    recommendation.recommended_timing_margin_us <= max &&
    recommendation.recommended_timing_margin_us % step === 0;
  const [requestedValue, setRequestedValue] = useState(value);
  const authoritativeValue = useRef(value);
  const pendingWrites = useRef(0);

  useEffect(() => {
    authoritativeValue.current = value;
    if (pendingWrites.current === 0) setRequestedValue(value);
  }, [value]);

  const requestValue = (nextValue: number) => {
    pendingWrites.current += 1;
    setRequestedValue(nextValue);
    void onChange(nextValue)
      .then((confirmedValue) => {
        if (confirmedValue !== null) authoritativeValue.current = confirmedValue;
      })
      .catch(() => undefined)
      .finally(() => {
        pendingWrites.current -= 1;
        if (pendingWrites.current === 0) setRequestedValue(authoritativeValue.current);
      });
  };

  return (
    <div className="timing-margin-control" aria-label="Timing Margin">
      <span className="timing-margin-label">Timing Margin</span>
      <div className="timing-margin-stepper">
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Decrease Timing Margin"
          disabled={requestedValue <= min}
          onClick={() => requestValue(Math.max(min, requestedValue - step))}
        >
          −
        </button>
        <output aria-live="polite">{requestedValue} µs</output>
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Increase Timing Margin"
          disabled={requestedValue >= max}
          onClick={() => requestValue(Math.min(max, requestedValue + step))}
        >
          +
        </button>
      </div>
      {requestedValue !== recommendation.recommended_timing_margin_us && (
        <button
          className="button timing-margin-recommendation"
          type="button"
          disabled={!recommendationWithinRange}
          onClick={() => requestValue(recommendation.recommended_timing_margin_us)}
        >
          Use recommended ({recommendation.recommended_timing_margin_us} µs)
        </button>
      )}
      {!recommendationWithinRange && (
        <span className="settings-note" role="status">
          Recommendation exceeds the current Timing Margin range.
        </span>
      )}
      <span className="settings-note">
        Applies to Hold and Release Gap. Changes take effect in the next prepared session.
      </span>
      <span className="settings-note">
        Recommended sender margin: {recommendation.recommended_timing_margin_us} µs · Source:{' '}
        {timingMarginRecommendationSourceLabel(recommendation.source)}. This is sender evidence,
        not proof that the game accepted a note.
      </span>
    </div>
  );
}
