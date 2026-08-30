# System Architecture

Sky Auto Player keeps application and policy work in Python and owns the
complete production input lifecycle in Rust. There is no Python dispatch
backend, runtime fallback, or low-level Python/Rust input adapter.

## Presentation surfaces

The packaged `Sky-Auto-Player.exe` Tauri/React application is the canonical
desktop interface for the v4 release: Library, Song Detail, Player Dock,
Diagnostics, Settings, and Updates are rendered by the desktop shell. The
Textual application remains a supported keyboard-first fallback and uses the
same extracted Python orchestration services. `src/main.py` remains the source
TUI/CLI entry point; it is not Tauri glue, and the packaged fallback is
`Sky-Auto-Player-Core.exe --tui` (also available through `play.bat`).

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
  -> immutable packet preparation, worker-owned target crossing, final gate
  -> one packetized SendInput call
  -> authored release ownership and diagnostic-only observation
  -> snapshot_lite() while playing
  -> session_report() after worker termination
  -> Python HUD and telemetry writer
```

The worker has one physical timing contract. The authored/effective timeline
deadline is not advanced by a learned send-cost lead. It prepares the packet,
crosses the absolute authored target with the one bounded precision spin, then
performs final command/control, target, and focus checks. It repeats the
program-owned control/target/focus atomic checks without a second foreground
query, records a worker-owned `final_policy_qpc`, evaluates the lease against
that sample, and passes the prepared packet to the trusted Win32 sender.
The sender resets Win32 last-error state, takes the true `pre_call_qpc` after
payload resolution, and performs only the Down late-grace cutoff comparison
before the single `SendInput` call; Up-only safety release has no Down cutoff.
The sender does not wait or re-open policy admission.
It then returns `sendinput_completion_qpc`. Completion is used as sender-side
ownership/diagnostic evidence only; it is not used for physical-hold
feasibility, subtracted from future authored timestamps, or used as a healthy
release floor.
The compatibility `send_started_ticks` and `send_completed_ticks` fields refer
to the true sender `pre_call_qpc` and `sendinput_completion_qpc`; the worker's
separate `final_policy_qpc` remains policy evidence.

Timing evidence has four distinct boundaries: the authored target, sender
pre-call QPC, SendInput completion QPC, and game observation. The application
can verify only the first three; completion evidence cannot prove game
sampling. `require_focus=true` is a safety profile with a final foreground
verification cost, so it is not promised to have the same latency as
`require_focus=false`.

For an authored same-key Down→Up pair, the schedule must satisfy:

```text
authored_up >= authored_down + effective_min_hold
next_same_key_down - previous_same_key_up >= min_release_gap_us
```

`effective_min_hold` is materialized by Python as
`ceil(hold_frames * ceil(1_000_000 / fps)) + 500 µs + transport_margin`.
The release visibility policy is materialized alongside it as
`ceil(1_000_000 / fps) + 500 µs + transport_margin`. The 500 µs Down grace
and calibrated transport component are one static sender headroom budget;
the release gap is not a claim that the game observed the Up transition.
The transport component defaults/falls back to `300 µs`; calibration may
replace only that component. PyO3 passes the materialized value verbatim; Rust
range-checks it and validates both relationships in QPC ticks. Same-timestamp
same-key overlaps are rejected, while disjoint masks may coalesce. Native
admission rejects an invalid interval before worker start; SendInput completion
is evidence only and never creates a second hold floor or a replacement
deadline. Runtime never delays or repairs an authored boundary.

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
emitted. After the first successful musical Down, the first overdue Down
discovered within its fixed late grace may consume a one-shot late-discovery
rescue credit, provided the exact future authorization and focus/control/target
lease proof remain valid. The credit is consumed immediately; a later overdue
boundary without an intervening future observation is missed. Sender cutoff is
authoritative, and no rescue bypasses it.

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
observer slack. Production forensics is separate: a fixed worker-local block
publishes availability/version plus bounded hold-pair, shrink, release-gap,
retrigger, anchor, unmatched-Up, and anomaly-ring scalars. It performs no new
QPC reads, allocations, locks, formatting, or unbounded scans.

## Python–Rust contract

Python validates user/session fields, the canonical scan-code allowlist, FPS,
hold selection, and schedule timing before creating a native session. The
native boundary exposes lifecycle commands, target/focus hints, a small live
snapshot, and one final report. `snapshot_lite` excludes trace records, maps,
generation detail, and compatibility estimator payloads.

The resolved `min_release_gap_us` is one application contract value: it is
materialized by `FrameTimingPolicy`, forwarded by `PlaybackEngine` and
`RustDispatchRuntime`, carried by PyO3 `SessionConfig` into native
`TimingOptions`, used by microsecond and QPC admission, and converted once for
production release-gap forensics. The Rust application path does not derive a
replacement value from FPS; only the backward-compatible omitted PyO3 argument
uses the legacy one-frame fallback for external callers.

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

## Sender calibration, preview, and failure policy

The production calibration artifact is sender-centric. For each Down/Up
packet pair it records scheduled target (`T`), SendInput pre-call (`P`), and
SendInput completion (`C`) boundaries and proves, in raw QPC ticks:

```text
sender_hold_shrink = (T_U - T_D) - (C_U - C_D)
sender_hold_shrink = ((P_D - T_D) - (P_U - T_U))
                    + ((C_D - P_D) - (C_U - P_U))
