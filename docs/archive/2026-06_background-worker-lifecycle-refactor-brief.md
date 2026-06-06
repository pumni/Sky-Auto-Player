# Background Worker Lifecycle Refactor Brief

Status: active implementation brief

Owner role split:
- Implementer: another AI agent.
- Reviewer / acceptance owner: Codex in this thread.

This document is the handoff spec for refactoring background worker lifecycle management in Sky
Player. It is intentionally narrow: make the picker-to-playback boundary deterministic and easier
to maintain without changing scheduling, SendInput behavior, CLI behavior, or game interaction.

## 1. Problem Statement

The reported production symptom was intermittent missing notes when running:

```powershell
uv run python src/main.py
```

then selecting a song and playing with:

```text
profile = local-precise
fps = 144
```

The important observation is that the same user flow sometimes plays correctly and sometimes misses
notes. When notes are heard, their timeline is correct. That pattern does not primarily point at
`min_hold`, authored timeline generation, or scheduler math. It points at run-to-run nondeterminism
around the realtime SendInput phase: leftover worker threads/processes, CPU/IO contention, or stale
background tasks crossing from picker into playback.

Recent focused fixes already improved the symptom:

- Classic picker now cancels metadata work and waits for metadata/cache executors before playback.
- Textual picker no longer starts a redundant `metadata-warmup` worker outside
  `MetadataCoordinator`.
- Textual picker calls `MetadataCoordinator.close(wait=True)` on unmount.
- Retired process executors from classic picker fallback are retained for final cleanup.

The next refactor should make that behavior systematic, testable, and future-proof.

## 2. Hard Constraints

Follow `AGENTS.md` exactly. In particular:

- Do not modify game files.
- Do not read game memory.
- Do not bypass anti-cheat or security systems.
- Use Windows SendInput only.
- Preserve current CLI behavior unless explicitly changed.
- Prioritize timing correctness, testability, and strict validation.
- Avoid broad rewrites without tests.
- Use `uv run <command>` for all Python executions.

Additional constraints for this refactor:

- Do not change scheduler timing logic.
- Do not change `local-precise`, `balanced`, or `audience-safe` profile semantics.
- Do not change `WinSendInputBackend` behavior unless a testable SendInput-side bug is proven.
- Do not change the shape of `SongPickerResult`.
- Do not remove classic picker in this task.
- Do not add a background worker without an explicit owner and close contract.
- Do not rely on garbage collection, `__del__`, process exit, or weak references for cleanup.
- Do not use `shutdown(wait=False)` on the final path from picker into playback.

## 3. Desired Mental Model

Background work is allowed, but only inside a phase with a clear owner.

The application should be understandable as:

```text
PickerPhase
  - UI rendering
  - metadata hydration
  - persistent cache warmup
  - risk analysis for visible songs

PickerPhase.close(wait=True)

PlaybackPhase
  - PlaybackEngine
  - realtime dispatch thread
  - SendInput backend
  - lightweight command/focus polling

PostPlaybackPhase
  - telemetry writing / inspection
  - diagnostics
```

The key invariant:

```text
No picker metadata/cache worker may still be running when PlaybackPhase starts.
```

This is not about having zero threads. It is about having the right threads alive in the right phase.

## 4. Current Worker Inventory

The implementer must inspect current code before editing, but this is the expected inventory.

### 4.1 Classic Picker

Primary file:

```text
src/sky_music/ui/picker.py
```

Current background resources:

- `ProcessPoolExecutor(max_workers=2)` for metadata/risk analysis.
- Fallback `ThreadPoolExecutor(max_workers=2, thread_name_prefix="sky-picker-meta")`.
- `ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-picker-cache")`.
- Future slots in `PickerState`:
  - `metadata_prefetch_future`
  - `metadata_prefetch_future_bulk`
  - `metadata_hydration_future`
- `metadata_generation` invalidation and coalesced UI refresh scheduling.
- Retired/fallback metadata executors that may need final cleanup.

Important behavior:

- Process pool fallback can retire a broken process executor while the picker stays open.
- Final picker cleanup must wait for both active and retired executors before playback.

### 4.2 Textual Picker

Primary files:

```text
src/sky_music/ui/textual_app/app.py
src/sky_music/ui/textual_app/workers.py
```

Current background resources:

- `MetadataCoordinator`.
- Internal single-thread `ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-metadata-coord")`.

Important behavior:

