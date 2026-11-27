import type { GenerationTimingAdvisoryDto } from '../../bridge/generated';

interface TimingAdvisoryProps {
  advisory: GenerationTimingAdvisoryDto | null | undefined;
}

const statusLabel: Record<GenerationTimingAdvisoryDto['status'], string> = {
  not_analyzed: 'Not analyzed',
  unavailable: 'Unavailable',
  analyzed: 'Nominal scheduling slack analyzed',
};

const dispatchModelStatusLabel: Record<
  GenerationTimingAdvisoryDto['dispatch_model_status'],
  string
> = {
  not_analyzed: 'Executor model not analyzed',
  unavailable: 'Executor model unavailable',
  no_modeled_displacement: 'No modeled executor displacement',
  modeled_displacement: 'Modeled executor displacement',
};

export function TimingAdvisory({ advisory }: TimingAdvisoryProps) {
  if (!advisory) {
    return (
      <div
        data-slot="timing-advisory"
        className="grid min-w-0 gap-1 rounded-control border border-border bg-surface p-2 text-xs text-text-secondary"
        data-status="unavailable"
      >
        <strong className="text-foreground">Prepared generation details are unavailable.</strong>
      </div>
    );
  }

  const reference = advisory.scenarios.find((scenario) => scenario.scenario_id === 'reference');
  const stress = advisory.scenarios.find((scenario) => scenario.scenario_id === 'stress');

  return (
    <div
      className="grid min-w-0 gap-1 rounded-control border border-border bg-surface p-2 text-xs text-text-secondary"
      data-slot="timing-advisory"
      data-status={advisory.status}
      data-plan-fingerprint={advisory.plan_fingerprint}
      aria-label="Prepared playback generation advisory"
    >
      <strong className="font-semibold text-foreground">{statusLabel[advisory.status]}</strong>
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
      {advisory.status === 'analyzed' && (
        <span>
          Same-key repeats: {advisory.same_key_repeat_count}
          {advisory.min_same_key_slack_us !== null
            ? ` · min nominal slack (I - H): ${advisory.min_same_key_slack_us} µs`
            : ' · min nominal slack (I - H): n/a'}
          {advisory.zero_slack_repeat_count > 0 &&
            ` · zero-slack (I = H): ${advisory.zero_slack_repeat_count}`}
          {advisory.effective_hold_floor_h_us !== null &&
            ` · minimum hold H: ${advisory.effective_hold_floor_h_us} µs`}
        </span>
      )}
      <div
        className="grid min-w-0 gap-1 border-t border-border pt-1.5 data-[status=modeled_displacement]:text-warning"
        data-status={advisory.dispatch_model_status}
      >
        <strong className="font-semibold text-foreground">
          {dispatchModelStatusLabel[advisory.dispatch_model_status]}
        </strong>
        <span>{advisory.dispatch_model_explanation}</span>
        {reference && (
          <span>
            Reference ({reference.estimated_sender_setup_us}+{reference.estimated_per_input_us}
            /ev+{reference.estimated_post_send_us} µs): max Down lateness{' '}
            {reference.max_modeled_down_lateness_us} µs (key readiness{' '}
            {reference.max_modeled_key_readiness_displacement_us} µs · cursor-order{' '}
            {reference.max_modeled_cursor_order_displacement_us} µs · worker occupancy{' '}
            {reference.max_modeled_worker_occupancy_residual_us} µs) · max Up H-floor{' '}
            {reference.max_modeled_h_floor_displacement_us} µs
          </span>
        )}
        {stress && (
          <span>
            Stress ({stress.estimated_sender_setup_us}+{stress.estimated_per_input_us}/ev+
            {stress.estimated_post_send_us} µs): max Down lateness{' '}
            {stress.max_modeled_down_lateness_us} µs · max Up H-floor{' '}
            {stress.max_modeled_h_floor_displacement_us} µs
          </span>
        )}
      </div>
      {advisory.dry_run_is_hypothetical && (
        <span>This is a prepared dry-run schedule; no input is sent.</span>
      )}
      <details className="border-t border-border pt-1.5">
        <summary className="cursor-pointer text-text-secondary hover:text-foreground">
          About this check
        </summary>
        <span className="mt-1 block">{advisory.model_cost_disclaimer}</span>
        <span className="mt-1 block">
          The advisory reports canonical Down generations and nominal same-key scheduling slack
          (I - H). Its illustrative execution scenarios reuse the production DispatchExecutor and
          do not model receiver observation or change authored timing, hold, tempo, admission, or
          playback.
        </span>
        <span className="mt-1 block">
          Bounds: {advisory.generation_limit} generations, {advisory.work_limit_units} work units,
          and {Math.round(advisory.working_memory_limit_bytes / 1_048_576)} MiB working memory.
        </span>
      </details>
    </div>
  );
}
