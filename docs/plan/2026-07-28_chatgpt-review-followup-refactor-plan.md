# ChatGPT-Review Followup Refactor Plan (2026-07-28)

**Status: PROPOSED, under implementation (2026-07-28).** Consumes the
ChatGPT-web static review of commit `8a47656554b7e4daced61a72e24461834a8737ff`,
which is accurate at the claim level (verified against current code; see the
in-line "Verify" notes). Normative docs (P2) win over this file if they
disagree. P0 surfaces (SendInput-only, ctypes-only-in-platform, scheduler
purity) untouched by every patch below.

## 0. Scope, boundaries, ordering

* No timing change in any patch. Each patch is correctness, observability,
  architecture, or dead-code cleanup — never a hot-path perf change.
* P0 untouched: SendInput-only, no game tampering, no new input mechanism.
* Each patch is one logical change; one Conventional Commit per patch.
* `Sky-Auto-Player.spec` and `src/build_app.py` are ask-first surfaces
  (AGENTS.md `Boundaries → Ask first`) — patch C1 requires user approval
  before merge.
* Normative doc updates ride in the same commit as the behaviour change
  when documented behaviour changes (P2: `docs/architecture.md` for layer
  fixes; spec comment for C1; `docs/rt-dispatch-architecture.md` if metric
  contract text moves).

### Order

| # | Patch | Loại | Ask-first? | Priority |
|---|---|---|---|---|
| A4 | `pre_send_spin_us` effective threshold | correctness (telemetry) | no | P1 |
| A3 | DirectProgressSink batch + drop legacy `update_counters` | correctness (observability) | no | P1 |
| A2 | HUD/counters use `visible_lateness_us` for onsets | correctness (display metric) | no | P1 |
| T1 | Rewrite `measure_dispatch_tail.py` for 3.14t + pytest-benchmark | tooling | no | P1 (parallel) |
| C1 | Spec comment fix + frozen smoke `__debug__` verify | correctness (build contract) | **yes (spec)** | P1 |
| B1 | Calibration loader → infrastructure | architecture (layer) | no | P2 |
| B2 | `SleepPolicy` out of `domain` | architecture (layer) | no | P2 |
| C2 | Drop `DispatchLoop.enable_event_wait` field (keep kwarg compat) | dead-code | no | P2 |
| C3 | `Queue.empty()` check-then-act simplification | simplify | no | P3 |

## 1. Patch A4 — `pre_send_spin_us` uses effective spin threshold

**File:** `src/sky_music/orchestration/core/loop.py:1136`

**Status after static verification: PHANTOM — DOWNGRADED.** Static trace shows the
`self._wait_spin_start_us` value set at loop.py:1136 is overwritten by loop.py:1087
(`elapsed_us >= target_elapsed_us`) or loop.py:1100 (`remaining_us <= effective_spin_threshold`)
before any `_execute_action` reads it. The only consumers of `_wait_spin_start_us` are
`pre_send_spin_us` (loop.py:477) and `idle_gap_us` (loop.py:478) — both computed inside
`_execute_action`, which runs after `_wait_until_runtime_deadline` returns None (always via
either line 1087 or line 1102 on the success path). The line 1136 value therefore never
survives into a record in any reachable production path. Empirical baseline
(`tests/test_send_warmup_telemetry.py:test_second_onset_after_gap_has_idle_gap`) is
consistent with this — already asserts `idle_gap_us > 5000` for the cold-path onset.

**Action:** skip the 1-line fix; reopen only if a controlled test surfaces a code
path where the line 1136 value reaches `_execute_action` without being overwritten
(e.g. an event-mode command-abort path that drops a record without dispatching).
The misleading comment at loop.py:1133-1135 stays as candidate cleanup for a later
doc/consistency pass — not blocking, not telemetry-changing.

## 1b. Patch A3 — DirectProgressSink restores counter delivery + drop legacy `update_counters`

**Files:**
`src/sky_music/orchestration/playback_supervisor.py:64-101`,
`scripts/bench_dispatch_fidelity.py:84-88`,
`tests/test_dispatch_fidelity_refactor.py:73-83`,
`tests/test_phase1_correctness.py:320-328`,
`tests/test_runtime_dispatch.py:94-116`,
`tests/test_threaded_dispatch.py:117-146`.
New: `tests/test_direct_progress_sink.py`.

