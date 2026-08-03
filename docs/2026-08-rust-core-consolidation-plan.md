# Rust dispatch core consolidation — Phase 0 baseline

> Status: PROPOSAL / working note. This document records the starting state for
> the 2026-08 consolidation. It is not a normative architecture document;
> `AGENTS.md` and the canonical documents listed in `docs/INDEX.md` remain the
> source of truth until the relevant phases land.

## 1. Baseline identity

- Repository: `pumni/Sky-Auto-Player`
- Branch requested by the task: `main`
- Baseline commit: `f022f4e2c0ecb9e82e1a980de977db5b1dab14d8`
- Baseline date: 2026-08-03 (Asia/Bangkok)
- Working tree at inventory start: clean
- Python runtime: CPython 3.14.3 free-threaded (`cp314t`)
- Rust toolchain observed by the native wheel gate: `rustc 1.97.1`
- Native ABI: `cp314t-win_amd64`
- Native schema: 2
- Native build at baseline: clean; build commit equals the baseline commit

The first `uv sync --frozen` invocation used uv's default cache and was denied
by the host ACL. Re-running with the repository-local cache required by the
build instructions (`UV_CACHE_DIR=.uv-cache`) completed successfully. The
local cache setting is used for the remaining `uv` commands below.

## 2. Current architecture observed in code

The current production composition root still admits two execution models:

```text
Python UI / CLI / main
        │
        └─ PlaybackEngine
             ├─ builds RuntimeSchedule with compile_runtime_intents()
             ├─ resolves DispatchPolicy (auto / rust / python)
             ├─ native branch:
             │    RustDispatchRuntime → PyO3 DispatchSession
             │    → Rust engine → sky_dispatch_core + sky_dispatch_win32
             └─ Python branch / diagnostic path:
                  RuntimeDispatchCoordinator
                  → DispatchLoop
                  → PlaybackSupervisor
                  → InputBackend
                  → WinSendInputBackend / DryRunBackend
                  → platform.win32.inputs / realtime / wait / MMCSS
```

The native branch is already the default for the real Win32 path, but it is
not yet the only execution model. In particular, `PlaybackEngine.__init__`
compiles a Python `RuntimeSchedule` before native selection
(`src/sky_music/orchestration/engine.py:486`), and `play()` recompiles it at
`engine.py:1137`. The native adapter also owns a Python heartbeat thread and
maps a large Rust dictionary snapshot into the existing Python health and
telemetry contracts.

The native public module at this commit reports:

```text
DispatchSession
NativeDispatchSessionPy
build_info
calibration_schema_version
measure_spin_overhead_rs
qpc_now_rs
run_calibration_rs
simulate_schedule_rs
sleep_until_rs
```

This is an inventory observation, not a decision to remove calibration or
testing APIs in the same change as the dispatch-core removal.

## 3. Python legacy component inventory

### 3.1 Production runtime components

