import { useEffect, useRef, useState } from 'react';
import type { PlaybackOptionSets } from '../../bridge/DesktopBridge';

interface LateNoteToleranceControlProps {
  value: number;
  options: PlaybackOptionSets;
  onChange: (value: number) => Promise<number | null>;
}

export function LateNoteToleranceControl({
  value,
  options,
  onChange,
}: LateNoteToleranceControlProps) {
  const min = options.normal_down_start_tolerance_min_us;
  const max = options.normal_down_start_tolerance_max_us;
  const step = options.normal_down_start_tolerance_step_us;
  const defaultUs = options.normal_down_start_tolerance_default_us;

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

  const isAboveDefault = requestedValue > defaultUs;
  const msDisplay = `${(requestedValue / 1_000).toFixed(1)} ms`;

  return (
    <div
      className="late-note-tolerance-control timing-margin-control timing-margin-control--full"
      role="group"
      aria-label="Late note tolerance"
    >
      <div className="timing-margin-heading">
        <span className="timing-margin-label">Late note tolerance</span>
        <span className="timing-margin-recommendation">
          · default {(defaultUs / 1_000).toFixed(1)} ms
        </span>
      </div>
      <div className="timing-margin-stepper">
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Decrease Late note tolerance"
          disabled={requestedValue <= min}
          onClick={() => requestValue(Math.max(min, requestedValue - step))}
        >
          −
        </button>
        <output aria-live="polite">{msDisplay}</output>
        <button
          className="button timing-margin-step"
          type="button"
          aria-label="Increase Late note tolerance"
          disabled={requestedValue >= max}
          onClick={() => requestValue(Math.min(max, requestedValue + step))}
        >
          +
        </button>
        {requestedValue !== defaultUs && (
          <button
            className="button button--subtle late-note-tolerance-reset"
            type="button"
            aria-label="Reset Late note tolerance to default"
            onClick={() => requestValue(defaultUs)}
          >
            Reset to {(defaultUs / 1_000).toFixed(1)} ms
          </button>
        )}
      </div>
      {isAboveDefault && (
        <span className="settings-note late-note-tolerance-warning" role="alert">
          Values above 2.5 ms increase late-note continuity at the cost of authored timing accuracy
          under load.
        </span>
      )}
      <span className="settings-note">
        Applies only to normal playback Down key events. Strict timing diagnostic playback ignores
        this tolerance.
      </span>
    </div>
  );
}
