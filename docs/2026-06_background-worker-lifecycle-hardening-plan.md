# Background Worker Lifecycle Hardening Plan

Status: active implementation plan

Owner role split:
- Implementer: another AI coding agent.
- Reviewer / acceptance owner: Codex in this thread.

This is the second-stage plan after the initial background lifecycle refactor. The first refactor
made the picker-to-playback boundary observable and, in the latest debug run, proved:

```text
[background] picker resource textual-picker-metadata closed=True pending=0 running=0
[play] start ...
```

So this plan is not a rewrite and not a scheduler/SendInput investigation. It is a hardening pass to
make the lifecycle architecture easier to extend, less fragile, and more auditable when new features
add background work later.

## 1. Scope

Improve only background worker lifecycle architecture around the picker and the picker-to-playback
boundary.

In scope:

- `src/sky_music/infrastructure/background.py`
- `src/sky_music/ui/picker.py`
- `src/sky_music/ui/textual_app/app.py`
- `src/sky_music/ui/textual_app/workers.py`
- tests around lifecycle and picker shutdown
- debug/telemetry visibility for lifecycle snapshots
- docs/index updates if needed

Out of scope:

- scheduler timing changes
- min-hold/profile/FPS formulas
- SendInput backend behavior
- game focus strategy, except for reading current lifecycle logs
- changing `SongPickerResult`
- removing classic picker
- broad UI rewrites

## 2. Current State

The current architecture has improved materially:

- Classic picker metadata/cache executors are registered in a `BackgroundScope`.
- Retired classic metadata resources are retained for final cleanup.
- `ExecutorResource.close(wait=False)` no longer calls `ProcessPoolExecutor.shutdown(wait=False)`,
  avoiding the Python 3.14 manager-thread trap.
- Textual picker uses a single `MetadataCoordinator`.
- Textual `on_unmount()` closes the picker scope with `wait=True`.
- Debug log is flushed at process exit and now shows picker resource snapshots.
- Current real run evidence shows the Textual metadata worker is closed before playback.

Remaining weaknesses:

- `MetadataCoordinator` uses `_closed` as a compatibility property alias for `_shutdown_started`.
  This works, but it is not a clean state model.
- `BackgroundScope.cancel_all()` still swallows cancel exceptions.
- Production cleanup failure before playback is logged, but playback can continue. For strict realtime
  safety, cleanup failure should become an explicit abort path.
- Classic picker lifecycle tests still rely on frame locals and private `_resources`; useful, but
  brittle.
- Textual picker tests mostly use fake coordinators; there is limited coverage for the real
  `MetadataCoordinator` worker loop.
- Lifecycle snapshots are visible in debug logs, but not summarized in telemetry JSON.
- There is no static guard preventing future code from adding a new executor/thread outside a phase
  owner.

## 3. Design Goals

The implementer must optimize for these goals:

1. **Phase clarity:** picker resources are not playback resources.
2. **State clarity:** lifecycle state must be explicit and readable.
3. **Failure clarity:** cleanup failure before playback must not be hidden.
4. **Test realism:** tests must cover real lifecycle behavior, not only fake call recording.
5. **Future safety:** adding a new worker should require an explicit lifecycle owner.
6. **Small surface area:** avoid turning this into a framework.

## 4. Required Architecture Changes

### 4.1 Add Explicit Lifecycle State

Replace ad hoc booleans where practical with a tiny state enum or frozen value model.

Recommended:

```python
from enum import Enum

class ResourceState(str, Enum):
    OPEN = "open"
    CLOSING = "closing"
    CLOSED = "closed"
    FAILED = "failed"
```

`WorkerSnapshot` should include:

```python
state: str
closed: bool
last_error: str | None = None
```

Compatibility note:

- `closed=True` must mean final wait completed successfully.
- `state="closing"` means no new submissions are accepted, but final wait has not completed.
- `state="failed"` means cleanup failed and must be visible in snapshot/logs/tests.

### 4.2 Clean Up MetadataCoordinator State

In `src/sky_music/ui/textual_app/workers.py`:

- Remove the `_closed` property alias if feasible.
- Update all internal checks to use a single explicit state helper, for example:

```python
def _is_stopping(self) -> bool:
    return self._state is not ResourceState.OPEN
```

or:

```python
def _should_stop(self) -> bool:
    return self._shutdown_started
```

Do not leave a split-brain model where `refresh()` checks one field and worker stages check another.

