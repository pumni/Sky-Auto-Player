# Real-Time Dispatch Architecture

This is the normative implementation contract for the Rust playback worker.
The scheduler remains pure; the platform boundary is the only place that
touches Windows. All physical input uses `SendInput`.

## 1. Ownership and thread split

The worker dispatch thread owns the real-time sequence and the coordinator
owns schedule/generation state. The dispatch thread may read or commit
coordinator state only at the defined planning and ownership boundaries. It
must not perform telemetry formatting, unbounded allocation, observer health
updates, or learned timing estimation on the physical path.

Strict-timing diagnostics use a separate consumer thread. A bounded
`crossbeam_queue::ArrayQueue<DispatchObservation>` (capacity 64) is shared
between the diagnostic dispatch path and that consumer:

```text
dispatch thread --nonblocking push--> ArrayQueue --single consumer--> observer thread
                                      full: drop new observation
```

The diagnostic producer records a drop and continues. It never waits for slack
and never evicts an older observation. Production allocates neither this queue
nor an observer thread; it retains only bounded scalar worker counters. The
diagnostic consumer owns health windows, telemetry materialization, shared
observer metrics, and its own local timing state. It cannot authorize,
reorder, retry, or commit physical input. Diagnostic shutdown signals the
consumer, joins it, merges its local metrics, and then publishes the final
report.

Playback pause ownership is a closed typed set (`Manual` and `Focus`) backed by
a bitmask. The clock retains the first opener as typed attribution, so pause
overlap bookkeeping has no hash table or reason-string allocation.

## 2. Immutable plan

One outer worker epoch builds one typed plan from the coordinator. The plan is
`NoWork`, a metadata boundary, or a physical boundary. A physical plan carries
one prepared authored/pending view, its commit proof, and one absolute QPC
target. Observer health budgets are not physical-plan inputs. The plan is
reused by waiting, due selection, and physical dispatch. Commands,
focus/pause transitions, target changes, lease-only wakes, and interrupts
invalidate it and cause a replan.

The planner writes this product into the caller-owned plan slot instead of
returning the large physical enum through the `Result` ABI. The release
assembly report in `scripts/audit_dispatch_assembly.ps1` covers the planner,
physical-plan construction, and the dispatch-loop caller, while its hard
copy/stack/division policy applies only to due dispatch and missed-Down
recovery. Planner materialization remains a report-only optimization objective
before the precision wait; the optimizer may inline planner helpers.

The `rt_handoff_bench` JSON separates structural/counter success from timing
acceptance. A deadline miss, non-dispatch, early dispatch, failure reason, or
observation gap makes the scenario and aggregate `acceptance_clean=false`; the
aggregate is `statistics_eligible` only when every scenario is clean and has at
least 10,000 iterations. A host-preemption event is therefore retained for
paired baseline comparison instead of being silently reported as green.
The feature-gated `rt-native-acceptance` binary exercises the existing
`NativeDispatchSession` and `BackendConfig::Production` path. It is not a raw
sender or a timing benchmark. Real-input qualification requires the explicit
`--allow-real-input` flag, an exact target HWND, schema-v3 ready evidence, and
the live project-owned `pwsh.exe` window identity. There is no foreground,
environment, PID-only, or no-focus fallback.

Start the two receive-only project-owned WinForms observers separately. The
normal sink and the inert focus probe both log KeyDown/KeyUp; the probe never
emits input and has its own evidence file:

`Pwsh -NoProfile -File scripts/native_acceptance_sink.ps1 -Mode ReceiveOnly -RunId <run-id> -ReadyFile .benchmarks/sink.json -EventLog .benchmarks/sink-events.json`

`Pwsh -NoProfile -File scripts/native_acceptance_sink.ps1 -Mode InertFocusProbe -RunId <run-id> -ReadyFile .benchmarks/focus-probe.json -EventLog .benchmarks/focus-probe-events.json`