```

There is exactly one authoritative sender value per packet pair; keys in a
packet do not receive separate worst-case values. The six required buckets
are `1/5/15 × hot/cold`, with 100 clean pairs and no more than 200 attempts in
each bucket. Only class mismatch is retryable. Up is scheduled from exact Down
completion plus the requested gap, with no Raw Input receipt wait between the
two sends.

Qualification is the maximum positive required-bucket
`sendinput_shrink_us.max` plus a `100 µs` guard, with a `300 µs` floor and
`2,000 µs` ceiling. Above the ceiling the status is `OUT_OF_ENVELOPE` and
playback uses the explicit `300 µs` transport fallback; the ceiling is never
raised to force validity. Protocol 10/native schema 15, artifact schema 11,
cache v8, source formula 6, and
`sender_completion_hold_shrink` evidence are required. Older protocol-9,
schema-13, cache-v5/v6/v7, or Raw Input evidence is rejected and never
reinterpreted.
The margin is materialized once in `effective_min_hold_us`; Note-On timestamps,
physical targets, and `down_late_grace_us = 500 µs` remain unchanged.

Each sender session first proves physical All-Up, sends one prepared full
All-Up priming packet, and proves All-Up again. The real completion of that
priming packet anchors the first Down target; it is setup evidence only and is
excluded from warm-up and production quantiles. This preflight is sender-only
and does not require a Raw Input observer. Sender close performs physical
All-Up cleanup before a bounded pump-thread shutdown.

Raw Input and `WM_INPUT` may be collected only by a separate observer
diagnostic. Receipt/queue timestamps and observer health cannot affect sender
quantiles, bucket cleanliness, retries, qualification, cache trust, or the
production cache.

## Historical protocol-9 Raw Input calibration (non-normative)

The following retired protocol description is retained only as historical
forensics. It is not the current calibration contract.

Preview never creates a production dispatch session or sends input. Calibration
is a separate native process and its Raw Input result is a host delivery
diagnostic, not game-observed timing evidence.

The publishable calibration contract is a balanced Down/Up pair for each of
`1/hot`, `1/cold`, `5/hot`, `5/cold`, `15/hot`, and `15/cold`. The native
process carries a required direction/sequence tag for publishable evidence,
records Raw Input at
the `WM_INPUT` handler entry, and pairs receipts by exact scan-code,
make/break direction, and extended-flag identity. Windows does not document
preservation of `KEYBDINPUT.dwExtraInfo` in `RAWKEYBOARD.ExtraInformation`, so
a missing or mismatched tag is terminal rather than silently correlated. One
active packet plus a pump-thread barrier handler
that explicitly removes already-queued `WM_INPUT` messages with an input-range
filter before publishing completion prevents stale receipts from aliasing the
next packet. The tagged
calibration INPUT array is prepared before the direct physical target wait;
the wait layer performs the single bounded target crossing and the shared fused
sender owns the final policy/pre-call boundary. Each direction has four authoritative QPC boundaries: absolute
target, fused sender crossing, SendInput completion, and first receipt. For each key it computes direct signed
total hold shrink `(up_target - down_target) - (up_receipt - down_receipt)`;
scheduler, SendInput, and delivery shrink are diagnostics that must sum to that
direct value. Receipt delivery is signed: a Raw Input receipt may be observed
before `SendInput` returns and remains valid evidence; only target/pre-call/
completion chronology and cross-direction identity/order failures reject a pair.
Diagnostic key evidence additionally preserves each correlated message's raw
uint32-millisecond `GetMessageTime()` queue timestamp. Queue-time differences
use modular uint32 subtraction across wraparound; the values are not mapped to
the QPC epoch and do not participate in production qualification.
An incomplete or timed-out packet invalidates the correlation boundary and
prevents the session from arming another packet; any stale-generation evidence
found by the pump, during a barrier or normal dispatch, likewise prevents
further packet arming. An active receipt with an incompatible identity or
direction, a duplicate, or a pending-receipt overflow is treated as the same
stale-generation evidence; a scheduling class mismatch remains a rejected
sample rather than an observer-boundary failure. Only clean pairs enter the
signed quantiles. At least
100 clean pairs are
required in every production bucket. Native output schema 14 and artifact
schema 10 record bounded anomaly evidence, the signed receipt-before-completion
counter, the acquired MMCSS label, PowerThrottling/HighQoS guard state, and
actual waiter mode alongside host provenance. The cache is version 5 (current
native schema 14, measurement protocol 9). Protocol-9 cache evidence produced
by native schema 13 remains compatible because schema 14 adds diagnostic-only
queue timestamps and does not change cached qualification data; older native
schemas are incompatible and fall back rather than being reinterpreted.
Qualification is:

```text
candidate = max(0, required_bucket_p99_total_proxy_shrink) + 100
candidate <= 2_000 -> VALID, applied margin = max(300, candidate)
candidate > 2_000  -> OUT_OF_ENVELOPE, applied margin = None
```

Completed out-of-envelope evidence overwrites the cache with its unhealthy
status and playback falls back to the clearly-sourced 300 µs transport
margin. The independent 500 µs Down late grace remains additive and is not
called a calibrated or hold-margin fallback.
Measurement or integrity failure preserves the previous cache. The hold
margin never changes Note-On timestamps, `physical_target`, dispatch lead, or
the independent fixed `down_late_grace_us` policy, which is `500 µs` in
production.

The five-key correlation self-test ends with a read-only physical All-Up
verification; it does not inject an untagged cleanup packet while the exact-tag
observer is active. Before a successful bucket is published, the final queue
barrier and trust check seal the observer, the pump is stopped and its Raw
Input registration is restored, and only then is physical All-Up cleanup sent.
Any boundary loss during that seal takes the emergency cleanup path and makes
the bucket non-publishable.

Invalid provenance, incomplete buckets, anomalies, cleanup failure, or a
failed startup Down/Up correlation self-test fail closed and preserve the
previous cache. Small diagnostic runs may produce a report, but can never
write the production cache.

Native admission fails closed on import, ABI, schema, free-threaded-runtime,
or Win32-backend errors. No exception path executes a Python sender. Partial
or mixed transport outcomes are never retried; full cleanup and termination
are required.
