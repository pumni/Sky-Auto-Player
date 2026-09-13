import { useEffect, useRef, useState } from 'react';
import type { PlaybackOptionSets, TimingMarginRecommendation } from '../../bridge/DesktopBridge';
import { timingMarginRecommendationSourceLabel } from '../timingMarginSource';

interface TimingMarginControlProps {
  value: number;
  options: PlaybackOptionSets;
  recommendation: TimingMarginRecommendation;
  density: 'compact' | 'full';
  onChange: (value: number) => Promise<number | null>;
}

export function TimingMarginControl({
  value,
  options,
  recommendation,
  density,
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
    <div
      className={`timing-margin-control timing-margin-control--${density}`}
      role="group"
      aria-label="Timing Margin"
    >
      <div className="timing-margin-heading">
        <span className="timing-margin-label">Timing Margin</span>
        <span className="timing-margin-recommendation">
          {` · rec. ${recommendation.recommended_timing_margin_us} µs${
            recommendationWithinRange ? '' : ' · out of range'
          }`}
        </span>
      </div>
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
      {density === 'full' && (
        <>
          <span className="settings-note timing-margin-source">
            Source: {timingMarginRecommendationSourceLabel(recommendation.source)}
          </span>
          <span className="settings-note">Applies to Hold and Release Gap</span>
        </>
      )}
    </div>
  );
}
