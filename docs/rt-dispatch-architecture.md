# Real-Time Dispatch Architecture

This document is normative for the real-time playback path. The application
reads authored schedules, materializes paired musical generations, and emits
physical keyboard input through the Windows `SendInput` boundary. The current
resumable lifecycle, terminal system-suspend contract, and final lease
contract are recorded in
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
                                   \                          /
                                    +-- stop, authority loss,
                                        system suspend, lease
                                        expiry, or failure -> terminal
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
readers. The monotonic system-suspend bit shares that atomic word: if it is set
before this transition, Armed publication fails and startup terminates; if
Armed commits first, suspend closes final Down admission and enters terminal
release. Readers load the startup gate once before projecting epoch, elapsed
time, and countdown, so they cannot combine a staged epoch with a pre-Armed
status. Resume never clears this bit or republishes an epoch.

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

## Final Down authority policy

The prepared and strict/dynamic paths now return one `FinalDownVerdict` for the
shared safety decisions. Path adapters still own cursor, pause, terminal-reason
and recovery transitions. This keeps scheduling policy separate while making
the authority checks, lease result, metric bucket and invalidation behavior
auditable in one place.

Before this refactor, the prepared suffix translated `DownAdmission` into a
separate `PreparedNormalAdmission` enum and counted/invalidate each case in a
large match. The authored finalizer separately matched `DownAdmission`, ran a
second control helper and repeated its rejection metrics and target invalidation.
The modifier/lease ordering was shared only by a low-level helper; the two
callers still owned parallel verdict taxonomies and outcome mapping.

```text
Before
prepared: control → target/foreground → control → modifier → atomic target/owner
          → lease QPC → control → PreparedNormalAdmission mapping
authored: control → target/foreground → control → modifier/atomic/lease/control
          → DownAdmission mapping → final-policy QPC

After
prepared ─┐
          ├─ final_down_target_admission → final_down_modifier_verdict
authored ─┘      → final_down_post_modifier_verdict
                    atomic target/owner → one lease-age QPC proof → final control
          └─ FinalDownVerdict → apply_final_down_rejection
             → small path-specific Continue/pause/terminate adapter
```

For Down-bearing packets, the policy order is:

| Gate | Shared owner | Result and state effect |
|---|---|---|
| Pre-finalizer control checkpoints | Each dispatcher; the common final-control projection is `final_down_control_verdict` | Stop, Skip, Panic, Pause or suspend blocks a new Down. Existing path control checkpoints remain where they bracket preflight, wait and final admission. |
| Frozen target proof and fresh foreground-owner query | `final_down_target_admission` / `SessionTarget` | Requires the same HWND and generation; when focus is required, published focus, fresh foreground HWND and retained process identity must match. Focus loss enters the existing pause/replan path; a changed target invalidates stale authorization; owner identity failures terminate. |
| Final control checkpoint before modifier | Each dispatcher | Prevents modifier probing or sending after an already-observed terminal/pause request. This intentionally remains a path adapter checkpoint. |
| Modifier observation | `final_down_modifier_verdict` / tracked backend | Held modifiers reject the whole packet and do not create a new musical owner. |
| Post-modifier authority | `final_down_post_modifier_verdict` | Rechecks exact target generation, focus and retained owner identity after modifiers. |
| Lease age | `final_down_lease_age_proof` | When the supervisor lease is enabled, takes exactly one QPC sample at the existing post-modifier boundary. Expired or unproven lease invalidates Down authorization and terminates. QPC failure follows the existing terminal mapping. |
| Final control | `final_down_control_verdict` | Rechecks Stop, Skip, Panic, Pause and suspend after modifier and lease. A rejection prevents the sender call. |
| Send transaction | Path's existing sender | The authored immutable packet is submitted once. A mixed Up/Down packet remains atomic and Up precedes Down inside that single SendInput attempt. |

`FinalDownVerdict` distinguishes Allowed, ControlRejected, ModifierHeld,
TargetChanged, FocusLost, typed owner failures, LeaseExpired and LeaseUnproven.
`apply_final_down_rejection` owns the common metric bucket and shared target
invalidation rules. Caller adapters preserve path-specific error strings and
transitions: focus loss pauses/replans, target or control rejection returns to
the existing planner, while owner, lease, modifier and QPC failures retain
their established fail-closed terminal handling. No control rejection is
converted into musical generation credit.

