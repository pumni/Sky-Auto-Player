# ADR-0013: Extended Normal-Playback Down Continuity Range (2.0–10.0 ms) and Reset Button Removal

Status: Accepted.

Supersedes: The `2,500–5,000 µs` bounds of [ADR-0012](ADR-0012-user-configurable-normal-down-continuity.md),
and documents the removal of the dedicated "Reset to 2.5 ms" button in #287.
All invariants from ADR-0010, ADR-0011, and ADR-0012 (non-additive formula, strict-mode sender
cutoff, immutable authored timeline, session freezing, physical feasibility precedence, and telemetry
schema) remain fully in force.

Parent work orders: [#287](https://github.com/pumni/Sky-Auto-Player/issues/287) and [#288](https://github.com/pumni/Sky-Auto-Player/issues/288).

Baseline commit: `c337412a15ec80cea76a86fe0c5d1f7b1c269e8c`.

---

## Context

In [ADR-0012](ADR-0012-user-configurable-normal-down-continuity.md), `normal_down_start_tolerance_us`
was exposed as an Advanced setting provisionally bounded to `[2,500, 5,000] µs` with default `2,500 µs`.
Experience across varied Windows environments demonstrated two user requirements:

1. **Jitter Headroom under Load:** Certain host hardware configurations and background CPU
   spikes produce Windows timer wake jitter exceeding 5.0 ms. Under the previous 5.0 ms ceiling,
   these boundaries were dropped as `FinalSenderWindowExpired` even though physical hold and release
   floors were completely feasible.
2. **Rhythmic Tightening:** Users with low-jitter environments requested the option to tighten
   continuity tolerance down to 2.0 ms to enforce tighter timing discipline closer to authored targets.
3. **UI Streamlining (#287):** The dedicated "Reset to 2.5 ms" button added visual clutter and
   redundancy alongside the persistent default indicator (`· default 2.5 ms`) and stepper controls (`− / +`).

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

- **Persistence Schema:** Remains schema version **8**. Because the schema already validates and
  normalizes invalid persisted values to the 2,500 µs default, widening the boundary requires no schema
  increment. Persisted values outside `[2_000, 10_000]` or unaligned to 500 µs continue to reset to 2,500 µs
  fail-closed (no clamping).
- **Session Freezing:** Unchanged. The resolved tolerance is converted to QPC ticks and frozen for
  the lifetime of the prepared session. In-flight sessions never observe mid-song mutations.

### 2. Removal of Dedicated Reset Button (#287)

- The dedicated button `Reset to 2.5 ms` and its associated CSS selector `.late-note-tolerance-reset`
  are removed from `LateNoteToleranceControl.tsx` and `overlays.css`.
- The default indicator `· default 2.5 ms` remains visible in the header.
- Users navigate back to default via the standard `−` and `+` stepper buttons.

### 3. Tiered Guidance and Warning Copy

To ensure safe user operation across the wider range without using misleading or forbidden terminology,
the control presents dynamic guidance and tiered warnings based on the active value:

- **Below Default (`< 2.5 ms`):**
  *"Lower values keep notes closer to authored timing, but Windows wake jitter may cause more late notes to be dropped."*
- **Default and Above (`>= 2.5 ms`):**
  *"If notes are still dropped on fast chords under load, increase this step-by-step. Rescued notes may play slightly later than their authored time."*
- **Moderate Tolerance Warning (`> 2.5 ms && <= 5.0 ms`):**
  *"Values above 2.5 ms increase late-note continuity at the cost of authored timing accuracy under load."*
- **High Tolerance Warning (`> 5.0 ms`):**
  *"High tolerance can rescue very late notes, but in dense passages it may cause following authored notes to become stale or physically infeasible. Use only when lower values still drop notes."*
- **Strict Mode Note:**
  *"Applies only to normal playback Down key events. Strict timing diagnostic playback ignores this tolerance."*

---

## Empirical Qualification Evidence

Qualification was executed via `rt_handoff_bench --scope phase_extended_range` in release profile on
native Windows 11 hardware with High Precision Event Timer / QPC clock. All 4 benchmark phases
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

- Across all arms, `UnobservedBacklog == actually_overdue == 56` with zero false backlogs.
- Net successful notes increased strictly monotonically with configured tolerance.
- Zero timeline rebases, zero chord integrity splits, zero transport anomalies.

### Phase B: Real-Wait Sequential Probe (Production Waiter)

Evaluated across 3 passes, 6 gaps (`2, 4, 6, 8, 10, 12 ms`), 30 sequences/gap/arm/pass = 2,700 total
sequences (5,400 boundary dispatches):

| Arm (µs) | First Rescued | First FSW | Second Success | Second FSW | Post-Send Ready p99 | Net Successful Notes |
| :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **2,500** | 2 | 26 | 531 | 0 | 54 µs | 1,050 |
| **5,000** | 10 | 11 | 531 | 0 | 50 µs | 1,058 |
| **10,000** | **16** | **2** | **532** | **0** | **49 µs** | **1,070** |

- The 10.0 ms arm demonstrated superior rescue performance (16 rescues vs 10 on 5.0 ms and 2 on 2.5 ms).
- Second boundaries suffered zero Final Sender Window expirations across all arms.
- Downstream post-send readiness latency remained virtually unchanged (p99 of 49 µs vs 50 µs vs 54 µs).

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

1. **Non-Additive Continuity Formula:**
   `normal_sender_cutoff = max(physical_latest_down_start, authored_target + normal_down_start_tolerance)`
2. **Physical Feasibility Precedence:**
   Packets whose physical floor exceeds `physical_latest_down_start` fail closed as `PhysicalWindowExpired`.
3. **Strict Playback Invariance:**
   Strict playback diagnostic mode uses `physical_latest_down_start` and ignores configured tolerance.
4. **Authored Timeline Immutability:**
   Missed notes are never caught up or retried; timeline is never rebased.
5. **Telemetry Schema Stability:**
   Remains schema version 16 with existing metrics tracking rescued notes.
