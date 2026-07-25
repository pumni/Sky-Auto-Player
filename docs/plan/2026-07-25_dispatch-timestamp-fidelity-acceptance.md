# Dispatch Timestamp Fidelity Refactor - Acceptance Packet

## §16.5 Acceptance Deliverables

### 1. Changed-file list + reason
- `src/sky_music/orchestration/core/loop.py`: Implemented Phase 1-E (tracking elapsed us instead of wall-clock logic for deadlines, and utilizing `effective_spin_threshold` for cold gaps). Removed dead hook `core_warmup_hook` and clamped budget expansion.
- `src/sky_music/infrastructure/wait_strategy.py`: Fixed the event-wait fallback bug. When `remaining_to_sleep <= 0` during high-res timer invocation, we now strictly fall through to spin instead of sleeping with negative timers.
- `src/sky_music/orchestration/engine.py`: Removed `_spin_warmup` which became dead code under the new core warmup expansion logic.
- `src/sky_music/orchestration/telemetry.py`: Ensured memory safety by dropping records without blocking the dispatch loop, and added `_dropped_count` into `get_summary()` for telemetry honesty (Phase 2 & Memory Hygiene).
- `src/sky_music/ui/renderer.py` & `src/sky_music/ui/components/`: Decoupled `ProgressCounters` using thread-safe structures to prevent lock contention between UI and Dispatch (Phase 3 — progress publication batched at 30 ms snapshot cadence).
- `tests/test_phase6_warmup_budget.py`: Updated mock assertions for `wait_strategy.wait_until_us` to expect direct `spin_threshold_us` manipulation. Removed stale `core_warmup_hook`.
- `tests/test_dispatch_fidelity_refactor.py`: Updated to assert correct bounds based on elapsed times.
- `docs/rt-dispatch-architecture.md`: Fixed docs drift (replaced `core_warmup_hook` with `core_warmup_budget_us`, corrected `MAX_RECORDS` to `_TELEMETRY_MAX_BUFFER`).
- `docs/timing-principles.md`: Clarified that Phase E expands the final spin threshold rather than running a separate short busy-spin.
- Rework additions: `src/sky_music/platform/win32/console.py` and `diagnostics.py` isolate Win32 ctypes; `src/sky_music/layouts.py`, `src/sky_music/cli/console_playback.py`, and `src/sky_music/infrastructure/doctor.py` now use those seams; `inputs.py` adds validated MapVirtualKey/window-process helpers.
- Rework correctness: `src/sky_music/orchestration/core/loop.py` publishes terminal progress counters for done/skipped/stopped/error exits; dispatch fidelity tests cover final publication, native chord batching, the trusted-path set tripwire, and actual progress-lock acquisitions.
- Rework evidence: `tests/test_dispatch_fidelity_edges.py`, `tests/test_dispatch_runtime_efficiency.py`, `tests/test_layouts.py`, `tests/test_phase_c_and_g_advisory.py`, and `tests/test_core_send_overhaul_invariants.py` cover the corrected edge contracts; `scripts/bench_dispatch_fidelity.py` reports wall and direct-thread CPU samples.
- `tests/golden_schedules/telemetry_summary_schema_v1.json`: Authorized schema update for the ten bounded-telemetry summary key paths now emitted by the retain-first cap.

### 2. Phase-by-phase summary (0–9)
- **Phase 0 (Baseline Setup):** Done. Evaluated the existing architecture and constraints. 
- **Phase 1 (Data Types / Cold Guard):** Done. Migrated `target_elapsed_us` / `_last_send_elapsed_us` in pure dispatch. 
- **Phase 2 (Native timestamp / Telemetry):** Done. Handled `send_completed_us` from the platform response, ensuring logical completion respects final native attempt.
- **Phase 3 (Cold Gap Warmup Budget):** Done. Replaced legacy secondary spins with an integrated `core_warmup_budget_us` addition to the standard spin threshold.
- **Phase 4 (Progress Sink Decoupling):** Done. Switched to `ProgressCounters` batch updates. Lock contention eliminated.
- **Phase 5 (Chord Send Refactor):** Done. Handled multi-key actions natively in one `SendInput` batch payload.
- **Phase 6 (Hold Overlap & Semantic Validation):** `AWAITING EVIDENCE`. I deliberately did not modify the same-key semantic rules per the plan's instructions.
- **Phase 7 (Testing):** Done. Realigned fidelity and boundary test suites to new timestamps. Test coverage matches new interfaces.
- **Phase 8 (Packet & Hand-off):** Updated with final gate outputs and explicit evidence limits.
- **Phase 9 (Final validation):** Correctness gates pass. Performance acceptance remains pending because the historical before-run has no comparable CPU sample and the current Windows thread-CPU clock is coarse; this packet does not self-approve that criterion.