**Bug:** `DirectProgressSink.publish` (supervisor.py:80-92) drops
`counters` (`_ = counters`). Threaded path (supervisor.py:548-549) calls
`update_counters_batch(snapshot.counters)`. Direct mode therefore loses the
entire observability contract.

**Plus (cleanup):** `update_counters(lateness_us, kind)` (single-call) at
supervisor.py:98-100 has no production caller (per full grep across
`src/`). `_NullProgressSink.update_counters` in `bench_dispatch_fidelity.py:84-87`
is a no-op for legacy support only. Test sinks at the four test files
above defined `update_counters` only to silence `hasattr(...)` probes.

**Fix:**

1. `DirectProgressSink.publish`: before `self.renderer.render(...)`, call
   `self.renderer.update_counters_batch(counters)` exactly as the threaded
   path does, guarded by `counters is not None` and
   `hasattr(self.renderer, "update_counters_batch")`.
2. Remove `DirectProgressSink.update_counters` method entirely.
3. Remove `_NullProgressSink.update_counters` in `bench_dispatch_fidelity.py`.
4. Drop `update_counters` no-op methods in the four test sinks.

**Verify gate:**

* New `tests/test_direct_progress_sink.py` (3 tests) — passes.
* Existing 141 baseline tests stay green (verified).
* `uv run ruff check .` — clean.
* `uv run pyright src/sky_music/orchestration/playback_supervisor.py` — clean.

**Status:** SHIPPED (2026-07-28). Commit message:
`fix(orchestration): DirectProgressSink forwards counters via update_counters_batch`.

## 3. Patch A2 — HUD/counters use `visible_lateness_us` for onsets

**Files:** `src/sky_music/orchestration/core/loop.py:1258-1278`, `src/sky_music/orchestration/core/loop.py:343-357` (counter snapshot), `src/sky_music/orchestration/core/ports.py:150-158` (`ProgressCounters`), `src/sky_music/ui/textual_app/playback_app.py:130-138, 188-199`, `src/sky_music/ui/hud.py:95-96`.

**Bug:** `observe_result` (loop.py:1258-1278) feeds `exec_result.lateness_us` (sender call-entry) into both `_max_lateness_us` and `_latencies`. `ProgressCounters` carries those into `recent_latencies_us`. The HUD (`SnapshotRenderer.update_counters_batch`) writes them to `self._latencies` and `max_lateness_us`, then `debug_stats()` computes p50/p95/σ and renders labels like "Onset: max ≥ X" — but **the metric is dispatch-call-entry lateness, not player-visible onset**. Telemetry already separates `visible_lateness_us` (completion) and `dispatch_lateness_us` (loop.py:480-481); only the HUD/counters leak.

**Fix:**

1. `observe_result` for `kind == "down"`: use `exec_result.visible_lateness_us`
   instead of `exec_result.lateness_us` for `_max_lateness_us` and the
   thresholds and the `_latencies` ring buffer. Release counters (kind ==
   "up") keep using `lateness_us` (the release path's `visible_lateness_us`
   is not the player-perceived metric for this surface).
2. Keep raw `lateness_us` in telemetry `ExecutionResult` (already separate).
3. Keep `ProgressCounters` field names (`max_lateness_us`, `recent_latencies_us`) — no public rename in this patch. The semantic they carry becomes "visible onset", matching what `telemetry.summary["visible_lateness_us"]` reports.
4. Update comment at `SnapshotRenderer` "Onset-only counters (key-down) — these are what the player hears" (playback_app.py:89) — now true.

**Verify gate:**

* Controlled-clock test: call-entry lands exactly on deadline (`lateness_us == 0`) but completion clocks in `+350 µs` (`visible_lateness_us == 350`). Counter onset must record `350`, not `0`. Reuse the existing `PlaybackEngine` harness to feed a fake backend that returns `complete_us = send_start_us + 350`.
* Snapshot HUD expectations in `tests/test_textual_playback.py` may shift; update snapshots per fixture (acceptable per AGENTS.md: "metric được đo đúng hơn, không phải regression").
* `summary["visible_lateness_us"]` (telemetry) unchanged — only HUD parity.

