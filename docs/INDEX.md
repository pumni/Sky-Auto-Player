# Sky Auto Player Documentation Map

This index defines the structure and hierarchy of truth for the Sky Auto Player project documentation.

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
* [timing-principles.md](timing-principles.md) — Source of truth for timing design, same-key feasibility limits (pure `min_hold` floor, no fixed margin), the completion-anchor contract, and the adaptive-lead/floor interaction.
* [rt-dispatch-architecture.md](rt-dispatch-architecture.md) — Current RT dispatch design after the 2026-06 decomposition: DispatchLoop/Supervisor/HybridWaitStrategy, adaptive lead (onset = dispatch completion), priority ladder, event-driven waits, production defaults and kill switches.
* [architecture.md](architecture.md) — Explains the 4-layer DDD codebase design, playback dispatch pipeline (MMCSS + waitable timer + timer-guard), and input hardening.
* [timing-profile-frame-model.md](timing-profile-frame-model.md) — Pure frame-relative formulas and default profiles (`local_precise`, `balanced`, `audience_safe`).
* [perf-baselines/2026-06-baseline.md](perf-baselines/2026-06-baseline.md) — Pipeline CPU baselines and post-optimization gate numbers.
* [distribution-and-update.md](distribution-and-update.md) — Distribution model, update architecture, and release contracts for Sky Auto Player (tracks the `pyproject.toml` `[project].version`, currently 2.4.1).

---

## 2. Active References & Experiments
* [plan/2026-07-23_dispatch-core-correctness-hardening-plan.md](plan/2026-07-23_dispatch-core-correctness-hardening-plan.md) — **Proposed (not started).** Consumes [dispatch-core-code-audit-2026-07-22.md](dispatch-core-code-audit-2026-07-22.md): AI-safe phased fixes for H1 focus-first-down gate, H3 degraded command wait, H4 supervisor join, H5 event ctypes prototypes, H6 boundary validation, M1 warmup-vs-pending, M2 lead snapshot honesty, M4/M5 wiring. **H2 rejected** (completion-anchor + equality same-key drop is design-correct; see plan §3.2). Order: correctness/safety before timing hygiene before resource thrift. **Do not merge phases; every fix starts with a failing test.**
* [2026-07-18_accuracy-refinement-and-fps-ux-plan.md](2026-07-18_accuracy-refinement-and-fps-ux-plan.md) — **Implemented (Phases 0–6 shipped, 2026-07-18).** Follow-on to the core-send overhaul: FPS + timing-profile modal guidance with shared `timing_guidance.py` (user must match Sky client FPS; **no game auto-detect** — security); Phase C residual closed — Textual play path surfaces `fps_play_advisory` via `notify()` and console path uses shared function; budget-aware idle-gap core warmup raised from 50 → 200 µs with 500 µs max cap; `_probe_timer_wake_error` docstring updated (mid-song re-probe is wired, not removed); degradation surface verified for `event_wait_degraded_to_polled` / `partial_note_on_count` in telemetry summary. Phase J (game-observed) still gated. **Priority: accuracy over CPU thrift; advisories non-blocking.**
* [2026-07-18_core-send-accuracy-full-overhaul-plan.md](2026-07-18_core-send-accuracy-full-overhaul-plan.md) — **Implemented (Phases A–I + K shipped, 2026-07-18).** Full core send-path overhaul: metric honesty + `timing_semantics` schema hardening (Phase B); FPS/`sub_60fps_frame_notes` schema + tests (Phase C partial — **residual advisory UX continued in** [accuracy-refinement-and-fps-ux-plan](2026-07-18_accuracy-refinement-and-fps-ux-plan.md)); cross-session EMA lead cache for cold-start (Phase D); idle-gap core warmup hook (Phase E); `min_hold_margin` device-cache productization + `runtime_options` transparency (Phase F); doctor UIPI text + focus-before-start regression test (Phase G); mid-song spin re-probe with kill switch + telemetry (Phase H); hot-path retry verified clean (Phase I). Phase J (game-observed WASAPI) gated on human approval. **Priority: timestamp/registration accuracy over default CPU thrift.**
* [2026-07_sendinput-lifecycle-and-timestamp-fidelity-plan.md](2026-07_sendinput-lifecycle-and-timestamp-fidelity-plan.md) — **Implemented (Phases 0–4 shipped, 2026-07-16).** Unified SendInput abort lifecycle (`_abort_input_safe` + focus dual-release), pre-down focus gate via runtime `FocusSignal`, partial-send outcome hygiene (`partial_note_on`), and timestamp fidelity verification (lead cache / cold-start). **Residual:** Phase 5 §5.1 doctor preflight (shipped as Phase G of the 2026-07-18 overhaul) and optional Phase 6 WASAPI measurement — absorbed by the 2026-07-18 core-send accuracy overhaul plan (Phases G / J). See the plan's §2.4 status snapshot and §11.1 as-built decisions for divergences. Supersedes archive keyboard plan focus strategy where it conflicts.
* [2026-07-18_distribution-mpv-pattern-plan.md](2026-07-18_distribution-mpv-pattern-plan.md) — **Implemented (Phases 0, 1, 2, 3, 5, 6, 8 shipped in 2.4.0, 2026-07-18).** Cuts the in-app auto-apply path (notify-only banner); external `updater.bat` + `installer/updater.ps1` apply the swap with HTTPS host allow-list, SHA256-verify-before-mutate, transactional copy + rollback, preserve-list for `config.json` + `songs/`, and a process guard; `build_app --manifest` emits `MANIFEST.json`; tag-triggered `release.yml` enforces the git-tag ↔ `pyproject.toml` version lock and uploads the release triple (`Sky-Player-v<ver>.zip` + `.sha256` + `MANIFEST.json`). **Phases 4 (optional `.skysheet` installer) and 7 (winget community channel) remain deferred.** Normative contract: `docs/distribution-and-update.md` and the `installer/updater.ps1` header comment.
* [archive/2026-07_ram-memory-hygiene-plan.md](archive/2026-07_ram-memory-hygiene-plan.md) — RAM/telemetry hygiene (surgical; not scheduler/SendInput rewrite).
* [archive/core-dispatch-hygiene-and-tail-latency-plan.md](archive/core-dispatch-hygiene-and-tail-latency-plan.md) — Proposed plan: clean up dispatch loops and Win32 backends, improve typing to Python 3.14 best practices, and run tail latency benchmarking under UI GIL contention.
* [archive/main-path-cleanup-and-build-quality-plan.md](archive/main-path-cleanup-and-build-quality-plan.md) — Proposed plan: make the GIL switch-interval knob self-aware on free-threaded 3.14, externalize env tuning as forker presets, and tighten build quality (assert audit + `--optimize`, excludes). Hygiene/build only — NOT send-path perf (that is proven optimal).
* [archive/2026-06_wasapi-loopback-measurement-plan.md](archive/2026-06_wasapi-loopback-measurement-plan.md) — After-send WASAPI loopback measurement (validation companion to Phase 6 of the SendInput lifecycle plan).
* [2026-07_core-dispatch-refactor-and-isolation-plan.md](2026-07_core-dispatch-refactor-and-isolation-plan.md) — **Implemented (Phases 0–6).** Phased refactor from the 2026-07-16 core review. Phase 0 harness + Phase 1 correctness (A1/A2/A6a/A6b) + Phase 2 hot-path (A3/A4/unfocused hook) + Phase 3 CPU floors (A5) + Phase 4 structural isolation (`orchestration/core/` package + boundary test) + Phase 5 Rust-plan alignment + Phase 6 docs. See plan §12 for the as-built table and divergence notes.
* [rust-migration-plan.md](rust-migration-plan.md) — Proposed plan: migrate the real-time dispatch hot path (send/wait/runtime) from Python ctypes into a dedicated Rust dispatch worker via PyO3, keeping Python for orchestration/UI only. **Unblocked** — the core-dispatch refactor (above) landed and Phase 5 aligned this plan to the shipped seam: §8 emit honors musical no-retry (I3), §5 uses the single-interval pause SM, §10 states drop-oldest/FIFO, §Phase 2 pins single-source focus ownership, §1.3 maps all core invariants I1–I9.
* [timing-experiments.md](timing-experiments.md) — Holds only open infrastructure investigation items:
  * **O10.5:** Global sleep policy benchmark (`spin_threshold_us`).
  * **O10.6:** Focus restore grace safety margin (`focus_restore_grace_us`).