Required behavior:

- `refresh()` refuses new work once shutdown starts.
- `cancel()` makes worker stages stop at their next checkpoint.
- `close(wait=False)` prevents new submissions and requests cancellation, but does not fake final
  closure.
- `close(wait=True)` waits for the internal executor to stop and only then marks closed.
- If executor shutdown fails, state becomes failed and snapshot exposes the error.

### 4.3 Make Cleanup Failure Policy Explicit

Current behavior logs cleanup failures in production and raises in tests. Harden this into a policy.

Recommended:

```python
class CleanupPolicy(str, Enum):
    RAISE = "raise"
    LOG_AND_CONTINUE = "log_and_continue"
```

For the picker-to-playback boundary, default should be strict:

```text
cleanup failure before playback => abort playback with a clear message
```

Rationale: if a worker cannot be proven stopped, starting realtime playback violates the lifecycle
contract.

If the implementer decides to keep `LOG_AND_CONTINUE` for production, they must justify it in code
comments and tests. The reviewer should prefer aborting before playback.

### 4.4 Return Cleanup Results

`BackgroundScope.close_all(wait=True)` currently raises on failure. That is fine, but the caller has
limited structured context.

Recommended addition:

```python
@dataclass(frozen=True, slots=True)
class ScopeCloseResult:
    phase: str
    snapshots: tuple[WorkerSnapshot, ...]
    errors: tuple[str, ...]
```

Either:

- return `ScopeCloseResult` on success and raise `BackgroundCleanupError` on failure, or
- return `ScopeCloseResult` always and require caller to check `errors`.

Do not silently swallow errors.

### 4.5 Add Lifecycle Snapshot To Telemetry Summary

When playback starts after picker selection, capture the final picker cleanup snapshots and include
them in the telemetry summary JSON, if telemetry/debug is enabled.

Target shape:

```json
"background": {
  "picker_cleanup": {
    "ok": true,
    "resources": [
      {
        "name": "textual-picker-metadata",
        "phase": "picker",
        "state": "closed",
        "closed": true,
        "pending_count": 0,
        "running_count": 0
      }
    ]
  }
}
```

This gives future runs a single artifact proving whether worker cleanup was clean.

Keep debug log output too.

### 4.6 Remove Brittle Classic Picker Test Shape

Current classic lifecycle tests may inspect local frames/private fields. Replace or supplement with a
small testable helper.

Recommended helper:

```text
src/sky_music/ui/picker_background.py
```

Possible responsibilities:

- create classic metadata/cache resources
- submit metadata/cache work
- retire process metadata resource and create fallback thread resource
- final cleanup with snapshots

`picker.py` can still own UI logic. The helper only owns background lifecycle.

Do not move scheduler or metadata computation logic into this helper.

### 4.7 Add Static Guard Test

Add a test that scans `src/` for raw worker creation and enforces ownership.

Examples:

- `ThreadPoolExecutor` allowed only in:
  - `ui/picker.py` or extracted `ui/picker_background.py` when wrapped by `ExecutorResource`
  - `ui/textual_app/workers.py` inside `MetadataCoordinator`
- `ProcessPoolExecutor` allowed only in classic picker background setup.
- `threading.Thread` allowed in playback engine dispatch thread.
- `run_worker(... thread=True)` should not appear for metadata/cache lifecycle.
- `shutdown(wait=False)` should not appear in `src/`.

This does not replace review, but it catches future drift.

## 5. Implementation Phases

### Phase 0 - Baseline Evidence

Before edits, run:

```powershell
uv run pytest tests\test_background_lifecycle.py tests\test_classic_picker_lifecycle.py tests\test_textual_picker.py tests\test_threaded_dispatch.py tests\test_runtime_dispatch.py tests\test_picker_metadata_optimizations.py
uv run python src\main.py --selftest-textual
```

Record results.

### Phase 1 - State Model Hardening

Update:

```text
src/sky_music/infrastructure/background.py
src/sky_music/ui/textual_app/workers.py
```

Required tests:

- `close(wait=False)` transitions to closing, not closed.
- `close(wait=True)` transitions to closed.
- shutdown failure transitions to failed and exposes `last_error`.
- `submit()` after closing/failed raises.
- `MetadataCoordinator` real worker helper no longer references legacy `_closed`.
- `MetadataCoordinator.cancel()` causes an in-progress staged worker to stop before later stages.