**Doc:** no P2 doc currently asserts `max_lateness_us` semantics; the fix
aligns HUD with telemetry's existing vocabulary. Update a comment in
`docs/INDEX.md` §0 note ("Deterministic Telemetry wins over experience") only
if reviewer requests.

**Implementation notes:**

* Combined refactor + behaviour-change as one atomic logical unit: the
  closure moved into `DispatchLoop._observe_exec_result` to make the metric
  behaviour directly unit-testable. AGENTS.md advises against merging
  refactor + behaviour change; the rationale for the merge here is that
  without extracting the closure first, no test can drive the fix.
* Release counters (`_release_max_us`, `_release_late_2ms`) continue to
  use `lateness_us` — the release path's metric is the bounded-retry
  contract, not the game-observed onset.

**Verify gate (verified 2026-07-28):**

* New `tests/test_hud_onset_metric.py` (4 tests) — passes.
* Existing 145 baseline tests stay green.
* `uv run ruff check .` — clean.
* `uv run pyright src/sky_music/orchestration/core/loop.py` — clean.

**Status:** SHIPPED (2026-07-28). Commit message:
`fix(orchestration): HUD onset counters use visible_lateness_us (refactor+fix observe_result closure)`.

## 4. Patch T1 — Rewrite `scripts/measure_dispatch_tail.py` for 3.14t + pytest-benchmark

**File:** `scripts/measure_dispatch_tail.py` (rewrite; archive old version under `scripts/archive/`).

**Bug:**

* `random.random()` without seed → non-deterministic.
* Matrix (lines 172-174) labels all three runs "stock 3.14 (GIL)" — but the repo requires 3.14 free-threaded (`.python-version`).
* `sys.setswitchinterval` matrix (line 101) is meaningless on no-GIL.
* No fail-fast when `sys._is_gil_enabled() is not False`.
* `pytest-benchmark>=5.2.3` is already a dep but unused.

**Fix (rewrite):**

1. Startup assertion at top of `main()`:
   ```python
   assert sys.version_info[:2] >= (3, 14), "3.14+ required"
   assert not sys._is_gil_enabled(), "free-threaded build required"
   ```
2. Fixed-seed `random.Random(seed=20260728)` for the synthetic backend's delivery-time sampling.
3. Replace the GIL matrix with a 3.14t matrix:
   - UI load off / on (60 Hz simulator already present, retain).
   - Waitable timer off / on (already configurable via `SleepPolicy`).
   - Priority scope off / auto (already configurable via `rt_priority_mode`).
   - Drop `sys.setswitchinterval` matrix.
4. Add a `pytest-benchmark.pedantic` mode (function `bench_send_dispatch`):
   `rounds=20_000, warmup_rounds=200, iterations=1, fixed input sequence`. Gate:
   `p50 improvement >= 10%, p99 regression <= 5%`.
5. Keep `bench_dispatch_fidelity.py` as the structural-fidelity sibling
   (already 5-iter, no pedantic) — it does not duplicate the pedantic
   microbench.
6. Archive the legacy harness as `scripts/archive/measure_dispatch_tail_gil.py`
   before overwrite, so historical GIL baseline can be reconstructed.

**Verify gate:**

* `uv run python scripts/measure_dispatch_tail.py --pedantic` exits 0 and
  prints a 3.14t matrix table.
* Startup assertion fails clear on a stock 3.14 (not free-threaded) — verify
  by temporarily importing under such interpreter (best-effort; skipped if
  unavailable in this environment).
* No production code touched — script-only; release build unaffected.

## 5. Patch C1 (ask-first) — Spec comment + frozen smoke `__debug__` verify

**Files:** `Sky-Auto-Player.spec:77-79`, `src/main.py` (new `--selftest-optimize`), `src/build_app.py:125-145` (extend `run_smoke_test`).

**Ask-first reason:** AGENTS.md `Boundaries → Ask first` covers
`Sky-Auto-Player.spec excludes` and `onedir/collect_*` strategy; `optimize=1`
is a different spec field but still spec-sensitive. User pre-approved
"Chỉ sửa comment + thêm smoke verify" in the question round.

