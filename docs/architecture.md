# System Architecture

Sky Auto Player keeps application and policy work in Python and owns the
complete production input lifecycle in Rust. There is no Python dispatch
backend, runtime fallback, or low-level Python/Rust input adapter.

## Layers

1. `sky_music/domain/` parses songs, validates authored actions, resolves the
   selected hold-frame policy, and remains Windows- and I/O-free.
2. `sky_music/orchestration/` prepares schedules, admits the native extension,
   creates one session, forwards commands, polls snapshots, and writes the
   final native report. It is not a scheduler.
3. `sky_music/infrastructure/` owns application glue such as hotkeys, focus
   requests, background workers, and native admission diagnostics.
4. `sky_music/platform/win32/` owns validated window targeting. Physical
   keyboard injection is implemented only by the Rust Win32 crate through
   Windows `SendInput`.

The native crates have fixed responsibilities:

- `sky_dispatch_core`: schedule compilation, generation ownership, authored
  and release deadlines, minimum-hold floors, and pure tests.
- `sky_dispatch_win32`: QPC, wait strategy, focus validation, priority scope,
  packet validation, and the only `SendInput` implementation.
- `sky_player_rs`: the session worker, dispatch orchestration, deferred
  observation consumer, and deliberately small PyO3 boundary.

The update checker remains Python domain/orchestration logic. Applying an
update is a separate Rust `sky_updater` process and never depends on the
playback scheduler or `SendInput`.

## Production playback

```text
Song file
  -> Python validation and authored KeyAction preparation
  -> native admission and effective hold materialization
  -> PyO3 SessionConfig + DispatchSession
  -> Rust coordinator and playback epoch
  -> absolute QPC target, wait/spin, final control gate
  -> one packetized SendInput call
  -> authored release ownership and diagnostic-only observation
  -> snapshot_lite() while playing
  -> session_report() after worker termination
  -> Python HUD and telemetry writer
```

The worker has one physical timing contract. The authored/effective timeline
deadline is not advanced by a learned send-cost lead. After final target,
focus, and control proof the worker takes `final_proof_qpc`; the lease is
evaluated against that sample. It then spins to the authored physical target.
The trusted Win32 sender takes the authoritative `pre_call_qpc` after the
prepared payload pointer/length have been resolved. After that sample, a
Down-bearing packet may perform only the session down-late grace cutoff comparison before
the single `SendInput` call; Up-only safety release has no Down cutoff. The
Win32 sender then returns `sendinput_completion_qpc`. Completion is used as
sender-side ownership/diagnostic evidence only; it is not used for
physical-hold feasibility, subtracted from future authored timestamps, or used
as a healthy release floor.
The compatibility `send_started_ticks` and `send_completed_ticks` fields refer
to `pre_call_qpc` and `sendinput_completion_qpc`; they do not cause an
additional production QPC sample.

For an authored same-key Down→Up pair, the schedule must satisfy:

```text
authored_up >= authored_down + effective_min_hold
```

`effective_min_hold` is materialized by Python as the selected frame hold plus
the calibrated static margin. PyO3 passes that value verbatim; Rust only
range-checks it and validates the authored interval in QPC ticks. The static
margin is applied once while building the authored schedule. Native admission
rejects an invalid interval before worker start; SendInput completion is
evidence only and never creates a second hold floor or a replacement deadline.

The worker owns active key masks, stale-Up suppression,
zero/partial progress handling, focus-loss pause and restore safety release,
panic/quit/skip cleanup, transport integrity, and terminal decisions. Any
sender-duration value is diagnostic evidence only; it does not become a lead,
estimator, or deadline adjustment. A session cannot report successful
completion while cleanup residue remains.

The coordinator also owns a fixed per-key pending-release table for recovery
state. It cannot move an authored Up or block an unrelated authored Down
chord. Same-key retriggers whose authored interval is infeasible are rejected
during schedule admission; no runtime reschedule, retry, or catch-up burst is
emitted.

## Observer profiles

Production has no observation queue or observer thread. After ownership
reconciliation it commits only bounded scalar counters. Strict diagnostic mode
uses a fixed-capacity `crossbeam_queue::ArrayQueue` and one observer thread;
the producer never waits, allocates, drains, mutates coordinator state, or
formats telemetry. When full, the diagnostic queue drops the new observation;
it does not evict an older observation. The diagnostic consumer owns health
windows, telemetry materialization, snapshot publication, and observer metric
updates. Its local metrics are merged after the consumer stops.

The observer is diagnostic only. It cannot authorize, reorder, retry, or
commit physical input, and observer failure is terminal only when its own
integrity contract cannot be completed. The dispatch thread never waits for
observer slack.

## Python–Rust contract

Python validates user/session fields, the canonical scan-code allowlist, FPS,
hold selection, and schedule timing before creating a native session. The
native boundary exposes lifecycle commands, target/focus hints, a small live
snapshot, and one final report. `snapshot_lite` excludes trace records, maps,
generation detail, and compatibility estimator payloads.

The active Python/native session contract does not carry estimator state.
Historical lead and estimator artifacts are not timing inputs and cannot
affect deadlines, SendInput admission, release floors, health, or transport
policy.

Focus hints are coarse wake/gate signals, never input authorization. Rust
compares the stamped target HWND with `GetForegroundWindow()` immediately
before each Down dispatch. The supervisor heartbeat is published by the
Python control loop; if it stops polling, the native lease expires.

Focus loss enters a native pause without physical cleanup while the target is
unfocused: the worker clears the verified target and restore marker, publishes
the pause, and keeps the session non-terminal. After the target is foreground
again and the restore grace has elapsed, the worker reloads the target stamp,
performs full-instrument release and physical preflight, rechecks fresh focus
and target identity, and only then exits the focus pause. Cleanup or preflight
failure is terminal; a focus or target race keeps playback paused for retry.
At playback startup, when focus is required, the Python supervisor makes one
best-effort minimal focus attempt before starting the native worker. That
attempt is outside the worker, does not authorize input, and trusts only a
fresh foreground observation. After startup, focus loss is observation and
publication only; the native worker remains authoritative for the recoverable
focus pause and restore sequence. Manual refocus remains available.

Progress is independent of that heartbeat. Rust publishes a transition-only
playback clock anchor; supervisor-side snapshots take their own QPC sample and
project elapsed time without worker or observer activity.

## Preview, calibration, and failure policy

Preview never creates a production dispatch session or sends input. Calibration
is a separate native process and its Raw Input result is a host delivery
diagnostic, not game-observed timing evidence.

The publishable calibration contract is a balanced Down/Up pair for each of
`1/hot`, `1/cold`, `5/hot`, `5/cold`, `15/hot`, and `15/cold`. The native
process tags every packet with direction and sequence, records Raw Input at
the `WM_INPUT` handler entry, and pairs receipts by scan code. For each key it
computes the signed hold shrink `D - U`; only clean pairs enter the signed
quantiles. At least 100 clean pairs are required in every production bucket.
The cache is version 2 (native schema 9, measurement protocol 4), and cache
version 1 is rejected rather than reinterpreted. The selected margin is:

```text
max(300, min(2_000, max(0, required_bucket_p99_shrink) + 100))
```

Invalid provenance, incomplete buckets, anomalies, cleanup failure, or a
failed startup Down/Up correlation self-test fail closed and preserve the
previous cache. Small diagnostic runs may produce a report, but can never
write the production cache.

Native admission fails closed on import, ABI, schema, free-threaded-runtime,
or Win32-backend errors. No exception path executes a Python sender. Partial
or mixed transport outcomes are never retried; full cleanup and termination
are required.
