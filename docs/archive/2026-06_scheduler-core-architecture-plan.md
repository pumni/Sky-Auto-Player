> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: kế hoạch nâng cấp scheduler lõi đã thực thi.

# Scheduler Core Architecture Plan

Date: 2026-06-04

This plan exists because the O10 audit exposed a deeper issue than a bad timing number:
the scheduler had fields whose names implied hard floors, while the executable schedule could not
always enforce those floors. Tuning values before fixing that contract produces false confidence.

## Current Diagnosis

The scheduler currently mixes five concerns in one pass:

1. parsing/normalising note keys;
2. mapping notes to physical scan codes;
3. deciding per-note hold/release timing;
4. grouping simultaneous key events;
5. producing diagnostics.

That made two problems hard to see:

- Same-key duplicates at the exact same timestamp were counted as impossible repeats before final
  event grouping deduped them. They were data/chord duplicates, not re-trigger attempts.
- `repeat_release_gap_us` looked like an enforced floor, but degraded scheduling could only change a
  hold inside the compression band:

  ```text
  min_hold_us + repeat_gap_us <= same_key_interval < hold_us + repeat_gap_us
  ```

  Current frame-aware profiles materialised `hold_us == min_hold_us`, so that band was empty. In
  those profiles, repeat gap changed diagnostics/strict aborts, not degraded playback. The field has
  now been removed from profile/CLI/runtime policy semantics; same-key feasibility is governed by
  `min_hold_us`.

## Target Contract

The scheduler should be a pure constraint planner with explicit inputs and outputs:

- Input: a normalised note-intent timeline.
- Output: an executable key-state timeline plus diagnostics that describe what was actually enforced.
- No profile field may be documented as a playback lever unless it can change the emitted timeline or
  explicitly reject an infeasible timeline.

## Proposed Pipeline

### Stage 1 — Note Intent Normalisation

Build a `ScheduledNoteIntent` list before any same-key analysis:

- `at_us`
- `scan_code`
- `note_key`
- `source_indices`

Rules:

- Same `at_us + scan_code` is one intent with multiple `source_indices`.
- Exact same-timestamp duplicates are not repeats.
- Same timestamp, different scan codes remain a chord.
- Tempo scaling happens before normalisation.

Acceptance:

- Duplicate same-key same-time notes produce one down/up pair.
- They do not increment impossible repeat counters.
- Audit tools and scheduler use the same normalisation rule.

### Stage 2 — Per-Key Lane Planning

Split intents by scan code and plan each key lane independently before global event grouping.

For each note in a lane:

```text
down_at = intent.at_us
target_hold = policy.hold_us
min_down = policy.min_hold_us
next_down = next lane intent, if any
```

Feasibility:

```text
available = next_down - down_at
required_cycle = min_down
```

Cases:

- No next same key: hold = target_hold.
- `available >= target_hold`: hold = target_hold.
- `required_cycle <= available < target_hold`: compress hold to `available`.
- `available < required_cycle`: infeasible.

### Stage 3 — Explicit Infeasible Policy

Do not silently pretend an infeasible same-key overlap was fixed.

Recommended production behavior:

- `strict`: reject before playback and recommend a tempo reduction.
- `degraded`: preserve `min_down` so the current note has a chance to register, emit the next down on
  time, and report the overlap.

Important: degraded mode may intentionally create an active-overlap schedule when authored same-key
spacing is below `min_hold_us`. Diagnostics must say that plainly.

### Stage 4 — Global Event Grouping

Only after lane planning:

- group same-time same-kind scan codes into `KeyAction`;
- sort with deterministic priority;
- preserve up-before-down at identical timestamps for the same scan code if such a degraded case is
  intentionally allowed.

Stage 4 should not hide semantic errors from earlier stages. If it dedupes something, Stage 1 should
already have known it.

### Stage 5 — Actual-Schedule Validation

Validation must distinguish:

- requested floor;
- actual hold;
- actual same-key up gap;
- infeasible repeats;
- schedule-changing compression.

The validation report should not treat a requested value as enforced unless the emitted action times
prove it.