**Bug:** spec comment "optimize=1: … preserves assert statements" is
factually wrong. Python strips `assert` and `__debug__` blocks when bytecode
is compiled with `optimize>=1` (Python docs: `simple_stmts.html`). The
duplicate-release `assert` at `loop.py:756-767` is therefore a **source/debug
contract only** — the frozen release ships without it. `build_app.run_smoke_test`
runs `--selftest-textual` (Textual picker smoke) but does not verify the optimization mode.

**Fix (user-approved scope):**

1. Spec comment (lines 77-79):
   ```python
   # optimize=1: removes docstrings and __debug__-only blocks (assert
   # statements are NOT preserved). The duplicate-release check at
   # loop.py:_dispatch_release_batch is a debug/source invariant; the
   # coordinator enforces uniqueness at insert via pending_scan_codes /
   # pending_by_generation. A release-specific smoke test confirms the
   # frozen build's optimization mode. Add explicit runtime guard only if
   # P0 mandates it (separate patch + benchmark).
   optimize=1,
   ```
2. `src/main.py`: add `--selftest-optimize` branch printing
   `sys.flags.optimize`, `__debug__`; exit non-zero if
   `__debug__` (i.e. optimize == 0) when the running binary should strip.
3. `src/build_app.py:run_smoke_test`: after `--selftest-textual`, also run
   `--selftest-optimize` and require assert-strip confirmation; fail build
   otherwise.
4. No change to `optimize=1` value; no change to `excludes`; no change to
   `onedir/collect_*`.

**Verify gate:**

* `uv run --env-file .env python -m build_app` smoke step succeeds with the
  new optimize-mode check.
* Existing source-level `test_dispatch_pending_releases_asserts_on_duplicate_scan_codes`
  (or equivalent) still green (unchanged).

**Verify gate (verified 2026-07-28):**

* `Sky-Auto-Player.spec:77-79` comment corrected (Python strips `assert` at
  `optimize>=1`).
* New `--selftest-optimize` branch in `src/main.py:_run_optimize_selftest`.
* `build_app.run_smoke_test` now runs BOTH `--selftest-textual` and
  `--selftest-optimize`.
* `uv run --env-file .env python -m build_app` — green, smoke both flags.
* Frozen binary manual probe: `--selftest-optimize` prints
  `__debug__: False` and exits 0 (assert-stripped as spec requires).
* `tests/test_selftest_optimize.py` (2 tests) — passes.
* `uv run --env-file .env python scripts/audit_security_mandates.py` — green.
* `uv run ruff check .` — clean.
* `uv run pyright src/main.py src/build_app.py` — clean.

**Status:** SHIPPED (2026-07-28). Three commits in order:
1. `feat(main): add --selftest-optimize branch (frozen build optimization contract smoke)`
2. `chore(spec): correct optimize=1 comment (assert strips at optimize>=1)`
3. `feat(build): extend run_smoke_test to run --selftest-optimize after --selftest-textual`

## 6. Patch B1 — Calibration loader → infrastructure

**Files:** `src/sky_music/domain/scheduler_types.py:25-55,161-171` (extract), new `src/sky_music/infrastructure/calibration_loader.py`, `tests/test_calibration.py`, `tests/test_core_send_overhaul_invariants.py`, `tests/conftest.py:28`.

**Architecture violation:** `domain.scheduler_types.get_calibrated_margin_recommendation()` opens `.cache/input_latency.json`, parses JSON, swallows I/O errors — `domain` knows about working directory, cache filename, JSON schema, persistence policy. This is `domain → filesystem` hidden dependency. AGENTS.md `Architecture Invariants` say domain may not import `ctypes`, `SendInput`, wall-clock, or Windows modules; filesystem I/O is an implicit platform dependency of the same kind.

**Fix:**

1. Create `src/sky_music/infrastructure/calibration_loader.py`:
   ```python
   def load_calibrated_margin_recommendation(
       cache_path: Path | None = None,
   ) -> tuple[int | None, str]:
       """Return (margin_us, source_label) where source in
       {"device_cache", "default_500", "profile_override"} — moved from
       domain/scheduler_types.py. Pure: opens the cache_path only when
       explicitly passed; raises FileNotFoundError when missing and no
       default requested. Returns (None, "default_500") on missing file."""
   ```
