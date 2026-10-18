# Real-Time Dispatch Architecture

This document is normative for the real-time playback path. The application
reads authored schedules, materializes paired musical generations, and emits
physical keyboard input through the Windows `SendInput` boundary. The current
resumable lifecycle and final lease contract is recorded in
[ADR-0018](adr/ADR-0018-resumable-release-barrier-and-final-lease-age.md).

## Ownership boundaries

`sky_dispatch_core` owns schedule compilation, generation pairing, authored
hold/release validation, and causal schedule identity. It has no Win32 or QPC
sender implementation.

`sky_player` owns worker orchestration, prepared packet selection, final
lifecycle/focus/control admission, physical waits, completion evidence, and
per-key physical floors. `sky_native_adapters` owns OS and process-facing
adapters. `sky_dispatch_win32` owns packet materialization and the one
`SendInput` transaction. No other crate may simulate gameplay input.

The coordinator owns authored cursor and generation state. The worker owns
runtime QPC state and a fixed-size `PhysicalTimingGuard`; completion timestamps
never enter the domain schedule or move authored targets.

## Worker-owned preparation and epoch publication

`NativeDispatchSession::arm()` validates the requested pre-roll and starts the
worker. It does not publish a playback epoch or claim that the worker is
Armed. The worker owns this transition:

```text
arm request -> Preparing -> Ready -> Arming --atomic commit--> Armed
                                   \                         /
                                    +-- stop, authority loss,
                                        suspend, lease expiry,
                                        or failure -> terminal
```

During `Preparing`, the worker constructs the coordinator, simulates and
materializes the immutable normal stream, restores the dispatch coordinator,
creates the coordinator's fixed fifteen-slot cancellation-token table and
bounded telemetry storage, creates the required
wait and observation resources, and acquires worker scheduling guards. Long
packet and coordinator traversals check cancellation at bounded intervals.
Prepared normal storage is admitted only when its conservative estimate is at
most 512 MiB; the telemetry ring is capped at 65,536 records. Allocation and
admission errors stay terminal and cannot publish an epoch.

`Ready` is an internal milestone: the engine continues to project
`EnginePollStatus::Preparing`, with zero elapsed time and no countdown. The
worker then enters `Arming`, rechecks quit/skip/panic, suspend, supervisor
lease, target generation, focus, and required owner identity, computes a fresh
QPC epoch as `epoch_base + requested_pre_roll`, and stages the clock projection.
One atomic `Armed` transition publishes the immutable epoch and pre-roll to
readers. A concurrent cancellation or authority change that commits first
prevents Armed publication; a change after Armed still meets the dispatch
loop's final per-packet gates. Readers load the startup gate once before
projecting epoch, elapsed time, and countdown, so they cannot combine a staged
epoch with a pre-Armed status.

All construction time is outside the published count-in. The first authored
target remains `epoch + authored_timestamp`; no packet timestamp is rebased.
The full engine snapshot reports `startup_ready` and `startup_armed`
separately. Native acceptance waits for `startup_armed` and reports both
milestones, so Ready alone is not evidence that countdown publication
committed. Desktop projection keeps Preparing and pre-roll in Starting.

Scheduling and power-throttling guards are acquired on the worker only after
large allocations finish. Their RAII owners remain with the worker resources
and are dropped on the acquiring worker thread. MMCSS remains best-effort and
reports the actual acquired mode; production dispatch fails admission when a
high-resolution waitable timer is unavailable.

## Authored generations and plans

The compiler opens a generation on a musical Down and closes that same
generation on its matching authored Up. A Down left open at end of input is a
typed compile error. The independent validator repeats the end-of-walk check
against the compiled schedule. Overlapping same-key Down, same-key matched
Up+Down at one authored timestamp, and invalid hold or release spacing remain
rejected. Stale unmatched Up metadata is retained only as non-musical
metadata. Cleanup and safety releases are outside the ledger.

The worker plans one of:

- metadata-only work with no physical packet;
- one immutable physical packet and one authored target; or
- no work.

The physical packet contains canonical Up entries before Down entries. A mixed
packet remains one transaction and is never split. The authored target is
derived once from the playback epoch and the immutable schedule timestamp.

## Current Down admission

For a live current normal Down, the exact authored target, packet identity,
target HWND/generation, and no-previous-attempt state are carried into final
admission. If lifecycle, focus, control, suspend, supervisor/lease, target,
and preflight gates pass, the boundary makes exactly one transport attempt.
Strictly-future observation is not a prerequisite, and scheduler lateness
alone is not a drop criterion.

