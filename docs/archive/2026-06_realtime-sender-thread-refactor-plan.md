> ARCHIVED 2026-06 — historical plan/audit. Không phải tài liệu hiện hành.
> Contract & sự thật hiện tại: ../timing-principles.md và ../architecture.md.
> CẢNH BÁO lệch code đã biết: kế hoạch refactor realtime sender thread đã thực thi.

# Refactor Plan — Real-Time Dispatch Thread Isolation (Option A)

> Status: APPROVED for implementation. Plan author = reviewing AI (acceptance/nghiệm thu).
> Implementer = a separate AI executing this spec cold. Read the whole file before editing.
> Project rules: `AGENTS.md` (HARD: "Use Windows SendInput only", no driver, no anti-cheat bypass,
> backend isolated behind interface, scheduler pure/unit-testable). Prior accepted work:
> `docs/completion-anchor-refactor-plan.md` (do not regress it).

## Locked decisions (do not relitigate)
- **Option A**: keep the `InputBackend` Protocol **synchronous and unchanged**. Isolation is achieved
  by splitting THREADS at the engine layer, not by making the backend async. (Owner approved.)
- **SendInput only.** No PostMessage, no kernel/HID driver, no input hooks of our own.
- The scheduler and `RuntimeDispatchCoordinator` logic (incl. completion-anchor) stay **byte-identical
  in behavior**. This refactor only changes *where* the dispatch loop runs and moves UI/focus/hotkey
  work OFF the real-time path.
- Existing unit tests must keep passing via a **single-thread mode** (see §5). Real threads are used
  only in production and in one new integration test.

## 0. Problem & goal

Today `PlaybackEngine.play()` runs everything on ONE thread: wait → `backend.key_down/up` (SendInput)
**plus** `controls.poll()` (hotkeys), `focus_guard.is_active()` (heavy Win32 chain), and
`renderer.render()` (terminal I/O). Any of those blocking jitters or stalls the onset dispatch on
*every* device — render/focus/GC are inline on the real-time path.

Goal: make the real-time send path (decide-deadline → SendInput) robust across devices by isolating
it on a dedicated thread that does **nothing but wait and send**, using modern Windows real-time
primitives, while UI/focus/hotkeys run on a separate control thread. Also surface a runtime
**input-path health** warning so that on a device whose OS input pipeline is throttled (global
low-level hook / Filter Keys), the player reports the cause instead of stuttering silently.

Honest boundary (state in docs, do not pretend to fix): if the OS Raw Input Thread is saturated by a
third-party global hook draining slower than real-time, **no SendInput-based design can avoid it**.
This refactor maximizes robustness on healthy machines and *detects+reports* the pathological case.

## 1. Architecture (Option A)

Two threads, one shared backend-owner rule.

**RT dispatch thread (NEW, owns the real-time loop):**
- The ONLY thread that ever calls the backend (`key_down`/`key_up`/`release_all`/`get_health`). This
  keeps SendInput serialized and `_TrackedKeyState` lock-free (its current non-thread-safe design is
  preserved).
- Runs the existing `wait → drain_due → send` loop and owns `coordinator`, `state`, telemetry
  recording, and the final `release_all` + telemetry save.
- Registered with **MMCSS** and waits with a **high-resolution waitable timer** (§3).
- Its in-wait polling is now ultra-cheap: read a shared focus bool + drain a command queue. NO render,
  NO heavy Win32 focus chain, NO hotkey scan on this thread.

**Control / UI thread (the main thread after spawn):**
- Polls hotkeys (`controls.poll()` → GetAsyncKeyState) and pushes commands into a thread-safe queue.
- Runs the heavy focus check (`focus_guard.is_active()` Win32 chain) on its own cadence (~10–20 ms) and
  publishes a single `focus_active: bool`.
- Owns `focus_guard.focus()` (refocus) — a Win32 UI call, NOT a backend call.
- Renders the HUD from a **published progress snapshot** (never touches the backend or coordinator).

**Backend-owner rule (critical invariant):** focus-loss / panic / pause must release keys, but the
control thread must NOT call the backend. Instead it enqueues a command; the RT thread performs the
`release_all`. Verified by an acceptance gate (§6 D).

## 2. Seams (so the dispatch loop is identical logic in both modes)

