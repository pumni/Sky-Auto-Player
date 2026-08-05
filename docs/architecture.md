# System Architecture

Sky Auto Player keeps Python for application work and Rust for the complete
production input lifecycle. There is no Python dispatch backend, runtime
fallback, or low-level Python/Rust input adapter.

## Layers

1. `sky_music/domain/` parses songs, resolves authored actions, applies user
   explicit hold-frame timing, and validates schedule input. It is Windows- and I/O-free.
2. `sky_music/orchestration/` prepares the song, admits the native extension,
   creates one `DispatchSession`, forwards commands, polls a small live
   snapshot, and writes the final native report. `PlaybackEngine` is an
   application facade, not a scheduler.
3. `sky_music/infrastructure/` owns application glue such as hotkeys,
   background workers, focus requests, and native admission diagnostics.
4. `sky_music/platform/win32/` owns validated window targeting. Keyboard
   injection itself is implemented only by the Rust Win32 crate through
   Windows `SendInput`.

## Production playback

```text
Song file
  -> Python parser and authored KeyAction preparation
  -> native admission check
  -> PyO3 SessionConfig + DispatchSession
  -> Rust compile/runtime generation state
  -> QPC deadline, wait/spin, focus gate, SendInput, retry, cleanup
  -> snapshot_lite() while playing
  -> session_report() once after worker termination
  -> Python HUD and telemetry writer
```

The three Rust crates have fixed responsibilities:

- `sky_dispatch_core`: schedule compilation, generation ownership, timing
  invariants, release floors, recovery, and property tests.
- `sky_dispatch_win32`: QPC, wait strategy, focus validation, priority scope,
  and the only keyboard `SendInput` implementation.
- `sky_player_rs`: the native session worker and the deliberately small PyO3
  boundary.

The worker owns the entire lifecycle: active/possibly-active key masks,
minimum hold, stale-Up suppression, partial/zero-progress handling, release
retry, focus-loss release, panic/quit/skip cleanup, adaptive lead, telemetry,
and terminal integrity decisions. A session cannot report successful
completion while cleanup residue remains.

## Python–Rust contract

The production extension exposes `SessionConfig` with only user/session
fields: minimum hold, focus requirement, target HWND, telemetry enablement, and
the native profile. `DispatchSession` accepts authored actions and the allowed
scan-code registry. It exposes only lifecycle commands, `set_target_hwnd`, a
small `snapshot_lite`, and one final `session_report`.

`snapshot_lite` returns a frozen typed `ProgressSnapshot` with a nested frozen
`BackendHealthSnapshot`. It contains state, elapsed/total time, completion
error, active and uncertain-key counts, backend failure counters, health, and
the control-loop flags needed by the HUD. Correctness-critical fields are
required by the Python adapter rather than silently defaulted. It does not
contain trace records, hash maps, build provenance, estimator state, or full
generation counts. `session_report` remains the one final mapping and is
materialized only after the worker has stopped; it is the sole source for final
native telemetry.

Focus has one source of truth: Python finds and validates the target process,
then sends its HWND with `set_target_hwnd`. Rust compares that HWND with
`GetForegroundWindow()` before dispatch. Python does not publish a second
focus boolean.

The supervisor heartbeat is published by the Python UI/control polling loop.
There is no separate heartbeat thread. If that loop stops polling, the native
lease can expire as intended.

## Preview and calibration

Dry-run is an explicitly named preview path. It never creates a production
dispatch session, sends input, uses QPC precision scheduling, or acts as a
timing oracle. It only completes the UI preview flow.

Calibration is separate from playback. The dedicated native calibration
process performs the host-side `SendInput`/Raw Input measurement and Python
only validates and publishes its artifact. Calibration state and artifacts
are not part of `SessionConfig`.

## Failure and rollback policy

Native admission fails closed on import, ABI, schema, free-threaded-runtime, or
Win32-backend failure. Source development also requires a non-empty native
build identifier, but does not require release provenance metadata. Frozen
production additionally requires the generated
`sky_music._native_build.APP_BUILD_COMMIT` to match the exact lowercase
40-character `native_build_commit` returned by `sky_player_rs.build_info()`.
No exception path executes a Python sender. Recovery is performed by rolling
back the application release, not by selecting a second dispatch engine inside
the binary.

Admission runs once after the free-threaded runtime check and before song
discovery or UI startup. The packaged application never runs Git, hashes the
Rust source tree, checks module mtimes, or accepts an environment override for
the release contract. Doctor may inspect and report mismatches without
creating a `DispatchSession`.

Current source and active tests therefore contain no `DispatchLoop`, Python
`RuntimeDispatchCoordinator`, Python `PlaybackSupervisor`, Python sender
backend, `RustInputAdapter`, or backend-selection environment flags. Rust's
native coordinator name remains internal to the native implementation and its
Rust tests.
