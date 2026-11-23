import type { GenerationTimingAdvisoryDto } from '../../bridge/generated';

interface TimingAdvisoryProps {
  advisory: GenerationTimingAdvisoryDto | null | undefined;
}

const statusLabel: Record<GenerationTimingAdvisoryDto['status'], string> = {
  not_analyzed: 'Not analyzed',
  unavailable: 'Unavailable',
  no_short_up_gap_risk: 'No short same-key Up gap',
  short_up_gap_risk: 'Short same-key Up gap risk',
};

export function TimingAdvisory({ advisory }: TimingAdvisoryProps) {
  if (!advisory) {
    return (
      <div className="timing-advisory" data-status="unavailable">
        <strong>Prepared generation details are unavailable.</strong>
      </div>
    );
  }

  return (
    <div
      className="timing-advisory"
      data-status={advisory.status}
      data-plan-fingerprint={advisory.plan_fingerprint}
      aria-label="Prepared playback generation advisory"
    >
      <strong>{statusLabel[advisory.status]}</strong>
      <span>{advisory.explanation}</span>
      {advisory.total_authored_timestamp_group_count !== null &&
        advisory.down_generation_count !== null && (
          <span>
            {advisory.total_authored_timestamp_group_count} authored timestamp groups ·{' '}
            {advisory.down_generation_count} Down generations
            {advisory.unmatched_up_count !== null &&
              ` · ${advisory.unmatched_up_count} unmatched Up metadata item(s)`}
          </span>
        )}
      {advisory.status !== 'unavailable' && advisory.status !== 'not_analyzed' && (
        <span>
          Short authored Up gaps: {advisory.short_up_gap_repeat_count}
          {advisory.effective_hold_floor_h_us !== null &&
            ` · minimum hold H: ${advisory.effective_hold_floor_h_us} µs`}
        </span>
      )}
      {advisory.dry_run_is_hypothetical && (
        <span>This is a prepared dry-run schedule; no input is sent.</span>
      )}
      <details>
        <summary>About this check</summary>
        <span>
          The advisory counts canonical Down generations and same-key authored Up-to-Down gaps. It
          does not model receiver timing or change authored timing, hold, tempo, or playback.
        </span>
        <span>
          Bounds: {advisory.generation_limit} generations, {advisory.work_limit_units} key visits,
          and {Math.round(advisory.working_memory_limit_bytes / 1_048_576)} MiB working memory.
        </span>
      </details>
    </div>
  );
}