| Component | Current location / call sites | Classification | Planned disposition |
|---|---|---|---|
| `RuntimeDispatchCoordinator` | `orchestration/core/coordinator.py:144`; imported and constructed by `engine.py:92,621,1254,1496`; typed by `playback_supervisor.py:21,271,286,311` | Python production dispatch core and compatibility path | Remove after native admission and golden-vector coverage |
| `compile_runtime_intents` | `core/coordinator.py:90`; `engine.py:94,486,1137,1494`; re-exported by `runtime_dispatch.py:34` | Python duplicate compiler | Remove from native path in Phase 1, delete when no preview consumer remains |
| `DispatchLoop` | `core/loop.py:270`; re-export `dispatch_loop.py:16`; built by `engine.py:998-1025`; cached by `engine.py:1486`; passed through `playback_supervisor.py:269,285,310` | Python real-time worker | Delete in Phase 4 |
| `DispatchHealthMonitor` | `core/loop.py:123`; constructed by `engine.py:634`; re-exported by `dispatch_loop.py:13` | Python dispatch health supervisor | Replace by native snapshot/report mapping, then delete |
| `PlaybackState` | `core/state.py:29`; imported by `engine.py:53`; constructed at `engine.py:1283`; passed to `PlaybackSupervisor` | Python pause/clock state | Delete with Python worker; keep only UI-facing state if a consumer proves necessary |
| `PlaybackSupervisor` | `orchestration/playback_supervisor.py:220`; constructed by `engine.py:1287` | Python dispatch-thread supervisor | Delete if no non-dispatch consumer remains |
| `SendLatencyEstimator` | `engine.py:101`; constructed at `engine.py:531` | Python sender latency estimator | Delete; Rust estimator remains the production estimator |
| `InputBackend` | `infrastructure/backend.py:68`, re-exported via `core/ports.py:38`; consumed by `engine.py:17` and `core/loop.py` | Python sender port | Delete after preview is separated |
| `_TrackedKeyState` | `infrastructure/backend.py:105` | Python physical-key tracker | Delete; Rust owns active/possibly-active state |
| `WinSendInputBackend` | `infrastructure/backend.py:457`; created by `console_playback.py:570` and `ui/textual_app/app.py:823` | Python Win32 sender | Remove from production path in Phase 5 |
| `DryRunBackend` | `infrastructure/backend.py:685`; selected by the same CLI/UI sites | Legacy dry-run implementation coupled to sender protocol | Replace with a named preview abstraction; do not rename the scheduler into a production backend |
| `DispatchPolicy` / `DispatchPlan` | `orchestration/dispatch_policy.py:18,36`; built by `engine.py:489,493`; resolved by `engine.py:674-765` | Backend selector and migration policy | Remove `auto/rust/python`; native is the only production mode |
| `probe_native_dispatch` | `native_dispatch.py:223`; used by `engine.py:680,752`, `main.py:634`, `infrastructure/doctor.py:115` | Per-admission native capability probe | Move admission to startup-only contract check; keep doctor-specific diagnostics separate |
| `NativeHeartbeatThread` | `native_dispatch.py:404`; spawned at `native_dispatch.py:664` | Extra native liveness thread | Remove; heartbeat comes from the control/UI polling loop |

Additional Python dispatch surfaces found by search:

- `orchestration/dispatch_loop.py` and `orchestration/runtime_dispatch.py` are
  compatibility re-export modules, not independent implementations.
- `orchestration/core/__init__.py` re-exports the core classes and therefore
  keeps the old public import surface alive.
- `orchestration/core/rust_adapter.py:38` defines `RustInputAdapter`, which
  implements the Python `InputBackend` protocol by calling the low-level
  PyO3 `RustInputBackend` one key at a time.
- `platform/win32/inputs.py` contains the Python `ctypes` `SendInput` binding,
  packet cache, partial-send/retry logic, and Python sender diagnostics.
- `infrastructure/realtime.py` and adjacent wait/MMCSS helpers contain
  dispatch support that is only needed by the Python sender path; each use
  must be checked before deletion because infrastructure also contains focus
  and hotkey concerns.

### 3.2 Non-production, test, benchmark, or migration consumers

The following files were returned by the Phase 0 symbol search. They are not
all equivalent: some test Python behavior directly, some are benchmarks, and
some lock migration compatibility that must be rewritten rather than shimmed.

**Python coordinator/compiler/loop tests and measurements**

```text
tests/test_runtime_dispatch.py
tests/test_runtime_dispatch_bounded_memory.py
tests/test_adaptive_lead.py
tests/test_adaptive_spin.py
tests/test_engine_equivalence.py
tests/test_phase1_correctness.py
tests/test_phase2_hotpath.py
tests/test_phase3_boundaries.py
tests/test_phase4_lifecycle.py
tests/test_phase5_degraded_wait.py
tests/test_phase6_warmup_budget.py
tests/test_phase7_lead_snapshot.py
tests/test_release_retry.py
tests/test_scalar_drain.py
tests/test_spin_reprobe.py
tests/test_threaded_dispatch.py
tests/measure_conflict_matrix.py
tests/bench_pop_due_pending.py
tests/bench_counter_aggregation.py
```

**Python sender/backend tests and benchmarks**

```text
tests/test_advanced.py
tests/test_backend_hotpath.py
tests/test_focus_input_lifecycle.py
tests/test_input_path_degraded_sync.py
tests/test_inputs_prewarm.py
tests/test_inputs_signature.py
tests/test_send_diagnostics.py
tests/bench_backend_result_normalization.py
tests/bench_dispatch_send_pedantic.py
```

