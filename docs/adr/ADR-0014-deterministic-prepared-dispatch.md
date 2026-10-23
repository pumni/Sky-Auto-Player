# ADR-0014: Deterministic Prepared Dispatch

Status: Accepted.

Date: 2026-09-17.

This decision superseded the normal-playback policy portions of ADR-0011,
ADR-0012, and ADR-0013. Those documents remain historical qualification
records. Current Down validity and hold policy is in ADR-0017; lifecycle and
lease policy is in ADR-0018.

The decision-time no-cutoff/always-send rule was later refined by
[ADR-0015](ADR-0015-prepared-down-validity-and-causal-admission.md) and
superseded by ADR-0016/0017; ADR-0018 adds lifecycle and final-lease rules.
The immutable prepared architecture, one-send transaction boundary, and
strict/diagnostic physical-timing separation remain part of the history.

Original decision: [ADR-0014 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0014-deterministic-prepared-dispatch.md).

## Decision

At this decision date, normal playback used a prepared absolute-frame
dispatcher:

```text
immutable prepared physical frame
  -> absolute target = playback epoch + immutable offset
  -> HybridWaiter and bounded calibrated spin
  -> final cheap command/control, target, and focus atomics
  -> final_policy QPC evidence
  -> authoritative pre-call QPC
  -> exactly one prepared SendInput transaction
  -> completion QPC
  -> bounded post-send state, commit, and telemetry
```

Prepared frames and authored deadlines were immutable; a clean authorized
boundary received one send attempt, without retry, rebase, or using completion
latency as the next target. Mixed packets used one atomic whole-packet
`SendInput` transaction with canonical Up-before-Down ordering and disjoint
masks. Same-key geometry was checked before realtime execution.

Completion QPC was transport/telemetry evidence, not a normal scheduling guard.
Strict/diagnostic mode retained `PhysicalTimingGuard` for physical floors.
Normal hold/release samples remained raw forensics, serialized and reported even
when nonzero, not a shipping verdict; strict diagnostics could reject a packet
that violated the physical floor. Same-timestamp Up/Down overlap was rejected
at compile time and infeasible release gaps at session admission.
Focus was a published control-plane state plus a final atomic check; no
synchronous foreground query ran inside the target-crossing-to-SendInput
envelope, so a bounded observer race remained if Windows changed foreground
before publication. The supervisor lease watchdog interrupted and hard-stopped
the worker on heartbeat expiry; this was a control-plane fail-safe, not a
musical deadline.
Zero/partial/integrity transport failures failed closed and never became
lateness success or a Down retry. Current admission and lifecycle authority is
ADR-0017/0018.

## Evidence retained

The dispatcher retains authored and physical target QPC, waiter wake QPC,
`final_policy_qpc`, authoritative pre-call QPC, completion QPC, interval
percentiles, transport status/counts, control/focus/lease rejection evidence,
preparation/no-allocation evidence, and bounded cleanup/hold/release forensics.
These measurements qualify the implementation; they are not additional
scheduling policy.

The legacy persisted key `default_normal_down_start_tolerance_us` is accepted
only for safe migration and is discarded from canonical settings. It is not
mapped to Timing Margin or any replacement grace period.
