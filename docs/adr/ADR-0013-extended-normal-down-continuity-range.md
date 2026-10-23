# ADR-0013: Extended Normal-Playback Down Continuity Range (2.0–10.0 ms) and Reset Button Removal

Status: Accepted.

> Historical range and qualification record only. Current normal Down and
> effective-min-hold semantics are in [ADR-0017](ADR-0017-current-down-continuity-and-physical-min-hold.md);
> lifecycle and lease policy are in [ADR-0018](ADR-0018-resumable-release-barrier-and-final-lease-age.md).

At this decision date, this ADR extended the setting range recorded by
[ADR-0012](ADR-0012-user-configurable-normal-down-continuity.md) and documented
the #287 removal of the dedicated reset button. Runtime behavior was later
revised by ADR-0014 through ADR-0017; this record is not current policy.

Parent work orders: [#287](https://github.com/pumni/Sky-Auto-Player/issues/287) and [#288](https://github.com/pumni/Sky-Auto-Player/issues/288).

Implementation history: [PR #289](https://github.com/pumni/Sky-Auto-Player/pull/289).

Baseline commit: `c337412a15ec80cea76a86fe0c5d1f7b1c269e8c`.

Original qualification: [ADR-0013 at the frozen WP6 checkpoint](https://github.com/pumni/Sky-Auto-Player/blob/d4e580cb46c164b3c564d8d99b2bc5f7835724d6/docs/adr/ADR-0013-extended-normal-down-continuity-range.md).

---

## Context

ADR-0012 bounded `normal_down_start_tolerance_us` to `[2,500, 5,000] µs`.
This follow-up recorded requests for more than 5 ms of jitter headroom, tighter
2 ms timing on low-jitter hosts, and removal of the redundant #287 reset button
in favor of the default indicator and steppers.

---

## Decision

### 1. Range Extension (#288)

We widen the permissible range for `normal_down_start_tolerance_us` across the domain, player,
tauri shell, and desktop bridge:

```text
Setting key: normal_down_start_tolerance_us
UI label:    "Late note tolerance"
Default:     2,500 µs (2.5 ms)
Minimum:     2,000 µs (2.0 ms)
Maximum:    10,000 µs (10.0 ms)
Step:          500 µs (0.5 ms)
```

Schema remained version **8**. Persisted values outside `[2_000, 10_000]` or
unaligned to 500 µs reset to 2,500 µs without clamping; the resolved value was
converted to QPC ticks and frozen for the prepared session.

### 2. Removal of Dedicated Reset Button (#287)

- The dedicated button `Reset to 2.5 ms` and its associated CSS selector `.late-note-tolerance-reset`
  are removed from `LateNoteToleranceControl.tsx` and `overlays.css`.
- The default indicator `· default 2.5 ms` remains visible in the header.
- Users navigate back to default via the standard `−` and `+` stepper buttons.

### 3. Tiered Guidance and Warning Copy

Guidance distinguished below-default, default/moderate, and high values, warned
that continuity trades against authored timing and following-note feasibility,
and stated that strict diagnostic playback ignored the setting.

---

## Empirical Qualification Evidence

Qualification was executed via `rt_handoff_bench --scope phase_extended_range` in release profile on
native Windows 11 hardware with QPC clock (frequency: 10 MHz). All 4 benchmark phases
passed 100% clean (`acceptance_clean: true`):

### Phase A: Deterministic Dense Matrix

Tested across 5 tolerance arms (`2.0`, `2.5`, `5.0`, `7.5`, `10.0 ms`), 7 inter-onset gaps
(`1, 2, 4, 6, 8, 10, 12 ms`), and 17 boundary wake offsets (`1.5` to `10.5 ms` in 0.5 ms increments):

| Arm (µs) | First Rescued | Second Success | Actually Overdue | False Backlogs | Timeline Rebases | Transport Anomalies | Net Successful Notes |
| :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **2,000** | 21 / 119 | 63 / 119 | 56 | 0 | 0 | 0 | 84 |
| **2,500** | 42 / 119 | 63 / 119 | 56 | 0 | 0 | 0 | 105 |
| **5,000** | 63 / 119 | 63 / 119 | 56 | 0 | 0 | 0 | 126 |
| **7,500** | 84 / 119 | 63 / 119 | 56 | 0 | 0 | 0 | 147 |
| **10,000** | 105 / 119 | 63 / 119 | 56 | 0 | 0 | 0 | **168** |

Across the arms, `UnobservedBacklog == actually_overdue == 56` with zero false
backlogs; net successful notes rose monotonically from 84 to 168. Timeline
rebases, chord splits, and transport anomalies were all zero.


### Phase B: Real-Wait Sequential Probe (Production Waiter)

Evaluated across 3 passes, 6 gaps (`2, 4, 6, 8, 10, 12 ms`), 30 sequences/gap/arm/pass = 2,700 total
sequences (5,400 boundary dispatches) on live Windows thread scheduling:

| Arm (µs) | First Sends | Second Sends | First Rescues | First FSW | Second UB / Overdue | Second FSW | PW | Timeline Rebases | Transport Anomalies | Post-Send Ready p99 | Net Successful Notes |
| :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **2,000** | 539 | 539 | 4 | 1 | 1 / 1 | 0 | 0 | 0 | 0 | 6 µs | 1,078 |
| **2,500** | 539 | 539 | 4 | 1 | 1 / 1 | 0 | 0 | 0 | 0 | 6 µs | 1,078 |
| **5,000** | 540 | 540 | 5 | 0 | 0 / 0 | 0 | 0 | 0 | 0 | 6 µs | 1,080 |
| **7,500** | 540 | 540 | 7 | 0 | 0 / 0 | 0 | 0 | 0 | 0 | 6 µs | 1,080 |
| **10,000** | 540 | 540 | 8 | 0 | 0 / 0 | 0 | 0 | 0 | 0 | 6 µs | 1,080 |

The 10 ms arm rescued 8 boundaries versus 7 at 7.5 ms, 5 at 5 ms, and 4 at
2.5/2 ms. Second-boundary UB matched overdue counts; second FSW, physical
window failures, rebases, and transport anomalies were zero. Post-send-ready
p99 was 6 µs for every arm; net successes were 1,078, 1,078, 1,080, 1,080,
and 1,080.


### Phase C: Sparse Benefit Probe (100 ms Gap)

Verified rescue regions and boundary drop enforcement:

- **2.0 ms arm:** Rescued offsets `<= 2.0 ms` (3 rescued); dropped offsets `>= 2.1 ms`.
- **2.5 ms arm:** Rescued offsets `<= 2.5 ms` (6 rescued); dropped offsets `>= 2.6 ms`.
- **5.0 ms arm:** Rescued offsets `<= 5.0 ms` (9 rescued); dropped offsets `>= 5.1 ms`.
- **7.5 ms arm:** Rescued offsets `<= 7.5 ms` (12 rescued); dropped offsets `>= 7.6 ms`.
- **10.0 ms arm:** Rescued offsets `<= 10.0 ms` (15 rescued); dropped offsets `10.1 ms` and `10.5 ms`.
- Proved exact monotonic widening of the rescue envelope and guaranteed fail-closed drop past the ceiling.

### Phase D: Same-Key Retrigger Safety Matrix

Evaluated physical floor feasibility and non-bypass across 70 combinations of inter-onset gaps
(`5, 6, 8, 10, 12, 16, 20 ms`) with on-time and delayed Up completions:

- **65 / 70 cases** were physically infeasible (gap < min hold + min release gap of ~17.17 ms under 60 FPS)
  and correctly classified as `PhysicalWindowExpired`.
- **5 / 70 cases** were physically feasible (20 ms gap under on-time Up) and successfully dispatched.
- Under delayed Up completion, physical release floors dynamically advanced by actual completion QPC,
  ensuring minimum hold and release separation was never violated.
- Verified: tolerance never overrides physical timing guard floors; zero timeline rebases; zero burst catch-up.

---

## Invariants Retained

The setting-era design used a non-additive cutoff, kept physical feasibility
ahead of rescue, excluded strict mode, and never retried, rebased, or caught up
missed notes. It recorded telemetry schema 16. The numerical qualification
results above remain the historical evidence; current runtime policy is in
ADR-0017/0018.