## Policy Shape Decision

Before tuning numbers, choose one of these:

### Option A — Remove Repeat Gap From Profile Semantics

Keep production profiles driven by `min_hold` only. For same-key repeats, rely on authored spacing and
tempo validation. Use strict/degraded diagnostics for intervals below min_hold.

Best fit if corpus binding stays zero.

### Option B — Keep Repeat Gap As Strict Validation Only

Rename/document it as a same-key feasibility threshold, not a degraded playback lever. In strict mode,
reject intervals below `min_hold + up_gap`; in degraded mode, report but preserve min_hold.

Best fit if the game mechanism is real but production songs rarely bind.

### Option C — Reintroduce A Real Compression Band

Make `hold > min_hold` by design, so repeat gap can actually compress normal holds. This restores
reachability, but it also makes profiles more complex and can make single notes longer/mushier.

Only choose this if O10.4A proves the game mechanism matters and O10.4B/O3 shows real songs or remote
playback benefit from schedule-changing compression.

## Recommendation

Implemented decision: **Option A**.

Reason:

- It stops pretending degraded playback enforces an up-gap floor.
- It avoids reintroducing a large `hold > min_hold` body just to make a rarely binding field reachable.
- It fits current corpus evidence: real songs have no positive repeat pressure after normalisation.
- Extreme/generated material is still guarded by `min_hold_us` feasibility and strict/degraded policy.

## Refactor Phases

### Phase 0 — Normalise Intent

Status: implemented.

- Deduplicate same `at_us + scan_code` before same-key repeat analysis.
- Update audit tool to share the same rule.
- Add regression tests for duplicate same-key same-time notes.

### Phase 1 — Extract Planner Types

Status: minimal extraction implemented.

Added/kept small frozen dataclasses:

- `ScheduledNoteDraft` as the current note-intent model;
- `PlannedKeyHold` for executable hold decisions.

`KeyLaneNote`/`RepeatFeasibility` remain optional future extractions if Phase 4 metrics need richer
per-lane reporting. Public `build_key_actions()` behavior stays stable.

### Phase 2 — Make Feasibility Explicit

Status: implemented for same-key hold planning.

Move same-key interval math into a pure helper:

```python
plan_same_key_hold(
    effective_delta_us: int | None,
    target_hold_us: int,
    min_hold_us: int,
) -> PlannedKeyHold
```

Tests must cover:

- no next same key;
- fully feasible target hold;
- same-key hold compression;
- impossible degraded;
- impossible strict via `build_key_actions()` policy behavior.

### Phase 3 — Rename Or Remove Repeat-Gap Semantics

Status: implemented with Option A.

- Removed profile repeat-gap fields.
- Removed CLI repeat-gap override.
- Removed runtime policy materialisation of repeat gap.
- Kept `scripts/audit_repeat_gap.py` as a counterfactual audit tool only.

### Phase 4 — Metrics And Audit

Status: implemented.

Schedule metadata now includes:

- `deduplicated_note_count`
- `duplicate_note_count`
- `compressed_holds`
- `same_key_compressed_holds`
- `impossible_same_key_repeats` / `infeasible_same_key_repeats`
- `shortest_same_key_interval_us`
- `min_same_key_up_gap_us`

This makes future timing audits mechanical instead of interpretive.

### Phase 5 — Contract Hardening

Status: implemented.

- Keep legacy `impossible_same_key_repeats` as a compatibility alias.
- Prefer `infeasible_same_key_repeats` in telemetry/calibration wording.
- Update scheduler/analyzer warnings to describe the real invariant:
  authored same-key interval below `min_hold_us` creates an overlap in degraded mode.
- Keep `risky_same_key_repeats` as the compatibility counter for same-key hold compression.

## Acceptance Gates

- `uv run pytest` passes.
- Scheduler remains pure and backend-independent.
- Real-song corpus audit reports zero fake duplicate repeats.
- A profile field is only called a lever if a test shows changing it changes `KeyAction` times.
- Any infeasible repeat diagnostic states which invariant was preserved and which was violated.