The physical command is an explicit, feature-gated qualification run:

`cargo run --locked --release --manifest-path rust/Cargo.toml -p sky_player --features real-input-acceptance --bin rt-native-acceptance -- run --allow-real-input --run-id <run-id> --sink-ready .benchmarks/sink.json --sink-events .benchmarks/sink-events.json --target-hwnd <sink-hwnd> --scenario <scenario> --evidence .benchmarks/rt-native-acceptance.jsonl`

The supported physical scenario names are `canonical-single`, `canonical-chord`,
`canonical-max-chord`, `hold`, `rapid-retrigger`, `mixed-up-down`, `focus-loss`,
`target-hwnd-change`, `pause-resume`, `stop-cleanup`, `skip-cleanup`,
`cleanup-full-release`, and `w4-noncanonical`. Each invocation appends one
JSONL report with the scenario name and its `PASS`, `FAIL`, or `INCONCLUSIVE`
verdict. The target-change scenario changes the session target to the invalid
sentinel `0` before the first authored Down and requires authoritative target
change rejection with no sink delivery; stop and skip similarly request their
control action before the first authored input and require clean termination.

Each ready record includes a process-generated `event_log_id` and explicitly binds
`event_schema_version = 3`. Before publishing ready evidence, the helper flushes
a schema-v3 `stream_start` header containing the run ID, role, and same event-log
ID to the exact event file. Keyboard records are observed synchronously in the
sink Form's `WndProc`; native scan code (`lParam` bits 16..23), extended bit
(bit 24), and direction are authoritative. Raw `wParam` VK, including
`VK_PROCESSKEY`, is diagnostic only. The harness rejects v2, empty, stale, or
mismatched event streams before `arm`; later records must retain the same
binding. The receive-side decoder self-test is no-input and runs with:

`pwsh -NoProfile -File scripts/native_acceptance_sink.ps1 -SelfTest`

After terminal/join, qualification drains the bound JSONL stream with a 10 ms
poll and a 1,000 ms maximum observation bound. Positive expected-event scenarios
require the expected physical set and then 100 ms of quiet before reconciliation.
The focus probe uses typed `ZeroEventSafety` semantics instead: it cannot
early-complete on quiet, and any keyboard event anywhere in the full 1,000 ms is
FAIL. Only a valid, continuously bound, event-free stream through the full
deadline can satisfy the probe zero-event predicate; truncation, sequence
corruption, or binding loss is INCONCLUSIVE. These are acceptance-observer
bounds and do not change production scheduler or timing policy.

The acceptance profile materializes the current default equation at 60 FPS:
`frame_us = 16,667`, and with the default 500 us Down grace plus 300 us
transport margin, both minimum hold and release gap are 17,467 us; focus
restore grace remains 100,000 us. Controlled A/B runs may pass
`--down-late-grace-us 500|750|1000`; the harness recomputes both schedule
constraints from that session-frozen value and records it in every report.
This override is acceptance-only and does not change the shipped production
default. The `focus-loss` scenario
first commits a canonical sink Down/Up pair, waits for `startup_ready` and that
pair's event evidence, then moves foreground to the validated project-owned
probe with the coarse focus hint still true before a later different-slot Down.
It observes the existing live pause state and requires the terminal final-gate
counter afterward. Sink/probe logs prove controlled Windows delivery and
wrong-window safety only; they do not prove game receipt, audio latency, or the
internal QPC timestamp chain.

Real `SendInput` cannot deterministically produce a partial or zero-progress
return. Those transport fault seams remain qualified by the scripted
test-support paths, which must be run alongside the physical matrix:

`cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support zero_progress`

`cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --lib --features test-support partial_fault`

`cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --lib --features test-support partial`

`cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_win32 --lib --features test-support zero_progress`