### Phase 2 - Cleanup Failure Policy

Update picker boundary callers:

```text
src/sky_music/ui/picker.py
src/sky_music/ui/textual_app/app.py
src/main.py
```

Required behavior:

- cleanup failure before playback produces a clear error path.
- tests fail if cleanup failure is swallowed.
- debug log includes cleanup failure details.

Recommended acceptance:

- If picker cleanup fails, `prompt_song_selection()` returns `None` or raises a controlled exception
  that aborts playback before SendInput starts.

### Phase 3 - Structured Lifecycle Evidence

Add lifecycle snapshots to telemetry summary or a run-adjacent debug artifact.

Required tests:

- A playback debug/telemetry-enabled path records picker cleanup snapshot.
- Snapshot includes `closed=True`, `pending_count=0`, `running_count=0` for successful cleanup.
- Failed cleanup records a failure state in the structured result.

### Phase 4 - Classic Picker Background Helper

Extract a narrow helper only if it reduces private/frame-introspection tests.

Required tests:

- process metadata resource starts open.
- fallback retire does not call `ProcessPoolExecutor.shutdown(wait=False)`.
- final cleanup calls real wait close.
- cache resource closes with final wait.
- helper tests do not inspect `choose_song_interactively` frame locals.

### Phase 5 - Static Drift Guard

Add a test file such as:

```text
tests/test_background_worker_static_guard.py
```

It should fail on unowned worker creation patterns.

Do not make it too broad; allow known playback dispatch thread and known picker resources.

## 6. Required Test Matrix

Run at minimum:

```powershell
uv run pytest tests\test_background_lifecycle.py tests\test_classic_picker_lifecycle.py tests\test_textual_picker.py tests\test_threaded_dispatch.py tests\test_runtime_dispatch.py tests\test_picker_metadata_optimizations.py
uv run python src\main.py --selftest-textual
```

Then run:

```powershell
uv run pytest
```

Manual smoke after implementation:

```powershell
uv run python src/main.py --debug-playback --debug-csv
```

Expected debug evidence before `[play] start`:

```text
[background] picker resource ... state=closed closed=True pending=0 running=0
```

Expected telemetry summary:

```text
sender_clean=true
background.picker_cleanup.ok=true
```

## 7. Acceptance Criteria

The reviewer should accept only if:

- No picker worker can remain running at playback start.
- Lifecycle state is explicit and not split across inconsistent flags.
- `MetadataCoordinator` no longer depends on a compatibility `_closed` alias.
- Cleanup failure before playback is not hidden.
- Debug logs and telemetry/summary provide lifecycle evidence.
- Tests cover real `MetadataCoordinator` behavior, not only fakes.
- Static guard prevents future unowned workers.
- Existing playback/runtime tests remain green.
- No scheduler, profile, or SendInput behavior is changed.

## 8. Rejection Criteria

Reject if:

- `shutdown(wait=False)` reappears in `src/`.
- A worker/executor/thread is created without an owner or documented exception.
- Cleanup failure is swallowed and playback continues silently.
- `WorkerSnapshot.closed=True` can mean anything other than final wait success.
- Textual worker tests only use fake coordinators.
- Classic picker tests rely only on frame-local introspection after a cleaner helper could be added.
- The refactor touches scheduler/timing profile/SendInput without a separate proven issue.

## 9. Reviewer Checklist

Run:

```powershell
rg -n "shutdown\\(wait=False|ThreadPoolExecutor|ProcessPoolExecutor|threading\\.Thread|run_worker\\(|thread=True" src tests
```

Check:

- all picker executors are scoped
- playback dispatch thread remains separate
- cleanup failure path is explicit
- debug log flush still works
- telemetry/debug evidence exists for successful cleanup
- manual run shows background resource closed before `[play] start`

## 10. Handoff Prompt For Implementer

```text
Read AGENTS.md and docs/2026-06_background-worker-lifecycle-hardening-plan.md. Continue hardening
the existing worker lifecycle refactor; do not rewrite scheduler, timing profiles, SendInput, or UI
behavior. Make lifecycle state explicit, remove split/alias state in MetadataCoordinator, make
cleanup failure before playback visible and preferably abort playback, add structured lifecycle
snapshots to telemetry/debug evidence, replace brittle classic picker lifecycle tests where possible,
and add a static guard against future unowned workers. Run the required uv test matrix and report
manual debug evidence.
```

