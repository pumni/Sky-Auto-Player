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

The production thread-priority ladder is explicit: `Auto` attempts the
documented MMCSS `Games` task with `AVRT_PRIORITY_HIGH`, then falls back to
`THREAD_PRIORITY_HIGHEST`; an explicit `MMCSS` request does not probe unrelated
task names. RAII restores any thread priority and reverts MMCSS registration.

## Worker module ownership

The worker is decomposed by invariant owner, not by call depth. `worker.rs`
wires the modules; `orchestration.rs` is the loop sequencer and owns
command/focus/pause transitions, plan creation, the pending/authored/wait
choice, and the terminal transition.

- `planning.rs` owns path classification, lead selection, the pending-release
  plan, and the next wait deadline. `plan_next_dispatch()` builds exactly one
  immutable `NextDispatchPlan` per loop epoch from a `RuntimeDispatchCoordinator`
  snapshot, a `SendLatencyEstimator`, and the `QpcClock`; it never reads QPC,
  mutates the coordinator, allocates, or formats strings on the success path.
  The same `AuthoredDispatchPlan` lead feeds both the prepare-due boundary and
  the wait deadline, so prepare and wait cannot disagree on lead selection.
- `dispatch/` owns the pending-release and authored-packet backend
  transactions, transaction-outcome interpretation, coordinator commit, and the
  dispatch telemetry record. Both note-on and note-off paths first finish
  physical/coordinator correctness, required recovery, the mandatory terminal
  SLO decision, and a `worker_ready_qpc` sample. They then enqueue one
  allocation-free raw `DispatchObservation` (tagged `Down`/`Up`) into a
  fixed-capacity (64) worker-owned ring and return. Worker-ready duration
  conversion, trace-record materialization, estimator update, health-window
  observation, lateness accounting, and diagnostic metric publication are
  deferred to `observer::drain_one_observer`. If the ring is full, the oldest raw record is
  materialized before the new record is admitted, preserving telemetry order
  without blocking the normal queue path.
  The dispatch loop applies the §8.7/§8.8 slack rule before admit/dispatch:
  fresh QPC → immutable `NextDispatchPlan` → drain at most one observation
  only when next-deadline slack ≥ `budget_us + margin_us` (or no deadline
  remains) → if drained, discard the plan and rebuild from a fresh QPC
  sample before wait/admit/dispatch.  Adaptive budget (§8.9) is
  `clamp(2 × observed_us, 5_000..20_000)`. Observer work never rolls back
  physical ownership; a deferred observer failure may still terminate the
  session after that ownership is safe. It returns a closed four-variant
  `DispatchStep`
  (`NoWork`, `Dispatched`, `Continue`, `Terminate`) instead of scalar tuples.
- `cleanup.rs` owns suspension/terminal cleanup, clean-completion proof, and
  terminal-error aggregation.
- `control.rs`, `admission.rs`, `wait.rs`, `health.rs`, `timing.rs`, `startup.rs`
  own their single named concern.

A plan is valid only for its current loop iteration. After an interrupt,
command, focus/pause transition, backend call, release-recovery change, or wait
wake, the worker discards the plan and rebuilds it from fresh QPC samples.

`SendInput` completion is sender-side evidence. It is not proof that the game
consumed the event. Any receiver/probe window used for acceptance is an
app-owned delivery proxy and must not be described as game receipt.

## Session contract

`SessionConfig` exposes only session/user inputs: the user-selected `game_fps` (15..=240), the
materialized `min_hold_us` from the selected hold-frame value,
`require_focus`, `focus_restore_grace_us`, `target_hwnd`, telemetry enablement,
and the native profile. The final Python session report includes an immutable
`effective_config` record with requested/effective min-hold values and the
wired focus/telemetry semantics. Internal wait strategy, priority, retry,
estimator, telemetry capacity, lease, and strict-completion policy are Rust
dispatch-mode details.

The session exposes lifecycle commands (`pause`, `resume`, `skip`, `quit`,
`panic`), `set_target_hwnd`, `snapshot_lite`, and `session_report`.

