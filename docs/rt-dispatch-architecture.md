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
- No Python callback runs in the native real-time worker.
- A successful terminal result requires no active, pending, possibly-active, or
  residue key.

## Focus and liveness

Python owns process-name validation and target discovery. It sends one HWND
with `set_target_hwnd`; Rust compares it to `GetForegroundWindow()` immediately
before dispatch. Python does not send a second focus boolean.

The UI/control polling loop publishes the supervisor heartbeat. There is no
separate heartbeat thread: if the control loop stops, native lease liveness
reflects that failure.

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