**Native migration/fallback tests**

```text
tests/test_native_dispatch_selection.py
tests/test_native_doctor.py
tests/test_native_heartbeat.py
tests/test_rust_phase2_differential.py
tests/test_rust_phase3_backend.py
tests/test_rust_phase5_adapter.py
tests/test_rust_phase7_native_engine.py
```

The high-risk migration locks are:

- `test_rust_phase2_differential.py`: compares Rust simulation against the
  Python coordinator and therefore makes Python the long-lived oracle.
- `test_rust_phase3_backend.py` and `test_rust_phase5_adapter.py`: exercise
  the low-level Rust/Python input adapter surface.
- `test_native_dispatch_selection.py`: asserts the `auto/rust/python` policy
  and the environment-controlled fallback behavior.
- `test_native_heartbeat.py`: asserts the separate Python heartbeat thread.
- `test_engine_equivalence.py`: compares execution models instead of testing
  the sole native session contract.

These tests must be rewritten or removed in their owning phases. No new
compatibility shim should be added merely to preserve these imports.

## 4. Rust production component inventory

| Component | Location | Current role | Phase 0 classification |
|---|---|---|---|
| `sky_dispatch_core::compile` | `rust/crates/sky_dispatch_core/src/compile.rs` | Authored-action validation and runtime schedule compilation | Keep; Rust becomes the only production compiler |
| `sky_dispatch_core::coordinator::RuntimeDispatchCoordinator` | `rust/crates/sky_dispatch_core/src/coordinator.rs` | Generation lifecycle, pending release and deadline planning | Keep as native worker internals; later state simplification is explicitly out of scope for core-removal PRs |
| `sky_dispatch_core::estimator::SendLatencyEstimator` | `rust/crates/sky_dispatch_core/src/estimator.rs` | Adaptive lead and sender residual estimation | Keep unchanged until a separately benchmarked estimator PR |
| `sky_dispatch_core::testing` | `rust/crates/sky_dispatch_core/src/testing.rs` | Rust simulation/test support | Review after golden vectors; do not expose production PyO3 knobs to retain it |
| `sky_dispatch_win32::input` | `rust/crates/sky_dispatch_win32/src/input.rs` | Validated Win32 SendInput, retry, rollback and tracked physical state | Keep; sole production sender |
| `sky_dispatch_win32::{wait,timer,event,sleeper,mmcss,power,cpu,clock,focus}` | `rust/crates/sky_dispatch_win32/src/` | QPC, waits, focus, thread/runtime scopes and Win32 support | Keep behind native worker |
| `sky_player_rs::DispatchSession` | `rust/crates/sky_player_rs/src/lib.rs:467` | PyO3 session boundary for the native worker | Keep and shrink/normalize in Phase 6 |
| `sky_player_rs::NativeDispatchSessionPy` | `rust/crates/sky_player_rs/src/lib.rs:1041` registration | Current alternate/native session binding surface | Consolidate with the intended `DispatchSession` public surface after consumer audit |
| `RustInputBackend` | `rust/crates/sky_player_rs/src/lib.rs:202,1040` | Low-level diagnostic adapter | Migration artifact; remove in Phase 3 |
| `sky_player_rs::engine` | `rust/crates/sky_player_rs/src/engine.rs` | Native worker, lifecycle, generation coordination, waits, focus, input, telemetry | Keep; modularize only in Phase 7 |
| calibration bindings | `sky_player_rs` and `sky_dispatch_win32` calibration modules | Calibration-only native path | Keep separate; feature-gate or split later, not in the Python-core deletion PR |

Rust core tests and benches currently use the native coordinator directly:

```text
rust/crates/sky_dispatch_core/tests/properties.rs
rust/crates/sky_dispatch_core/benches/coordinator_benchmark.rs
rust/crates/sky_dispatch_core/benches/drift_benchmark.rs
rust/crates/sky_dispatch_core/benches/delivery_proxy_benchmark.rs
rust/crates/sky_dispatch_core/benches/soak_benchmark.rs
rust/crates/sky_dispatch_core/src/compile.rs       (unit tests)
rust/crates/sky_dispatch_core/src/coordinator.rs   (unit tests)
rust/crates/sky_dispatch_core/src/estimator.rs     (unit tests)
rust/crates/sky_dispatch_core/src/testing.rs       (unit tests)
rust/crates/sky_player_rs/src/engine.rs            (unit tests)
```

