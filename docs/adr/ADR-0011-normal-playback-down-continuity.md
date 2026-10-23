# ADR-0011: Normal-Playback Down Continuity Under Wake Jitter

Status: Accepted.

> Historical qualification record only. Current normal Down/effective-min-hold
> semantics are in [ADR-0017](ADR-0017-current-down-continuity-and-physical-min-hold.md);
> lifecycle and final lease policy are in [ADR-0018](ADR-0018-resumable-release-barrier-and-final-lease-age.md).

At this decision date, this ADR superseded only the normal-playback sender-cutoff
decision in [ADR-0010](ADR-0010-physical-timing-feasibility.md). Its fixed
2.5 ms policy and rescue counters are historical; ADR-0014 through ADR-0017
later revised runtime admission and hold semantics.

Parent work order: [#269](https://github.com/pumni/Sky-Auto-Player/issues/269).

Original qualification: [ADR-0011 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0011-normal-playback-down-continuity.md).

## Context

Sparse/slow Note-On trials separated physical feasibility from Windows wake and
sender timing. Phase A measured authored target-to-wake, `wake → final_policy`,
`final_policy → pre_call`, and `pre_call → completion`: observed misses were
`FinalSenderWindowExpired`; physical-window and backlog failures were not
observed. Target-to-wake outliers dominated the smaller post-wake intervals.
The campaign also found bursty contention: more busy-spin reduced some misses
but did not cover multi-millisecond stalls and cost substantial CPU on short
gaps. The `20 ms` cold label was not a policy boundary; misses occurred at
short gaps and 20 ms while some longer gaps were clean.

## Decision

The historical normal-playback constant was:

```text
NORMAL_PLAYBACK_DOWN_START_TOLERANCE_US = 2,500
```

It was a total bound from the authored target, not an addition to Timing Margin:

```text
physical_latest_down_start = authored_target + Timing Margin
continuity_cutoff          = authored_target + 2,500 µs

normal sender cutoff = max(physical_latest_down_start, continuity_cutoff)
strict sender cutoff = physical_latest_down_start
```

Equality at that cutoff was admissible and the first later QPC tick was a
`FinalSenderWindowExpired` result. This was not a setting, persisted value,
dispatch lead, target change, adaptive estimator, Timing Margin update, or claim
about game polling, rendering, audio onset, or perception.

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

`packet_not_before` had to be at most `physical_latest_down_start` before a
continuity rescue; a violation was `PhysicalWindowExpired`, even if the sender
cutoff allowed the Down. Missing exact future authorization was
`UnobservedBacklog`. Neither failure was rescued; missed targets were not
retried, rebased, or caught up. A rescue required a pre-call after the physical
latest start and at/before the continuity cutoff, a complete packet with valid
completion QPC, and acceptance by `PhysicalTimingGuard.observe_successful_packet()`.
Partial/zero-progress/integrity/malformed-completion/guard failures failed
closed. Mixed packets stayed one canonical Up-before-Down transaction. Sender
and completion metrics did not prove game observation. Current runtime authority
is ADR-0017/0018.

## Telemetry and compatibility

The historical scalar metrics were `late_rescued_down_boundaries`,
`late_rescued_down_keys`, and `max_late_rescued_down_lateness_ticks`;
`FinalSenderWindowExpired`, `PhysicalWindowExpired`, `UnobservedBacklog`, and
transport anomalies remained distinct. The native telemetry schema was version
16 at this decision. No DTO, public setting, Win32 sender, `SendInput`, spin, or
release/updater surface was changed.

## Tradeoffs and non-claims

The 2.5 ms bound rescued some ordinary scheduling slips while larger stalls
remained misses. It could admit a physically feasible normal Down after the
Timing-Margin boundary; strict mode received no allowance. It was a bounded
engineering qualification, not a guarantee of Windows wake latency, game input
registration, visual/audio timing, or user perception.
