import { Minus, Plus } from 'lucide-react';
import { Button } from '../ui/button';
import { useEffect, useRef, useState } from 'react';
import type { PlaybackOptionSets } from '../../bridge/DesktopBridge';

interface TimingMarginControlProps {
  value: number;
  options: PlaybackOptionSets;
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
  density,
  onChange,
}: TimingMarginControlProps) {
  const min = options.timing_margin_min_us;
  const max = options.timing_margin_max_us;
  const step = options.timing_margin_step_us;

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
      className={`grid min-w-0 ${density === 'compact' ? 'grid-cols-1 content-start gap-1' : 'grid-cols-1 items-center gap-x-3 gap-y-1.5 @min-[340px]/settings-content:grid-cols-[minmax(0,1fr)_auto]'}`}
      role="group"
      aria-label="Timing margin"
      data-slot="timing-margin-control"
    >
      <div className="grid min-w-0 gap-0.5">
        <span
          className={
            density === 'compact'
              ? 'text-xs font-medium text-text-secondary'
              : 'text-[13px] font-medium leading-5 text-foreground'
          }
        >
          Timing margin
        </span>
        {density === 'full' && (
          <span className="text-xs leading-[18px] text-text-secondary">
            Extra time added to the minimum key hold.
          </span>
        )}
      </div>
      <div className="grid h-8 w-fit grid-cols-[32px_minmax(64px,1fr)_32px] items-center gap-1 rounded-control border border-border bg-surface px-1">
        <Button
          variant="ghost"
          size="icon-xs"
          className="size-7 rounded-control p-0 text-text-secondary hover:bg-control-hover hover:text-foreground"
          type="button"
          aria-label="Decrease timing margin"
          disabled={requestedValue <= min}
          onClick={() => requestValue(Math.max(min, requestedValue - step))}
        >
          <Minus size={14} aria-hidden="true" />
        </Button>
        <output className="text-center text-xs tabular-nums text-foreground" aria-live="polite">
          {requestedValue} µs
        </output>
        <Button
          variant="ghost"
          size="icon-xs"
          className="size-7 rounded-control p-0 text-text-secondary hover:bg-control-hover hover:text-foreground"
          type="button"
          aria-label="Increase timing margin"
          disabled={requestedValue >= max}
          onClick={() => requestValue(Math.min(max, requestedValue + step))}
        >
          <Plus size={14} aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