The `cleanup-full-release` scenario is the only acceptance scenario that claims
fresh physical All-Up evidence. It authors one bounded Down chord across all 15
logical slots, waits for all 15 sink KeyDown records after `startup_ready`, then
requests normal `quit()` before the authored Up. Its pass evidence requires
full-mask tracked cleanup (`attempted_mask = 0x7fff`, at least one attempt,
successful release, no stuck mask, inconclusive verification, or transport
anomaly) and exactly 15 matching sink KeyDown/KeyUp records. All non-cleanup
scenarios require their authored/logical and transport evidence only; a zero-mask
terminal release result is not a fresh physical All-Up probe.

Authored logical preparation validates and consumes the selected packet's
compact intents in one primary pass, freezing the commit proof and the batch
source metadata from that same packet view; deferred Up ownership may perform
the bounded source-batch resolution required to identify its original action.
That resolution is part of preparation evidence and never occurs after the
frozen product is handed to the player layer. Pending-release merging remains
a separate bounded mask operation. Physical
packet storage initializes only its `0..len` prefix. Borrowed packet views expose
that initialized prefix and never expose the unused fixed-capacity tail; Mixed
missed-Down recovery reuses the primary packet's canonical Up prefix instead of
materializing a second payload.

The authored commit token stores immediate and deferred Up identities in one
bounded vector. `immediate_up_mask` and `deferred_up_mask` classify those
entries during the single bounded commit pass, while only deferred entries
use the source-action identity needed by the pending-release table. The
active-generation ledger stores ownership identity, scan code, slot, source
action, and the authored hold floor; dispatch start/completion timestamps remain
transport observations rather than per-key hot-state fields.

Test-support preparation counters are emitted at the coordinator operations
that acquire the packet view, visit each intent, perform registry lookups,
resolve deferred source batches, and construct the frozen commit. They are not
derived afterward from slice lengths.

Production planning has no adaptive dispatch-cost estimator and no lead
subtraction. Authored timestamps are used as authored. Release deadlines
include only the authored minimum-hold policy. Any remaining lead-shaped
arguments are test-only
compatibility seams; production coordinator APIs do not accept a dispatch lead
and the publication adapter reports the historical applied-lead field as zero.

The physical target is derived once from the playback epoch:

```text
physical_target_qpc = playback_epoch_qpc + effective_deadline_ticks
```

Waitable-timer guard and bounded spin are wake mechanics only. Neither is an
applied dispatch lead and neither changes the coordinator deadline. The target
is carried through the final gate and sender; it is not replaced by the wake
sample or reconstructed after `SendInput`.

Stale-Up compiler metadata with an empty packet is handled as coordinator
metadata. It has no physical path, no sender boundary, and does not consume a
startup or normal physical target. Physical packets are packetized from the
canonical masks.

## 3. Final physical sequence

The Down/Mixed and Up-only paths share the same authoritative transport order.
Down adds the target/focus checks; Up-only never uses the focus gate.

```text
frozen plan
  -> prepare immutable packet before the target wait
  -> one interruptible hybrid wait to the authored target
  -> worker-owned bounded QPC spin across the target
  -> final command/control, target, and foreground proof
  -> cheap program-owned control/target/focus atomic revalidation
  -> final_policy_qpc sample and lease admission
  -> sender SetLastError(0), true pre_call_qpc sample, and Down-only cutoff
  -> one packetized SendInput call
  -> completion QPC and transport-mask validation
  -> coordinator ownership commit on clean success
  -> terminal fail-closed cleanup on transport anomaly
  -> essential scalar state commit
  -> diagnostic-only bounded observation enqueue
```

The worker-owned target crossing happens before final policy admission. The
worker's `final_policy_qpc` is the post-revalidation policy/lease evidence
sample. The trusted prepared sender then resolves the fixed payload pointer
and length, resets thread-local Win32 error state, takes the true
`pre_call_qpc` immediately before the syscall, and applies the Down-only
cutoff against that sample.
The sender performs no target wait or policy recheck after receiving the
prepared packet.
The transport reports `sendinput_completion_qpc`; production does not subtract
a learned send cost from the target. Completion is used for diagnostics and
ownership evidence only; it does not create a completion-relative hold floor.