Refactor the inner loop into a thread-agnostic method, e.g.
`_run_dispatch(coordinator, state, *, command_source, focus_signal, progress_sink)`. It contains the
EXACT current logic of `_wait_until_runtime_deadline` + `_drain_due`, except it talks to three small
interfaces instead of `controls`/`focus_guard`/`renderer` directly:

```python
class CommandSource(Protocol):
    def poll(self) -> str | None: ...          # "pause"|"skip"|"quit"|"refocus"|"panic"|None

class FocusSignal(Protocol):
    def is_active(self) -> bool: ...           # cheap, no Win32 on the hot path in threaded mode

class ProgressSink(Protocol):
    def publish(self, *, elapsed_us: int, total_us: int, status: str,
                lateness_us: int | None, health, input_path_degraded: bool) -> None: ...
```

- **Single-thread mode (tests + `--no-thread` fallback):** adapters that wrap the current objects —
  `command_source` over `controls`, `focus_signal` over `focus_guard.is_active()`, `progress_sink`
  over `renderer.render(...)`. Behaviour == today (render/focus inline). All existing tests pass
  unchanged because they already inject `controls`, `focus_guard`, `renderer`, `clock`, `sleeper`.
- **Threaded mode (production):** `command_source` = consumer of the command queue; `focus_signal` =
  reader of the shared bool; `progress_sink` = snapshot publisher. The control thread feeds the queue
  + flag and renders from the snapshot.

This keeps the dispatch/coordinator hot logic untouched and unit-testable on the calling thread.

## 3. Modern Windows real-time primitives (RT thread only)

Add ctypes bindings in `src/sky_music/platform/win32/inputs.py` and a small wrapper module
`src/sky_music/infrastructure/realtime.py`.

1. **High-resolution waitable timer** (Win10 1803+):
   - `CreateWaitableTimerExW(NULL, NULL, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION (0x2), TIMER_ALL_ACCESS (0x1F0003))`.
   - `SetWaitableTimer(h, &dueTime /*negative 100ns relative*/, 0, NULL, NULL, FALSE)` then
     `WaitForSingleObject(h, INFINITE)`. Spin the final ~50 µs with QPC.
   - Implement as a `Sleeper`-compatible waiter (`WaitableTimerSleeper`) so it slots behind the
     existing `Sleeper`/`PreciseSleeper` seam. Tests keep injecting `FakeSleeper`.
   - **Fallback:** if `CreateWaitableTimerExW` returns NULL / flag unsupported (pre-1803), fall back to
     the current `PreciseSleeper` + `timeBeginPeriod(1)` path. Detect once at startup; never hard-fail.
2. **MMCSS** via `avrt.dll`:
   - `AvSetMmThreadCharacteristicsW("Pro Audio", &taskIndex)` at RT-thread start;
     `AvRevertMmThreadCharacteristics(handle)` at end. Wrap in try/except; if it fails, log + continue.
   - Do NOT also force `THREAD_PRIORITY_TIME_CRITICAL` by default (MMCSS already elevates; stacking can
     starve the control thread). Leave a config flag `rt_time_critical=false`.
3. Keep `QueryPerformanceCounter` (current `PerfCounterClock`) as the clock.

All three are best-effort and gated so the build still runs on any Windows/device.

## 4. Input-path health detection (device-agnostic honesty)

On the RT thread, keep a rolling window (e.g. last 64) of measured `send_duration_us`. Compute p95 over
the window; if it exceeds `input_path_warn_us` (config, default **300**) for a sustained span (e.g. ≥ 1 s
of events), set `input_path_degraded = True` in the published snapshot. The control thread shows a HUD
line: *"Input path throttled (global hook / Filter Keys?) — playback may stutter; this is OS-side, not
the player."* Also record `input_path_degraded` (bool) and `input_path_warn_us` in the telemetry
summary. This turns the un-fixable case into a clear, on-device diagnostic.

## 5. Engine wiring

- `play()` (main thread): keep the initial focus-wait, then:
  - Build `coordinator`, `state`.
  - If threaded (production default): create command queue + shared focus flag + snapshot holder;
    spawn the RT thread running `_run_dispatch(...)` with the queue/flag/snapshot seams; the main
    thread runs the **control loop** (hotkey poll → enqueue; focus check → flag; render from snapshot;
    handle refocus). When the RT thread finishes (or a quit/skip command), join it and return the
    result the RT thread produced.
  - If `--no-thread` / tests: run `_run_dispatch(...)` inline with the direct adapters (today's
    behaviour) and return its result.