- `on_mount()` should call `self.metadata.refresh(paths)`.
- There should not be a separate `run_worker(... thread=True)` metadata warmup path for the same work.
- `on_unmount()` must close active metadata work with `wait=True`.
- Replacing metadata after profile/fps/tempo changes may be non-waiting inside the picker for UI
  responsiveness, but replaced/retired resources must not escape final app shutdown.

### 4.3 Playback

Primary files:

```text
src/sky_music/orchestration/engine.py
src/sky_music/infrastructure/realtime.py
src/sky_music/platform/win32/inputs.py
```

Current background/realtime resources:

- Playback dispatch thread named `sky-music-dispatch`.
- High-resolution timer scope.
- MMCSS registration.
- Optional high-resolution waitable timer sleeper.
- Main/control thread polling commands/focus while dispatch thread is alive.

Important behavior:

- Playback lifecycle is separate from picker lifecycle.
- Do not register playback dispatch thread inside the picker background scope.
- Do not weaken current dispatch-thread tests.

## 5. Target Design

Create a small lifecycle abstraction for non-playback background resources. Keep it boring and local.

Recommended module:

```text
src/sky_music/infrastructure/background.py
```

Recommended domain models:

```python
from __future__ import annotations

from concurrent.futures import Executor, Future
from dataclasses import dataclass
from typing import Protocol


@dataclass(frozen=True, slots=True)
class WorkerSnapshot:
    name: str
    phase: str
    closed: bool
    pending_count: int | None = None
    running_count: int | None = None


class BackgroundResource(Protocol):
    @property
    def name(self) -> str: ...

    @property
    def phase(self) -> str: ...

    def cancel(self) -> None: ...

    def close(self, *, wait: bool) -> None: ...

    def snapshot(self) -> WorkerSnapshot: ...
```

Recommended scope:

```python
class BackgroundScope:
    def __init__(self, *, phase: str) -> None: ...

    def register(self, resource: BackgroundResource) -> BackgroundResource: ...

    def retire(self, resource: BackgroundResource) -> None: ...

    def cancel_all(self) -> None: ...

    def close_all(self, *, wait: bool) -> None: ...

    def assert_closed(self) -> None: ...

    def snapshots(self) -> tuple[WorkerSnapshot, ...]: ...
```

Recommended executor wrapper:

```python
class ExecutorResource:
    def __init__(self, *, name: str, phase: str, executor: Executor) -> None: ...

    def submit(self, fn, /, *args, **kwargs) -> Future: ...

    def cancel(self) -> None: ...

    def close(self, *, wait: bool) -> None: ...

    def snapshot(self) -> WorkerSnapshot: ...
```

Implementation notes:

- Track submitted futures inside `ExecutorResource`.
- `cancel()` should call `future.cancel()` for incomplete futures.
- `close(wait=...)` should call `executor.shutdown(wait=wait, cancel_futures=True)`.
- `close()` must be idempotent.
- `BackgroundScope.close_all()` must be idempotent.
- Closing order should be deterministic: retire/cancel first, close in reverse registration order.
- `assert_closed()` should be test-friendly. It can raise a `RuntimeError` if any resource reports
  `closed=False`.
- Do not build a complex scheduler or async framework here. This is lifecycle bookkeeping only.

## 6. Implementation Plan

### Phase 0 - Baseline Verification

Before editing, the implementer must run:

```powershell
uv run pytest tests\test_textual_picker.py tests\test_threaded_dispatch.py tests\test_runtime_dispatch.py tests\test_picker_metadata_optimizations.py
uv run python src\main.py --selftest-textual
```

Record the result in the final handoff.

### Phase 1 - Add Lifecycle Infrastructure

Add:

```text
src/sky_music/infrastructure/background.py
tests/test_background_lifecycle.py
```

Tests must cover:

- `ExecutorResource.close(wait=True)` forwards `wait=True` and `cancel_futures=True`.
- `ExecutorResource.close(wait=False)` forwards `wait=False`.
- `cancel()` cancels pending futures.
- `close()` is idempotent.
- `BackgroundScope.close_all(wait=True)` closes all registered resources.
- `BackgroundScope.close_all()` closes retired resources too.
- Close order is deterministic.

Use fake executors/futures where possible. Do not sleep in unit tests unless absolutely necessary.

### Phase 2 - Refactor Classic Picker Resource Ownership

In:

```text
src/sky_music/ui/picker.py
```

Replace loose executor variables with resource wrappers registered in a picker scope.

Target shape:

```python
picker_scope = BackgroundScope(phase="picker")

metadata_resource = picker_scope.register(
    ExecutorResource(
        name="classic-picker-metadata-process",
        phase="picker",
        executor=ProcessPoolExecutor(max_workers=2),
    )
)

cache_resource = picker_scope.register(
    ExecutorResource(
        name="classic-picker-cache",
        phase="picker",
        executor=ThreadPoolExecutor(max_workers=1, thread_name_prefix="sky-picker-cache"),
    )
)
```

When process pool creation fails, register the fallback thread resource instead.

When process pool breaks during callback:

```python
picker_scope.retire(metadata_resource)
metadata_resource.close(wait=False)
metadata_resource = picker_scope.register(fallback_thread_resource)
```

The exact variable names can differ, but these contracts must remain:

- Active executor is always represented by a `BackgroundResource`.
- Retired executors remain known to the scope.
- `finally` performs:

```python
try:
    _invalidate_metadata_work()
finally:
    picker_scope.cancel_all()
    picker_scope.close_all(wait=True)
    picker_scope.assert_closed()
```

If `assert_closed()` is too strict for production after an exception, the implementer may guard it
behind debug mode or keep it in tests only. The final cleanup still must be `wait=True`.

Testing guidance:

- It is acceptable to factor a small helper such as `ClassicPickerBackground` to make lifecycle
  testable without launching prompt-toolkit.
- Avoid testing by driving the full interactive classic picker unless that is already easy.
- Prefer small unit tests around helper lifecycle and one integration-ish test that monkeypatches
  executor classes.

### Phase 3 - Refactor Textual MetadataCoordinator Lifecycle

In:

```text
src/sky_music/ui/textual_app/workers.py
src/sky_music/ui/textual_app/app.py
```

Make `MetadataCoordinator` conform to `BackgroundResource` or wrap it in one.

Required behavior:

- `MetadataCoordinator.name` should identify the resource, e.g. `textual-picker-metadata`.
- `MetadataCoordinator.phase` should be `picker`.
- `close(wait=True)` should shut down its internal executor with `wait=True`.
- `close(wait=False)` remains allowed for in-picker replacement.
- `cancel()` should cancel the current pending future and set `_closed` or an equivalent state.
- `snapshot()` should expose enough state for tests/debug.

Textual app lifecycle:

- On mount:

```python
self.metadata.refresh(paths)
```

- On profile/fps/tempo replacement:

```python
self.metadata.close(wait=False)
```

or:

```python
self.picker_scope.retire(self.metadata)
self.metadata.close(wait=False)
```

If resources are retired in the Textual picker, final unmount must close them with `wait=True`.

- On unmount:

```python
self.metadata.close(wait=True)
```

or:

```python
self.picker_scope.close_all(wait=True)
```

There must not be a separate Textual `run_worker(..., thread=True)` warmup path that duplicates
`MetadataCoordinator.refresh()`.

### Phase 4 - Optional Debug Snapshots

Add lightweight debug logging only if it fits existing debug conventions.

Recommended behavior:

- When playback is about to start and debug is enabled, print or log picker resource snapshots:

```text
[background] picker resource classic-picker-cache closed=True pending=0 running=0
```

Do not spam normal CLI output.

Do not make gameplay depend on debug logging.

### Phase 5 - Preserve Playback Scope

Do not move `PlaybackEngine` dispatch thread into the picker scope.

Playback currently has its own lifecycle:

- dispatch thread starts in `_run_threaded_dispatch()`
- realtime sleeper is created inside `dispatch_target()`
- high-resolution timer scope and MMCSS registration are scoped to the dispatch thread
- thread is joined before returning

This may be documented, but it should not be refactored as part of this task unless a narrow testable
bug is found.

## 7. Required Tests

The implementer must add or update tests for all of the following.

### 7.1 Infrastructure Tests

File:

```text
tests/test_background_lifecycle.py
```

Required assertions:

- Resources register with a phase and name.
- `close_all(wait=True)` forwards `wait=True` to each resource.
- `close_all(wait=False)` forwards `wait=False` where requested.
- Retired resources are still closed during final scope cleanup.
- Repeated `close_all(wait=True)` does not double-close underlying executors in a harmful way.
- `cancel_all()` is called before final close in the picker helper tests.

### 7.2 Textual Picker Tests

File:

```text
tests/test_textual_picker.py
```

Required assertions:

- Opening and closing the Textual picker closes metadata with `wait=True`.
- Profile/fps/tempo replacement closes the old metadata resource with `wait=False`.
- Final app shutdown still closes the active metadata resource with `wait=True`.
- There is no dependency on `app_module.warm_persistent_metadata_cache`.
- `uv run python src/main.py --selftest-textual` still passes.