The primary sender-side timing evidence is the signed start residual:

```text
dispatch_start_error_ticks = pre_call_qpc - physical_target_qpc
pre_call_to_completion = sendinput_completion_qpc - pre_call_qpc
completion_error = sendinput_completion_qpc - physical_target_qpc  # diagnostic only
```

The start residual is benchmark/observability output only. It is never fed
back into scheduling or used as adaptive compensation.

Packet construction validates scan-code masks and sends Up entries before Down
entries in one call. A zero, partial, skipped, mixed, or otherwise inconsistent
transaction is handled fail-closed. Production never retries `SendInput` and
never queues a release for a later transport attempt. Any transport anomaly
forces full-instrument cleanup and termination.

Cleanup verification keeps transport and physical evidence separate. A
`VerifiedAllUp` result is possible only when the target HWND is still the
foreground window, the physical probe observes no held instrument key, and the
entire requested Up mask is confirmed by the transport. A zero- or partial-
progress transport result combined with physical all-up is therefore
inconclusive, never cleanup success. Focus loss also makes the physical probe
inconclusive; it must not be interpreted as all keys being up.

## Focus pause and recovery

The supervisor's focus hint is a coarse wake/gate signal. The worker's outer
focus-loss transition is a pause edge, not a cleanup edge:

```text
focus invalid
  -> clear verified target and restore marker
  -> increment focus_lost once on entry
  -> enter focus pause and publish progress
  -> perform no physical release or preflight while unfocused
```

This keeps an unfocused physical probe `Inconclusive` rather than treating it
as evidence that all instrument keys are up. No Down can be admitted while the
focus pause is active, and the fresh foreground HWND check remains the final
physical Down authority.

After focus is observed again, the worker waits for the configured restore
grace and validates the current foreground/target identity. If a manual pause
is already active, it clears only the focus pause; it does not release or
preflight physical input a second time. The manual-resume path owns the one
preflight required before playback continues. Without an active manual pause,
the worker performs the safety sequence below:

```text
load current target stamp
  -> clear verified target
  -> suspend_live_input
  -> physical preflight
  -> fresh foreground HWND and target-stamp checks
  -> exit focus pause
```

Cleanup or preflight failure is terminal and fail-closed. If focus or target
identity changes after cleanup, the worker clears the restore marker and stays
paused for another attempt. A manual pause remains independent; restoring
focus never clears it.

When focus is required, the native desktop supervisor makes one minimal
`ShowWindow(SW_RESTORE)` plus `SetForegroundWindow` attempt at playback
startup, before the native worker starts. It refreshes the cached target and
actual foreground state afterward and treats an OS refusal as an ordinary
non-terminal outcome. After startup, focus polling only publishes observed
state; it never reclaims foreground focus automatically. Explicit manual
refocus remains the user-controlled retry path.

## 4. Authored timestamp and minimum-hold model

The native application materializes the fixed floor before worker startup:

```text
frame_us = ceil(1_000_000 / game_fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
down_late_grace_us = policy.down_late_grace_us
transport_margin_us = max(0, calibrated_or_default_transport_margin_us)
effective_min_hold_us = frame_base_hold_us + down_late_grace_us + transport_margin_us
sender_headroom_us = down_late_grace_us + transport_margin_us
min_release_gap_us = frame_us + sender_headroom_us
```

Native admission checked tick arithmetic enforces before worker start:

```text
authored_up >= authored_down + effective_min_hold
next_same_key_down - previous_same_key_up >= min_release_gap_us
```