The policy helpers are static, `#[inline(always)]` Rust functions. They add no
allocation, lock, virtual call, wait, or new QPC sample. The lease-age proof
retains its one existing QPC read. `final_down_target_admission`
does not create a proof that is carried across the wait: both paths validate
the frozen target before waiting and repeat the final HWND/generation,
foreground-owner, focus and control proofs after the wait. The prepared path
does not take a production `final_policy_qpc`; its physical-study sample is
test-support-only. The strict path retains its existing final-policy QPC after
the shared safety verdict and before the sender; the sender then takes its own
pre-call and completion samples. Neither sample moved across the final policy
boundary.

The two paths intentionally differ outside that shared safety judgment:

| Concern | Normal prepared stream | Strict/dynamic diagnostic path |
|---|---|---|
| Schedule and packet | Compiler-owned immutable packet identity, validated after Due; current valid late Down is attempted once. | Runtime planner and strict lateness/causal admission remain active. |
| Control checkpoints | For Down, early control, post-foreground control, then shared final control. Up-only has only the shared late control gate. | Preflight/wait checkpoints remain; finalizer keeps control before target, after target and shared final control. Up-only retains its surrounding path-specific controls. |
| Focus, owner and modifier gates | Down-bearing packets only. | Down-bearing packets only. |
| Lease-age sample | One enabled-lease sample for Down; none for Up-only. | One enabled-lease sample for Down; none for Up-only. |
| Final-policy QPC | No production sample after the verdict. | Existing sample after the shared verdict, before sender. |

Neither path checks foreground or retained owner for Up-only release packets;
Up-only traffic also skips modifier observation and lease-age sampling. Exact
target freshness still gates every Down. Pause, target change, interruption or
an unauthorized boundary discards stale prepared work and replans through the
existing path. Partial/zero/ambiguous SendInput progress or uncertain sender
timestamps terminate and trigger tracked cleanup; packet work is not replayed.

Execution-based parity coverage is in
`prepared_and_strict_paths_match_shared_final_down_fault_matrix`, with real
prepared and strict dispatch harnesses comparing attempts, typed outcome class,
metrics, cursor, active/release masks, lease samples, ownership accounting and
focus-pause state under post-modifier and post-lease injected races. Existing
tests separately cover path-specific timing, Up-only and mixed packets,
15-key chords, late normal Down, transport partial/zero and QPC failures,
resume barriers, and independent H/G completion-floor forensics. Source-order
checks remain supplementary; they are not the safety-equivalence evidence.

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
and deferred manual suspension use the same cleanup operation; their resume
paths do not reset the barrier. System suspend ends the physical session and
does not use the resumable cleanup path. Resume cannot reactivate that worker
or its QPC epoch. The barrier applies to every later Down-bearing packet,
including all Down keys in a mixed packet, and never delays Up-only packets. A
completed Down for one key does not clear it for other keys. Once the barrier
time has passed it adds no delay, and authored targets later than the barrier
are preserved. A new session constructs a clean guard. Failed or inconclusive
cleanup, a failed post-cleanup QPC read, or barrier overflow keeps the guard
invalid and Down dispatch fail-closed.

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

### Snapshot and observer ownership

Live metric snapshots use separate safe `RwLock` slots for dispatch-owned and
observer-owned values. Packet metric updates and observer updates each use
nonblocking `try_write`; if a reader currently holds a slot, that best-effort
publication is skipped and a later cumulative update can replace it. The
physical worker never waits for a UI snapshot reader in the packet dispatch
path. Focus/manual-pause lifecycle publication is different: after the
existing pause or tracked-cleanup boundary, the worker publishes the current
dispatch projection with a blocking write so the lifecycle and physical
ownership state cannot disappear when no later packet is dispatched. This
write stays outside the QPC-to-SendInput suffix. Snapshot readers hold both
read guards while copying and merge the two source snapshots once, so observer
fields cannot replace independent dispatch counters. The observer's publish
interval is independent of the worker's.

The observer is stopped and joined before terminal aggregation. The finalizer
merges its returned metrics into the dispatch-owned terminal values, then
publishes those values while clearing the observer slot in the same guarded
operation. Terminal publication may yield while readers release their guards;
it cannot be skipped or double-count observer data.

### Desktop safety supervisor and presentation ownership

The desktop owns one real `sky-native-playback-supervisor` monitor for each
active session. It observes Stop and terminal control flags, updates the
worker's focus hint from the current foreground window, polls the retained
window-owner identity every 100 ms, reads the small `poll_state` projection,
and renews the worker's supervisor lease on its normal 20 ms loop. The monitor
does not build UI snapshot or diagnostics DTOs and does not acquire the
diagnostics publication gate. It renews the lease before enqueueing UI work.
The three-second lease and the worker watchdog remain authoritative; a stalled
monitor still expires the lease.

