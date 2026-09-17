# ADR-0011: Normal-Playback Down Continuity Under Wake Jitter

Status: Accepted.

> Historical record only. Superseded for the current normal-dispatch contract
> by [ADR-0014](ADR-0014-deterministic-prepared-dispatch.md). The former
> normal continuity cutoff and rescue counters are retired; strict physical
> feasibility and transport safety evidence remain useful.

Supersedes: only the normal-playback sender-cutoff decision in
[ADR-0010](ADR-0010-physical-timing-feasibility.md). ADR-0010 remains the
historical record of the physical-feasibility redesign; its strict-mode,
physical-floor, authored-timeline, transport, safety, and migration decisions
remain in force.

Parent work order: [#269](https://github.com/pumni/Sky-Auto-Player/issues/269).

## Context

Sparse or slow Note-On boundaries exposed a distinction between physical
feasibility and the time at which a runnable Windows worker reaches the sender.
Phase A decomposed the path into the authored target, target-to-wake,
`wake → final_policy`, `final_policy → pre_call`, and
`pre_call → completion`. The observed misses were `FinalSenderWindowExpired`
outcomes; physical-window and backlog failures were not observed. The large
target-to-wake outliers contrasted with small post-wake intervals, so the
continuity problem is primarily a scheduler/wake problem at the sender
boundary, not a reason to move authored targets or loosen physical hold/release
floors.

The campaign also showed that Windows contention is bursty. A larger busy-spin
can reduce some misses, but it does not cover multi-millisecond scheduling
stalls consistently and can consume a substantial fraction of one core on
short gaps. A cold-only classification at `20 ms` is not a scheduler-policy
boundary: misses occurred at short gaps and at 20 ms, while some longer gaps
were clean. The 20 ms value remains an evidence label only.

## Decision

For normal playback, the production constant is:

```text
NORMAL_PLAYBACK_DOWN_START_TOLERANCE_US = 2,500
```

The tolerance is a total bound measured from the authored target, not an
addition to Timing Margin. For a prepared Down-bearing packet:

```text
physical_latest_down_start = authored_target + Timing Margin
continuity_cutoff          = authored_target + 2,500 µs

normal sender cutoff = max(physical_latest_down_start, continuity_cutoff)
strict sender cutoff = physical_latest_down_start
```

The worker converts the fixed tolerance once at admission. Equality at the
effective sender cutoff is admissible; the first QPC tick beyond it is a
`FinalSenderWindowExpired` outcome. The sender continues to use the existing
true QPC pre-call sample and prepared whole-packet `SendInput` seam.

This is bounded continuity for normal playback. It is not a user setting, a
persisted value, a dispatch lead, a change to authored targets, an adaptive
estimator, a Timing Margin update, or a claim about game polling, rendering,
audio onset, or human perception.

## Evidence for the bound

Phase B0 showed that increasing the calibrated spin floor was not a complete
solution. The startup-calibrated baseline was approximately `868 µs`; a fixed
`1,000 µs` probe changed one campaign from 8 to 7 misses, while `1,500–2,000
µs` probes used approximately 23–34% of one core in the short-gap scenarios and
still left scheduler outliers.

Phase C1.1 compared fresh current, 1.5 ms, and 2.5 ms arms with rotated order.
The tri-arm campaign recorded:

| Policy | `FinalSenderWindowExpired` | Rescued boundaries |
| --- | ---: | ---: |
| Current | 93 | 0 |
| 1.5 ms | 61 | 30 |
| 2.5 ms | 52 | 52 |

The 2.5 ms arm rescued 22 boundaries in the `1.5–2.5 ms` region that the
1.5 ms bound did not. Its rescued total-lateness distribution was
`p50=1,314 µs`, `p95=2,329 µs`, `p99=2,453 µs`, and `max=2,488 µs`.

The sequential dense probe did not show a meaningful next-boundary regression:
second-boundary loss was `5/750` for current, `6/750` for 1.5 ms, and `4/750`
for 2.5 ms. For the 2.5 ms arm, `UnobservedBacklog=3` matched
`second-overdue=3`; the rescue did not create additional backlog. The probe
also preserved authored target identity, with no timeline rebase or catch-up.

## Safety and ownership invariants

The physical feasibility check remains independent and happens before
continuity rescue:

```text
packet_not_before <= physical_latest_down_start
```

A packet that violates this relation is `PhysicalWindowExpired`, even when its
pre-call time would fit within the normal continuity cutoff. A boundary without
the exact future authorization remains `UnobservedBacklog`. Neither outcome is
rescued. Missed targets are not retried, rebased, or emitted as a catch-up
burst.

A normal late rescue is counted only when all of the following hold:

1. the sender pre-call is after `physical_latest_down_start`;
2. it is at or before the normal sender cutoff;
3. the prepared packet completes successfully with a valid completion QPC; and
4. `PhysicalTimingGuard.observe_successful_packet()` accepts the completion
   evidence.

Partial, zero-progress, integrity, malformed-completion, and physical-guard
failures remain fail-closed and are not rescue successes. Prepared mixed
packets retain canonical Up-before-Down ordering and are sent as one packet;
the policy never splits a chord or retries a transport outcome. A successful
rescue supplies the real completion QPC to the existing physical guard, so a
later same-key hold/release floor remains authoritative. A future authored
boundary remains on its authored target and is independently subject to its
authorization, backlog, physical, sender, focus, lease, and transport gates.

## Telemetry and compatibility

Internal fixed scalar metrics distinguish successful on-time sends,
`late_rescued_down_boundaries`, `late_rescued_down_keys`, and
`max_late_rescued_down_lateness_ticks`. `FinalSenderWindowExpired` records only
residual sender-cutoff rejects. `PhysicalWindowExpired`, `UnobservedBacklog`,
and transport anomalies remain separate outcomes. Rescue metrics are sender and
completion evidence; they do not assert that the game observed the transition.

The native telemetry schema remains version 16. No desktop DTO, public setting,
Win32 sender API, `SendInput` implementation, spin policy, or release/updater
surface is changed by this decision.

## Tradeoffs and non-claims

The bound preserves a small amount of continuity under ordinary scheduling
slips while accepting that larger scheduler stalls remain hard-stale misses.
It may therefore deliver a physically feasible Down slightly after its
Timing-Margin boundary in normal playback, and strict mode intentionally does
not receive this allowance. The 2.5 ms value is a qualified bounded engineering
policy from the campaign above; it is not a guarantee of Windows wake latency,
game input registration, visual timing, audio timing, or user-perceived timing.
