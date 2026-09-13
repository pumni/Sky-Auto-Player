import { LoaderCircle } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { playbackIssuePresentation } from '../../state/store';
import { admissionDecisionLabel } from './playerFormatting';

interface PlaybackAdmissionProps {
  useStore: DesktopStoreHook;
}

export function PlaybackAdmission({ useStore }: PlaybackAdmissionProps) {
  const playback = useStore((store) => store.playback);
  const prepared = playback.prepared;
  const transportOperation = playback.transportOperation;
  const start = useStore((store) => store.startPreparedPlayback);
  const prepare = useStore((store) => store.prepareSelectedPlayback);
  const stop = useStore((store) => store.stopPlayback);
  const retryStatus = useStore((store) => store.retryPlaybackStatus);
  const cancel = useStore((store) => store.cancelPreparedPlayback);
  const admissionActionRef = useRef<HTMLButtonElement>(null);
  const admissionRequired = prepared?.admission === 'confirmation_required';
  const issue = playbackIssuePresentation(playback);
  const blockedMessage =
    prepared?.admission === 'blocked' ? (prepared.error_message ?? playback.error) : null;

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
              disabled={transportOperation !== null}
              ref={index === 0 ? admissionActionRef : undefined}
              onClick={() => void start(decision.decision)}
            >
              {admissionDecisionLabel(decision.decision, decision.label)}
            </button>
          ))}
          <button
            className="button"
            type="button"
            aria-label="Cancel playback confirmation"
            disabled={transportOperation !== null}
            onClick={cancel}
          >
            Cancel
          </button>
        </div>
      </section>
    );
  }

  const presentedIssue =
    issue ??
    (blockedMessage
      ? {
          kind: 'playback_failure' as const,
          title: 'Playback could not start',
          message: blockedMessage,
        }
      : null);
  if (!presentedIssue) return null;

  const ownedSession = playback.sessionId !== null;
  const canStop =
    ownedSession &&
    ['starting', 'playing', 'paused', 'stopping', 'finished', 'failed'].includes(playback.state);
  const statusRecovery =
    presentedIssue.kind === 'recoverable_status' ||
    presentedIssue.kind === 'conflict' ||
    ownedSession;
  const canTryAgain =
    !ownedSession && playback.startRequestId === null && !statusRecovery && blockedMessage === null;

  const tryAgain = async () => {
    if (useStore.getState().playback.prepared?.admission === 'confirmation_required') cancel();
    await prepare({ dry_run: false });
    if (useStore.getState().playback.prepared?.admission === 'ready') await start();
  };

  return (
    <section
      className={`player-admission player-admission-error player-admission-${presentedIssue.kind}`}
      role="alert"
      aria-labelledby="playback-recovery-title"
      aria-describedby="playback-recovery-message"
    >
      <div className="admission-copy">
        <strong id="playback-recovery-title">{presentedIssue.title}</strong>
        <span id="playback-recovery-message" className="player-message">
          {presentedIssue.message}
        </span>
      </div>
      <div className="admission-recovery-actions">
        {statusRecovery && (
          <button
            className="button admission-recovery-action"
            type="button"
            aria-label="Retry status"
            disabled={playback.statusRetryPending}
            onClick={() => void retryStatus()}
          >
            <span className="admission-recovery-spinner-slot" aria-hidden="true">
              {playback.statusRetryPending && <LoaderCircle size={13} />}
            </span>
            <span>Retry status</span>
          </button>
        )}
        {canStop && (
          <button
            className="button admission-recovery-action"
            type="button"
            disabled={transportOperation === 'stopping'}
            onClick={() => void stop()}
          >
            Stop playback
          </button>
        )}
        {canTryAgain && (
          <button
            className="button button-primary admission-recovery-action"
            type="button"
            disabled={transportOperation !== null}
            onClick={() => void tryAgain()}
          >
            Try again
          </button>
        )}
        {blockedMessage && (
          <button
            className="button admission-recovery-action"
            type="button"
            disabled={transportOperation !== null}
            onClick={cancel}
          >
            Dismiss
          </button>
        )}
      </div>
    </section>
  );
}
