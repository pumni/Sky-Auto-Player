# Playback Engine

This is the current contract for native playback scheduling, sender admission,
cleanup, and session retirement. The implementation map points to the current
source; [playback verification](playback-verification.md) owns commands and
qualification evidence. [SECURITY.md](../SECURITY.md) owns the security policy.

## Ownership and Progress Partitioning

- The Tauri playback service prepares a sheet, starts and controls a native
  session, monitors worker progress, joins the worker, commits retirement by
  session ID, and coordinates recovery and UI publication.
- **sky_player** owns the playback worker, target authority, final admission,
  dispatch control, timing, and the worker cleanup result.
- **sky_dispatch_core** owns the fixed musical executor and schedule policy.
- **sky_dispatch_win32** owns Windows focus queries, waits, and physical
  **SendInput** effects.

Native worker progress is strictly partitioned into four states:
`Starting`, `Active`, `Paused`, and `Draining`.
Shell-level **Quarantine** and **Retired** states remain exclusively owned by
the desktop shell (`apps/desktop/src-tauri/src/app_state.rs` and
`apps/desktop/src-tauri/src/native_runtime/playback/retirement.rs`).

The desktop supervisor thread retains an RAII `SupervisorLifetimeGuard`
throughout active physical playback. If the supervisor thread unwinds or panics
unexpectedly, the guard triggers prompt safety release with terminal reason
`"supervisor_unwind_or_exit"` via atomic one-winner signaling without blocking
on metrics locks. The guard is safely disarmed upon intentional termination
(`quit`, `skip`, `panic_release`, and confirmed `join`).

Preparation compiles and validates the immutable dispatch schedule before a
worker starts. The worker moves prepared storage into its executor; it does not
compile or materialize the sheet. Healthy dispatch does not allocate.

Prepare reports two independent timing signals. The authored Up-gap check counts
feasible same-key repeats whose authored Up-to-next-Down gap is shorter than one
frame. A bounded offline estimate also advances the canonical schedule through
the production `DispatchExecutor` with explicit sender-cost assumptions and
reports lateness, H-floor displacement, and head-of-line delay. The estimate is
diagnostic only: it does not change admission, schedule timing, or playback, and
does not predict whether the game observes an Up pulse.

## Session authority and final admission

A physical session binds one immutable target HWND, owner PID, retained process
object, and process incarnation. The worker is the session's only physical
writer. Recovery may take the journal only after that worker has joined.

Pure **Control A** dispatch is the authoritative path for all Down, Up, and
mixed packets. `ProcessQosPolicy` is pinned to **Default**; experimental B/C
process QoS bit toggling and adaptive pre-spin adjustments are not enabled in
production. The pre-spin threshold is frozen.

Every musical packet, including an Up-only packet, passes a fresh final check
against the staged HWND and retained owner identity. A packet with Down also
requires an unpressed modifier set and a valid supervisor lease. The check
uses a fresh foreground HWND/PID query and process-liveness proof. Cached focus
state and callback notifications cannot grant admission.

When supervisor lease age checking is enabled, the current sender applies its
lease proof to Up-only packets as well as Down-bearing packets. An expired or
unproven lease is terminal for either packet kind. Up-only packets remain
independent of modifier state; this lease behavior is retained intentionally.

### Modifier hold and replan policy

- When a modifier key is detected as `Held`, the worker enters a soft wait
  bounded by `max_hold_deadline`. An `Inconclusive` query fails closed and
  blocks admission.
- A modifier-blocked Down onset does not rebase the timeline epoch or delay due
  Ups. Any Up traffic due during modifier backoff proceeds independently.
- Releasing one modifier key or sending an independent Up does not reset the
  hold budget of other active keys.
- Modifier hold timeout strictly requires `modifier_hold_timeout_us > 0` at
  admission. Duration arithmetic overflow fails closed as a terminal error.

The foreground WinEvent callback is a wake hint only. The worker or recovery
owner rechecks current identity before acting; polling is the fallback if the
watcher is unavailable. Loss of authority, Stop, Skip, Panic, lease expiry, or
suspend prevents a new musical Down. Cleanup safety Up is a separate,
journal-limited path.

Admission fails closed if a required identity, foreground, modifier, or lease
check is unavailable or invalid. A musical Down is never sent outside the
exact current foreground owner.

## Musical executor and timing

One fixed 15-slot executor owns musical Down and Up eligibility. A healthy
chord uses one prepared packet. When disjoint Up and Down obligations are due
together, they may share a packet with every Up ordered before every Down.
Authored Down order is retained, and each slot's eligible Up is considered
independently.

Let T be the ceiling of 1,000,000 divided by configured FPS, in microseconds.
The sender timing policy is:

