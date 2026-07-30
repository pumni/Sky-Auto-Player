# Source map — baseline `45fa3c83b1642fb6c91caa2c13b3ce7d1a6cad52`

## Core runtime

| Path | Symbols/contract |
|---|---|
| `src/sky_music/orchestration/core/loop.py` | `DispatchLoop`, wait/drain/execute, focus/commands, abort, telemetry |
| `src/sky_music/orchestration/core/coordinator.py` | generation compiler, active/pending state, deadlines, no-early-conflict |
| `src/sky_music/orchestration/core/state.py` | single-interval pause clock, display snapshot |
| `src/sky_music/orchestration/core/ports.py` | command/focus/progress/backend/estimator seam |
| `src/sky_music/orchestration/playback_supervisor.py` | Python control thread, command event, focus sampling, worker lifecycle |
| `src/sky_music/orchestration/engine.py` | composition, estimator, prewarm, adaptive probe, teardown |

## Infrastructure/platform

| Path | Symbols/contract |
|---|---|
| `src/sky_music/infrastructure/backend.py` | InputBackend, tracked keys, WinSendInputBackend, release_all |
| `src/sky_music/infrastructure/wait_strategy.py` | hybrid timer/event/poll/spin behavior |
| `src/sky_music/infrastructure/timing.py` | clock/sleeper/policy |
| `src/sky_music/infrastructure/realtime.py` | realtime process scope/sleeper creation |
| `src/sky_music/infrastructure/rt_priority.py` | MMCSS/thread priority/power scope |
| `src/sky_music/platform/win32/inputs.py` | ctypes Win32, INPUT cache, SendInput retry, HWND/focus, timers/events |
| `src/sky_music/orchestration/telemetry.py` | CSV schema, retain-first capacity, summaries |

## Normative docs

- `AGENTS.md`
- `docs/rt-dispatch-architecture.md`
- `docs/timing-principles.md`
- `docs/timing-profile-frame-model.md`
- `docs/architecture.md`
- `docs/distribution.md`

## Existing proposal docs requiring reconciliation

- `docs/rust-migration-plan.md`
- `docs/PORTING_GUIDE.md`

## High-value tests

- `tests/test_core_send_overhaul_invariants.py`
- `tests/test_runtime_dispatch.py`
- `tests/test_dispatch_fidelity_refactor.py`
- `tests/test_dispatch_fidelity_edges.py`
- `tests/test_phase1_correctness.py`
- `tests/test_phase4_lifecycle.py`
- `tests/test_core_boundary.py`
- `tests/test_send_diagnostics.py`
- `tests/test_engine_refactor.py`

## Fixed commit links

Base URL:

```text
https://github.com/pumni/Sky-Auto-Player/blob/45fa3c83b1642fb6c91caa2c13b3ce7d1a6cad52/<path>
```

AI phải dùng fixed-commit links trong design/PR evidence, không chỉ link `main` thay đổi theo thời gian.
