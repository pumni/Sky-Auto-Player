# ADR-0017: Current Down Continuity and Physical Effective Min-Hold

- Status: Accepted
- Date: 2026-09-23
- Supersedes: the two active runtime decisions listed below from
  [ADR-0016](ADR-0016-causal-authorization-and-floor-only-physical-timing.md)

## Context

ADR-0016 remains the historical record of the earlier causal authorization
model. P1-P4 established the runtime behavior needed for the current
contract: a prepared current Down is admitted by live identity and lifecycle
gates, and physical hold timing uses the materialized effective minimum hold.
This ADR supersedes only those two active decisions; it does not rewrite
ADR-0016 or its historical evidence.

## Decisions

### Normal current Down continuity

For a live prepared session, a current normal Down has exactly one transport
send attempt when all of the following are true:

```text
live session/identity
+ final lifecycle/focus/control gates pass
+ no previous transport attempt
=> exactly one send attempt
```

Scheduler lateness alone is not a drop criterion. A current prepared Down does
not require a prior strictly-future observation. Target HWND and generation,
fresh foreground proof where required, control, pause/quit/skip/panic, suspend,
supervisor/lease, and preflight gates remain fail-closed. A failed, partial,
ambiguous, or clock-uncertain transport result is terminal for that musical
Down and is never retried.

The authored timeline remains immutable. There is no catch-up scheduler,
per-note lateness cutoff, adaptive rebase, or packet splitting. Mixed and
chord packets remain atomic and use canonical Up-before-Down ordering.

### Physical timing floors

The materialized boot policy supplies `effective_min_hold`; the physical guard
does not recompute it:

```text
musical_up_not_before
= successful_down_completion + effective_min_hold

effective_min_hold
= frame_base_hold + timing_margin

next_same_key_down_not_before
= successful_up_completion + frame

packet_wait_target
= max(authored_target, relevant physical floors)
```

The timing margin is included exactly once in the materialized effective
minimum hold. The next same-key Down floor remains one frame and is not the
minimum release gap. Safety and cleanup Ups bypass the musical hold floor.

### Musical ownership and safety obligations

Musical generation accounting is distinct from physical safety cleanup:

```text
release_obligation_mask
= active_mask | possibly_active_mask | failed_release_mask
```

Successful musical Down and its matching musical Up are the only normal
generation pairing. A safety/cleanup Up may be idempotently emitted without
creating a musical `Released` transition. Raw physical Up count may therefore
exceed musical Down count. Partial, ambiguous, and clock-uncertain Downs
retain conservative release obligations and do not replay the musical Down.

### Evidence boundary

`SendInput` success is sender/Windows injection evidence only. It is not proof
that the game sampled the key, rendered the transition, or began audio.
Acceptance evidence reports sender pre-call/completion timing, transport
anomalies, generation accounting, and final release obligations separately.

## Consequences

The current normative documentation and qualification reports use this
contract. ADR-0016, earlier issue evidence, and historical reports may retain
superseded terminology when they are explicitly identified as historical.