Pause, focus or epoch reset, target-generation changes, suspend, and consumed
or completed boundaries still invalidate stale prepared work. A late current
Down is not `UnobservedBacklog` merely because it was not observed strictly in
the future. Partial, ambiguous, or clock-uncertain transport is terminal and
is never retried.

## Physical floors and waiting

After a complete successful musical packet with trustworthy sender completion
QPC, the guard updates only the affected keys:

```text
musical_up_not_before[key] = Down completion + effective_min_hold
down_not_before[key]        = Up completion + frame_us
resume_down_not_before      = verified_cleanup_qpc + frame_us
packet_not_before           = max(authored_target, relevant per-key floors,
                                  resume barrier for Down-bearing packets)
```

`effective_min_hold = frame_base_hold + timing_margin` is materialized at boot
and passed into the guard; the guard does not recompute the sum. The guard is
floor-only. It has no latest start, sender cutoff, or lateness rejection. The
normal and strict/diagnostic completion paths share the same update helper.
Transport failures do not create completion evidence.

After verified resumable cleanup returns, the worker samples QPC and sets the
global absolute Down barrier to that sample plus one frame. It clears canceled
musical hold floors while retaining this barrier. Manual pause, focus restore,
deferred manual suspension, and system suspend use the same cleanup operation;
resume paths do not reset the barrier. It applies to every later Down-bearing
packet, including all Down keys in a mixed packet, and never delays Up-only
packets. A completed Down for one key does not clear it for other keys. Once
the barrier time has passed it adds no delay, and authored targets later than
the barrier are preserved. A new session constructs a clean guard. Failed or
inconclusive cleanup, a failed post-cleanup QPC read, or barrier overflow keeps
the guard invalid and Down dispatch fail-closed.

### Bounded resumable generation reconciliation

Only after physical release is verified and the resume Down barrier is
established does the coordinator cancel live musical owners and mint resumable
Up tokens. Each token is stored in the authoritative key slot and binds the
generation, slot, and immutable compiler-owned packet/intent location. The
mutable schedule invalidates that generation identity but retains the authored
physical Up bit. Both dynamic planning and the immutable normal stream freeze
the exact token identity into the matching Up commit. The worker validates it
before sending; a complete matching Up commit consumes it once. Wrong slot,
generation, packet, intent location, stale identity, or a second live owner
fails closed before a mixed packet can authorize its Down.

The compiler's paired, chronological, same-key-nonoverlap schedule allows at
most one unmatched cancelled Up per key: the next generation on that key
cannot become active before the prior authored Up boundary. The coordinator
checks this locally and refuses to overwrite a token. Pause/resume walks the
fixed fifteen slots and the current packet intents only. It performs no
historical generation-ID search, song-sized list allocation, or full-ledger
scan. `check_local_invariants` proves slot ownership, token identity, masks,
and incremental counter bounds in fixed work. `check_invariants` remains the
deliberate full-ledger conservation audit used at terminal publication and
explicit test checkpoints; terminal accounting was not weakened.

The cleanup Up and the later authored Up are separate `SendInput` events. The
sender cannot identify whether a virtual key-down came from Sky Auto Player or
the user, so it cannot safely suppress an authored Up when a user holds the
same instrument key. The resumed authored Up and mixed Up-before-Down packet
are retained; this can release that user's held virtual key. The regression
fixture proves only the emitted packet under a simulated physical-held probe,
not the game's response. No hook, alternate input transport, or ownership
claim is introduced.

For a physical plan the loop queries the guard before waiting. It waits to
`packet_not_before_qpc`; metadata-only work waits to its authored target. Causal
authorization still compares against the authored target. If a floor delays an
authorized packet, the packet is sent when the floor is reached. A mixed packet
waits to the maximum member floor and remains atomic. No authored timeline
rebase occurs.

Up-prefix recovery for an unobserved mixed Down is bounded and immutable. A
valid prepared Up prefix may be sent once; a trustworthy successful completion
updates the released keys' Down floors. The dropped Down remains unauthorized
and is never emitted by recovery. Safety and cleanup Ups bypass musical floors.

## Final physical path

The worker performs the following sequence:

1. Resolve and validate the immutable packet before the wait.
2. Wait to the authored target or queried physical floor.
3. Apply quit, skip, pause, suspend, lease, target, and focus gates.
4. For eligible `require_focus=true` Down traffic, make one fresh foreground
   proof and then recheck target, published focus, and late control.
