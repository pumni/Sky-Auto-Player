# ADR-0010: Physical Timing Feasibility

Status: Accepted campaign contract; implementation is staged across #244-#249.

Parent work order: [#243](https://github.com/pumni/Sky-Auto-Player/issues/243).
W0 characterization: [#244](https://github.com/pumni/Sky-Auto-Player/issues/244).

## Context

The current timing policy validates an immutable authored schedule using a
user-owned Timing Margin, then independently permits a Down to start within a
session-frozen Late Down grace. That independent grace can consume the authored
hold/release headroom after the schedule has been validated. Sender diagnostics
measure this compression, but measurement alone does not make a physically
short transition valid.

The redesign replaces that split policy with one feasibility window. Actual
successful sender completion timestamps provide conservative per-key physical
visibility anchors. A late Down may still be valid while its physical floors
fit inside the authored headroom. If they do not fit, the complete Down chord is
missed and future authored times remain unchanged.

## Decision

The authored timeline is immutable. Timing Margin is the only authored
execution headroom, and there is no independent Late Down tolerance or hidden
grace. For a prepared session:

```text
frame_us              = ceil(1_000_000 / fps)
frame_base_hold_us    = ceil(hold_frames * frame_us)
timing_margin_us      = persisted user value

authored_min_hold_us  = frame_base_hold_us + timing_margin_us
authored_release_gap  = frame_us + timing_margin_us
```

At 60 FPS, one frame of hold, and a 500 µs margin:

```text
frame_us              = 16,667 µs
frame_base_hold_us    = 16,667 µs
authored_min_hold_us  = 17,167 µs
authored_release_gap  = 17,167 µs
Down window           = [authored_target, authored_target + 500 µs]
```

The margin is static authored headroom around the base physical floors. Zero
is a valid strict/no-headroom setting. With zero margin, positive pre-call
lateness can make a Down infeasible; the runtime must not silently add an
allowance.

## Runtime physical floors

One fixed-size, worker-owned `PhysicalTimingGuard` will retain only QPC-domain
values and masks for at most the 15 instrument slots. It consumes completion
timestamps already returned by the sender. It does not call QPC, inspect
settings, walk the schedule, allocate after initialization, or lock. It exposes
precomputed packet queries for the worker and resets or invalidates when sender
state is no longer trustworthy.

For each key:

```text
musical_up_not_before[key]
    = last_successful_down_completion_qpc + frame_base_hold_ticks

musical_down_not_before[key]
    = last_successful_up_completion_qpc + frame_ticks
```

For one authored packet:

```text
packet_not_before_qpc = max(
    authored_target_qpc,
    all relevant Up hold floors,
    all relevant Down release floors
)

latest_down_start_qpc = authored_target_qpc + timing_margin_ticks
```

A Down-bearing packet is eligible only when its exact frozen boundary was
observed and authorized while future; control, target, focus, and lease gates
pass; `packet_not_before_qpc <= latest_down_start_qpc`; and the authoritative
sender pre-call QPC sample is still at or before `latest_down_start_qpc`. The
sender-side recheck remains mandatory to close the race after worker admission.

For example, if a successful Down completes at `D_complete`, its authored
musical Up cannot start before `D_complete + frame_base_hold`. If a successful
Up completes at `U_complete`, the next same-key musical Down cannot start
before `U_complete + frame`. If `max(authored_target, release_floor)` is later
than `authored_target + timing_margin`, the Down chord is missed and later
authored targets do not move.

Actual completion QPC evidence stays at the player/Win32 execution boundary.
It must not be moved into `sky_dispatch_core`, which owns authored and
generation state.

## Down authorization and miss behavior

Every production Down boundary requires exact future authorization. The target
state is:

```rust
enum DownBoundaryState {
    AwaitingFuture,
    FutureAuthorized(PhysicalBoundaryStamp),
}
```

There is no one-shot late-discovery rescue, hidden rescue credit, hidden grace,
or exception that lets the first musical Down send without future observation.
Production preroll must authorize the first frozen Down through the normal
observation path. If startup stalls past that boundary, the Down is missed
rather than sent as stale work.

The Down chord is accuracy-first and atomic. If any Down key makes a
Down-bearing packet infeasible, no subset of that chord is sent. A worker may
classify guaranteed infeasibility before transport; the trusted sender still
checks its final pre-call QPC independently. No future authored target is
rebased and no catch-up burst is allowed.

The semantic miss vocabulary is `UnobservedBacklog`,
`PhysicalWindowExpired`, and `DownExpiredBeforeSend` for transport/status
reporting (with the final transport name subject to refinement). Legacy names
such as `Backlog`, `HardLate`, and `DeadlineMissedBeforeSend` do not describe
the physical failure and are migration targets.

## Up classes

Up behavior keeps three distinct classes:

1. Authored musical Up honors the per-key Down-completion hold floor.
2. An authored Up-prefix recovery after that packet's Down is missed remains
   musical and honors the same hold floor before releasing active keys.
3. Emergency/safety Up for panic, focus-loss suspension, terminal cleanup, or
   uncertain/partial transport bypasses musical floors and releases immediately.

Mixed-packet recovery sends only the prepared Up-prefix view at its musical
floor, then commits the packet's Down identities as expired/missed. It does not
rebuild a payload or split a Down chord into a playable subset.

## Calibration

The recommendation is advisory and uses:

```text
recommended_timing_margin_us
    = qualified_transport_reserve_us + fixed_guard_us
```

It never changes a live or prepared session. The six-sample startup waiter
probe is not p99 scheduler-reserve evidence. Unqualified evidence retains a
clearly labeled fallback; a wake-jitter-derived margin requires separate
qualification.

## Preserved architecture and boundaries

The redesign preserves the immutable authored timeline, dedicated native
dispatch worker, QPC as the authoritative clock, high-resolution waitable timer
with bounded spin, fixed prepared `INPUT` payloads before the precision
boundary, one normal `SendInput` call per physical packet, fresh final Down
admission, canonical Up-before-Down ordering, generation ownership, and commit
only after clean transport evidence. Partial or uncertain musical sends are
never retried; cleanup remains fail-closed. The precision sender path gains no
allocation, lock, configuration parse, schedule walk, or payload rebuild.

This decision does not introduce async scheduling in the real-time path,
per-note workers/timers, direct RDTSC/RDTSCP, CPU affinity as a QPC workaround,
Time Critical priority, adaptive dispatch lead, authored-target rebasing,
automatic tempo changes, packet retry, chord splitting, or a replacement
Late Down setting.

## W0 characterization and migration boundary

W0 records the pre-cutover behavior without changing production dispatch
semantics. The following legacy expectations are temporary scaffolding and
must be replaced or removed during W2 (#246):

- `late_rescue_cutoff_matrix_is_inclusive_and_one_shot`,
  `future_observation_rearms_rescue_only_after_boundary_commit`, and
  `randomized_rescue_sequence_never_catches_up_without_future_observation` in
  `worker.rs`;
- rescue/backlog/grace assertions in `engine/tests.rs`, including
  `three_overdue_downs_are_dropped_before_next_future_boundary`,
  `late_discovery_rescue_still_obeys_sender_cutoff`, and
  `authorized_down_first_tick_beyond_grace_misses_without_down_syscall`;
- the W0 test that records the current first-preroll Down admission from
  `Initial` without future authorization;
- `down_late_grace_*` admission tests and cutoff-specific assertions whose
  names/expected policy still treat Late Down as an independent setting.

The final sender pre-call race tests remain required, but their policy input
must become the authored latest-start window. Hold/release forensics tests are
characterization of the current telemetry contract and should be updated only
with their owning observability work, not used to imply physical enforcement
before the guard exists.

W0 is documentation and characterization only. It does not change public
settings or schemas, sender APIs, production dispatch behavior, timing/MMCSS/
spin policy, runtime allocations, or power management.