These are native implementation tests, not reasons to keep a Python oracle.

## 5. Dependency graph and migration seams

```text
Song / authored KeyAction
        │
        ▼
PlaybackEngine
        ├─ current: Python compile_runtime_intents → RuntimeSchedule
        ├─ current: DispatchPolicy + probe_native_dispatch
        ├─ native: RustDispatchRuntime
        │          → PyO3 DispatchSession / NativeDispatchSessionPy
        │          → sky_player_rs::engine
        │          → sky_dispatch_core + sky_dispatch_win32
        └─ legacy: RuntimeDispatchCoordinator
                   → DispatchLoop
                   → PlaybackSupervisor
                   → InputBackend
                   → WinSendInputBackend
                   → platform.win32.inputs

Independent migration bridge:

RustInputAdapter
        → PyO3 RustInputBackend
        → sky_dispatch_win32 input
```

The Phase 1 seam is before `RuntimeSchedule` construction: native admission
must occur before any Python compiler/coordinator allocation. The Phase 2 seam
is the test boundary: Rust golden fixtures replace Python differential output.
The Phase 3 seam is the direct low-level adapter. The Phase 4/5 seams are the
Python coordinator and Python Win32 sender respectively. Phase 6 then reduces
the remaining PyO3 contract to session/config/snapshot/report behavior.

## 6. Environment flags and policy inventory

The following migration controls are still present in source, tests, or active
documentation:

```text
SKY_USE_PYTHON_DISPATCH
SKY_USE_RUST_DISPATCH
SKY_REQUIRE_RUST_DISPATCH
dispatch_backend = auto | rust | python
fidelity_mode = normal | strict
```

At baseline, `SKY_USE_RUST_DISPATCH` and `SKY_REQUIRE_RUST_DISPATCH` are mostly
legacy/test/documentation surfaces, while `SKY_USE_PYTHON_DISPATCH` still
actively selects rollback behavior. Removal belongs to Phase 8, after the
native-only path and test replacements exist.

## 7. Baseline gates and results

All commands below were run against the baseline commit. The successful Python
commands used `UV_CACHE_DIR=.uv-cache` and `--env-file .env`.

| Gate | Result |
|---|---|
| `uv sync --frozen` | PASS after repository-local cache/network setup |
| `uv run --env-file .env ruff check .` | PASS — all checks passed |
| `uv run --env-file .env pyright` | PASS — 0 errors, 0 warnings, 0 informations |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS |
| `cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --all-features` | PASS |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-features` | PASS — 145 tests, 0 failed |
| `uv run --env-file .env pytest -m "not slow"` | PASS — 1108 passed, 10 skipped, 1 xpassed, 1 warning |
| `uv run --env-file .env python scripts/build_rust_wheel.py` | PASS — cp314t wheel, GIL disabled, ABI/commit/fingerprint verified |
| `uv run --env-file .env python scripts/audit_security_mandates.py` | PASS — no forbidden Windows API references |

The first pytest collection attempt, before rebuilding the native wheel, was
not a code failure: `uv sync --frozen` had removed the local `sky_player_rs`
wheel and collection reported seven `ModuleNotFoundError` errors. The required
Windows wheel build restored the exact native wheel; the complete non-slow
suite then passed as recorded above.

### Native acceptance benchmark

Command:

```powershell
uv run --env-file .env python scripts/bench_native_acceptance.py `
  --actions 256 `
  --repeats 3 `
  --polyphony 1,2,3,5,8,15 `
  --baseline .github/native_acceptance_baseline.json `
  --label pre-consolidation
```

Result: PASS, deterministic mock backend, 9,216 completion samples, all
outcomes `finished`, `keys_dropped=0`, `failed_release_count=0`,
`chord_split_events=0`, `sendinput_partial_events=0`,
`sendinput_zero_progress_failures=0`.

Aggregate sender-completion metrics:

```text
p50  = 651 us
p95  = 2121 us
p99  = 3039 us
max  = 11252 us
```

