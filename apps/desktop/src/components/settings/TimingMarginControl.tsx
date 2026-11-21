import { Minus, Plus } from 'lucide-react';
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

interface OptimisticMarginState {
  value: number;
  pending: number;
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

  const [optimistic, setOptimistic] = useState<OptimisticMarginState | null>(null);
  const [prevPropValue, setPrevPropValue] = useState(value);
  const mountedRef = useRef(true);
  const mutationIdRef = useRef(0);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  if (value !== prevPropValue) {
    setPrevPropValue(value);
    if (!optimistic || optimistic.pending === 0 || value === optimistic.value) {
      setOptimistic(null);
    }
  }

  const activeOptimistic =
    optimistic !== null && (optimistic.pending > 0 || optimistic.value !== value)
      ? optimistic
      : null;
  const requestedValue = activeOptimistic ? activeOptimistic.value : value;

  const requestValue = (nextValue: number) => {
    const currentMutationId = ++mutationIdRef.current;
    setOptimistic((previous) => ({
      value: nextValue,
      pending: (previous?.pending ?? 0) + 1,
    }));

    void onChange(nextValue)
      .then((confirmedValue) => {
        if (!mountedRef.current) return;
        setOptimistic((previous) => {
          if (previous === null) return null;
          const remaining = Math.max(0, previous.pending - 1);
          if (currentMutationId < mutationIdRef.current) {
            return { ...previous, pending: remaining };
          }
          if (confirmedValue === null) {
            return remaining === 0 ? null : { ...previous, pending: remaining };
          }
          if (remaining === 0) {
            return confirmedValue !== value ? { value: confirmedValue, pending: 0 } : null;
          }
          return { value: confirmedValue, pending: remaining };
        });
      })
      .catch(() => {
        if (!mountedRef.current) return;
        setOptimistic((previous) => {
          if (previous === null) return null;
          const remaining = Math.max(0, previous.pending - 1);
          if (currentMutationId < mutationIdRef.current) {
            return { ...previous, pending: remaining };
          }
          return remaining === 0 ? null : { ...previous, pending: remaining };
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
