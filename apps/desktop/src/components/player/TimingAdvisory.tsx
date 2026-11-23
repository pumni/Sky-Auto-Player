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

const dispatchModelStatusLabel: Record<
  GenerationTimingAdvisoryDto['dispatch_model_status'],
  string
> = {
  not_analyzed: 'Executor model not analyzed',
  unavailable: 'Executor model unavailable',
  no_modeled_risk: 'No modeled executor pressure',
  modeled_risk: 'Modeled executor pressure',
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
      <div className="timing-model-advisory" data-status={advisory.dispatch_model_status}>
        <strong>{dispatchModelStatusLabel[advisory.dispatch_model_status]}</strong>
        <span>{advisory.dispatch_model_explanation}</span>
        <span>
          {advisory.modeled_send_count} modeled sends · {advisory.modeled_negative_slack_send_count}{' '}
          late boundaries · {advisory.modeled_h_floor_displaced_send_count} H-floor delays (max{' '}
          {advisory.max_modeled_h_floor_displacement_us} µs) ·{' '}
          {advisory.modeled_head_of_line_send_count} head-of-line delays (max{' '}
          {advisory.max_modeled_head_of_line_delay_us} µs) · max lateness{' '}
          {advisory.max_modeled_lateness_us} µs
        </span>
      </div>
      {advisory.dry_run_is_hypothetical && (
        <span>This is a prepared dry-run schedule; no input is sent.</span>
      )}
      <details>
        <summary>About this check</summary>
        <span>
          The advisory counts canonical Down generations and authored Up-to-Down gaps. Its
          illustrative execution model reuses the production DispatchExecutor with{' '}
          {advisory.estimated_sender_setup_us} µs sender setup, {advisory.estimated_per_input_us} µs
          per input, and {advisory.estimated_post_send_us} µs post-send assumptions. It does not
          model receiver observation or change authored timing, hold, tempo, admission, or playback.
        </span>
        <span>
          Bounds: {advisory.generation_limit} generations, {advisory.work_limit_units} key visits,
          and {Math.round(advisory.working_memory_limit_bytes / 1_048_576)} MiB working memory.
        </span>
      </details>
    </div>
  );
}
