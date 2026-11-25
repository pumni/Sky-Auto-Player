import { LoaderCircle } from 'lucide-react';
import { useEffect, useRef } from 'react';
import type { DesktopStoreHook } from '../../state/store';
import { playbackIssuePresentation } from '../../state/store';
import { admissionDecisionDescription, admissionDecisionLabel } from './playerFormatting';
import { TimingAdvisory } from './TimingAdvisory';
import { Button } from '../ui/button';
import {
  ScrollAreaContent,
  ScrollAreaRoot,
  ScrollAreaScrollbar,
  ScrollAreaThumb,
  ScrollAreaViewport,
} from '../ui/scroll-area';

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
  const admissionRequired =
    prepared?.admission === 'confirmation_required' && prepared.prepared_id !== null;
  const issue = playbackIssuePresentation(playback);
  const riskExplanation = prepared
    ? prepared.risk.reasons.join(' ') ||
      (prepared.risk.short_up_gap_repeat_count > 0
        ? `${prepared.risk.short_up_gap_repeat_count} feasible same-key repeat(s) have an authored Up-to-Down gap shorter than one frame. The game may miss a short Up pulse.`
        : 'Review the prepared schedule before starting.')
    : '';
  const blockedMessage =
    prepared?.admission === 'blocked' ? (prepared.error_message ?? playback.error) : null;

  useEffect(() => {
    if (!admissionRequired) return;
    const frame = window.requestAnimationFrame(() => admissionActionRef.current?.focus());
    return () => window.cancelAnimationFrame(frame);
  }, [admissionRequired]);

  if (admissionRequired && prepared) {
    return (
      <ScrollAreaRoot
        style={{ position: 'absolute' }}
        className="bottom-[calc(100%+8px)] left-1/2 z-20 flex max-h-[min(68dvh,520px)] w-[min(600px,calc(100vw-24px))] max-w-[calc(100vw-24px)] -translate-x-1/2 flex-col rounded-popover border border-warning-border border-l-[3px] bg-popover shadow-2xl transition-opacity duration-admission-enter ease-enter motion-reduce:transition-none"
        data-slot="playback-admission"
      >
        <ScrollAreaViewport
          className="min-h-0 flex-1"
          role="group"
          aria-labelledby="playback-confirmation-title"
          aria-describedby="playback-confirmation-reason playback-confirmation-consequence"
        >
          <ScrollAreaContent className="grid min-w-0 grid-cols-1 gap-3 px-4 py-3 pr-6">
            <div className="grid min-w-0 gap-1.5">
              <div className="flex min-w-0 flex-wrap items-baseline justify-between gap-2">
                <strong
                  className="text-sm font-semibold text-foreground"
                  id="playback-confirmation-title"
                >
                  Playback confirmation
                </strong>
                <span
                  data-slot="playback-admission-song"
                  className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-xs text-text-secondary"
                  title={prepared.song.title}
                >
                  {prepared.song.title}
                </span>
              </div>
              <span className="overflow-visible whitespace-normal text-[13px] font-semibold text-warning">
                {prepared.risk.headline}
              </span>
              <p
                id="playback-confirmation-reason"
                className="m-0 break-words text-xs leading-relaxed text-foreground"
              >
                {riskExplanation}
              </p>
              <p
                id="playback-confirmation-consequence"
                className="m-0 break-words text-xs leading-relaxed text-muted-foreground"
              >
                Timing risk affects note detection in-game, not your account. Choose an option below
                or cancel without starting.
              </p>
            </div>
            <div className="grid min-w-0 gap-2.5">
              <div className="grid min-w-0 grid-cols-[repeat(auto-fit,minmax(min(100%,165px),1fr))] items-stretch gap-2.5">
                {(() => {
                  const primaryDecision = prepared.decisions.some(
                    (item) => item.decision === 'use_recommended',
                  )
                    ? 'use_recommended'
                    : 'proceed';
                  return prepared.decisions.map((decision, index) => {
                    const variant = prepared.variants.find(
                      (item) => item.decision === decision.decision,
                    );
                    const status = variant?.timing_advisory?.status;
                    const isPrimary = decision.decision === primaryDecision;
                    const estimateLabel =
                      status === 'short_up_gap_risk'
                        ? 'Short same-key Up gap risk'
                        : status === 'no_short_up_gap_risk'
                          ? 'No short same-key Up gap'
                          : 'View timing estimate';
                    return (
                      <div
                        className="grid min-w-0 grid-rows-[minmax(48px,auto)_auto_auto] content-start gap-1.5"
                        data-slot="admission-option"
                        key={decision.decision}
                      >
                        <Button
                          variant={isPrimary ? 'default' : 'outline'}
                          className="h-full min-h-12 w-full justify-center whitespace-normal px-2.5 py-1.5 text-center text-xs leading-[1.3]"
                          type="button"
                          disabled={transportOperation !== null}
                          ref={
                            isPrimary ||
                            (!prepared.decisions.some((d) => d.decision === primaryDecision) &&
                              index === 0)
                              ? admissionActionRef
                              : undefined
                          }
                          onClick={() => void start(decision.decision)}
                        >
                          {admissionDecisionLabel(decision.decision, decision.label)}
                        </Button>
                        <p className="m-0 text-[11px] leading-4 text-text-secondary">
                          {admissionDecisionDescription(decision.decision)}
                        </p>
                        <details
                          data-slot="admission-option-estimate"
                          className="min-w-0 text-[11px] text-text-secondary"
                        >
                          <summary className="cursor-pointer break-words py-1 hover:text-foreground">
                            {estimateLabel}
                          </summary>
                          <TimingAdvisory advisory={variant?.timing_advisory} />
                        </details>
                      </div>
                    );
                  });
                })()}
              </div>
              <Button
                variant="ghost"
                className="min-h-8 min-w-[100px] justify-self-end"
                type="button"
                title="Dismiss the prepared playback without starting"
                disabled={transportOperation !== null}
                onClick={cancel}
              >
                Not now
              </Button>
            </div>
          </ScrollAreaContent>
        </ScrollAreaViewport>
        <ScrollAreaScrollbar orientation="vertical">
          <ScrollAreaThumb />
        </ScrollAreaScrollbar>
      </ScrollAreaRoot>
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
  // A ready plan auto-starts from Play. Rendering its informational timing
  // advisory here makes a transient popover flash between prepare and start.
  // Keep plan diagnostics in the prepared DTO, but only interrupt for actual
  // confirmation or actionable playback errors.
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
      className={`absolute bottom-[calc(100%+8px)] left-3 right-3 z-20 grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-4 rounded-control border border-danger-border border-l-[3px] bg-danger-surface px-3 py-2.5 shadow-xl transition-opacity duration-admission-enter ease-enter motion-reduce:transition-none max-[700px]:grid-cols-1 ${['recoverable_status', 'conflict'].includes(presentedIssue.kind) ? 'border-warning-border bg-warning-surface' : ''}`}
      role="alert"
      data-slot="playback-recovery"
      aria-labelledby="playback-recovery-title"
      aria-describedby="playback-recovery-message"
    >
      <div className="grid min-w-0 gap-px">
        <strong className="text-xs font-semibold text-foreground" id="playback-recovery-title">
          {presentedIssue.title}
        </strong>
        <span
          id="playback-recovery-message"
          className={`block break-words text-xs leading-[1.35] ${['recoverable_status', 'conflict'].includes(presentedIssue.kind) ? 'text-text-secondary' : 'text-danger'}`}
        >
          {presentedIssue.message}
        </span>
      </div>
      <div className="flex flex-none flex-wrap justify-end gap-2 max-[700px]:justify-start">
        {statusRecovery && (
          <Button
            variant="outline"
            className="inline-flex min-h-8 items-center justify-center whitespace-nowrap px-2.5 text-xs"
            type="button"
            aria-label="Retry status"
            disabled={playback.statusRetryPending}
            onClick={() => void retryStatus()}
          >
            <span className="mr-1 grid size-3.5 place-items-center" aria-hidden="true">
              {playback.statusRetryPending && (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" size={13} />
              )}
            </span>
            <span>Retry status</span>
          </Button>
        )}
        {canStop && (
          <Button
            variant="destructive"
            className="inline-flex min-h-8 items-center justify-center whitespace-nowrap px-2.5 text-xs"
            type="button"
            disabled={transportOperation === 'stopping'}
            onClick={() => void stop()}
          >
            Stop playback
          </Button>
        )}
        {canTryAgain && (
          <Button
            variant="default"
            className="inline-flex min-h-8 items-center justify-center whitespace-nowrap px-2.5 text-xs"
            type="button"
            disabled={transportOperation !== null}
            onClick={() => void tryAgain()}
          >
            Try again
          </Button>
        )}
        {blockedMessage && (
          <Button
            variant="ghost"
            className="inline-flex min-h-8 items-center justify-center whitespace-nowrap px-2.5 text-xs"
            type="button"
            disabled={transportOperation !== null}
            onClick={cancel}
          >
            Dismiss
          </Button>
        )}
      </div>
    </section>
  );
}