### 3. Tests added + which failed before fix
- `test_dispatch_fidelity_refactor.py` — 14 tests covering the Phase 1, 2, 3, 4, 5, 7
  regression findings (F1, F5, F6, F7, F8, F9, F10) plus Phase 2 partial-send
  bounded-evidence (plan §16.2) plus Phase 2 telemetry timestamp-relationship
  acceptance samples (plan §16.5 mục 6). The Phase 1, 2, 3, 4, 5 tests were
  written **after** the production fixes landed in commit `6573593`, but every
  test asserts an invariant that the pre-refactor `commit 6d02453` code
  violated — verified by running the same test file against
  `D:\Dev\Sky-Auto-Player-pre-refactor` worktree at `6d02453`: 13 of the 14
  fail (the exception is `test_partial_send_release_completes_remainder`,
  which exercises a release path that pre-refactor already implemented
  correctly).

- `test_accuracy_refinement_invariants.py::test_core_warmup_budget_us_default`
  replaces the previous `test_core_warmup_spin_us_exists`, switching from an
  existence check on the now-removed dead constant `CORE_WARMUP_SPIN_US =
  200` to a behavioral check asserting `DispatchLoop.__init__`'s default
  `core_warmup_budget_us >= 200 µs`.

Behavioral-only tests added (no `inspect.getsource` anti-pattern):
  * `test_single_key_trusted_batch_avoids_duplicate_set_allocation` — patches
    `user32.SendInput` to return 1 for a 1-key request and asserts
    `send_mock.call_count == 1`, `result.inserted == 1`, and that
    `_DIAG.keys_retried == keys_dropped == partial_send_events == 0`.
    It also trips on any `builtins.set` allocation during the trusted call.
    (Replaces the previous `inspect.getsource` shape check that would have
    broken on future benign refactors.)
  * `test_progress_publication_does_not_lock_per_dispatch` — runs 20 dense
    dispatches through the real `SnapshotProgressSink` and counts actual lock
    acquisitions; the count remains below the dispatch count (Phase 3 cadence contract).
  * `test_telemetry_cap_never_flushes_on_dispatch_thread` — fills ≥2× the
    `_TELEMETRY_MAX_BUFFER` and asserts no CSV file was created, that
    `_dropped_count > 0`, and `len(records) <= _TELEMETRY_MAX_BUFFER`.
  * `test_waitable_timer_recomputes_remaining_immediately_before_arm` —
    drives `wait_until_us` with an advancing fake clock; asserts the value
    armed on the waitable timer equals the recomputed remaining from the
    second clock read, not the stale first-read value (Phase 4 F9 contract).
  * `test_auto_priority_never_selects_time_critical_fallback` — stubs every
    priority rung to fail under `mode='auto'` and asserts the outcome tier
    is `'off'` (no `'thread:time_critical'`).
  * `test_partial_send_remains_bounded_and_safe` — partial down-send with a
    failing retry: asserts exactly 2 `SendInput` calls, no `_retry_wait_seconds`
    sleep, and `_DIAG.keys_dropped == 2`.
  * `test_partial_send_release_completes_remainder` — partial up-send: the
    safety path must complete the remainder; asserts `inserted == 4`,
    `keys_dropped == 0`.
  * `test_one_key_and_chord_telemetry_timestamp_relationships` — end-to-end
    verification through the orchestration core that the dispatch-completion,
    pure-send-duration, bookkeeping, and visible-lateness fields come from
    one native-return timestamp sample, for both a 1-key and a 3-key chord
    fidelity-mode send (plan §16.5 mục 6).
  * `test_terminal_progress_is_published_on_dispatch_error` and the stop-path
    assertion prove final counters are forced on early command and error exits.
  * `test_exact_timestamp_chord_uses_one_sendinput_call` additionally calls the
    platform seam directly and asserts one native `SendInput` call for the full chord.

The "flaky on re-runs" `test_card_anchored_after_debug_toggle_grows`
mentioned in the original packet is unrelated to this plan's surface and is
not affected by any of the production changes here.

### 4. Raw gate outputs
**Gate: uv run ruff check .**
```text
All checks passed!
```

**Gate: uv run pyright**
```text
0 errors, 0 warnings, 0 informations
```

**Gate: uv run pytest**
```text
============================= test session starts =============================
platform win32 -- Python 3.14.3, pytest-8.4.2, pluggy-1.6.0
rootdir: D:\Dev\Sky-Auto-Player
configfile: pyproject.toml
testpaths: tests
plugins: textual-snapshot-1.1.0, syrupy-4.8.0
collected 758 items

[... Output omitted for brevity ...]
tests\test_win32_event_prototypes.py ..                                  [100%]

======================= 758 passed in 93.56s (0:01:33) =========================
```

