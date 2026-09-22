# Timing Principles

This document is normative for the production dispatch loop. It defines
sender-side timing only; completion evidence does not prove that Sky sampled a
transition or that audio began.

## Domains and authored timing

Authored schedule timestamps are immutable `TimelineTicks`. Runtime QPC values
are used for waiting, final admission evidence, and physical completion
floors. The worker never rebases authored timestamps from wake time, sender
lateness, or completion time.

The materialized policy is:

```text
frame_us           = ceil(1_000_000 / game_fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us   = persisted user value
min_hold_us        = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

Every authored musical generation is paired and satisfies:

```text
Down(G, K) -> Up(G, K)
authored_up - authored_down >= min_hold_us
next_same_key_down - previous_same_key_up >= min_release_gap_us
```

The compiler rejects an authored Down left open at end of input. The
independent schedule validator performs the same end-of-walk completeness
check, so a truncated or tampered schedule cannot bypass pairing. Stale
unmatched Up metadata can remain representable, but it does not become a
musical physical Up. Safety, focus-loss, and cleanup releases are outside the
musical generation ledger.

Timing Margin is materialized once. It contributes to authored `min_hold`,
authored `min_release_gap`, and the physical musical-Up floor through
`effective_min_hold`. It is not added a second time, is not a lateness cutoff,
and is not a latest-start deadline.

## Current Down continuity and physical floors

A live normal current Down with no previous transport attempt is admitted when
the final lifecycle, focus, control, target, supervisor/lease, suspend, and
preflight gates pass. It does not require a strictly-future observation, and
scheduler lateness alone is not a drop criterion. Each such boundary makes
exactly one transport send attempt. A partial, ambiguous, or clock-uncertain
transport result is terminal and is never retried.

Lifecycle and identity changes still invalidate stale prepared work and floor
evidence. `UnobservedBacklog` is not the normal classification for a late
current Down; it must not be produced merely because the scheduler was late.

After a complete successful musical packet with trustworthy sender completion
QPC, the fixed-size `PhysicalTimingGuard` records only per-key not-before
floors:

```text
musical_up_not_before[key] = successful_down_completion[key] + effective_min_hold
down_not_before[key]        = successful_up_completion[key] + frame
physical_wait_target       = max(authored_target, relevant floors)
```

where `effective_min_hold = frame_base_hold + timing_margin` is materialized
at boot and passed to the guard. The guard does not recompute that sum. It has
no latest start or feasibility rejection. A normal and a strict/diagnostic
successful completion use the same update path.
An authorized Down delayed by a floor is still sent once when the non-time
final gates pass. A matching Up waits for the Down completion floor, and the
next same-key Down waits for the Up completion floor. A mixed packet waits for
the maximum relevant floor and remains one atomic Up-before-Down packet.

The guard is reset or invalidated at the existing lifecycle boundaries. A
failed, partial, ambiguous, or untrustworthy transport result does not create
completion evidence; the existing fail-closed cleanup policy applies.

## Physical dispatch ordering

The physical path is ordered as follows:

1. Prepare and validate one immutable packet before waiting.
2. Derive the authored QPC target. For physical entries, query the guard and
   wait to `packet_not_before_qpc`; metadata-only entries use the authored
   target.
3. Classify and record authored-target lateness for evidence only; do not use
   lateness as a current-Down existence gate, including after applying a
   physical floor.
4. Run command, target, focus, and lease admission. Eligible
   `require_focus=true` Down traffic receives one fresh foreground proof,
   followed by target, published-focus, and late-control rechecks.
5. Take `final_policy_qpc`, enter the trusted sender, reset Win32 last error,
   take the sender-owned `pre_call_qpc`, and make exactly one `SendInput`
   call.
6. Validate completion QPC and transport masks, commit confirmed ownership,
   update physical floors on complete success, and enqueue bounded diagnostics.

Up-only safety and cleanup releases retain their immediate safety semantics and
do not wait on musical floors. Partial or ambiguous Down transport is never
retried. Mixed packets are never split.

## Waiting and observation

The waiter uses one interruptible high-resolution wait and bounded QPC spin to
the selected physical wait target. Wait guard time is a wake mechanism only;
it is not an applied dispatch lead. An interrupt, focus change, command, or
lease transition replans the current boundary. The final gates remain
authoritative after the wait.

Production keeps bounded scalar sender and floor evidence. Strict/diagnostic
mode may enqueue a fixed-capacity observation, but observer state cannot
authorize, reorder, retry, or split physical input. Useful evidence includes
authored target, physical floors, sender pre-call and completion QPC, transport
anomalies, final-gate rejections, floor delays, late Down attempts, missing
expected Down count, generation accounting, and final release obligations.

Musical generation accounting is separate from safety cleanup. A safety or
cleanup Up can be emitted idempotently without creating a musical `Released`
generation, so raw physical Up count can exceed musical Down count. The
authoritative cleanup evidence is:

```text
release_obligation_mask = active_mask | possibly_active_mask | failed_release_mask
```

The following public counters may remain for schema compatibility:

- `final_sender_window_expirations`;
- `prepared_normal_sender_expirations`;
- `missed_physical_window_boundaries`;
- `release_floor_infeasible_boundaries`.

They are deprecated, remain zero in production, and do not control dispatch.
Native telemetry schema version 17 exports the authored target and physical
floors directly; it contains no latest-start field. Legacy deadline-rejection
outcomes are historical terminology and are not active runtime outcomes.

## Security and verification

Only the Windows adapter owns Win32 bindings and `SendInput`. Scheduling and
domain crates remain independent of Win32 details. No hooks, process memory,
alternate input injection, retries, adaptive lead, or authored timeline rebase
are permitted.

Relevant verification includes the paired-generation compiler and validator
tests, sender and player suites, the no-allocation dispatch test, static
security checks, and the Windows receive-only acceptance matrix recorded in
issue #412. `SendInput` success is sender/Windows injection evidence only; it
does not prove that Sky sampled the transition.