2. `TimingPolicy.from_dict` accepts an injected `calibrated_margin_us: int | None` and `source_label: str` (resolved upstream by orchestration) — domain module no longer touches the filesystem.
3. Orchestration (`runtime_session.py` or new helper in `orchestration/calibration.py`) calls `load_calibrated_margin_recommendation()` once at session build time, then passes primitives to `from_dict`.
4. Keep formula, clamp and `500` fallback bit-for-bit identical. **No timing change.**
5. Update 10 test sites (see `tests/test_calibration.py:525-599`, `test_core_send_overhaul_invariants.py:81-111`, `conftest.py:28`) — the import moves from `domain.scheduler_types` to `infrastructure.calibration_loader`; existing assertions on the function's behaviour (poisoned JSON, missing file, absurd values, wrong version, valid cache → 400/300/2000) still pass.
6. `docs/architecture.md` layering note: add `calibration_loader` under `infrastructure/`.

**Verify gate:**

* `uv run pytest tests/test_calibration.py tests/test_core_send_overhaul_invariants.py` green post-move.
* `uv run pyright` clean (no new import cycle).
* `uv run ruff check .` clean.
* Golden policy comparison: build `TimingPolicy.local_precise()` before and after; assert all fields identical including `min_hold_margin_source`.

## 7. Patch B2 — `SleepPolicy` out of `domain`

**Files:** `src/sky_music/domain/session_context.py:19,182-193` (move), `src/sky_music/orchestration/runtime_session.py:51` (adjust caller), `src/sky_music/cli/console_playback.py:429,506`, `src/sky_music/ui/textual_app/playback_controller.py:56`.

**Architecture violation:** `PlaybackSessionContext` (in `domain/`) imports `sky_music.infrastructure.timing.SleepPolicy` and constructs a concrete infrastructure strategy. This is `domain → infrastructure` — the layer direction AGENTS.md `Architecture Invariants` forbid. `SleepPolicy` is the platform-adjacent wait strategy shape (spin threshold + poll cadence), not a musical construct.

**Fix:**

1. `PlaybackSessionContext.resolve_sleep_policy` returns `(spin_threshold_us: int, poll_s: float)` instead of `SleepPolicy`. Validate ranges there.
2. Move the `SleepPolicy(...)` construction into the 3 callers:
   - `runtime_session.py:51` (orchestration — owns it)
   - `console_playback.py:429,506` (CLI plumbing)
   - `playback_controller.py:56` (Textual plumbing)
3. Keep default `poll_s = 0.025` and `spin_threshold_for_profile` resolution bit-for-bit.
4. `docs/architecture.md` layering note: `domain.session_context` no longer imports infrastructure.

**Verify gate:**

* `uv run pyright` clean (no new cycle).
* `uv run pytest` green (mostly mechanical call-site changes).
* Golden `SleepPolicy` comparison at both CLI and Textual construction paths —
  spin_threshold_us and poll_s identical before/after.
* No timing behaviour change (defaults preserved).

## 8. Patch C2 — Drop `DispatchLoop.enable_event_wait` instance field

**Files:** `src/sky_music/orchestration/core/loop.py:257,278`, downstream constructors (compat).