### 7.3 Classic Picker Lifecycle Tests

Potential file:

```text
tests/test_classic_picker_lifecycle.py
```

Required assertions:

- Final picker cleanup closes metadata and cache resources with `wait=True`.
- If process metadata executor is retired during fallback, it remains known to the scope and is closed
  again with `wait=True` during final cleanup.
- `_invalidate_metadata_work()` or equivalent cancellation happens before final close.

The implementer may need to extract a helper from `picker.py` to make this testable without launching
the full prompt-toolkit UI. That extraction is acceptable if it is narrow and does not change CLI
behavior.

### 7.4 Regression Suite

Run:

```powershell
uv run pytest tests\test_background_lifecycle.py tests\test_textual_picker.py tests\test_threaded_dispatch.py tests\test_runtime_dispatch.py tests\test_picker_metadata_optimizations.py
uv run python src\main.py --selftest-textual
```

If the repository has a full suite that is practical to run, run:

```powershell
uv run pytest
```

Report whether full suite was run.

## 8. Acceptance Criteria

The reviewer should accept only if all conditions are true:

- Every picker executor/thread has a clear owner.
- Picker cleanup before playback is centralized.
- Final picker cleanup uses `wait=True`.
- Retired/fallback executors are not lost.
- Textual picker has a single metadata lifecycle.
- Playback dispatch lifecycle remains separate.
- Tests prove `wait=True` at the picker-to-playback boundary.
- Tests prove in-picker replacement may be non-waiting without escaping final cleanup.
- CLI behavior is unchanged.
- Existing playback/runtime tests pass.

## 9. Rejection Criteria

Reject the refactor if any of these appear:

- New `ThreadPoolExecutor` or `ProcessPoolExecutor` created without registration or ownership.
- New Textual `run_worker(..., thread=True)` for metadata/cache work outside a lifecycle owner.
- `shutdown(wait=False)` on the final path from picker into playback.
- Cleanup depends on object destruction or process exit.
- Scheduler, timing profiles, or SendInput backend are changed without a separate proven bug.
- Tests only check that methods were called, but not whether `wait=True` is used at final shutdown.
- The refactor hides errors by swallowing all exceptions in the lifecycle layer.
- The playback phase depends on picker resources.

## 10. Reviewer Procedure

When the implementer hands back the diff, review in this order:

1. Search for background resource creation:

```powershell
rg -n "ThreadPoolExecutor|ProcessPoolExecutor|run_worker\\(|thread=True|threading.Thread|shutdown\\(" src tests
```

2. Verify every picker resource has ownership and close semantics.
3. Verify final picker-to-playback cleanup uses `wait=True`.
4. Verify retired/fallback resources are retained.
5. Verify Textual picker has no duplicate metadata warmup worker.
6. Verify playback dispatch was not unnecessarily rewritten.
7. Run required tests.
8. If possible, manually smoke:

```powershell
uv run python src/main.py --ui textual
```

Select a song with `local-precise` and `144 FPS`, then play. The reviewer must not infer game
acceptance from telemetry alone; game audio/onset observation wins.

## 11. Suggested Handoff Prompt For The Implementer

Use this prompt for the implementation agent:

```text
Refactor Sky Player background worker lifecycle for the picker-to-playback flow only.

Read AGENTS.md and docs/2026-06_background-worker-lifecycle-refactor-brief.md first. Do not change
scheduler timing, timing profiles, SendInput backend, game interaction, CLI behavior, or
SongPickerResult. Introduce a small lifecycle abstraction for picker background resources so all
metadata/cache executors have explicit ownership, cancellation, close(wait=...) semantics, and
snapshots for tests/debug. Classic picker final shutdown before playback must cancel and close all
active and retired metadata/cache resources with wait=True. Textual picker must use a single
MetadataCoordinator lifecycle and no redundant metadata warmup worker. In-picker profile/fps/tempo
replacement may use wait=False, but final unmount must close all picker resources with wait=True.
Add tests proving those contracts, then run the required uv commands from the brief.
```

## 12. Notes For Future Features

Future features should follow these rules:

- A feature may create background work only inside a phase scope.
- The feature must name its resource.
- The feature must define whether it may run during playback.
- If it may not run during playback, it must be registered in the picker scope and closed with
  `wait=True` before playback starts.
- If it may run during playback, it belongs to playback lifecycle and must be reviewed for realtime
  impact.
- Any new resource should come with at least one lifecycle test.

This keeps the codebase scalable without pretending that "no threads" is a realistic goal.

