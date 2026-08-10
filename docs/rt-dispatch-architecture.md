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

- `planning.rs` owns projected Hot/Cold classification, lead selection, the
  pending-release plan, frozen health budgets, and the next wait deadline.
  `plan_next_dispatch_projected()` builds exactly one immutable
  `NextDispatchPlan` per loop epoch from the coordinator's next uncompensated
  physical boundary, the previous SendInput completion, and the estimator. It
  never samples QPC itself, mutates the coordinator, allocates, or formats
  strings on the success path.
  The same `AuthoredDispatchPlan` lead feeds both the prepare-due boundary and
  the wait deadline, so prepare and wait cannot disagree on lead selection.
- `dispatch/` owns the pending-release and authored-packet backend
  transactions, transaction-outcome interpretation, coordinator commit, and the
  dispatch telemetry record. Both note-on and note-off paths first finish
  physical/coordinator correctness, required recovery, the mandatory terminal
  SLO decision, and a raw `dispatch_ready_qpc` sample. They then enqueue one
  allocation-free raw `DispatchObservation` (tagged `Down`/`Up`) containing
  QPC/timeline ticks and physical requested/confirmed/skipped masks into a
  fixed-capacity (64) worker-owned ring and return. Transport counts are derived
  from those masks only during drain; no duplicate microsecond lead/count facts
  are stored in the raw record. Tick-to-microsecond conversion, wake/send/ready
  deltas, completion errors, trace-record materialization, estimator update,
  health-window observation, lateness accounting, and diagnostic metric
  publication are deferred to `observer::drain_one_observer`. If the ring is
  full, the oldest raw record is dropped in O(1) and the new record is admitted;
  no observer work runs synchronously to relieve queue pressure.
  The dispatch loop applies a fixed 5,500 µs-equivalent QPC guard before
  observer work: fresh QPC → immutable `NextDispatchPlan` → drain at most one
  observation only when next-deadline slack is at least `observer_guard_ticks`
  (or no deadline remains) → if drained, discard the plan and rebuild from a
  fresh QPC sample before wait/admit/dispatch. Observer work never rolls back
  physical ownership; a deferred observer failure may still terminate the
  session after that ownership is safe. It returns a closed four-variant
  `DispatchStep`
  (`NoWork`, `Dispatched`, `Continue`, `Terminate`) instead of scalar tuples.
- `cleanup.rs` owns suspension/terminal cleanup, clean-completion proof, and
  terminal-error aggregation.
- `control.rs`, `admission.rs`, `wait.rs`, `health.rs`, `timing.rs`, `startup.rs`
  own their single named concern.

A plan is invalidated by an interrupt, command, focus/pause transition, backend
call, or release-recovery change. A normal waitable-timer deadline wake is
different: it preserves the immutable plan and hands it directly to the
dispatch helper, so the worker does not restart the full orchestration epoch
between the deadline and `SendInput`.

The precision boundary is intentionally short. Health thresholds are frozen in
the plan, the final admission checks panic/quit/skip/pause, supervisor lease,
target generation, and focus as applicable, and an `Allowed` result proceeds
directly to the backend transport. Estimator, health-window, telemetry
materialization, and observer work remain deferred after the fixed raw
observation enqueue.

Every physical send uses the shared control-and-lease gate with a fresh QPC
sample. Down-bearing authored traffic then applies the target-generation and
foreground/focus gate; UpOnly authored traffic and pending releases are
cleanup traffic and do not require focus or target stability. The control
atomics are the authoritative last-mile command state. The event's monotonic
generation is only a spin interruption hint, and an event handle is consumed
only after a replan outside the precision boundary.

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
`panic`), `set_target_hwnd`, transition-only `set_focus_hint`,
`snapshot_lite`, and `session_report`.

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
`core_post_send_max_us` (a typed `dispatch_ready_qpc - sender_completed` QPC
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
  immutable physical packet and one low-level `SendInput` attempt. A partial Down/Mixed insertion is terminal
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
with `set_target_hwnd` and uses its cheap cached-HWND foreground probe to send
`set_focus_hint(bool)` only on supervisor-observed transitions. Rust uses this
hint as a coarse fail-closed loop gate and wake-up, then performs one
authoritative `GetForegroundWindow()` comparison against the stamped HWND
immediately before a Down dispatch. A `true` hint never authorizes input.

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

Every successful focus-pause transition publishes the progress-clock anchor
immediately after `enter_pause("focus", ...)`, including the authored early
unfocused admission and the final last-mile Down admission. This keeps
`snapshot_lite.is_paused` and elapsed-time freezing correct even when focus is
lost between supervisor samples or during a long event gap.

Each physical-state pass resolves the target thread and keyboard layout once,
maps only the requested fixed scan-code mask, and then reads the aggregate key
state. Mapping or state ambiguity remains fail-closed. The resulting layout
work is therefore limited to admission/cleanup boundaries and is not repeated
for each healthy chord.

The UI/control polling loop publishes the supervisor heartbeat. There is no
separate heartbeat thread: if the control loop stops, native lease liveness
reflects that failure.

Playback progress uses a separate transition-only projection of the native
`PlaybackClockState`. The worker publishes its epoch and pause anchor through a
non-blocking atomic seqlock only at clock transitions; `snapshot_lite()` and the
full snapshot sample QPC on the supervisor side and derive elapsed time there.
This projection is independent of SendInput, telemetry, and observer draining,
so a long event gap does not freeze the HUD and UI polling never adds work to
the realtime dispatch path.

Layout acceptance is a Windows manual matrix, not a CI claim: run preflight,
resume, pause, and panic-release checks under English US, German, and French
layouts, recording the layout identifier and result. A receiver/probe may
count scan-code events and QPC order, but its result is host-side evidence and
does not establish game receipt.

## Healthy worker path

The final wait spin observes an event signal generation and QPC ticks only; it
does not convert ticks to microseconds or issue a zero-time Win32 event wait on
each spin iteration. A successful deadline handoff does not consume the event
handle at all; it revalidates the authoritative command atomics immediately
before transport. The event handle remains the blocking-wait primitive for
long waits and is drained only on the non-precision replan path. Estimator
lead-cache refreshes update the preallocated cache in place, and one clean
observation refreshes the affected cache once. CPU-time telemetry is sampled
on a bounded 100 ms interval with a final worker sample, while healthy shared
metrics publication is rate-limited and anomaly/terminal transitions publish
immediately. When telemetry is disabled, trace-record construction is not
performed.

When adaptive spin is enabled, startup probes exactly 32 wake samples and
derives the session threshold with the existing safety margin, 700 µs floor,
and 3,000 µs cap. The production priority policy remains the `Auto` MMCSS
Games → `THREAD_PRIORITY_HIGHEST` fallback ladder; diagnostic priority modes
are not production policy.

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