This is a deterministic coordinator-delivery simulation (`backend: mock`),
not evidence of live game/audio timing. It is retained as the pre-consolidation
performance reference and must not be edited to make a later gate pass.

## 8. Security and invariant safety net

Phase 0 made no production behavior change and removed zero legacy lines. The
security audit passed at the baseline. Every later phase must continue to
verify directly that:

- game files, game memory, process hooks, injection, debugger attach, and
  anti-cheat bypass remain untouched;
- production input simulation uses only the existing validated Windows
  `SendInput` seam, now owned by Rust;
- partial sends, stale ups, minimum hold, focus-loss release, bounded cleanup,
  cleanup residue, and fail-closed native errors remain explicit outcomes;
- QPC/tick-domain scheduling and no-Python-on-the-deadline-path remain intact;
- no broad exception handler turns native integrity failures into warnings or
  silently selects Python playback.

## 9. Phase 0 exit criteria

- [x] Baseline commit, architecture, component inventory, dependency graph,
  fallback/oracle tests, and Definition of Done are recorded.
- [x] All listed legacy symbols were searched in `src`, `tests`, and `rust`.
- [x] Baseline gates and their actual results are recorded.
- [x] Native wheel and native acceptance baseline were run on Windows.
- [x] No production behavior, schema, estimator, or native timing was changed.
- [x] No legacy lines were removed (`0` at Phase 0).
- [ ] A clean commit containing this safety-net document must be made before
  the next PR/phase; this agent does not commit unless explicitly asked.

## 10. Definition of Done for the full consolidation

- [ ] Production Python contains no `SendInput`, sender latency estimator,
  dispatch coordinator, or real-time dispatch loop.
- [ ] Native playback does not compile a Python runtime schedule; authored
  actions are compiled once in Rust.
- [ ] There is no `auto/rust/python` production selector, runtime fallback, or
  legacy dispatch environment flag.
- [ ] `RustInputAdapter` and PyO3 `RustInputBackend` are gone from active
  source/tests.
- [ ] Production native API has no mock/fault-injection knobs.
- [ ] Python only creates the native session, sends commands, updates target
  HWND, polls a small live snapshot, receives one final report, and renders or
  persists application-level data.
- [ ] Native telemetry has one final-report path; heartbeat is driven by the
  control loop; target HWND is the focus source of truth.
- [ ] Frozen golden vectors replace the Python differential oracle; Rust unit,
  property, fault-injection, and native integration tests remain green.
- [ ] Native wheel build, acceptance benchmark, full pytest, Rust gates, and
  security audit pass.
- [ ] Quit, skip, pause, focus loss, panic, and error paths leave no held key;
  uncertain cleanup is reported as failure.
- [ ] Current documentation describes Rust as the only production dispatch
  implementation and describes rollback as application-version rollback.

## 11. Next phase

### Phase 1 result — native path bypasses Python compilation

**Summary.** Native admission is resolved before Python sender diagnostics,
prewarm, or schedule preparation. Native `PlaybackEngine` instances leave
`runtime_schedule` unset and pass authored actions to `RustDispatchRuntime`;
non-native/preview compatibility instances still compile the Python schedule.
The estimator polyphony capacity is now derived directly from authored down
actions, so sizing it does not require Python schedule compilation.

**Files changed.**

- Modified: `src/sky_music/orchestration/engine.py`
- Modified: `tests/test_native_dispatch_selection.py`
- Added: this Phase 0/Phase 1 working-note document
- Deleted: none

**Legacy removal.** `0` legacy lines removed in Phase 1. This phase changes
admission order only; it does not delete the Python implementation.

**Invariant checks.** The native path does not call
`compile_runtime_intents`, does not build a Python `RuntimeSchedule`, does not
prewarm Python `INPUT` arrays, and continues to use the existing Rust session
for scheduling, input, cleanup, and timing. Native-unavailable behavior remains
fail-closed. No P0 security surface was changed.

**Tests actually run.**

