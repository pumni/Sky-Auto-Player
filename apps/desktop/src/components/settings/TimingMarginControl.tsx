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

interface InFlightMutation {
  id: number;
  value: number;
}

interface MutationState {
  pending: InFlightMutation[];
  confirmed: { id: number; value: number } | null;
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

  const [mutationState, setMutationState] = useState<MutationState>({
    pending: [],
    confirmed: null,
  });
  const [prevPropValue, setPrevPropValue] = useState(value);
  const mountedRef = useRef(true);
  const mutationIdRef = useRef(0);
  const latestPropValueRef = useRef(value);

  useEffect(() => {
    latestPropValueRef.current = value;
  }, [value]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  if (value !== prevPropValue) {
    setPrevPropValue(value);
    setMutationState((prev) => ({
      pending: prev.pending,
      confirmed: null,
    }));
  }

  const activePending = mutationState.pending.filter(
    (m) => mutationState.confirmed === null || m.id > mutationState.confirmed.id,
  );
  const latestPending = activePending.at(-1);
  const requestedValue =
    latestPending !== undefined
      ? latestPending.value
      : mutationState.confirmed !== null
        ? mutationState.confirmed.value
        : value;

  const requestValue = (nextValue: number) => {
    const mutationId = ++mutationIdRef.current;
    setMutationState((prev) => ({
      ...prev,
      pending: [...prev.pending, { id: mutationId, value: nextValue }],
    }));

    void onChange(nextValue)
      .then((confirmedValue) => {
        if (!mountedRef.current) return;
        setMutationState((prev) => {
          const nextPending = prev.pending.filter((m) => m.id !== mutationId);
          if (confirmedValue === null) {
            return {
              ...prev,
              pending: nextPending,
            };
          }
          const isNewerConfirmation = prev.confirmed === null || mutationId >= prev.confirmed.id;
          const nextConfirmed = isNewerConfirmation
            ? { id: mutationId, value: confirmedValue }
            : prev.confirmed;
          const nextPendingCleaned = isNewerConfirmation
            ? nextPending.filter((m) => m.id > mutationId)
            : nextPending;
          return {
            pending: nextPendingCleaned,
            confirmed:
              nextConfirmed !== null && nextConfirmed.value !== latestPropValueRef.current
                ? nextConfirmed
                : null,
          };
        });
      })
      .catch(() => {
        if (!mountedRef.current) return;
        setMutationState((prev) => ({
          ...prev,
          pending: prev.pending.filter((m) => m.id !== mutationId),
        }));
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