- Add a constructor/CLI flag `use_dispatch_thread: bool = True` (CLI `--no-dispatch-thread` to force
  single-thread for debugging). Tests pass `use_dispatch_thread=False` or use the existing `play()`
  path which must default to single-thread when `clock`/`sleeper` are injected fakes (simplest:
  threaded only when running with the real PerfCounterClock + RealSleeper; otherwise single-thread).
- The RT thread must remain responsive to `quit`/`pause`/`panic` during waits: `_run_dispatch`'s wait
  step drains the command queue + checks the focus flag at the existing `_runtime_poll_interval_us`
  cadence (cheap now). No render/focus Win32 on this thread.

## 6. Acceptance gates (reviewer runs these)

A. **No dispatch regression.** Full suite green incl. `tests/test_runtime_dispatch.py`; the
   completion-anchor gate `tests/acceptance_completion_anchor.py` still PASS for all 115×2 songs
   (0 below frame, 0 dropped). `compileall` clean. Single-thread mode behaviour identical to today.
B. **UI-isolation gate (the whole point).** New integration test: run the REAL threaded engine on a
   short synthetic song with a `ProgressSink`/control whose render blocks ~50 ms per call AND a
   focus check that blocks ~20 ms. Assert the RT thread's recorded onset send intervals match the
   scheduled intervals within a tight tolerance (e.g. p99 |dev| < 2 ms) — i.e. a slow UI does NOT
   delay onsets. This test MUST be RED if rendering is left on the dispatch thread, GREEN after.
C. **Backend single-thread invariant.** Wrap the backend in a recorder that captures
   `threading.get_ident()` on every `key_down/key_up/release_all`; assert all calls share ONE thread
   id even under pause/focus-loss/panic commands issued from the control thread.
D. **Input-path health.** Drive synthetic high `send_duration` (≥ warn threshold) → assert
   `input_path_degraded` flips true and the HUD warning string is emitted; low latency → stays false.
E. **Primitive fallback.** Simulate `CreateWaitableTimerExW` unavailable → engine still runs via the
   `PreciseSleeper` fallback (no crash); MMCSS failure → continues.
F. **Constraints.** Code review: SendInput-only (no PostMessage/driver/hook added); backend Protocol
   unchanged; scheduler/coordinator behaviour unchanged.

## 7. Files

- `src/sky_music/orchestration/engine.py` — extract `_run_dispatch`; add CommandSource/FocusSignal/
  ProgressSink seams + adapters; thread spawn + control loop in `play()`; backend-owner rule.
- `src/sky_music/infrastructure/realtime.py` (NEW) — `WaitableTimerSleeper`, MMCSS register/revert,
  capability detection + fallback.
- `src/sky_music/platform/win32/inputs.py` — ctypes for CreateWaitableTimerExW/SetWaitableTimer/
  WaitForSingleObject + `avrt.dll` AvSetMmThreadCharacteristicsW/AvRevertMmThreadCharacteristics.
- `src/sky_music/ui/hud.py` — render from snapshot; add the `input_path_degraded` warning line.
- `src/sky_music/orchestration/telemetry.py` — add `input_path_degraded` + `input_path_warn_us` to
  summary.
- `src/sky_music/config.py` — `input_path_warn_us` (300), `rt_time_critical` (false),
  `use_dispatch_thread` (true) defaults.
- `src/main.py` — `--no-dispatch-thread` flag; pass config through.
- `tests/` — new: UI-isolation integration test (gate B), backend-single-thread test (gate C),
  input-path-health test (gate D), fallback test (gate E). Keep all existing tests on single-thread.

## 8. Phasing (land incrementally, keep green between)
1. Extract `_run_dispatch` + seams + single-thread adapters. Suite stays green (pure refactor).
2. Add threaded mode + control loop + backend-owner rule (gates B, C).
3. Add realtime primitives (waitable timer + MMCSS) behind the `Sleeper` seam + fallback (gate E).
4. Add input-path health + HUD warning + telemetry (gate D).

## 9. Out of scope (do not do)
- Do NOT make `InputBackend` async / change its Protocol (that was Option B, rejected).
- Do NOT merge up+down into one SendInput call in this refactor — it needs a backend method change and
  is a marginal win; defer to a separate proposal.
- Do NOT change scheduler, coordinator, completion-anchor, profile values, or chord batching.
- Do NOT add PostMessage / kernel driver / input hooks. SendInput only.