The static components are materialized once into the authored schedule. The
`FrameTimingPolicy.min_hold_margin_us` compatibility field contains the
Down-grace-plus-transport aggregate; explicit fields retain the frame base,
grace, transport margin, release gap, and transport provenance. The release
gap reserves one frame plus the same bounded sender headroom; it is sender-side
visibility policy, not evidence that the game sampled the Up transition.
An invalid
interval fails native admission before any musical SendInput. The worker never
combines Down completion with the authored hold to create a second floor, never
creates a completion-derived minimum-hold terminal state, and never rewrites an
authored Up target. Runtime
deadline/overdue policy handles a late boundary; recovery-only pending
releases are stored in a fixed `[Option; 15]` per-key table with mask and
generation ownership. There is no transport retry state.

The session-fixed `down_late_grace_us` is an independent sender correctness
policy, currently `500 µs`, converted once to QPC ticks at admission. The
transport margin defaults/falls back to `300 µs` and is not part of this grace.
The grace bounds authorized Down lateness only. It is never derived from the hold margin,
calibration, or dispatch lead, and never changes an authored target. The
trusted sender repeats the same cutoff check immediately before `SendInput`,
while Up-only safety releases remain exempt.

## 5. Wait and interrupt ordering

For a future physical plan, the worker uses one high-resolution waitable timer
and event-interruptible hybrid wait directly to the absolute physical target.
The waiter sleeps while the target is farther away than the frozen spin
threshold, then performs the bounded QPC spin until the target. There is no
per-note `T - guard` admission wake and no second precision wait. A lease-only,
command, focus, pause, or interrupt wake replans and cannot dispatch the old
plan. The timer is first in the Windows multi-wait handle array so a
simultaneous timer/event wake enters the QPC classification path. Before the
physical target, an interrupt returns `Interrupted`; once QPC reaches the
target, the waiter returns `Deadline`. Final command, target, focus, and lease
admission remains authoritative after that result and may still reject
`SendInput`.

Production calibration runs once during worker startup: six bounded wake
samples are used only when the full 20 ms probe budget plus the 2 ms startup
readiness reserve fits before the startup readiness deadline. Each sample is
also checked against that bounded probe deadline. The threshold is
`clamp(max(p99, robust) + 50 µs, 250 µs, 1,000 µs)`; a skipped or failed probe
uses the 1,000 µs fallback. The chosen value is frozen for the session.
Calibration changes waiting cost only: it never changes an authored target and
never introduces a dispatch lead.

The final precision spin performs only its QPC target/down-late-grace comparison and
`spin_loop`. Interrupt, lease, command, focus, and pause invalidation decisions
are completed before that stage; no interrupt-generation polling or control
branch is inserted into the final spin. The QPC deadline check remains
authoritative and cannot be bypassed by an event.
Production admission requires the high-resolution waitable timer and event wait
and terminates on startup or runtime wait failure; it does not degrade to sleep
timing. `WaitBoundary::Due` carries the authoritative wake QPC into dispatch;
the same sample is reused as target-crossing evidence for the final gate.
Neither path reconstructs the physical target from wake time or enters a
redundant physical wait.

MMCSS Games/High and process power-throttling opt-out are scoped to the worker.
TimeCritical is not the default and priority setup failure is reported rather
than silently changing the timing contract.

## 6. Startup and stale work

Startup establishes the playback epoch and waits for the first physical target
using the same target formula as steady state. There is no adaptive startup
lead. It may run the bounded startup wake probe described above, but that probe
only selects the frozen wait threshold. The first physical call still performs
the complete final control/lease/target gate. Stale metadata may be committed
before physical work, but it cannot run after the final target wait on the
physical call stack.

The worker may project a pre-epoch deadline for control-loop wake/replan
bookkeeping, but it never calls `SendInput` before the authoritative physical
QPC target/epoch gate. This projection distinction must not be used to create
an early physical send.

After a successful musical Down boundary, the worker tracks a Down-only
authorization state. A future Down-bearing boundary is authorized by an exact
stamp containing the frozen authored packet identity, masks, and physical QPC
target. The stamp survives waiter-entry latency and a same-boundary
`Continue`/replan, but not a changed plan, target, epoch, pause, focus rebase,
or completed/missed commit. The kernel wait result is not the musical proof.

