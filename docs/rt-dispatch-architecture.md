# Real-Time Dispatch Architecture

This document is normative for the real-time playback path. The application
reads authored schedules, materializes paired musical generations, and emits
physical keyboard input through the Windows `SendInput` boundary.

## Ownership boundaries

`sky_dispatch_core` owns schedule compilation, generation pairing, authored
hold/release validation, and causal schedule identity. It has no Win32 or QPC
sender implementation.

`sky_player` owns worker orchestration, prepared packet selection, exact future
authorization, physical waits, final admission, completion evidence, and
per-key physical floors. `sky_native_adapters` owns OS and process-facing
adapters. `sky_dispatch_win32` owns packet materialization and the one
`SendInput` transaction. No other crate may simulate gameplay input.

The coordinator owns authored cursor and generation state. The worker owns
runtime QPC state and a fixed-size `PhysicalTimingGuard`; completion timestamps
never enter the domain schedule or move authored targets.

## Authored generations and plans

The compiler opens a generation on a musical Down and closes that same
generation on its matching authored Up. A Down left open at end of input is a
typed compile error. The independent validator repeats the end-of-walk check
against the compiled schedule. Overlapping same-key Down, same-key matched
Up+Down at one authored timestamp, and invalid hold or release spacing remain
rejected. Stale unmatched Up metadata is retained only as non-musical
metadata. Cleanup and safety releases are outside the ledger.

The worker plans one of:

- metadata-only work with no physical packet;
- one immutable physical packet and one authored target; or
- no work.

The physical packet contains canonical Up entries before Down entries. A mixed
packet remains one transaction and is never split. The authored target is
derived once from the playback epoch and the immutable schedule timestamp.

## Causal authorization

Every Down-bearing prepared boundary has an exact identity: authored target,
packet masks, source/generation identity, and target generation. The worker
records authorization only when that exact authored target is strictly in the
future. Waiting to a later physical floor does not change the identity used by
authorization.

Once authorized, waiter or scheduler lateness does not revoke the boundary.
Pause, focus or epoch reset, target-generation changes, suspend, and consumed
or completed boundaries invalidate it. An overdue boundary that was never
authorized is `UnobservedBacklog`, performs zero Down `SendInput` attempts, and
is dropped. Later overdue boundaries are not caught up; the next future
boundary can authorize normally.

## Physical floors and waiting

After a complete successful musical packet with trustworthy sender completion
QPC, the guard updates only the affected keys:

```text
musical_up_not_before[key] = Down completion + frame_base_hold
down_not_before[key]        = Up completion + frame_us
packet_not_before           = max(authored_target, relevant floors)
```

The guard is floor-only. It has no Timing Margin, latest start, sender cutoff,
or lateness rejection. The normal and strict/diagnostic completion paths share
the same update helper. Transport failures do not create completion evidence.
Lifecycle reset and invalidation clear stale floors.

For a physical plan the loop queries the guard before waiting. It waits to
`packet_not_before_qpc`; metadata-only work waits to its authored target. Causal
authorization still compares against the authored target. If a floor delays an
authorized packet, the packet is sent when the floor is reached. A mixed packet
waits to the maximum member floor and remains atomic. No authored timeline
rebase occurs.

Up-prefix recovery for an unobserved mixed Down is bounded and immutable. A
valid prepared Up prefix may be sent once; a trustworthy successful completion
updates the released keys' Down floors. The dropped Down remains unauthorized
and is never emitted by recovery. Safety and cleanup Ups bypass musical floors.

## Final physical path

The worker performs the following sequence:

1. Resolve and validate the immutable packet before the wait.
2. Wait to the authored target or queried physical floor.
3. Apply quit, skip, pause, suspend, lease, target, and focus gates.
4. For eligible `require_focus=true` Down traffic, make one fresh foreground
   proof and then recheck target, published focus, and late control.
5. Record `final_policy_qpc`, enter the trusted sender, reset Win32 last error,
   take the sender-owned `pre_call_qpc`, and call `SendInput` exactly once.
6. Validate completion QPC, confirmed/skipped masks, and packet integrity.
7. Commit coordinator ownership and update floors only after complete success.
8. Record bounded production evidence or enqueue one diagnostic observation.

Up-only and `require_focus=false` paths do not perform the fresh foreground
query. Partial or ambiguous Down transport is never retried. A skipped key
still owned by the coordinator is a state disagreement and follows fail-closed
cleanup.

## Timing materialization

The authored timing equations are:

```text
frame_us           = ceil(1_000_000 / game_fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us   = persisted user value
min_hold_us        = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

Timing Margin is authored headroom only. It is not added to completion floors,
does not suppress a late authorized Down, and does not change an authored
timestamp. Completion evidence remains sender-side evidence and does not claim
game observation.

## Lifecycle and diagnostics

Focus loss pauses physical admission and retains existing fail-closed cleanup
semantics. Pause, suspend, target-generation changes, and session reset clear
future authorization and reset or invalidate floor evidence as required by the
existing lifecycle state machine. Cleanup releases remain safety operations.

Production diagnostics use bounded worker-local scalars and fixed-size state.
Strict/diagnostic observations use a bounded queue and cannot authorize,
reorder, retry, or split input. Useful output includes authored target,
physical floor delay, sender pre-call and completion timing, transport
anomalies, final-gate rejections, and causal backlog misses.

These compatibility counters may remain in snapshots and UI DTOs:

- `final_sender_window_expirations`;
- `prepared_normal_sender_expirations`;
- `missed_physical_window_boundaries`;
- `release_floor_infeasible_boundaries`.

They are deprecated, remain zero in production, and do not control execution.
The retired latest-start field is zero or unavailable. There is no active
`DownExpiredBeforeSend` or `PhysicalWindowExpired` runtime outcome.

## Security and verification

Gameplay input is restricted to Win32 `SendInput`. The repository does not
use hooks, process memory, code injection, alternate input APIs, adaptive
timeline rebasing, or transport retries.

Verification covers compiler and validator pairing, native transport ordering,
player final-gate behavior, completion floors, mixed-packet atomicity,
no-allocation dispatch, static security checks, and the Windows receive-only
acceptance matrix required by issue #379.