```text
pytest tests/test_native_dispatch_selection.py tests/test_runtime_dispatch.py
  -k "native_playback_does_not_compile_python_schedule or
      runtime_compilation_happens_before_playback_clock_starts"
  2 passed

pytest tests/test_engine_refactor.py tests/test_engine_equivalence.py
  tests/test_memory_hygiene_prs.py tests/test_phase4_lifecycle.py
  tests/test_phase8_resource_wiring.py tests/test_adaptive_lead.py
  81 passed

pytest -m "not slow"
  1109 passed, 10 skipped, 1 xpassed, 1 warning

ruff check (changed files)       PASS
pyright                           PASS
```

The acceptance benchmark was not rerun on the dirty working tree because its
own provenance guard requires a clean tree. It was not bypassed and the Phase
0 benchmark remains the unmodified pre-consolidation reference. Rust source,
native timing, and native ABI were unchanged, so the Phase 1 change has no
native benchmark input beyond Python admission overhead.

**Remaining legacy symbols.** `RuntimeDispatchCoordinator`,
`compile_runtime_intents`, `DispatchLoop`, `PlaybackSupervisor`,
`SendLatencyEstimator`, `InputBackend`, `WinSendInputBackend`,
`RustInputAdapter`, the selector policy, environment flags, and
`NativeHeartbeatThread` remain intentionally for their Phase 2–8 owners. The
Python differential tests and low-level adapter tests remain until their
replacement phases.

**Risks.** Native admission now occurs during `PlaybackEngine` construction,
so an invalid native extension can fail at construction instead of at
`play()`. This is intentional fail-closed admission and is covered by the
selection tests. The compatibility schedule remains available for non-native
instances and poisoned Python teardown behavior remains covered.

### Phase 2 result — frozen Rust core vectors replace the Python differential oracle

**Summary.** The Python-vs-Rust differential test was deleted. Its replacement
is a committed JSON corpus under `tests/golden/native_dispatch/` and a Rust
integration loader at `rust/crates/sky_dispatch_core/tests/golden.rs`. The
loader calls `sky_dispatch_core::testing::simulate_schedule` directly, so the
production PyO3 surface is not widened for test loading. The now test-only
`simulate_schedule_rs` function was removed from `sky_player_rs`.

**Files changed.**

- Added: `tests/golden/native_dispatch/core_simulation.json`
- Added: `rust/crates/sky_dispatch_core/tests/golden.rs`
- Deleted: `tests/test_rust_phase2_differential.py` (550 lines)
- Modified: `rust/crates/sky_player_rs/src/lib.rs` (removed 25-line test-only
  PyO3 function/registration)
- Modified: `rust/crates/sky_player_rs/Cargo.toml` was checked and left with
  `serde_json`, because the native engine still uses it for telemetry and
  estimator state

**Legacy removal.** 575 legacy test/bridge lines removed in Phase 2: 550
Python differential-oracle lines and 25 PyO3 simulation-surface lines. No
production scheduler, sender, estimator, or cleanup implementation was
removed in this phase.

**Corpus coverage.** The initial frozen corpus has 11 deterministic core
scenarios: single lifecycle, chord lifecycle, minimum-hold extension,
repeated same-key lifecycle, stale-up suppression, empty schedule, polyphony
15, same-key overlap rejection, duplicate scan-code rejection, allowlist
rejection, and non-monotonic timestamp rejection. Focus/pause, SendInput fault
injection, recovery, and supervisor lease behavior remain covered by existing
Rust native tests and are deliberately not represented by the Python
simulation API; expanding those into native-session fixtures is still a
follow-up before the final Definition of Done claim.

**Invariant checks.** `rg` reports no active `simulate_schedule_rs` match in
`src`, `tests`, or `rust`. No Python coordinator is imported by the deleted
differential test. Rust's existing property and fault-injection suites remain
in place. The security audit still reports no forbidden Windows API
references.

**Tests actually run.**

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check       PASS
cargo check --manifest-path rust/Cargo.toml --workspace
  --all-targets --all-features                                  PASS
cargo clippy --manifest-path rust/Cargo.toml --workspace
  --all-targets --all-features -- -D warnings                   PASS
cargo test --manifest-path rust/Cargo.toml --workspace
  --all-features                                                PASS
  core 60 + golden 1 + properties 12 + win32 48 + engine 25

build_rust_wheel.py --allow-dirty-development-build             PASS
  cp314t ABI, GIL disabled, fingerprint and dirty provenance verified
