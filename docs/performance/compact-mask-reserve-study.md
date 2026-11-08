# Issue #640 — compiled-mask INPUT arena reserve: isolated offline A/B

**Research only.** This change does not modify the shipping inline prepared
entry, coordinator/planner, real-time worker, Win32 sender, or `SendInput`.
It adds a third **test-support-only** process-probe mode named
`compact-masks`. Existing `inline` and `compact` modes and the accepted
[#638](https://github.com/pumni/Sky-Auto-Player/pull/638) runner stay as-is.

## Specific, falsifiable hypothesis

The existing direct-compact builder initially reserves two INPUT slots per
*authored batch*, independent of chord width. Once a chord has more INPUTs
than that hint, the frozen arena's `Vec<INPUT>` grows before freeze. These
extra reallocations and the retained excess capacity may contribute to the
negative width-15 results in the full Windows A/B. **No measured attribution
has been established yet.**

The alternate builder precomputes a reserve **hint** from compiled physical
packet `(up_mask | down_mask).count_ones()`, summed with checked arithmetic.
This scan happens inside the measured prepare/reset region. The normal
coordinator planner and materialized `PreparedPhysicalPacket` remain the
source of truth for the actual INPUT bytes. If planned events exceed the
hint, the arena still grows safely before freeze. There is no second
concurrently held prepared representation and no live sending.

The original 512-MiB inline-stream admission check, packet-index bounds,
freeze invariants, 15-key limit and cancellation checkpoints are unchanged.

## What an exact initial reserve cannot fix

Windows x64 sizes already established in the prior accepted test-support study:

| Retained physical representation | Formula | Width 1 | Width 14 | Width 15 |
| --- | ---: | ---: | ---: | ---: |
| Inline entry | 1,392 B | 1,392 | 1,392 | 1,392 |
| Direct compact entry + descriptor + used INPUTs | 792 + 12 + 40 × width | 844 | 1,364 | 1,404 |
| Difference compact − inline | 40 × width − 588 | −548 | −28 | **+12** |

These are **physical-entry component bytes**, excluding vector capacity slack,
metadata entries, coordinator/other allocations, process heap fragmentation and
OS memory counters. Width 15 has a small intrinsic storage disadvantage
**even with perfect INPUT arena reservation**; this hypothesis alone cannot
guarantee a width-15 memory win or improve execution time.

## Independent comparison design

Use new `scripts/qualify_compact_mask_hint_ab.ps1` on a clean detached
Windows validation worktree pinned to the draft PR's **exact HEAD and tree**.
It reuses the qualified manifest-controlled process probe and corpus control,
but compares:

- **A:** `compact` — untouched #638 two-INPUT-per-batch reserve.
- **B:** `compact-masks` — compiled-mask-count reserve hint.

One executable child holds only one of these variants; interleave AB/BA
(3 AB and 2 BA per workload/stage with five matched pairs).
The newly exposed structural counters report physical/metadata entries,
INPUT used events/bytes, allocated arena capacity events/bytes,
descriptor/entry reserved bytes, total reserved stream bytes, and the
same Windows PrivateUsage and WorkingSetSize, allocation calls and
prepare/reset duration. Both modes undergo physical/metadata/INPUT histogram
and rebuilt-coordinator parity checks, source SHA, and raw JSON/CSV accounting.
Nothing is compared in a single process's memory measurement.

- **Smoke:** three corpus songs + synthetic generations 100 and 1,000 at
  widths **1, 7, 14, 15**: **11 workloads × 2 stages × 5 pairs × 2 child
  processes = 220 children, 110 pairs**.
- **Full only after smoke and CI:** all 110 prepared songs + 100, 1,000 and
  10,000 generations at widths **1, 7, 14, 15**:
  **122 workloads, 2,440 children, 1,220 pairs**.

Mandatory summary per workload/stage: median/min/max paired
`compact-masks minus compact` prepare/reset, current private commit, total
working set, Rust allocation/reallocation calls, **actual arena reserve
versus used INPUT bytes**, order split, negative cases, and variance. Full
means all workloads and all five pairs; do not cherry-pick results.

## Acceptance / failure criteria

- Before running smoke, Windows unit tests must pass for all 1..15 widths,
  stale-up metadata and maximal mixed 7-Up/8-Down Up-prefix recovery. Probe
  regression tests also build the **actual** legacy and mask-reserved compact
  owners at width 1/7/14/15 and generations 100/1,000/10,000.
- The 112/110/2 corpus identity and exact historical 16-bin histogram
  `[0,63324,23810,14698,4958,284,42,2,0,0,0,0,0,0,0,0]` must match.
- If any pair fails packet/metadata/INPUT histogram/reset identity, or a
  child aborts, **BLOCKED**, no performance conclusion.
- Do **not** merge a performance-claim PR based only on synthetic dimensions.
  Distinguish structural reserved capacity, process private committed bytes,
  total working set (shared pages may be included), lifetime process peak,
  and Rust-only allocation counters. Cold is not OS cold cache.
- The algorithm is strictly offline; even a measured win would **not**
  authorize production compact dispatch or `SendInput`. Dense-width
  regression issue #640 and separate realtime/receiver safety gates remain.

## Current status

**Unmeasured hypothesis / draft experiment.** No Windows native A/B results
for `compact-masks` exist yet. The earlier #638 2,320-process full study
remains the fixed legacy compact vs inline baseline, not evidence for this
new capacity hint.