Each session owns one `sky-native-playback-presentation` consumer. It receives
ordered state transitions and terminal finalization messages through a
nonblocking critical queue with capacity 64. A separate one-slot snapshot
handoff retains only the latest frame; replacing or failing to lock that slot
drops a best-effort update and increments its counters. The consumer builds
playback snapshot and diagnostics DTOs, calculates diagnostic statistics, and
inserts resulting events into the app-scoped event queue. Diagnostics enable,
disable, rate-limit, and re-enable sequence behavior remains linearized by the
diagnostics gate on this consumer.

The app-scoped event queue holds at most 128 buffered or in-flight events.
Pending snapshots are coalesced by session and a snapshot may be dropped when
the queue contains only lifecycle events. Neither case fails the session. A
full critical presentation queue, lifecycle queue overflow, publisher failure,
or presentation consumer failure sets the shared delivery-failure flag and
requests panic-release plus worker quit. The monitor observes that flag and
performs the normal worker join and tracked release handoff. A confirmed
lifecycle delivery failure is never treated as a snapshot drop.

The initial `Starting` state is inserted into the event queue before the
worker can arm, so a closed transport cannot start physical playback. Later
state transitions use the per-session critical queue. At termination, the
monitor joins the input worker and verifies release or records quarantine
before it enqueues final presentation. The presentation consumer appends the
forced final diagnostics sample, terminal state, and terminal event in order,
then clears active-session ownership. A successor cannot enqueue its
`Starting` event until that terminal event has entered the shared event queue.
The monitor does not wait for the presentation consumer or for `Channel::send`
to finish. A blocked publisher remains bounded by the existing app-level
publisher and close budget.

The ownership flow is:

```text
Before:
  safety monitor -> focus / owner / poll -> snapshot DTO + diagnostics + gate
                 -> event queue -> lease renewal
  app publisher -> Channel::send

After:
  safety monitor -> focus / owner / controls / poll -> lease renewal
                 -> bounded per-session presentation handoff
  presentation consumer -> snapshot + diagnostics -> bounded event queue
  app publisher -> Channel::send
  terminal monitor -> join + release verification/quarantine -> enqueue finalize
  presentation consumer -> final diagnostics + terminal state/event -> retire slot
```

#### Risk memo

The snapshot path contains no `UnsafeCell` or unsafe `Sync` implementation.
`parking_lot::RwLock` guards prevent concurrent reads and writes to each
`WorkerMetricsLocal`. Only the dispatch worker writes the dispatch slot and
only the observer thread writes the observer slot. Readers take both read
guards in dispatch-then-observer order, so they copy one coherent pair before
merging observer-owned fields. Packet and observer publishers use `try_write`;
contention drops that individual cumulative publication without delaying the
physical path. Focus/manual-pause lifecycle updates use a blocking dispatch
write only after pause or cleanup work has completed, because their resulting
physical state must be observable even if the worker dispatches no more
packets. Terminal cleanup runs after the observer has joined and may yield
while acquiring both write guards. The terminal marker makes later readers
return the already-merged dispatch slot without merging the cleared observer
slot again.

For an observed session, a real wait result is captured into worker-owned
pending state at wake time, before later control changes can replace its
deadline attribution. A due wait is inserted into the observer queue after
the current dispatch attempt returns, including attempts rejected by a final
gate or returning a terminal dispatch result. If the loop exits before that
handoff, its exit path inserts the pending observation. An interrupted replan
is inserted immediately because no physical dispatch follows that wait.
Metadata-only and physical packet waits use the same capture/handoff path. An
already-due target, a failed wait, or an observer-disabled session produces no
wait sample. The fixed queue uses drop-new behavior; each failed enqueue
increments the dropped-sample counter. Its high-watermark is the largest
occupancy observed after a successful enqueue, so a concurrent drain can make
it a conservative sample of the actual peak.

The existing final-gate DTO counters keep their wire names and group related
causes: control and modifier rejections share `final_gate_control_rejections`,
target-generation and bound-owner rejections share
`final_gate_target_changes`, and expired or unproven lease decisions share
`final_gate_lease_expirations`. Focus rejections remain separate. Rejected
Down packets stay fail-closed and do not reach `SendInput`.

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

### Desktop supervisor and UI event delivery

