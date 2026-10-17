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

Let `H = frame_base_hold + timing_margin` and `G = frame + margin` for
authored admission. The physical same-key release floor is one frame,
`G_physical = frame`; it does not include the margin. At 60 FPS with a 500 µs
margin, `H = 17,167 µs`, authored `G = 17,167 µs`, and physical `G = 16,667
µs`. The musical Up floor is `successful_down_completion + H`. The margin is
included once in H and authored G, not added again by the physical guard.

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

## Worker preparation and count-in

The caller's `arm()` request does not publish an epoch. The worker prepares the
immutable stream, coordinator state, bounded allocations, wait/admission
resources, and scheduling guards before publishing `Ready`. It then performs
the final control, lease, target, focus, and owner checks in an `Arming` state.
Only a successful atomic Ready-to-Armed commit publishes the immutable QPC
epoch and starts the requested count-in:

```text
epoch_qpc = fresh_qpc_after_preparation + requested_pre_roll
authored_target(packet) = epoch_qpc + packet.authored_timestamp
```

Preparation duration is therefore outside the count-in and does not move an
authored packet target. Readers remain in `preparing` with zero elapsed time
and no countdown until Armed is committed. The worker checks cancellation
through long coordinator and stream traversals; a stop or authority change
that wins the Armed transition cannot be followed by an epoch publication or
musical input. After Armed, the ordinary final gates still protect every
Down-bearing packet.

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
QPC, the fixed-size `PhysicalTimingGuard` records per-key not-before floors:

```text
musical_up_not_before[key] = successful_down_completion[key] + effective_min_hold
down_not_before[key]        = successful_up_completion[key] + frame
physical_wait_target       = max(authored_target, relevant per-key floors,
                                  resume barrier for Down-bearing packets)
```

where `effective_min_hold = frame_base_hold + timing_margin` is materialized
at boot and passed to the guard. The guard does not recompute that sum. It has
no latest start or feasibility rejection. A normal and a strict/diagnostic
successful completion use the same update path.
An authorized Down delayed by a floor is still sent once when the non-time
final gates pass. A matching Up waits for the Down completion floor, and the
next same-key Down waits for the Up completion floor. A mixed packet waits for
the maximum relevant floor and remains one atomic Up-before-Down packet.

Authored timestamps remain immutable. A late Down can reduce the delivered
duration from its authored interval, although its musical Up still waits for
the physical H floor. An earlier physical floor or sequential worker processing
can delay unrelated Down packets, and already-overdue packets can be emitted
close together as the worker advances. This is head-of-line delay and overdue
output, not schedule rebase, reordering, or replay. Custom native durations
need distinct duration semantics; the frame-based policy floor does not
promise full authored-duration preservation after a late Down.

After verified resumable cleanup, the worker samples fresh QPC Q after the
cleanup call returns and sets `resume_down_not_before = Q + frame`. Every later
Down-bearing packet applies this global absolute floor alongside its authored
target and per-key floors. A mixed packet waits as a whole; Up-only packets are
unaffected. A successful Down does not clear the barrier for other keys, and
an authored target later than the barrier remains unchanged. Once the barrier
has passed it adds no delay. Manual, focus, and system-power resume paths keep
the same barrier. A new session constructs a clean guard. Unverified cleanup,
QPC failure, or checked-add overflow leaves the guard fail-closed and prevents
later Down dispatch. A failed, partial, ambiguous, or untrustworthy transport
result does not create completion evidence; the existing terminal cleanup
policy applies.

After that verified cleanup and barrier establishment, resumable reconciliation
stores at most one exact authored-Up token per instrument slot. The token binds
the live generation, key slot, and immutable packet/intent location. The mutable
schedule invalidates the generation while preserving its authored physical Up;
the matching frozen Up consumes the token once. Validation is bounded by the
fifteen slots and current packet, while full generation conservation remains a
separate terminal audit. Because `SendInput` cannot distinguish a user-held
virtual key from a player-held one, the resumed authored Up is still emitted and
can release a user's overlapping key. See [the dispatch architecture](rt-dispatch-architecture.md#bounded-resumable-generation-reconciliation)
for the token ownership contract and its limits.

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
   followed by target, published-focus, and late-control rechecks. After
   modifier observation and final target atomic revalidation, an enabled
   supervisor lease receives one fresh QPC age proof before the final control
   check and sender handoff.
5. Take `final_policy_qpc`, enter the trusted sender, reset Win32 last error,
   take the sender-owned `pre_call_qpc`, and make exactly one `SendInput`
   call.