**Dead state:** `self.enable_event_wait = enable_event_wait` (loop.py:278) is
assigned once and never read in `loop.py`. The core's polling vs event-wait
decision is driven by `command_event is not None` (the supervisor's intent),
not by the boolean flag. The flag's role as a config knob lives higher up —
at `engine.enable_event_wait`, `supervisor.enable_event_wait`, and
`HybridWaitStrategy.enable_event_wait`, all of which stay.

**Fix (two-phase):**

1. **Phase 1 (this patch):** drop the `self.enable_event_wait` field; keep
   `enable_event_wait: bool = False` parameter on `DispatchLoop.__init__` as
   a no-op (callers remain source-compatible). Add a `# noqa: ARG002` or
   similar to silence linters. Add an inline comment explaining the kwarg is
   kept for source compatibility and will be removed in a later release.
2. **Phase 2 (separate PR, after a release):** grep the codebase; if no
   downstream caller depends on the kwarg, remove it.

**Verify gate:**

* `uv run pytest tests/test_threaded_dispatch.py tests/test_phase4_lifecycle.py tests/test_phase5_degraded_wait.py tests/test_dispatch_fidelity_refactor.py tests/test_dispatch_fidelity_edges.py tests/test_spin_reprobe.py` — all green (these pass `enable_event_wait=True/False` to `DispatchLoop` directly).
* `uv run pyright` clean.
* `uv run ruff check .` clean.
* Compare degraded/event-mode tests bit-for-bit.

## 9. Patch C3 — `QueueCommandSource.poll()` drops redundant `empty()`

**File:** `src/sky_music/orchestration/playback_supervisor.py:115-125`.

**Simplification:** `poll()` calls `queue.empty()` then `get_nowait()` — but still has to catch `queue.Empty`. The check is a redundant advisory operation. Under no-GIL the extra synchronized op costs very little, but the code is clearer without it.

**Fix:**

```python
def poll(self) -> str | None:
    try:
        return self._commands.get_nowait()
    except queue.Empty:
        return None
```

**Verify gate:**

* New `tests/test_queue_command_source.py`:
  - empty queue → `poll() is None`
  - one command enqueued → `poll()` returns it; second `poll()` returns `None`
  - concurrent producer + consumer loop: every enqueued command is dequeued exactly once (no drop, no dup).
* `uv run pytest tests/test_threaded_dispatch.py` green (consumer of this source).

## 10. Hot-path candidates — NOT in this plan

The ChatGPT report lists two perf candidates that I am explicitly **not**
scheduling:

1. `WinSendInputBackend._emit()` accepting only `PlatformSendResult` (drop
   tuple/None/int compatibility shim).
2. Outer-tuple in `_ARRAY_CACHE` (split down/up, scan-tuple direct key).

Per AGENTS.md `Workflow Rules`, a perf change to a hot path requires
`pytest-benchmark.pedantic` evidence on 3.14t Windows 11 (p50 ≥10% gain,
p99 ≤5% regression) **after** Patch T1 lands the harness. These stay open
as future experiments gated on T1.

## 11. Push-back rejected (do not implement)

| # | Proposal | Why rejected |
|---|---|---|
| R1 | Switch SendInput to PostMessage / driver | P0 violation (SendInput only). |
| R2 | Drop spin to reduce CPU without benchmark | Accuracy-first; no evidence yet. |
| R3 | `TIME_CRITICAL` default / realtime class | Windows starvation warning. |
| R4 | Note-on retry after sleep 1–2 ms / unbounded | Staggered late chord. |
| R5 | Drop second release on focus-regain | Safety model. |
| R6 | `time.time()` on dispatch timeline | Epoch mixing. |
| R7 | Lock around every scalar "for free-threaded" | Blind lock = jitter. |
| R8 | Drop every lock because "single writer" | Cross-thread snapshot contract. |
| R9 | Convert all defensive guards to `assert` | Frozen build strips assert. |
| R10 | Memoize every getter | Adaptive interpreter — needs evidence. |
| R11 | Stream/chunk schedule | No memory workload gate. |
| R12 | Split `core/loop.py` across classes | Refactor surface increase. |
| R13 | Tail by p50/p90 to lower spin | Tail defines accuracy. |
| R14 | Profile name as weak/strong proxy | Two independent axes. |
| R15 | Hold StrEnum to PEP 663 | PEP rejected. |

## 12. Verification ladder (per AGENTS.md `Validation`)

| Change scope | Command |
|---|---|
| Per-patch types | `uv run pyright` |
| Per-patch lint | `uv run ruff check .` |
| Tests | `uv run pytest tests/<path> -x` (narrow), `uv run pytest` (gate) |
| Pre-merge (multi-scope) | `uv run ruff check . && uv run pyright && uv run pytest` |
| Security-touch (none in this plan) | `uv run --env-file .env python scripts/audit_security_mandates.py` |
| Frozen build (Patch C1 only) | `uv run --env-file .env python -m build_app` |

## 13. Definition of Done

* The narrowest gate for each patch is green (see §12).
* No unrelated code changed.
* Owner P2 doc updated in the same commit when documented behaviour changed
  (Patch B1/B2 touch `docs/architecture.md`; Patch C1 updates spec comment).
* P0 surfaces untouched.
* Plan doc (`docs/INDEX.md` swipe) gets an entry under §2 "Active References &
  Experiments" with status PROPOSED; updated to IMPLEMENTED as each patch merges.