5. After modifier observation and final target atomic revalidation, prove the
   enabled supervisor lease age with one fresh QPC sample before the final
   control check and sender handoff.
6. Record `final_policy_qpc`, enter the trusted sender, reset Win32 last error,
   take the sender-owned `pre_call_qpc`, and call `SendInput` exactly once.
7. Validate completion QPC, confirmed/skipped masks, and packet integrity.
8. Commit coordinator ownership and update floors only after complete success.
9. Record bounded production evidence or enqueue one diagnostic observation.

Up-only and `require_focus=false` paths do not perform the fresh foreground
query. Up-only traffic also takes no new lease-age proof. Partial or ambiguous
Down transport is never retried. A skipped key still owned by the coordinator
is a state disagreement and follows fail-closed cleanup.

## Timing materialization

The authored timing equations are:

```text
frame_us           = ceil(1_000_000 / game_fps)
frame_base_hold_us = ceil(hold_frames * frame_us)
timing_margin_us   = persisted user value
min_hold_us        = frame_base_hold_us + timing_margin_us
min_release_gap_us = frame_us + timing_margin_us
```

Timing Margin is materialized once. It contributes to authored `min_hold`,
authored `min_release_gap`, and the physical musical-Up floor through
`effective_min_hold`; it is not added a second time. It does not suppress a
late current Down, act as a lateness cutoff, become a latest-start deadline, or
change an authored timestamp. Completion evidence remains sender-side evidence
and does not claim game observation. `SendInput` success is sender/Windows
injection evidence only.

## Lifecycle and diagnostics

Focus loss immediately blocks new Downs and pauses musical time. Resumable
release and reconciliation may remain deferred until trustworthy target
restoration or existing terminal cleanup; a long unfocused period does not
prove that held keys were physically released. Pause, suspend, and
target-generation changes clear stale prepared work. Verified resumable cleanup
clears canceled per-key floor evidence and establishes the absolute Down
barrier above; failure paths invalidate the guard. A new session starts with a
fresh guard. Cleanup releases remain safety operations and bypass the musical
hold floor.

Musical generation accounting and physical cleanup obligations are separate.
Only a matching musical Up produces a normal generation `Released` transition;
safety/cleanup Ups may be idempotent and may make the raw physical Up count
larger than the musical Down count. The cleanup contract is:

```text
release_obligation_mask = active_mask | possibly_active_mask | failed_release_mask | in_flight_mask
```

Cleanup keeps transport and physical verification as separate evidence. A
physical `AllUp` observation means the foreground-scoped key-state snapshot
saw no requested instrument virtual-key down bits; it does not identify which
actor previously held those keys. This probe's documented zero/failure
ambiguity remains as described in the [physical-key study](physical-key-study-method.md).
A physical `AllUp` observation resolves the requested keys only when the Up
transport confirmed the complete requested mask. A `Held(mask)` observation
retains both physically held keys and requested keys whose Up transport was
incomplete or ambiguous. An `Inconclusive` observation retains the entire
requested mask even when `SendInput` reported every Up event inserted. The
`SendInput` return count is insertion evidence, not proof that the key is now
up. Each cleanup invocation retries only its remaining obligations, with a
four-send upper bound; an unverified invocation leaves those obligations for
resumable failure escalation or terminal cleanup.

Partial, ambiguous, or clock-uncertain Downs retain conservative obligations
and are not replayed.

### Terminal release ownership and ordering

Command control observes quit, skip, panic-release, and supervisor-lease
signals, latches the first terminal trigger, and exits the dispatch loop. It
does not release keys, walk or cancel the generation ledger, stop observers,
or export diagnostics. The worker finalizer is the sole terminal release owner
for an admitted session. Before any full-ledger audit, cancellation,
diagnostic aggregation, observer shutdown, or export, it invokes the tracked
release state machine on the obligation union above. The first physical Up
attempt therefore precedes those potentially expensive operations. The
release remains bounded by the tracked cleanup state machine's four send
rounds per invocation and uses `SendInput` only.

After that first terminal release call returns or panics, finalization records
the result and residual obligation mask, checks and cancels the coordinator,
checks post-cleanup ledger invariants, stops the observer, and exports
generation, terminal metrics, and telemetry. Trigger identity is preserved
separately from `terminal_error` and secondary cleanup failures. Quit and skip
retain their distinct outcomes. Panic requests, worker panic, lease expiry,
release uncertainty, and cleanup failures cannot produce `Finished`.

