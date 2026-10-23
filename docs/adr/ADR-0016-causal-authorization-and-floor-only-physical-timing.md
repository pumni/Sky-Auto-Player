# ADR-0016: Causal Authorization and Floor-Only Physical Timing

- Status: Accepted
- Date: 2026-09-20
- Historical runtime decision. ADR-0017 removes this ADR's strictly-future
  authorization requirement for an eligible current Down: when live session
  identity and final lifecycle/focus/control gates pass, and no transport
  attempt has occurred, it receives exactly one attempt. Scheduler lateness
  alone is not a drop cutoff. ADR-0017 also replaces this ADR's frame-only
  physical musical-Up floor with the materialized effective minimum hold.
  ADR-0018 owns current lifecycle and final-lease semantics.

Original decision: [ADR-0016 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0016-causal-authorization-and-floor-only-physical-timing.md).

## Decision

At the 2026-09-20 decision date, authored musical generations had to be paired:
the compiler and independent validator rejected an open musical Down at
schedule end. Stale unmatched Up metadata was non-musical; safety/cleanup
releases stayed outside the generation ledger.

The historical policy required exact future observation of the authored target
and generation. An authorized Down was sent once after normal final gates,
regardless of waiter lateness; without proof it became `UnobservedBacklog`,
made zero Down `SendInput` attempts, and was never caught up.

Under the historical floor-only policy, complete sender transactions provided
physical not-before floors:

```text
musical Up floor = Down completion + frame_base_hold
next same-key Down floor = Up completion + frame
packet wait = max(authored target, relevant floors)
```

The guard was per-key, fixed-size, and floor-only; Timing Margin stayed in
authored materialization, not physical floors. That historical decision removed
latest-start/pre-send cutoff rejection for an otherwise authorized Down.
ADR-0017 supersedes the strictly-future proof above for an eligible current
Down: live session/identity and final lifecycle/focus/control gates admit
exactly one attempt, regardless of scheduler lateness, with no lateness cutoff.
For the physical musical-Up floor, ADR-0017 uses
`successful_down_completion + effective_min_hold`, where the materialized
`effective_min_hold = frame_base_hold + timing_margin`; it does not use this
ADR's frame-only floor. Mixed packets remained one atomic Up-before-Down
transaction; authored timestamps never moved.

Transport reported transport facts only. A complete packet updated floors only
with trustworthy completion evidence. Partial or ambiguous Down transport was
not retried and followed fail-closed cleanup; safety/cleanup Ups bypassed
musical floors.

## Compatibility

Existing public fields for sender-window expirations, physical-window misses,
and release-floor infeasibility remain only to avoid unrelated schema churn.
They are deprecated, remain zero in production, and do not control execution.
Native telemetry schema version 17 exports authored target and physical-floor
fields without a latest-start field. These were decision-date compatibility
facts; current behavior is defined by ADR-0017/0018 and normative timing docs.

## Consequences

That policy preserved the causal decision and physical floors while allowing
late authorized playback. No-catch-up, atomic mixed packets, and allocation-free
final admission remained required. Current runtime behavior is defined by
ADR-0017/0018.