The desktop playback monitor remains the sole issuer of supervisor progress.
It samples the target focus hint, checks retained window-owner authority on
the existing 100 ms cadence, polls worker lifecycle state (including terminal
control and system suspend), and stops progressing once a terminal state or
delivery failure is observed. Each progress publication still takes a fresh
QPC sample and applies the unchanged three-second lease rule; there is no
independent heartbeat thread.

The monitor enqueues UI events and never calls `Channel::send` itself. One
application-scoped event publisher drains a FIFO queue with a 128-event
capacity, including its in-flight send. Playback and diagnostics snapshots
and calibration progress are keyed best-effort values: a pending value with
the same key is replaced, older snapshots may be evicted to make room for
lifecycle events, and a new snapshot may be dropped when only lifecycle
events occupy the queue. Accepted lifecycle and terminal events retain FIFO
order. If a lifecycle event cannot be admitted, the hub latches the failure
and rejects further publication. An actual channel-send error or panic closes
the hub, latches the failure, and requests `panic_release` and `quit` through
the active player. The monitor observes that latch and stops lease progress;
the worker still owns tracked release and the supervisor still owns join and
session retirement.

The monitor copies worker metrics into each event DTO before queueing it. No
snapshot read guard or dispatch lock is held during `Channel::send`, so the
blocking focus/manual-pause metrics write remains independent of subscriber
time.

The per-session snapshot handoff and app-scoped event queue count coalesced
and dropped best-effort snapshots separately from critical lifecycle failure.
At session retirement, the presentation worker reports its counters and the
current app-queue counters in a one-line `playback_presentation_snapshots`
diagnostic. App-queue totals are reported again as
`native_event_snapshots` when the event hub closes. Operators can capture
these lines by launching the desktop process with stderr attached; this does
not add fields to the `UiEvent` wire contract or run on the safety monitor or
physical sender. A coalesced or dropped snapshot does not imply lifecycle
delivery failure. Confirmed failure of the shared lifecycle channel remains
fail-closed.

Shutdown requests worker cleanup before closing the event queue. Closing
rejects new events and discards queued events; an already in-flight channel
call may finish, but no later queued event is published after close. Shutdown
waits at most 50 ms for the event publisher. A blocked channel call cannot hold
the playback activity lease, physical cleanup, monitor join, or retirement.
If an underlying channel call does not return within that bound, only the
single application-scoped publisher handle is detached; it does not own the
session, activity reservation, or any physical input path.

### Desktop physical-session handoff

The desktop playback supervisor treats Stop, Skip, event-delivery failure, and
shutdown as requests to end a session. It does not publish a final outcome or
release the physical activity lease because one of those requests was made.
The engine's terminal snapshot supplies the outcome after the worker reports a
terminal status. The supervisor then retries bounded joins off the UI thread;
an incomplete or failed join keeps the active slot, activity lease, and power
authority in place while the session remains Stopping.

The desktop publishes Starting and parks its monitor before it calls
`NativeDispatchSession::arm`, so Stop, Skip, or shutdown can arrive while the
native lifecycle is still New. Those terminal requests are accepted in New and
latch cancellation in the native startup publication state. That state uses
one atomic transition for pre-arm cancellation and another for Armed
publication: if cancellation wins, Armed cannot publish and the worker cannot
admit a musical Down; if Armed wins, the request is ordered after activation
and follows normal worker finalization. A pre-arm Quit or Skip that joined
without Armed publication, a release result, or any residual obligation has no
physical release to reconcile.

Worker quiescence and physical release verification are separate proofs. A
successful join permits retirement, but a physical session is reported
successful only when its terminal release is verified and its residual
obligation mask is zero. If the worker is joined but release remains
unverified, the desktop records a failed terminal result and quarantines
physical playback, calibration, and update admission for the rest of the
process. Restart is required to clear that conservative quarantine. Startup
failures before worker activation retire under an explicit no-worker-started
proof; a worker launched for a pre-arm cancellation is instead joined with no
release obligation. A failed UI event delivery requests safe termination and
does not replace the engine outcome.

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

These public compatibility fields remain in Rust snapshots and the UI DTO with
unchanged names, types, ordering, and numeric zero values:

- `missed_physical_window_boundaries`;
- `final_sender_window_expirations`;
- `release_floor_infeasible_boundaries`.

Their deprecated `WorkerMetricsLocal` storage had no production writers and has
been removed. `prepared_normal_sender_expirations` was worker-local only and is
also removed; it was not a Rust snapshot or UI DTO field. These counters do not
control execution. The compatibility projections remain numeric zero, while
active floor-delay, final-gate, lease, H/G, and release evidence stays available.
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