`snapshot_lite` is the frequent control/UI read and returns a frozen typed
`ProgressSnapshot` with a nested `BackendHealthSnapshot`. It contains state,
elapsed/total time, completion error, active/possibly-active/release counts,
and every live backend counter needed by the HUD. Correctness-critical counters
are part of the native contract; Python must not replace a missing field with a
zero. It has no trace, hash maps, generation ledger, estimator internals, or
build provenance. Latency degradation is reported as an input-path health
signal; the typed snapshot separates `sendinput_path_degraded`,
`core_post_send_degraded`, `observer_degraded`, and `wait_path_degraded`.
`input_path_degraded` is the OR of SendInput and core post-send health only;
observer slowdown is an independent domain. SendInput warning thresholds use
the estimator's polyphony-aware syscall budget plus a fixed margin; core
post-send, observer, and wait thresholds remain independent. Each path also
publishes its degraded-sample count and active threshold. UI text must not
infer an OS hook, Filter Keys, or game-side cause from any of these sender-side
signals.

Each performance sample is classified once against the budget frozen before
dispatch; the rolling windows retain only that boolean classification, not raw
durations. SendInput, post-send occupancy, and scheduler wake latency have
independent fixed-capacity hysteresis windows. A single spike therefore cannot
latch degradation for a session, and a later threshold/polyphony change cannot
reclassify history. Backend rejection, partial packets, clock failures, and
uncertain key state remain immediate session-latched correctness failures.

The native final snapshot also reports `post_send_max_us`,
`dispatch_occupancy_max_us`, directional Down/Up/Mixed SendInput counters,
wait failure counters, and timeline-rebase count/duration/reason.  The
deferred dispatch observer additionally reports
`core_post_send_max_us` (a typed `worker_ready_qpc - sender_completed` QPC
sample), `observer_duration_max_us`, `observer_dropped_samples`, and
`observer_queue_high_watermark`.  Authored
lateness is measured before any recovery rebase. These fields are diagnostic
evidence owned by the sender; none establishes that the game observed or
played a note.

`session_report` is called once after worker termination. It contains the full
terminal snapshot, native telemetry, estimator output, cleanup result, and
build metadata. Python enriches it only with song/application metadata; it does
not reinterpret native timing.

The native lifecycle/status domain is separate from UI presentation status:
`ready`, `playing`, `paused`, `finished`, `quit`, `skipped`, `error`,
`panicked`, and `poisoned` are native values. A terminal `is_finished` snapshot
is accepted before live-status validation; Python then joins once, materializes
`session_report`, parses telemetry and estimator state, and only then surfaces a
terminal error. Normal completion never invokes panic cleanup.
`input_path_degraded` remains the aggregate of SendInput and core post-send
health; observer and wait-path degradation remain separate signals.

## Invariants

- Every valid Down owns a generation; authored same-key overlap is rejected.
- Actions sharing an authored timestamp compile to one physical packet; one
  `SendInput` call emits all Up events before all Down events.
- Stale Up is suppressed and no Up precedes the minimum hold floor.
- The native worker anchors the physical hold at the actual Down completion,
  using `max(configured_min_hold_us, frame_period_us + 500us)` as its initial
  frame-safe floor. It never snaps note-on timestamps to a frame grid.
- Authored timestamps are immutable; only an actual release-recovery pause may
  shift the effective epoch, and it preserves later authored spacing.
- Pause and focus loss release physical keys before resumable cancellation.
- Quit, skip, panic, worker error, lease expiry, and join timeout use bounded
  cleanup. Uncertain cleanup is an error, never a successful finish.
- Partial `SendInput` is not success; zero progress and recovery are handled
  by Rust and remain visible in the final report.
- Normal authored Down/Mixed packets and pending release sends use one
  low-level `SendInput` attempt. A partial Down/Mixed insertion is terminal
  chord uncertainty and is never replayed; zero progress is returned as
  explicit transport evidence rather than immediately retrying the packet.
  Pending-release retry belongs to the coordinator-owned recovery state
  machine, while terminal cleanup owns its bounded one-attempt-per-FSM-step
  retry budget. This prevents nested transport-retry × recovery-FSM behavior.
