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
      </div>
      {requestedValue < 2_500 ? (
        <span className="settings-note">
          Lower values can tighten late-note timing when they are above the active Timing Margin,
          but Windows wake jitter may cause more late notes to be dropped.
        </span>
      ) : (
        <span className="settings-note">
          If notes are still dropped on fast chords under load, increase this step-by-step. Rescued
          notes may play slightly later than their authored time.
        </span>
      )}
      {requestedValue > 2_500 && requestedValue <= 5_000 && (
        <span className="settings-note late-note-tolerance-warning" role="alert">
          Values above 2.5 ms increase late-note continuity at the cost of authored timing accuracy
          under load.
        </span>
      )}
      {requestedValue > 5_000 && (
        <span className="settings-note late-note-tolerance-warning" role="alert">
          High tolerance can rescue very late notes, but in dense passages it may cause following
          authored notes to become stale or physically infeasible. Use only when lower values still
          drop notes.
        </span>
      )}
      <span className="settings-note">
        Applies only to normal playback Down key events. Strict timing diagnostic playback ignores
        this tolerance.
      </span>
    </div>
  );
}
