# Sky Player Documentation Map

This index defines the structure and hierarchy of truth for the Sky Player project documentation.

## 0. Hierarchy of Truth (Evidence Hierarchy)
1. **Observed Game Behavior** (onsets/audio captured in-game) — wins over everything.
2. **Deterministic Telemetry** (coordinator/scheduler simulator) — wins over "experience/intuition".
3. **Current Codebase** (`src/`) — wins over description in any document.
4. **Documentation** — only interpretive; if a document conflicts with 1, 2, or 3, it is OUTDATED/INCORRECT and must be updated.

> [!NOTE]
> [AGENTS.md](../AGENTS.md) remains the single source of truth for project rules.

---

## 1. Canonical Documents
These files represent the current system state and contracts:
* [timing-principles.md](timing-principles.md) — Source of truth for timing design, same-key feasibility limits (pure `min_hold` floor, no fixed margin), and the completion-anchor contract.
* [architecture.md](architecture.md) — Explains the 4-layer DDD codebase design, playback dispatch pipeline (MMCSS + waitable timer + timer-guard), and input hardening.
* [timing-profile-frame-model.md](timing-profile-frame-model.md) — Pure frame-relative formulas and default profiles (`local_precise`, `balanced`, `audience_safe`).

---

## 2. Active References & Experiments
* [2026-06_background-worker-lifecycle-refactor-brief.md](2026-06_background-worker-lifecycle-refactor-brief.md) — Active implementation brief for making picker background worker ownership, cancellation, and shutdown deterministic before playback.
* [2026-06_background-worker-lifecycle-hardening-plan.md](2026-06_background-worker-lifecycle-hardening-plan.md) — Follow-up hardening plan for explicit lifecycle state, cleanup failure policy, structured lifecycle evidence, and future worker drift guards.
* [timing-experiments.md](timing-experiments.md) — Holds only open infrastructure investigation items:
  * **O10.5:** Global sleep policy benchmark (`spin_threshold_us`).
  * **O10.6:** Focus restore grace safety margin (`focus_restore_grace_us`).

---

## 3. Historical Archives (`docs/archive/`)
These files contain completed refactor plans, historic audits, and legacy design documents. They are read-only and include warning stamps specifying discrepancies with the current codebase.
* `2026-06_completion-anchor-refactor-plan.md` — Implementation plan for completion-anchor.
* `2026-06_runtime-hold-refactor-plan.md` — Historical scheduler anchor and runtime hold discussion.
* `2026-06_timing-architecture-audit.md` — Audit that justified the removal of dead knobs.
* `2026-06_floor-removal-three-profile-plan.md` — Decision details on moving to pure frame holds.
* `2026-06_hold-min-hold-unification-plan.md` — Deriving normal hold directly from min_hold.
* `2026-06_down-hold-up-scheduling-audit.md` — Legacy audit of scheduler state transitions.
* `2026-06_scheduler-core-architecture-plan.md` — Early architecture ideas for scheduler changes.
* `2026-06_realtime-sender-thread-refactor-plan.md` — Introduction of the real-time sender thread.
* `2026-06_play-input-architecture-refactor-plan.md` — Original input dispatch flow audit.
* `2026-06_playback-flow-hardening-plan.md` — Initial hardening proposal.
* `2026-06_playback-input-investigation-2026-06-06.md` — Investigation of missing notes and FPS toggles (findings folded into Principles §6).
* `2026-06_timing-guard-binding-audit.md` — Legacy audit of timing guard threads.
* `2026-06_ui-overhaul-textual-plan.md` — Textual UI plan (Live). *Classic UI removal pending.*
* `2026-06_timing-experiments.md` — Full history of experiments (O1 - O10.4).
* `2026-06_docs-cleanup-plan.md` — Meta-plan for documentation cleanup.
