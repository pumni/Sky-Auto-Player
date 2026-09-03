import { useEffect, useRef } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { admissionDecisionLabel } from './playerFormatting';

interface PlaybackAdmissionProps {
  useStore: DesktopStoreHook;
}

export function PlaybackAdmission({ useStore }: PlaybackAdmissionProps) {
  const prepared = useStore((store) => store.playback.prepared);
  const playbackError = useStore((store) => store.playback.error);
  const start = useStore((store) => store.startPreparedPlayback);
  const admissionActionRef = useRef<HTMLButtonElement>(null);
  const admissionRequired = prepared?.admission === 'confirmation_required';
  const errorMessage =
    prepared?.admission === 'blocked' ? (prepared.error_message ?? playbackError) : playbackError;

  useEffect(() => {
    if (!admissionRequired) return;
    const frame = window.requestAnimationFrame(() => admissionActionRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [admissionRequired]);

  if (admissionRequired && prepared) {
    return (
      <section
        className="player-admission"
        role="group"
        aria-labelledby="playback-confirmation-title"
        aria-describedby="playback-confirmation-reason"
      >
        <div className="admission-copy">
          <strong id="playback-confirmation-title">Playback confirmation</strong>
          <span>{prepared.risk.headline}</span>
          <span id="playback-confirmation-reason" className="muted">
            {prepared.risk.reasons[0] ?? 'Review the selected song before starting.'}
          </span>
        </div>
        <div className="admission-actions">
          {prepared.decisions.map((decision, index) => (
            <button
              key={decision.decision}
              className={`button${decision.decision === 'proceed' ? ' button-primary' : ''}`}
              type="button"
              ref={index === 0 ? admissionActionRef : undefined}
              onClick={() => void start(decision.decision)}
            >
              {admissionDecisionLabel(decision.decision, decision.label)}
            </button>
          ))}
        </div>
      </section>
    );
  }

  if (!errorMessage) return null;
  return (
    <section
      className="player-admission player-admission-error"
      role="alert"
      aria-label="Playback error"
    >
      <div className="admission-copy">
        <strong>Playback unavailable</strong>
        <span className="player-message player-message-danger">{errorMessage}</span>
      </div>
    </section>
  );
}