**Gate: uv run pytest tests/test_dispatch_fidelity_refactor.py**
```text
============================= test session starts =============================
14 passed in 0.60s
```

Final rerun after the authorized schema update collected 758 items and passed all
tests, including `test_telemetry_summary_schema_key_paths`. The earlier 737/741/738
counts and the previous schema failure are historical evidence, not current results.

### 5. Benchmark evidence and limits

Method: `scripts/bench_dispatch_fidelity.py` drives `DispatchLoop` + `RuntimeDispatchCoordinator` + `DryRunBackend` with a deterministic fast-forward clock. It measures structural sender timing, wall time, and `time.thread_time_ns()` on the direct benchmark thread. It does not exercise real `SendInput`, real Windows waits, game state, or OS frame sampling.

Historical before run from the prior packet (commit `6d02453`, five runs):
```text
wall raw=[11.591, 10.606, 11.264, 11.240, 11.820] ms
wall median=11.26 ms; p95/p99=11.82 ms
visible lateness p50/p99/max=0/0/0 us; cumulative drift=0 us/run
```
The historical before run did not record CPU with the current CPU-clock method, so the
old packet claim of `0.13 ms -> 0.15 ms` is removed rather than treated as measured evidence.

Final current run on the same workspace and profile:
```text
Song: We Wish You A Merry Christmas   notes=180
Profile: balanced @60fps   min_hold_us=17300
wall raw=[24.7227, 16.0149, 17.9277, 15.5317, 17.7634] ms
wall min/median/p95/p99/max/mean=15.53/17.76/24.72/24.72/24.72/18.39 ms
thread CPU raw=[0.0, 15.625, 15.625, 15.625, 15.625] ms
visible lateness n=420, min/median/p95/p99/max=0/0/0/0/0 us
cumulative drift per run=[0, 0, 0, 0, 0] us
```

The Windows `GetThreadTimes()` sample is coarse in this environment despite the nominal
clock metadata, so it is reported transparently but is not a reliable CPU-regression verdict.
The deterministic zero lateness and zero drift prove structural fidelity only; they do not
prove game acceptance or live-OS scheduling performance.

**Performance verdict:** structural timing passes; Phase 9 CPU/performance acceptance remains
`AWAITING EVIDENCE` until an independent same-machine before/after capture records comparable
dispatch-thread CPU and real-wait conditions. No unsupported tradeoff is self-accepted.


### 6. Timestamp relationship samples

**Sample 1 — single-key down dispatch** (from
`test_one_key_and_chord_telemetry_timestamp_relationships`):

The platform seam returns `send_completed_us = 100` (raw `perf_counter` µs, sampled
immediately after the native `SendInput` call returns — see `inputs.py` lines 615–617).
The orchestration core computes the elapsed-at-completion via
`state.get_elapsed_us(clock, send_completed_us)` and propagates that single timestamp
to every consumer:

```text
T_call_entry  (actual_us)              = state.get_elapsed_us(clock, 0)   = 0 µs
T_call_return (send_completed_us raw) = 100  µs  (sampled at platform seam)
dispatch_completed_us (elapsed)      = state.get_elapsed_us(clock, 100) = 100 µs
send_duration_pure_us  = send_completed_us - actual_us = 100 µs
bookkeeping_us         = send_end_raw  - send_completed_us = 120 - 100 = 20 µs
visible_lateness_us    = dispatch_completed_us - scheduled_us = 100 - 0 = 100 µs
```

The `TelemetryRecord` row written by `TelemetryLogger.record()` has all four of
`actual_us`, `dispatch_completed_us`, `send_duration_pure_us`, `bookkeeping_us`, and
`visible_lateness_us` populated from this single native-return sample — no later clock
read substitutes for the platform timestamp (Phase 2 contract).

**Sample 2 — 3-key chord (fidelity mode, `chord_stagger_us == 0`)** (same test):

The chord is compiled to ONE `KeyAction` (3 scan codes, `at_us = 0`). The dispatch loop
sees one batch, makes ONE call to `backend.key_down((0x1E, 0x1F, 0x20))`, the platform
seam samples `send_completed_us = 500` once after the single native `SendInput` returns,
and all three chord members share the same `dispatch_completed_us = elapsed(500)` and
the same `visible_lateness_us`. `backend.key_down.call_count == 1` after the run (verified
in the test) — proving plan §16.2 ("one `KeyAction`, one native `SendInput` call").

### 7. Evidence 1 SendInput/chord
Phase 5 ensures chords natively bundle all scans into one array:
`backend_result = backend.key_down(action.scan_codes)`
Resulting in exactly 1 `SendInput` invocation per chord block.

### 8. Security audit raw
**Gate: uv run --env-file .env python scripts/audit_security_mandates.py**
```text
=== Sky Auto Player: AGENTS.md P0 security-mandate audit ===

Scanning:        D:\Dev\Sky-Auto-Player\src
Baseline file:   D:\Dev\Sky-Auto-Player\.config\security_audit_baseline.json

[OK] No forbidden Windows API references in src/.
```

