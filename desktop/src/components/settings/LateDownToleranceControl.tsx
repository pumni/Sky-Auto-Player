import { useEffect, useRef, useState } from 'react';
import type { PlaybackOptionSets } from '../../bridge/DesktopBridge';

interface LateDownToleranceControlProps {
  value: number;
  options: PlaybackOptionSets;
  onChange: (value: number) => Promise<number | null>;
}

export function LateDownToleranceControl({
  value,
  options,
  onChange,
}: LateDownToleranceControlProps) {
  const min = options.down_late_grace_min_us;
  const max = options.down_late_grace_max_us;
  const step = options.down_late_grace_step_us;
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
      className="timing-margin-control late-down-tolerance-control"
      aria-label="Late Down tolerance"
    >
      <span className="timing-margin-label">Late Down tolerance</span>
      <div className="timing-margin-stepper">
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Decrease Late Down tolerance"
          disabled={requestedValue <= min}
          onClick={() => requestValue(Math.max(min, requestedValue - step))}
        >
          −
        </button>
        <output aria-live="polite">{requestedValue} µs</output>
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Increase Late Down tolerance"
          disabled={requestedValue >= max}
          onClick={() => requestValue(Math.min(max, requestedValue + step))}
        >
          +
        </button>
      </div>
      <span className="settings-note">
        A Down later than this is dropped instead of being sent late.
      </span>
      {requestedValue === min && (
        <span className="settings-note">
          At 0 µs, any measured lateness beyond the exact target may drop a Down.
        </span>
      )}
    </div>
  );
}
