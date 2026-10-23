# ADR-0012: User-Configurable Normal-Playback Down Continuity

Status: Accepted.

> Historical product/qualification record only. Current normal Down and
> effective-min-hold semantics are in [ADR-0017](ADR-0017-current-down-continuity-and-physical-min-hold.md);
> lifecycle and lease policy are in [ADR-0018](ADR-0018-resumable-release-barrier-and-final-lease-age.md).
> The former tolerance setting is retired; the legacy key is accepted only to
> discard it during migration.

At this decision date, this ADR superseded only the fixed, internal tolerance
from [ADR-0011](ADR-0011-normal-playback-down-continuity.md). Later runtime
decisions ADR-0014 through ADR-0017 superseded the tolerance behavior.

Partially superseded by: [ADR-0013](ADR-0013-extended-normal-down-continuity-range.md)
(range extended to 2.0–10.0 ms in #288; dedicated Reset button removed in #287).

Parent work order: [#280](https://github.com/pumni/Sky-Auto-Player/issues/280)
(Follow-up to [#279](https://github.com/pumni/Sky-Auto-Player/issues/279) and
[#269](https://github.com/pumni/Sky-Auto-Player/issues/269)).

Implementation history: [PR #289](https://github.com/pumni/Sky-Auto-Player/pull/289).

Baseline commit: `5964f28527926c8dddd499a7ad9668a352a12657`.

Original qualification: [ADR-0012 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0012-user-configurable-normal-down-continuity.md).

## Context

ADR-0011 qualified a fixed `2,500 µs` total bound and rescued boundaries in the
`1.5–2.5 ms` region without extra backlog or chord damage. This follow-up
recorded a user-configurable range for hosts with larger wake jitter while
allowing lower-jitter users to tighten timing; neither choice changed physical
feasibility requirements.

## Decision

We expose bounded normal Down continuity tolerance to users as an Advanced setting:

```text
Setting key: normal_down_start_tolerance_us
UI label:    "Late note tolerance"
Default:     2,500 µs (2.5 ms)
Minimum:     2,500 µs (2.5 ms)
Maximum:     5,000 µs (5.0 ms, provisionally bounded and empirically qualified)
Step:          500 µs (0.5 ms)
```

### 1. Non-additive formula

The then-current formula was non-additive with Timing Margin:

```text
physical_latest_down_start = authored_target + Timing Margin
normal_continuity_cutoff   = authored_target + normal_down_start_tolerance

normal sender cutoff = max(physical_latest_down_start, normal_continuity_cutoff)
strict sender cutoff = physical_latest_down_start
```

At that time, Timing Margin governed authored physical feasibility and the
tolerance supplied the separate normal-playback continuity cutoff; strict mode
used only `physical_latest_down_start`. This runtime policy is historical.

### 2. UI presentation, guidance, and qualification evidence

The setting was placed in **Settings → Advanced**, defaulted to 2.5 ms, and
warned that higher tolerance trades authored timing accuracy for continuity.
The dedicated “Reset to 2.5 ms” button was later removed in #287; the default
indicator and steppers remained.

Phase-F1.1 real-wait sequential dense-probe qualification covered 3 passes and
5 gaps (1, 2, 3, 4, 5 ms), comparing 2.5, 3.5, and provisional 5.0 ms arms:
0 timeline rebases; `UnobservedBacklog == actually_overdue` on every iteration;
450/450 successful second sends on the 5.0 ms arm. In the sparse 100 ms-gap
probe, 100% of 2.6–5.0 ms wake-lateness offsets were rescued under 5.0 ms,
dropped under 2.5 ms, and failed closed at 5.5 ms.

### 3. Session-freezing invariant

At the time, native admission converted the setting once to checked QPC ticks
and froze it per prepared session; changes took effect only on the next session
and invalidated cached preparation through `settings_fingerprint`.

### 4. Configuration schema migration and validation

Schema 7 migrated to schema 8 with default `normal_down_start_tolerance_us: 2500`.
Persisted values outside `[2_500, 5_000]` or unaligned to 500 µs reset to 2.5 ms
(never clamp); the native session rejected invalid timing contracts fail-closed.

### 5. Invariants and non-claims

That design did not rescue physical-window failures or unauthorized/unobserved
boundaries, retry or catch up missed notes, or split mixed Up-before-Down
packets. It recorded telemetry schema 16 and the scalar rescue metrics listed
in ADR-0011, with mirrored constants across the app core, player, and desktop
shell. These are historical setting-era details, not current runtime policy;
see ADR-0017/0018.
