# ADR-0012: User-Configurable Normal-Playback Down Continuity

Status: Accepted.

Supersedes: only the fixed internal/non-configurable nature of normal-playback
down-start tolerance in [ADR-0011](ADR-0011-normal-playback-down-continuity.md).
ADR-0010 and ADR-0011 non-additive formula, strict-mode sender cutoff,
authored-timeline immutability, transport, safety, and physical feasibility
floors remain fully in force.

Partially superseded by: [ADR-0013](ADR-0013-extended-normal-down-continuity-range.md)
(range extended to 2.0–10.0 ms in #288; dedicated Reset button removed in #287).

Parent work order: [#280](https://github.com/pumni/Sky-Auto-Player/issues/280)
(Follow-up to [#279](https://github.com/pumni/Sky-Auto-Player/issues/279) and
[#269](https://github.com/pumni/Sky-Auto-Player/issues/269)).

Baseline commit: `5964f28527926c8dddd499a7ad9668a352a12657`.

## Context

Phase C1.1 ([ADR-0011](ADR-0011-normal-playback-down-continuity.md)) qualified
a fixed `2,500 µs` (2.5 ms) total bound from the authored target as the baseline
for normal-playback Down continuity rescue under Windows scheduler wake jitter.
That decision successfully rescued late boundaries in the `1.5–2.5 ms` region
without increasing backlog or compromising chord integrity.

However, across diverse Windows hosts, background workloads, or timer-resolution
fluctuations, wake jitter exceeding 2.5 ms can still trigger
`FinalSenderWindowExpired` misses even when physical feasibility floors
(`Hold`, `ReleaseGap`) are completely satisfied.

Hard-coding a larger global constant would unilaterally degrade rhythmic tightness
for all users. Conversely, leaving the tolerance fixed at 2.5 ms prevents users on
higher-jitter systems from recovering dropped notes on dense musical passages.

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

The formula remains strictly non-additive with Timing Margin:

```text
physical_latest_down_start = authored_target + Timing Margin
normal_continuity_cutoff   = authored_target + normal_down_start_tolerance

normal sender cutoff = max(physical_latest_down_start, normal_continuity_cutoff)
strict sender cutoff = physical_latest_down_start
```

- Timing Margin continues to govern authored physical hold/release materialization
  and the physical feasibility limit `physical_latest_down_start`.
- `normal_down_start_tolerance` defines an independent continuity cutoff from the
  authored target.
- Strict playback mode never applies continuity tolerance; its sender cutoff
  remains strictly `physical_latest_down_start`.

### 2. UI presentation, guidance, and qualification evidence

- The control is placed exclusively in **Settings → Advanced** (not in quick
  playback profiles and not in Settings → Playback).
- The qualified, canonical default is **2.5 ms**.
- Clear user copy guides configuration:
  *"If notes are still dropped on fast chords under load, increase this step-by-step. Rescued notes may play slightly later than their authored time."*
- When configured above 2.5 ms, an inline warning informs the user:
  *"Values above 2.5 ms increase late-note continuity at the cost of authored timing accuracy under load."*
- A dedicated **"Reset to 2.5 ms"** button was initially provided alongside the control (subsequently removed in #287; users navigate via steppers).
- **Empirical qualification evidence:** Phase-F1.1 real-wait sequential dense-probe qualification
  across 3 passes and 5 gaps (1, 2, 3, 4, 5 ms) comparing arms 2.5 ms, 3.5 ms, and provisional
  5.0 ms confirmed:
  - 0 timeline rebases across all arms.
  - `UnobservedBacklog == actually_overdue` on every iteration (no false backlogs).
  - No systematic second-boundary degradation (450/450 successful second sends on 5.0 ms arm).
  - Sparse 100 ms gap comparison proved Note-Ons with 2.6–5.0 ms wake lateness are 100% rescued
    under 5.0 ms (dropped under 2.5 ms) and fail closed at 5.5 ms.

### 3. Session-freezing invariant

The configured tolerance is resolved at native worker admission, converted once
via checked QPC duration conversion into `normal_down_start_tolerance_ticks`, and
frozen for the lifetime of that playback session:

- In-flight or active playback sessions never observe mid-song mutations.
- Changing the setting updates persistent settings and takes effect on the next
  session preparation.
- The desktop shell's `settings_fingerprint` incorporates
  `normal_down_start_tolerance_us`, invalidating cached session preparation when
  modified.

### 4. Configuration schema migration and validation

- The settings schema version is incremented from **7** to **8**.
- Older configuration files migrate cleanly by populating
  `normal_down_start_tolerance_us: 2500`.
- If an invalid persisted value is encountered (outside `[2_500, 5_000]` or unaligned
  to 500 µs steps), it is reset to the 2.5 ms default (not clamped).
- At the native engine boundary, `NativeDispatchSession` self-rejects timing contracts
  with tolerance outside `[2_500, 5_000]` or unaligned with 500 µs steps, failing closed.

### 5. Invariants and non-claims

- **Physical feasibility precedence:** A packet whose physical floor exceeds
  `physical_latest_down_start` is classified as `PhysicalWindowExpired`. Late
  continuity tolerance cannot rescue physical-window failures.
- **Backlog and authorization:** Unobserved or unauthorized boundaries remain
  `UnobservedBacklog` and are never rescued.
- **No retries or catch-up:** Missed notes are never retried or burst as catch-up.
- **Chord and ordering integrity:** Mixed packets retain canonical Up-before-Down
  ordering and are dispatched as atomic single packets.
- **Telemetry schema stability:** Native telemetry schema remains version 16.
  Existing internal scalar metrics (`late_rescued_down_boundaries`,
  `late_rescued_down_keys`, `max_late_rescued_down_lateness_ticks`) provide
  complete observability for rescued events across the configurable range.
- Mirrored constant contract tests enforce compile-time equality across
  `sky_app_core`, `sky_player`, and `sky_desktop_shell`.
