# ADR-0016: Causal Authorization and Floor-Only Physical Timing

- Status: Accepted
- Date: 2026-09-20
- Supersedes: the active runtime cutoff decisions in ADR-0010 through
  ADR-0015; those records remain historical.

## Decision

Authored musical generations must be paired. The compiler and independent
validator reject a musical Down that remains open at schedule end. Stale
unmatched Up metadata remains non-musical, and safety or cleanup releases stay
outside the generation ledger.

A prepared Down is authorized only by an exact future observation of its
authored target and target generation. An authorized Down is sent once when
the normal final gates pass, regardless of waiter or scheduler lateness. An
overdue Down without that proof is `UnobservedBacklog`, performs zero Down
`SendInput` attempts, and is never caught up.

Successful complete sender transactions provide only physical not-before
floors:

```text
musical Up floor = Down completion + frame_base_hold
next same-key Down floor = Up completion + frame
packet wait = max(authored target, relevant floors)
```

The guard is per-key, fixed-size, and floor-only. Timing Margin remains part of
authored materialization and validation; it is not added to physical floors.
There is no latest-start boundary, pre-send sender cutoff, or lateness rejection
for an otherwise authorized Down. Mixed packets remain one atomic
Up-before-Down transaction, and authored timestamps never move.

Transport still reports only transport facts. A complete packet updates floors
only with trustworthy completion evidence. Partial or ambiguous Down transport
is not retried and follows fail-closed cleanup. Safety and cleanup Ups bypass
musical floors.

## Compatibility

Existing public fields for sender-window expirations, physical-window misses,
and release-floor infeasibility remain only to avoid unrelated schema churn.
They are deprecated, remain zero in production, and do not control execution.
Native telemetry schema version 17 exports the authored target and physical
floor fields without a latest-start field. Historical ADR text is not rewritten;
current behavior is defined by this ADR and the normative timing documents.

## Consequences

Late authorized playback preserves the live causal decision while respecting
actual physical hold and release floors. The no-catch-up rule remains explicit,
mixed packets remain atomic, and the real-time send path remains allocation-free
between final admission and `SendInput`.