- Physical preflight and cleanup verification map the requested instrument
  scan-code mask through the keyboard layout of the current target window
  thread using `MapVirtualKeyExW`. Full admission and full terminal cleanup use
  all 15 allowlisted keys; tracked cleanup and stuck-key retries use only their
  bounded masks. A zero/invalid target window, unavailable layout, or failed
  scan-code mapping is inconclusive, never equivalent to “key is up”. Mock
  emitters remain exempt from host physical-state verification but never invent
  a physical verdict on their own: without an explicit test-only probe the
  cleanup FSM resolves Inconclusive and fails closed rather than synthesizing
  `AllUp`/`Held` from transport confirmation.
- Cleanup is a single bounded state machine (`TrackedKeyState::release_scope`),
  never nested cleanup: `release_all_full_instrument` does not call
  `release_all`. One invocation resolves an unresolved mask, sends key-up,
  reconciles transport and physical evidence, and retries only the unresolved
  mask (delays 15/50/100 ms, at most four attempts) with a single coherent
  `ReleaseAllOutcome`. A physical `AllUp` is the final evidence of success and
  clears the active/possibly-active/failed-release tracking masks even if the
  preceding `SendInput` reported partial or zero progress; transport anomalies
  stay visible in counters but do not fabricate a stuck-key set.
  `ReconciledRelease::Held` reports only the held subset and `Inconclusive`
  fails closed to the transport-unconfirmed subset. Transport and physical
  verification stay independent: `verification_inconclusive` is probe-derived
  only and is never OR-ed with the transport-anomaly flag. Normal clean-up never
  sleeps.
- No Python callback runs in the native real-time worker.
- A successful terminal result requires no active, pending, possibly-active, or
  residue key.

## Focus and liveness

Python owns process-name validation and target discovery. It sends one HWND
with `set_target_hwnd`; Rust uses the focus-tracker atomic as the coarse loop
gate and performs one authoritative `GetForegroundWindow()` comparison against
the stamped HWND immediately before a Down dispatch. Python does not send a
second focus boolean.

The HWND is also passed directly to preflight and cleanup verification. The
sender continues to inject the same physical scan codes; the layout-aware VK
mapping exists only for checking physical state and is not part of the hot
`SendInput` path. The current 15-key allowlist has no E0/E1 extended scan
codes.

Physical-key preflight is an admission boundary for every initial playback,
manual resume, focus restoration, and target-HWND generation. The worker
stores verification as a typed `(HWND, target-generation)` stamp, and clears
that stamp at every new manual/focus admission epoch; therefore resuming the
same HWND still requires a fresh preflight. A target change invalidates the
previous verification before the worker processes the next chord.
Cleanup/preflight runs while the playback clock remains paused; only after a
successful, still-current verification does the worker take a new QPC sample
and leave the pause. The steady-state loop uses only the focus-tracker atomic
as its coarse gate. Immediately before a Down, the worker performs one fresh
focus query against the exact stamped HWND, rechecks the target stamp, and
gates quit, skip, panic, and pause state before `SendInput`, so a change during
the Win32 verification window cannot send an unverified chord.

Each physical-state pass resolves the target thread and keyboard layout once,
maps only the requested fixed scan-code mask, and then reads the aggregate key
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

The final wait spin observes an event signal generation and QPC ticks only; it
does not convert ticks to microseconds or issue a zero-time Win32 event wait on
each spin iteration. The event handle remains authoritative for long waits and
command interruption, with at most one final zero-time handoff probe before a
deadline is reported. Estimator
lead-cache refreshes update the preallocated cache in place, and one clean
observation refreshes the affected cache once. CPU-time telemetry is sampled
on a bounded 100 ms interval with a final worker sample, while healthy shared
metrics publication is rate-limited and anomaly/terminal transitions publish
immediately. When telemetry is disabled, trace-record construction is not
performed.

The waiter returns raw `wake_qpc` and `spin_ticks` evidence. The regular worker
loop stores at most one fixed raw wait observation and hands it to the same
deferred observer path as dispatch observations; wake lateness, tick-to-
microsecond conversion, spin counters, and wait-health windows are therefore
not computed on the wake-to-dispatch handoff. A full observer queue drops the
wait observation without evicting a physical dispatch observation. Startup
gating may account for its own spin sample immediately because it is outside
the steady-state dispatch boundary.

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
