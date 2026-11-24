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
      className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 gap-y-1"
      role="group"
      aria-label="Timing Margin"
      data-slot="timing-margin-control"
    >
      <div className="flex min-w-0 items-center justify-between">
        <span className="text-xs font-medium text-text-secondary">Timing Margin</span>
      </div>
      <div className="grid h-8 grid-cols-[32px_minmax(0,1fr)_32px] items-center gap-1 rounded-control border border-border bg-surface px-1">
        <Button
          variant="ghost"
          size="icon-xs"
          className="size-7 rounded-control p-0 text-text-secondary hover:bg-control-hover hover:text-foreground"
          type="button"
          aria-label="Decrease Timing Margin"
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
          aria-label="Increase Timing Margin"
          disabled={requestedValue >= max}
          onClick={() => requestValue(Math.min(max, requestedValue + step))}
        >
          <Plus size={14} aria-hidden="true" />
        </Button>
      </div>
      {density === 'full' && (
        <span className="col-span-full text-xs text-muted-foreground">
          Applies to the minimum key hold
        </span>
      )}
    </div>
  );
}
