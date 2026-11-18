# Playback Engine

This is the current contract for native playback scheduling, sender admission,
cleanup, and session retirement. The implementation map points to the current
source; [playback verification](playback-verification.md) owns commands and
qualification evidence. [SECURITY.md](../SECURITY.md) owns the security policy.

## Ownership

- The Tauri playback service prepares a sheet, starts and controls a native
  session, joins its worker, commits retirement by session ID, and coordinates
  recovery and UI publication.
- **sky_player** owns the playback worker, target authority, final admission,
  dispatch control, timing, and the worker cleanup result.
- **sky_dispatch_core** owns the fixed musical executor and schedule policy.
- **sky_dispatch_win32** owns Windows focus queries, waits, and physical
  **SendInput** effects.

Preparation compiles and validates the immutable dispatch schedule before a
worker starts. The worker moves prepared storage into its executor; it does not
compile or materialize the sheet. Healthy dispatch does not allocate.

## Session authority and final admission

A physical session binds one immutable target HWND, owner PID, retained process
object, and process incarnation. The worker is the session's only physical
writer. Recovery may take the journal only after that worker has joined.

Every musical packet, including an Up-only packet, passes a fresh final check
against the staged HWND and retained owner identity. A packet with Down also
requires an unpressed modifier set and a valid supervisor lease. The check
uses a fresh foreground HWND/PID query and process-liveness proof. Cached focus
state and callback notifications cannot grant admission.

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
The sender floors are:

- H = ceiling(hold_frames × T) + timing margin.
- G = T + timing margin.

An Up is eligible no earlier than both its authored Up time and successful
Down completion plus H. A same-key Down is not eligible before successful Up
completion plus G. Completion is measured at the sender boundary using QPC;
these floors do not promise which game frame samples a key.

A musical onset is attempted at most once. The worker commits a musical action
only from valid transport and QPC completion evidence. A packet with a
transport anomaly is not replayed. It terminates or replans according to the
current control outcome while preserving any journaled obligation for cleanup.
Required QPC samples and the production waiter policy remain part of the
timing contract.

## Journal and bounded cleanup

The tracked key journal is written before calling **SendInput**. Only journaled
keys may be released. The journal distinguishes:

- **needs_up**: a key still needs a transport-qualified safety Up.
- **needs_probe**: transport already attempted its Up, but release is not yet
  verified.

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