The first overdue Down discovered within the fixed grace may use the
one-shot late-discovery rescue credit carried by `AwaitingFuture`, but only
after playback has started and while the exact future authorization,
focus/control/target, and lease proof remain valid. The credit is consumed at
admission; a second overdue boundary without an intervening future observation
is `MissedBacklog`. The sender's pre-call cutoff remains authoritative, so a
rescue never sends after the cutoff and never retries or catches up a missed
Down.

An unobserved overdue Down is a recoverable Production deadline miss after the
first successful musical Down: the Down portion is omitted, the frozen
coordinator frame is committed as missed, and playback advances to the next
authored target without rebasing or changing timestamps. A Mixed frame sends
only its required Up subset through one borrowed view of the prepared primary
packet's canonical Up prefix; no second recovery payload is materialized. A
failed or uncertain safety Up remains terminal. Up-only safety releases are exempt from
the musical backlog rule and are sent even when late. Strict-timing diagnostic
mode may retain terminal behavior for qualification. In every mode, missed
Downs are never retried or emitted as a catch-up burst.

## 7. Failure and publication boundaries

Every QPC query used for a correctness decision is terminal on failure.
Coordinator commit follows confirmed transport evidence; a typed
`DeadlineMissedBeforeSend` result is handled as a missed authored frame only
after startup and only when no Down syscall occurred. Cleanup releases
active/possibly-active keys and verifies the resulting state before successful
completion. The ready boundary is published only after startup gates and the
required physical ownership and cleanup state are complete.

Production has no observer failure or queue-overflow path. Its fixed
worker-local forensics block publishes an availability/version marker and
bounded scalar evidence for hold pairs, pre-call/completion shrink,
release-gap policy, same-call retriggers, anchor overwrites, unmatched Ups,
and a fixed anomaly ring. It adds no QPC sample, allocation, lock, formatting,
or unbounded scan to the production send path. Diagnostic observer
failure, telemetry overflow, or metric conversion failure cannot
rewrite physical ownership. The worker terminates through the normal cleanup
path and preserves the primary and secondary errors.

The live snapshot is a small projection of authoritative playback state.
The diagnostic observer publishes sender hold-forensics scalars and the last
classified missed-Down sample only in the terminal/full snapshot; the
lightweight polling snapshot remains unchanged. The diagnostic final report is
materialized after worker and observer shutdown;
the production report uses worker scalar state. Historical estimator plumbing
is not part of the active production session contract and cannot affect timing.

Hold-forensics ownership must also follow physical releases that are outside a
regular successful authored observation. After a successful recovery safety Up,
the dispatch producer enqueues the bounded FIFO lifecycle event
`RecoveryUp(mask)`; the observer clears only those physical anchors. After a
successful global release (for example focus suspension or manual pause), it
enqueues `ResetAll`; the observer clears every anchor. These lifecycle events
are diagnostic state synchronization only and never authorize, split, retry,
or catch up a physical packet. Dropping lifecycle evidence is reported through
`observer_dropped_samples` and makes diagnostic qualification invalid, because
the observer can no longer prove generation ownership.

## 8. Verification matrix

- `sky_dispatch_core`: controlled-clock deadline, hold-floor, authored
  ownership, stale/retrigger/mixed-packet, and soak tests.
- `sky_dispatch_win32`: packet ordering, strict mask validation, QPC/wait,
  focus, and `SendInput` seam tests.
- `sky_player`: final-gate ordering, completion evidence, diagnostic observer queue
  overflow, startup/stale handling, cleanup, and no-allocation dispatch tests.
- Security audit: only the platform crate contains Win32 bindings and
  `SendInput`; forbidden hook/injection/process-tampering mechanisms remain
  absent.