native integration subset                                           PASS
  63 passed, 10 skipped
pytest -m "not slow"                                           PASS
  1097 passed, 10 skipped, 1 xpassed, 1 warning
ruff check .                                                   PASS
pyright                                                        PASS
security audit                                                  PASS
```

The clean-tree wheel command was intentionally attempted first and failed at
the script's provenance guard. The local development-only dirty flag was then
used; it marks the artifact `native_build_commit=<sha>-dirty` and still runs
the exact wheel, ABI, GIL, import, and fingerprint checks. No release baseline
was modified.

**Remaining legacy symbols.** Python coordinator/loop/backend tests remain for
the Phase 4 and Phase 5 migrations. `RustInputAdapter`, PyO3
`RustInputBackend`, `diagnostic-backend`, selector policy, environment flags,
and `NativeHeartbeatThread` are unchanged and are not hidden behind renamed
compatibility symbols.

**Risks.** The frozen corpus currently covers the pure core simulation seam,
not every native-session fault mode listed in the end-state proposal. The
existing Rust tests are the safety net for those modes until a native-session
fixture format is added. Removing the PyO3 simulation function also means
development code must use Rust integration tests rather than a Python oracle.

### Phase 3 result — low-level Rust/Python input adapter removed

**Summary.** The one-key-at-a-time `RustInputAdapter` and its PyO3
`RustInputBackend` have been removed. The `diagnostic-backend` Cargo feature
and the adapter-only tests are gone. `parking_lot` remains because the native
engine still uses it for worker metrics; it was not incorrectly removed as an
adapter-only dependency. Existing Rust `TrackedKeyState`, input retry, and
fault-injection tests remain the coverage for the actual native session seam.

**Files changed.**

- Deleted: `src/sky_music/orchestration/core/rust_adapter.py`
- Deleted: `tests/test_rust_phase3_backend.py`
- Deleted: `tests/test_rust_phase5_adapter.py`
- Modified: `rust/crates/sky_player_rs/src/lib.rs`
- Modified: `rust/crates/sky_player_rs/Cargo.toml`
- Modified: `docs/architecture.md` to remove the obsolete diagnostic adapter
  claim
- Deleted: no Rust core input implementation; `sky_dispatch_win32` remains the
  sole native input owner

**Legacy removal.** 621 adapter-related lines removed: 298 Rust PyO3 bridge
lines, 135 Python adapter lines, 127 adapter backend-test lines, 60 adapter
lifecycle-test lines, and the one Cargo feature declaration. No native sender
or tracked-key implementation was removed.

**Invariant checks.** `rg` over `src`, `tests`, and `rust` reports no active
match for `RustInputAdapter`, `RustInputBackend`, or `diagnostic-backend`.
The Rust input module still owns validated `SendInput`, partial/zero-progress
handling, rollback and cleanup. No Python per-key call remains on the native
surface. The P0 security audit passes.

**Tests actually run.**

```text
cargo fmt --manifest-path rust/Cargo.toml --all -- --check       PASS
cargo clippy --manifest-path rust/Cargo.toml --workspace
  --all-targets --all-features -- -D warnings                   PASS
cargo test --manifest-path rust/Cargo.toml --workspace
  --all-features                                                PASS
  core 60 + golden 1 + properties 12 + win32 48 + engine 25

build_rust_wheel.py --allow-dirty-development-build             PASS
  cp314t ABI, GIL disabled, dirty commit/fingerprint verified
native session subset                                          PASS
  74 passed, 2 skipped
pytest -m "not slow"                                           PASS
  1095 passed, 2 skipped, 1 xpassed, 1 warning