The frequent engine progress snapshot publishes a typed terminal outcome
separately from `has_terminal_error`, which reports diagnostic failure
provenance. `Error`, `Panicked`, `Poisoned`, and invalid terminal states map to
error playback and backend health even when tracked release is verified and
`terminal_error` is empty. `Finished`, `Quit`, and `Skipped` are not errors by
themselves. The full engine snapshot and native acceptance JSON retain the
trigger reason and cleanup details. Acceptance output reports
`release_attempts_total` across resumable and terminal cleanup; nested
`release_outcome.attempts` counts only the represented terminal invocation.

Admission failures after backend construction have one startup/admission
cleanup owner and use the same tracked release state machine. Schedule or QPC
failures before backend construction have no input backend or outstanding
physical obligations to release. If an admission cleanup panics or cannot
verify release, startup remains failed and the residual mask, cumulative
release attempt count, primary admission cause, and cleanup failure are
preserved.

### Desktop physical-session handoff

The desktop playback supervisor treats Stop, Skip, event-delivery failure, and
shutdown as requests to end a session. It does not publish a final outcome or
release the physical activity lease because one of those requests was made.
The engine's terminal snapshot supplies the outcome after the worker reports a
terminal status. The supervisor then retries bounded joins off the UI thread;
an incomplete or failed join keeps the active slot, activity lease, and power
authority in place while the session remains Stopping.

Worker quiescence and physical release verification are separate proofs. A
successful join permits retirement, but a physical session is reported
successful only when its terminal release is verified and its residual
obligation mask is zero. If the worker is joined but release remains
unverified, the desktop records a failed terminal result and quarantines
physical playback, calibration, and update admission for the rest of the
process. Restart is required to clear that conservative quarantine. Startup
failures before worker activation retire under an explicit no-worker-started
proof. A failed UI event delivery requests safe termination and does not
replace the engine outcome.

The updater's `activity.admission_closed` safety marker records that new
physical operations are blocked; it does not assert worker quiescence. The
`playback.keys_released` marker is written only when shutdown observes both
terminal retirement and verified physical release. A timed-out shutdown omits
that marker and retains the in-process reservation until process exit.

Resumable lifecycle release remains a separate operation because it must
verify cleanup before establishing the resume Down barrier. If it fails, the
terminal finalizer may perform its one terminal cleanup transaction. The
backend's cumulative release attempt count spans both operations; it is not a
second retry budget. Each state-machine invocation retains its own bounded
retry behavior, and failed or ambiguous obligations remain visible for later
cleanup attempts. A panic after sender return or state commit keeps the key in
the conservative in-flight or active obligation union until tracked cleanup
resolves it.

Production diagnostics use bounded worker-local scalars and fixed-size state.
Strict/diagnostic observations use a bounded queue and cannot authorize,
reorder, retry, or split input. Useful output includes authored target,
physical floor delay, sender pre-call and completion timing, transport
anomalies, final-gate rejections, late Down attempts, missing expected Down
count, generation accounting, and final release obligation state.

Version 19 traces pair absolute QPC timestamps with explicit availability and
provenance. Production-normal leaves the final-policy timestamp unavailable
instead of inserting a nearby sample or schedule value. Always-on timing
snapshot counts and maxima are worker-local and independent of trace mode.
See [real-time timing evidence](rt-dispatch-timing-evidence.md) for field
boundaries, lower-bound semantics, bounded drops, and the offline risk model.

These compatibility counters may remain in snapshots and UI DTOs:

- `final_sender_window_expirations`;
- `prepared_normal_sender_expirations`;
- `missed_physical_window_boundaries`;
- `release_floor_infeasible_boundaries`.

They are deprecated, remain zero in production, and do not control execution.
Native telemetry schema version 19 reports authored and physical-floor
differences, not absolute authored/floor QPC targets, and contains no
latest-start field. Legacy deadline-rejection outcomes are historical
terminology and are not active runtime outcomes.

## Security and verification

Gameplay input is restricted to Win32 `SendInput`. The repository does not
use hooks, process memory, code injection, alternate input APIs, adaptive
timeline rebasing, or transport retries.

Verification covers compiler and validator pairing, native transport ordering,
player final-gate behavior, completion floors, mixed-packet atomicity,
no-allocation dispatch, static security checks, and the Windows receive-only
acceptance matrix required by issue #412.