6. Validate completion QPC and transport masks, commit confirmed ownership,
   update physical floors on complete success, and enqueue bounded diagnostics.

Up-only safety and cleanup releases retain their immediate safety semantics and
do not wait on musical floors. Partial or ambiguous Down transport is never
retried. Mixed packets are never split.

## Waiting and observation

The production waiter requires an interruptible high-resolution waitable timer
and bounded QPC spin to the selected physical wait target. Failure to create
the high-resolution timer is a startup failure; there is no production
coarse-timer fallback. Wait guard time is a wake mechanism only; it is not an
applied dispatch lead. Startup uses six wake samples to initialize a heuristic,
not a percentile or service-level guarantee. An interrupt, focus change,
command, or lease transition replans the current boundary. The final gates
remain authoritative after the wait.

Production keeps bounded scalar sender and floor evidence. Strict/diagnostic
mode may enqueue a fixed-capacity observation, but observer state cannot
authorize, reorder, retry, or split physical input. Useful evidence includes
authored target, physical floors, sender pre-call and completion QPC, transport
anomalies, final-gate rejections, floor delays, late Down attempts, missing
expected Down count, generation accounting, and final release obligations.
The trace schema pairs every absolute QPC timestamp with provenance and
availability. Essential fixed-width sender timing counts remain available
when trace diagnostics are off. See [real-time timing evidence](rt-dispatch-timing-evidence.md)
for exact formulas, unavailable samples, drop accounting, and the offline
non-authoritative whole-stream model.

Musical generation accounting is separate from safety cleanup. A safety or
cleanup Up can be emitted idempotently without creating a musical `Released`
generation, so raw physical Up count can exceed musical Down count. The
authoritative cleanup evidence is:

```text
release_obligation_mask = active_mask | possibly_active_mask | failed_release_mask | in_flight_mask
```

Focus loss immediately blocks new Downs and pauses musical time. Resumable
release/reconciliation may remain deferred until trustworthy target restoration
or existing terminal cleanup, so an unfocused interval does not establish that
held keys were physically released. Cleanup Ups are safety operations and
bypass H.

For enabled-lease Down-bearing traffic, final admission takes one fresh QPC
sample after modifier observation and final target atomic revalidation, before
the last control check and sender handoff. The proof uses at most two atomic
state loads and one compare-exchange. A heartbeat age strictly greater than
the configured timeout is Expired; equality remains Fresh. A missing enabled
heartbeat, an inconclusive concurrent replacement, or QPC failure is
Unproven/fail-closed as appropriate. Expired is latched monotonically; the
proof does not retry in a loop. UpOnly traffic takes no new lease-age sample,
and a disabled timeout preserves its diagnostic behavior. The watchdog remains
active. Descheduling between a successful proof and the actual syscall remains
an unavoidable race window.

The following public counters may remain for schema compatibility:

- `final_sender_window_expirations`;
- `prepared_normal_sender_expirations`;
- `missed_physical_window_boundaries`;
- `release_floor_infeasible_boundaries`.

They are deprecated, remain zero in production, and do not control dispatch.
Native telemetry schema version 18 exports authored and physical-floor timing
differences with explicit sample availability; it does not export absolute
authored/floor QPC targets and contains no latest-start field. Legacy
deadline-rejection outcomes are historical terminology and are not active
runtime outcomes.

## Security and verification

Only the Windows adapter owns Win32 bindings and `SendInput`. Scheduling and
domain crates remain independent of Win32 details. No hooks, process memory,
alternate input injection, retries, adaptive lead, or authored timeline rebase
are permitted.

Relevant verification includes the paired-generation compiler and validator
tests, sender and player suites, the no-allocation dispatch test, static
security checks, and the Windows receive-only acceptance matrix recorded in
issue #412.

The transport return count (events inserted), sender-owned pre-call and
completion QPC, a project-owned ReceiveOnly sink observation, and game
sampling are distinct evidence layers. The application records them
separately; none of the sender timestamps prove that Sky sampled the
transition. Frame-plus-margin is the application's receiver model, not a
Windows guarantee. See [ADR-0018](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md)
for API references and the source/test/evidence crosswalk.

The production waiter arms `SetWaitableTimer` with a negative relative
100-nanosecond interval. It does not pass QPC ticks as a FILETIME value. The
high-resolution timer flag is documented for Windows 10 version 1803 and
later; the product target is Windows 11 x64. `timeBeginPeriod` may affect
waitable-timer resolution but Microsoft documents that it does not improve
QPC precision.