ruff check .                                                   PASS
pyright                                                        PASS
security audit                                                  PASS
```

The full release wheel gate still requires a clean commit; the local dirty
development flag was used only because this workspace intentionally contains
the phase changes. It did not skip wheel import, ABI, GIL, or fingerprint
verification.

**Remaining legacy symbols.** Python `RuntimeDispatchCoordinator`,
`DispatchLoop`, `PlaybackSupervisor`, `SendLatencyEstimator`,
`InputBackend`, `WinSendInputBackend`, selector policy, environment flags, and
`NativeHeartbeatThread` remain for their separate phases. The Rust
`RuntimeDispatchCoordinator` remains as the native core implementation and is
not a migration artifact.

**Risks.** Any external consumer importing the deleted adapter now fails at
import time. The Phase 0 inventory found only in-repository test/compatibility
consumers, all of which were removed or are covered by the native session.
No replacement adapter was introduced.

**Next phase.** Phase 4 should remove the Python real-time dispatch core and
create a clearly named preview/dry-run path only where an actual UI/CLI
consumer requires one. Before editing it must repeat `git status --short`,
`git rev-parse HEAD`, and the full production/test symbol inventory. The
Python Win32 sender stack remains for the separately scoped Phase 5.

## 12. As-built consolidation status — 2026-08-03

The implementation now completes the requested Rust-only production dispatch
direction. The earlier phase notes above describe the baseline and are kept as
historical inventory; this section records the resulting code state.

### Phase 4 and 5 result

- Deleted the Python coordinator, real-time loop, runtime state, supervisor,
  dispatch policy, runtime-dispatch module, Python backend, Python wait/priority
  helpers, and Python sender watchdog.
- Deleted Python `ctypes.SendInput` packet construction, key tracking, sender
  retry/cleanup, waitable-timer dispatch helpers, and the direct Python
  calibration sender.
- Added `platform.win32.window_target` for validated window discovery,
  foreground checks, focus requests, and target HWND caching only.
- Kept preview as an explicit no-input path; it is not a dispatch backend or
  timing oracle.
- Removed the Python differential oracle and Python-only dispatch benchmarks;
  Rust golden, property, and fault-injection coverage remains the production
  safety net.

### Phase 6 result

- Reduced the PyO3 playback constructor to authored actions, the prepared
  allowlist, and `SessionConfig`; production code cannot request mock backend,
  mock latency, fault-injection, telemetry-capacity, wait, priority, or
  estimator knobs.
- Removed the `update_focus(bool)` control path. `set_target_hwnd(hwnd)` is
  the sole focus input; Rust derives foreground state from that target.
- Added `snapshot_lite()` for UI polling and `session_report()` for one final
  report after worker termination.
- Removed `NativeHeartbeatThread`; heartbeat is driven by the control/UI loop.
- Removed backend selector flags and dead dispatch tuning CLI/config fields.
- Removed unused public PyO3 timing helpers and the old native-session alias.

### Rust boundary support result

- Added a `test-support` feature for the native-only acceptance mock session;
  production wheels do not compile or expose that class.
- Added a separate `calibration` feature. The normal playback wheel exposes
  only `DispatchSession`, `SessionConfig`, and `build_info`; calibration remains
  in its intentionally separate Rust process/package path.

### Verification recorded for this working tree

The following gates have passed during this consolidation:

```text
ruff check .                                                     PASS
pyright                                                         PASS
cargo fmt --workspace -- --check                                PASS
cargo check --workspace --all-targets --all-features             PASS
cargo clippy --workspace --all-targets --all-features -D warnings PASS
cargo test --workspace --all-features                            PASS
build_rust_wheel.py --allow-dirty-development-build              PASS
security audit                                                   PASS
pytest: 511 passed, 1 xpassed, 1 warning                        PASS
native TestDispatchSession smoke                                PASS
```

The native acceptance command was also attempted exactly as specified, but
its provenance guard rejected this intentionally dirty working tree before
running a session: `RuntimeError: acceptance evidence requires a clean
worktree`. No baseline artifact was modified or bypassed. The production wheel
was rebuilt after the separate `test-support` smoke check, so the active
environment does not expose `TestDispatchSession`.

### Final active-symbol audit

No active Python source or test consumer remains for
`RuntimeDispatchCoordinator`, `DispatchLoop`, `PlaybackSupervisor`,
`SendLatencyEstimator`, `InputBackend`, `WinSendInputBackend`,
`RustInputAdapter`, `RustInputBackend`, `NativeHeartbeatThread`, or the three
dispatch-selection environment flags. The remaining
`RuntimeDispatchCoordinator` symbol in Rust is the native implementation, not
the removed Python migration artifact. Textual `SendInput` references are
security/documentation diagnostics; the Python Win32 sender implementation and
ctypes binding are gone.