### 8.1 Free-threaded runtime audit
**Gate: uv run --env-file .env python scripts/audit_free_threaded_wheels.py**
```text
[OK] CPython 3.14.3 free-threaded interpreter detected (Py_GIL_DISABLED=1).
[OK] Runtime GIL disabled.
[OK] rapidfuzz 3.14.5 satisfies its PEP 440 requirement and imports under no-GIL.
[OK] textual 8.2.7 satisfies its PEP 440 requirement.
```

### 9. Memory/lifecycle evidence
Telemetry uses `_TELEMETRY_MAX_BUFFER`. At capacity:
```python
    if len(self.records) >= self._telemetry_capacity:
        # Constant-time cap path: retain existing records and stop accepting new ones.
        self._dropped_count += 1
        self._truncated = True
        return
```
At capacity the dispatch thread retains the first records, increments exact dropped and
truncation markers, and performs no slicing, record construction, or file I/O. Lifecycle
`save()` exports the retained records later, off the dispatch hot path.

### 10. Phase 6 AWAITING EVIDENCE
This phase explicitly remains `AWAITING EVIDENCE`. I did not alter the core semantic release policies (e.g. strict overlap filtering). 

### 11. Deviations + justification

**Deviation A — `core_warmup_hook` and `_spin_warmup` removed entirely.**
Justification: instead of triggering an explicit mockable secondary spin loop,
the budget (`core_warmup_budget_us`) simply inflates the main high-res sleeper
`spin_threshold_us` (`effective_spin_threshold = spin_threshold_us +
min(core_warmup_budget_us, CORE_WARMUP_SPIN_MAX_US)`). This reduces nested
calls, prevents clock entanglement, and improves purity.

**Deviation B — Phase 1 fixed early (single commit, not phased).**
Justification (honest disclosure): the implementation landed in one large
commit (`6573593`) instead of the 9-commit logical sequence mandated by
plan §5 / §18. The reviewer is asked to weigh this against the fact that all
production changes are surgical and described per-phase in the packet.
**Future rework commits are explicitly titled with conventional prefixes
(`test(scheduler):`, `fix(windows):`, `docs(scheduler):` …) so the
phase-by-phase split is recoverable from `git log --format=%s` after the
independent reviewer accepts this rework.**

**Deviation C — Dead constant `CORE_WARMUP_SPIN_US` removed, not retained.**
Justification (rework): the original implementation kept the deprecated
constant `CORE_WARMUP_SPIN_US = 200` for the sake of one test (`test_core_
warmup_spin_us_exists`) that pinned its existence — a self-justifying cruft
that §8.8 explicitly forbade. The rework deletes the constant and replaces
the test with a behavioral check on `DispatchLoop.__init__`'s default
`core_warmup_budget_us >= 200 µs`.

**Deviation D — Doc merge orphan in `docs/timing-principles.md` cleaned.**
Justification (rework): the original implementation left an orphan tail
("(≤ 200 µs) to warm the CPU core before the next send…") after the
rewritten body, creating a two-clause orphan. Plan §15 keeps doc updates
strict-containing; the rework rewrites the whole §Phase E block cleanly.

**Deviation E — Test anti-pattern removed.**
Justification (rework): four of the original Phase tests used
`inspect.getsource(...)` + string-containment — these verify code shape
not behavior and would break on innocent refactors. The rework replaces
all four with behavioral assertions that spy on real call counts, file
existence, or return values.

### 12. Remaining risks
- Residual flakiness in TUI testing (e.g. `test_card_anchored_after_debug_toggle_grows`) due to Rich render delays might surface under heavy CPU loads, but it is functionally decoupled from the Windows realtime dispatch hot path.
- Phase 6 remains `AWAITING EVIDENCE` because same-key overlap semantics require independent external game-observed evidence; the unit tests do not establish that observation.
- Phase 9 performance acceptance remains `AWAITING EVIDENCE`: the current deterministic benchmark is a structural/regression signal, but its fake clock does not exercise real waits or `SendInput`, and this Windows host's thread-CPU clock is too coarse for a reliable before/after CPU comparison.

### 13. Current acceptance status
**CORRECTNESS GATES PASS; FINAL PLAN ACCEPTANCE PENDING INDEPENDENT REVIEW**

The repository now passes the complete local correctness/security/runtime gate set:
758 pytest tests, Ruff, Pyright, the P0 security audit, and the free-threaded wheel
audit. The packet deliberately does not mark the original proposal as
`IMPLEMENTED`: Phase 6 external evidence and comparable real-wait performance
evidence remain explicit plan gates, and acceptance must be made by an independent
reviewer.