- H = ceiling(hold_frames × T) + timing margin.

An Up is eligible no earlier than both its authored Up time and successful
Down completion plus H. After a successful Up commits, the next same-key Down
can be sent at its authored time after a fresh scheduler evaluation; there is
no additional musical release delay. Resume waits for verified cleanup, a
reconciled release journal, current generation and foreground ownership, a
valid owner lease, and the committed Resume control. The next send uses a fresh
QPC admission sample and the playback epoch updated at Resume; cleanup does
not add an elapsed-time floor. H and sender completion are measured at the
sender boundary using QPC; these policies do not promise which game frame
samples a key.

A musical onset is attempted at most once. The worker commits a musical action
only from valid transport and QPC completion evidence. A packet with a
transport anomaly is not replayed. The first anomalous SendInput or required
QPC sender attempt terminates musical playback without replaying the attempted
onset. The worker reconciles only the exact journaled physical obligations
through bounded safety cleanup. A proven raw-zero Down creates no new physical
release obligation, but still terminates musical playback. Pre-send
control/admission replans are separate; they do not resume playback after a
sender anomaly.
Required QPC samples and the production waiter policy remain part of the
timing contract.

Worker metrics keep QPC tick counters as their source values and retain the
existing 50 ms live-publication cadence. A snapshot reader copies the dispatch
and observer source slots as one readout, then derives public microsecond
fields after releasing the slots. This keeps unit conversion out of the
dispatch publisher while preserving the public snapshot and telemetry schema.
Observer sample expectations, pending work, missing samples, and queue drops
remain explicit in the snapshot.

Terminal metrics publish once after terminal cleanup, observer shutdown,
telemetry export, and scheduling-guard teardown, and before the typed terminal
result becomes visible. The desktop treats live snapshots as progress and
diagnostic data; worker join remains the fence for retirement and release
ownership transfer.

## Pause, Resume, and Journal

When the engine is paused with a clean journal (all obligations reconciled),
the worker performs zero periodic waitable-timer wakes and sleeps indefinitely
on the lifecycle interrupt event (`interrupt.wait_indefinitely()`).

Before resuming dispatch (`exit_pause`), the worker revalidates all authority
prerequisites via `validate_resume_prerequisites`:
1. Supervisor lease freshness (`!is_expired()` and `!check_expired()`).
2. Target window identity validity and generation match.
3. Target owner process continuity (`WindowIdentityContinuity::Continuous`).
4. Target window foreground match (`focus_matches_hwnd`).
5. Release-owner proof currency (matching exact target HWND, generation, and
   continuous owner PID).

If uncommitted obligations exist in the journal upon resume, they are drained
via `suspend_live_input_for_dispatch_with_control` before re-establishing the
dispatch timeline.

## Journal and bounded cleanup

The tracked key journal is written before calling **SendInput**. Only journaled
keys may be released. The journal distinguishes:

- **needs_up**: a key still needs a transport-qualified safety Up.
- **needs_probe**: a foreground-qualified Up was transported, but release is
  not yet verified. A bounded terminal Up sent while the origin is in the
  background is retained as raw sender evidence and leaves the original
  **needs_up** obligation intact; it cannot become probe-only evidence.

Worker cleanup is bounded to at most four raw Up calls per drain. Retries apply
only to **needs_up**; **needs_probe** can be probed but never causes another Up.
A Held or Inconclusive probe keeps the residual unresolved. No cleanup path
releases keys outside the session journal.

After worker join, one recovery attempt can make at most one raw Up call, then
probe the retained origin. Recovery uses the original HWND/PID/process
incarnation and never runs concurrently with the session worker. A Held or
Inconclusive result preserves the residual.

## Retirement, quarantine, and publication

The shell joins the worker before committing terminal retirement or transferring
the journal to post-join recovery. Retirement is fenced by session ID. If
release is not verified, the shell quarantines that physical session and denies
successor admission until release is verified. The retained residual and its
original authority remain associated with that session.

The shell commits terminal state and release/quarantine ownership before it
publishes a UI event. UI publication does not own retirement. Existing IPC
payloads and status-query recovery remain the compatibility path for stale or
missed events; session IDs fence old results from a newer playback.

## Source map

- Shell ownership, retirement, and recovery: **apps/desktop/src-tauri/src/native_runtime/playback/**.
- Worker authority and cleanup: **rust/crates/sky_player/src/engine/**.
- Executor: [executor.rs](../rust/crates/sky_dispatch_core/src/executor.rs).
- Windows target, packet, journal, and watcher:
  **rust/crates/sky_dispatch_win32/src/**.
- No-allocation evidence:
  [rt_dispatch_no_alloc.rs](../rust/crates/sky_player/tests/rt_dispatch_no_alloc.rs).
