import { Minus, Plus } from 'lucide-react';
import { useState } from 'react';
import type { PlaybackOptionSets, TimingMarginRecommendation } from '../../bridge/DesktopBridge';
import { timingMarginRecommendationSourceLabel } from '../timingMarginSource';

interface TimingMarginControlProps {
  value: number;
  options: PlaybackOptionSets;
  recommendation: TimingMarginRecommendation;
  density: 'compact' | 'full';
  onChange: (value: number) => Promise<number | null>;
}

interface OptimisticMarginState {
  baseProp: number;
  value: number;
  pending: number;
  confirmed: number | null;
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
  const recommendedMargin = recommendation.recommended_timing_margin_us;
  const recommendationWithinRange =
    recommendedMargin !== null &&
    recommendedMargin >= min &&
    recommendedMargin <= max &&
    recommendedMargin % step === 0;
  const [override, setOverride] = useState<OptimisticMarginState | null>(null);
  const activeOverride = override !== null && override.baseProp === value ? override : null;
  const requestedValue = activeOverride ? activeOverride.value : value;

  const requestValue = (nextValue: number) => {
    setOverride((previous) => {
      const active = previous !== null && previous.baseProp === value ? previous : null;
      return {
        baseProp: value,
        value: nextValue,
        pending: (active?.pending ?? 0) + 1,
        confirmed: active?.confirmed ?? null,
      };
    });

    void onChange(nextValue)
      .then((confirmedValue) => {
        setOverride((previous) => {
          if (previous === null || previous.baseProp !== value) return previous;
          const remaining = Math.max(0, previous.pending - 1);
          const nextConfirmed = confirmedValue !== null ? confirmedValue : previous.confirmed;
          if (remaining === 0) {
            return nextConfirmed !== null
              ? { baseProp: value, value: nextConfirmed, pending: 0, confirmed: nextConfirmed }
              : null;
          }
          return { ...previous, pending: remaining, confirmed: nextConfirmed };
        });
      })
      .catch(() => {
        setOverride((previous) => {
          if (previous === null || previous.baseProp !== value) return previous;
          const remaining = Math.max(0, previous.pending - 1);
          if (remaining === 0) {
            return previous.confirmed !== null
              ? {
                  baseProp: value,
                  value: previous.confirmed,
                  pending: 0,
                  confirmed: previous.confirmed,
                }
              : null;
          }
          return { ...previous, pending: remaining };
        });
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
          {recommendedMargin === null
            ? density === 'compact'
              ? 'No recommendation'
              : ' · no recommendation'
            : density === 'compact'
              ? `Recommended ${recommendedMargin} µs${recommendationWithinRange ? '' : ' · out of range'}`
              : ` · rec. ${recommendedMargin} µs${recommendationWithinRange ? '' : ' · out of range'}`}
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
          <Minus size={14} aria-hidden="true" />
        </button>
        <output aria-live="polite">{requestedValue} µs</output>
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Increase Timing Margin"
          disabled={requestedValue >= max}
          onClick={() => requestValue(Math.min(max, requestedValue + step))}
        >
          <Plus size={14} aria-hidden="true" />
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
