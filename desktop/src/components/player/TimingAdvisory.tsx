import type { WholeStreamTimingAdvisoryDto } from '../../bridge/generated';

interface TimingAdvisoryProps {
  advisory: WholeStreamTimingAdvisoryDto | null | undefined;
}

const statusLabel: Record<WholeStreamTimingAdvisoryDto['status'], string> = {
  not_analyzed: 'Not analyzed',
  invalid_assumptions: 'Assumptions invalid',
  unavailable: 'Unavailable',
  no_modeled_risk: 'No modeled timing pressure',
  modeled_risk: 'Modeled timing pressure',
};

export function TimingAdvisory({ advisory }: TimingAdvisoryProps) {
  if (!advisory) {
    return (
      <div className="timing-advisory" data-status="unavailable">
        <strong>Offline timing advisory unavailable for this plan.</strong>
      </div>
    );
  }

  const inspected =
    advisory.total_packet_count === null
      ? `${advisory.packet_count_inspected} packets inspected (limit ${advisory.packet_limit})`
      : `${advisory.packet_count_analyzed} of ${advisory.total_packet_count} packets analyzed (limit ${advisory.packet_limit})`;

  return (
    <div
      className="timing-advisory"
      data-status={advisory.status}
      data-plan-fingerprint={advisory.plan_fingerprint}
      aria-label="Offline whole-stream timing advisory"
    >
      <strong>{statusLabel[advisory.status]}</strong>
      <span>{advisory.explanation}</span>
      <span>{inspected}</span>
      {(advisory.status === 'no_modeled_risk' || advisory.status === 'modeled_risk') && (
        <>
          <span>
            Modeled pressure: {advisory.negative_slack_packet_count} late ·{' '}
            {advisory.h_floor_displaced_packet_count} H-floor ·{' '}
            {advisory.g_floor_displaced_packet_count} G-floor · {advisory.head_of_line_packet_count}{' '}
            HOL packet(s)
          </span>
          <span>
            Maximum displacement: H {advisory.max_h_floor_displacement_us} µs · G{' '}
            {advisory.max_g_floor_displacement_us} µs · HOL {advisory.max_head_of_line_delay_us} µs
          </span>
        </>
      )}
      {advisory.effective_hold_floor_h_us !== null &&
        advisory.authored_release_gap_floor_g_us !== null &&
        advisory.physical_release_gap_floor_g_us !== null && (
          <span>
            H {advisory.effective_hold_floor_h_us} µs · authored G{' '}
            {advisory.authored_release_gap_floor_g_us} µs · physical G{' '}
            {advisory.physical_release_gap_floor_g_us} µs
          </span>
        )}
      {advisory.structural_same_key_cycle_minimum_us !== null && (
        <span>
          Same-key authored cycle floor (H + authored G):{' '}
          {advisory.structural_same_key_cycle_minimum_us} µs
        </span>
      )}
      {advisory.dry_run_is_hypothetical && (
        <span>
          A physical playback projection is hypothetical for this dry-run; no input is sent.
        </span>
      )}
      <details>
        <summary>About this estimate</summary>
        <span>
          Assumed sender setup {advisory.assumptions.estimated_sender_setup_us} µs,{' '}
          {advisory.assumptions.estimated_per_input_us} µs per input,{' '}
          {advisory.assumptions.estimated_post_send_us} µs post-send, and{' '}
          {advisory.assumptions.estimated_metadata_cost_us} µs per metadata packet.
        </span>
        <span>
          Bounds: {advisory.packet_limit} packets, {advisory.work_limit_units} key-mask visits, and
          an estimated {Math.round(advisory.working_memory_limit_bytes / 1_048_576)} MiB working
          set.
        </span>
        <span>
          Offline illustration only. It is not measured receiver or audio timing, Sky does not
          acknowledge these estimates, and this advisory does not change authored timing or playback
          settings.
        </span>
      </details>
    </div>
  );
}