---

## 3. Historical Archives (`docs/archive/`)
These files contain completed refactor plans, historic audits, and legacy design documents. They are read-only and include warning stamps specifying discrepancies with the current codebase.
* `keyboard-reliability-and-safety-plan.md` — Older deliverability/safety proposal; **focus KEYUP strategy and partial-send notes partially superseded** by `2026-07_sendinput-lifecycle-and-timestamp-fidelity-plan.md` (§2.3 dual-release, G5 no late note-on retry). Watchdog/hotkey ideas may still be useful historical context.
* `2026-06_rt-pipeline-extreme-optimization-plan.md` — Completed 7-phase RT pipeline optimization plan (adaptive lead, priority ladder, event waits, engine decomposition) with outcome stamps.
* `2026-06_background-worker-lifecycle-refactor-brief.md` — Implementation brief for making picker background worker ownership, cancellation, and shutdown deterministic before playback.
* `2026-06_background-worker-lifecycle-hardening-plan.md` — Hardening plan for explicit lifecycle state, cleanup failure policy, structured lifecycle evidence, and future worker drift guards.
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
* `2026-06_ui-overhaul-textual-plan.md` — Textual UI plan (Live).
* `2026-06_remove-classic-ui-plan.md` — Plan for removing the legacy classic UI (Completed).
* `2026-06_timing-experiments.md` — Full history of experiments (O1 - O10.4).
* `2026-06_docs-cleanup-plan.md` — Meta-plan for documentation cleanup.
