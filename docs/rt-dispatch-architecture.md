# Real-Time Dispatch Architecture

Status: CURRENT. Rust is the only production dispatch implementation.

## Ownership

Python prepares an immutable authored `KeyAction` stream, validates the target
process, creates a native session, forwards user commands, polls the HUD, and
stores the final report. Python does not compile runtime generations, calculate
deadlines, wait/spin, estimate send latency, call `SendInput`, or supervise a
dispatch worker.

```text
Python PlaybackEngine
  -> sky_player_rs.SessionConfig
  -> sky_player_rs.DispatchSession
       -> sky_dispatch_core schedule + generation state
       -> sky_dispatch_win32 QPC/wait/focus/SendInput
       -> bounded native telemetry
```

The Rust worker owns all production timing and cleanup decisions. It remains in
QPC tick domains for control-path arithmetic and only converts at API or
telemetry boundaries. Completion timing is measured at the `SendInput` return
boundary; it is not a claim about game polling, rendering, or audio onset.

`SendInput` completion is sender-side evidence. It is not proof that the game
consumed the event. Any receiver/probe window used for acceptance is an
app-owned delivery proxy and must not be described as game receipt.

## Session contract

`SessionConfig` exposes only session/user inputs: `min_hold_us`,
`require_focus`, `target_hwnd`, telemetry enablement, and the native profile.
Internal wait strategy, priority, retry, estimator, telemetry capacity, lease,
and strict-completion policy are Rust profile details.

The session exposes lifecycle commands (`pause`, `resume`, `skip`, `quit`,
`panic`), `set_target_hwnd`, `snapshot_lite`, and `session_report`.

`snapshot_lite` is the frequent control/UI read and contains only state,
elapsed/total time, completion error, active key count, health, and the flags
needed to render. It has no trace, hash maps, generation ledger, estimator
internals, or build provenance.

`session_report` is called once after worker termination. It contains the full
terminal snapshot, native telemetry, estimator output, cleanup result, and
build metadata. Python enriches it only with song/application metadata; it does
not reinterpret native timing.

## Invariants

- Every valid Down owns a generation; authored same-key overlap is rejected.
- Stale Up is suppressed and no Up precedes the minimum hold floor.
- Pause and focus loss release physical keys before resumable cancellation.
- Quit, skip, panic, worker error, lease expiry, and join timeout use bounded
  cleanup. Uncertain cleanup is an error, never a successful finish.
- Partial `SendInput` is not success; zero progress and recovery are handled
  by Rust and remain visible in the final report.
- Physical preflight and cleanup verification map each instrument scan code
  through the keyboard layout of the current target window thread using
  `MapVirtualKeyExW`. A zero/invalid target window, unavailable layout, or failed
  scan-code mapping is inconclusive, never equivalent to “key is up”. Mock
  emitters remain exempt from host physical-state verification.
- No Python callback runs in the native real-time worker.
- A successful terminal result requires no active, pending, possibly-active, or
  residue key.

## Focus and liveness

Python owns process-name validation and target discovery. It sends one HWND
with `set_target_hwnd`; Rust compares it to `GetForegroundWindow()` immediately
before dispatch. Python does not send a second focus boolean.

The HWND is also passed directly to preflight and cleanup verification. The
sender continues to inject the same physical scan codes; the layout-aware VK
mapping exists only for checking physical state and is not part of the hot
`SendInput` path. The current 15-key allowlist has no E0/E1 extended scan
codes.

Physical-key preflight is an admission boundary for every initial playback,
manual resume, focus restoration, and target-HWND generation. A target change
invalidates the previous verification before the worker processes the next
chord. Cleanup/preflight runs while the playback clock remains paused; only
after a successful, still-current verification does the worker take a new QPC
sample and leave the pause. Immediately before a Down, the worker rechecks
both the target generation and focus, so a focus or target change during the
Win32 verification window cannot send an unverified chord.

Each physical-state pass resolves the target thread and keyboard layout once,
maps the fixed 15 scan-code allowlist once, and then reads the aggregate key
state. Mapping or state ambiguity remains fail-closed. The resulting layout
work is therefore limited to admission/cleanup boundaries and is not repeated
for each healthy chord.

The UI/control polling loop publishes the supervisor heartbeat. There is no
separate heartbeat thread: if the control loop stops, native lease liveness
reflects that failure.

Layout acceptance is a Windows manual matrix, not a CI claim: run preflight,
resume, pause, and panic-release checks under English US, German, and French
layouts, recording the layout identifier and result. A receiver/probe may
count scan-code events and QPC order, but its result is host-side evidence and
does not establish game receipt.

## Healthy worker path

The final wait spin observes an event signal generation and QPC only; it does
not issue a zero-time Win32 event wait on each spin iteration. The event handle
remains authoritative for long waits and command interruption. Estimator
lead-cache refreshes update the preallocated cache in place, and one clean
observation refreshes the affected cache once. CPU-time telemetry is sampled
on a bounded 100 ms interval with a final worker sample, while healthy shared
metrics publication is rate-limited and anomaly/terminal transitions publish
immediately. When telemetry is disabled, trace-record construction is not
performed.

## Preview, calibration, and rollback

Preview is a separate no-input simulation path. It is not a backend, scheduler,
timing oracle, or fallback. Calibration runs in its dedicated native process
and is not part of the playback session contract.

Native admission is startup-only and fail-closed. In source development,
admission validates the native commit is present plus the runtime schema, ABI,
free-threaded, and Win32 backend metadata; a dirty native commit is allowed.
In frozen production, admission additionally validates generated
`APP_BUILD_COMMIT` against the exact lowercase native commit once before
opening the playback UI. Playback never probes again, runs Git, hashes the
Rust source tree, or accepts a SHA environment override. Removing or
invalidating the extension never selects Python. Rollback is an application
release rollback, not a second dispatch engine in the same binary.
