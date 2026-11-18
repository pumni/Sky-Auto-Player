# Playback Engine Architecture

The playback engine is the realtime subsystem of Sky Auto Player responsible
for executing music sheets as timed Windows keyboard events. It is implemented
in Rust (`sky_player`, `sky_dispatch_core`, `sky_dispatch_win32`) and runs on
a dedicated worker thread with MMCSS scheduling.

The SendInput boundary, single-owner dispatch, Up-before-Down ordering, H/G
floors, final focus/admission checks, and verified cleanup are required behavior.
The executor, waiter, observation schema, and implementation map below describe
the current Rust design and may evolve with source and test coverage while those
contracts remain intact. Sender-side timing floors do not promise which frame
the game samples.

## Core Boundaries and Invariants

- **SendInput only:** All physical gameplay input uses the public Win32
  `SendInput` API. The engine never accesses game memory, injects DLLs,
  installs hooks, or bypasses anti-cheat boundaries. See [SECURITY.md](../SECURITY.md).
- **Single 15-slot executor:** Musical playback is driven by a single
  15-slot executor ([`DispatchExecutor`](../rust/crates/sky_dispatch_core/src/executor.rs)).
  There are no independent dual cursors for Down and Up events.
- **Musical timing owner:** `DispatchExecutor` owns all 15 musical slots and
  production Up/Down eligibility. The worker stages candidates from that
  executor and commits only confirmed packet completions back to the same owner.
- **Pairing at compilation:** Each musical Down chord is compiled with
  paired authored Up offsets for its constituent keys. Up obligations are
  instantiated strictly upon successful Down completion.
- **Fail-closed admission:** Physical Down events require verified foreground
  window focus, target process identity, modifier clearance, and active supervisor
  leases at the final boundary before transmission.
- **Frozen production waiter:** Production uses a fixed, robust wake guard
  derived from host calibration, combining high-resolution waitable timers
  with a microsecond-accurate QPC spin loop.

## Prepared Native Runtime

Desktop `Prepare` compiles the authored schedule once and shares that
`RuntimeSchedule` with the offline timing advisory. For a physical variant, it
then creates an immutable `PreparedDispatchSchedule` in the current QPC
frequency domain and validates its timestamps against the selected H/G policy.
The plan retains compact Down chords and intents, the key registry, schedule
metadata, and the unmatched-Up diagnostics needed by the worker. Source notes
and the full `RuntimeSchedule` are released before the plan is published.
Each variant's advisory applies its action, work, and memory bounds to that
variant's authored action count. If an optional physical `UseRecommended`
variant cannot be materialized within the retained-capacity budget, Prepare
omits that decision, DTO variant, fingerprint, and native plan together. A
runnable `Proceed` and any valid offered `DryRun` remain available. Cancellation
or supersession during recommendation materialization cancels the entire
Prepare rather than treating the recommendation as unavailable.

`Start` rechecks the prepared ID and settings identity, selects the requested
variant, and moves its plan into native session creation. It does not compile,
walk, or materialize the authored schedule. Native session admission checks the
plan against the current QPC frequency and H/G policy; worker boot repeats that
compatibility gate before moving the prepared arrays into a new
`DispatchExecutor`. Worker boot also checks the retained-capacity budget before
dispatch. Each playback creates fresh cursor, slot, epoch, barrier, and
transport state, so replay begins with empty session state while reusing only
immutable schedule data. Dry-run variants do not require a physical QPC plan;
their advisory remains offline and cannot block a valid dry-run.

`NativeSessionOptions` accepts only `PreparedDispatchSchedule`. The former
`NativeScheduleInput::Raw` worker route has been removed. Rust callers that
compile or receive a `RuntimeSchedule` prepare it before constructing native
session options, using the same QPC frequency, H/G values, retained-capacity
accounting, and overflow checks. They may use the cancellable preparation
variant when the caller can be interrupted. The worker never materializes a
`RuntimeSchedule`. The timing advisory
continues to analyze the shared compiled `RuntimeSchedule` before native
preparation; its offline `DispatchExecutor::new_with_cancellation` call remains
separate from the production worker route and keeps its independent oracle.

Schedule compilation, timing validation, QPC conversion, and arena creation
run during cancellable Prepare. Worker startup only checks compatibility and
budget, moves the already allocated arrays, and initializes fixed-size state.
Startup still emits existing per-batch telemetry rows for unmatched Up
metadata to preserve schema 25; that diagnostics-only loop is proportional to
the number of unmatched-Up batches, which is zero for ordinary valid songs.
The compatibility `elapsed_us` snapshot field is derived from the shared
session clock projection; the unused worker-local duplicate has been removed.
Schema 25 keys, values, QPC provenance, and missing-sample semantics are
unchanged. Earlier revision-bound implementation and measurement records are
indexed in [HISTORY.md](HISTORY.md); they are not qualification evidence for
the current source revision.

## 15-Slot Dispatch Executor

The executor manages up to 15 instrument key slots corresponding to the
Sky instrument layout:

```text
Compiled Down Chord Stream
        |
        v
+-------------------------------------------------------+
|                 DispatchExecutor                      |
|                                                       |
|  - Current Down chord pending dispatch                |
|  - 15 Active Slots (release_due, next_down_due)       |
|  - Session Resume Barrier (cleanup completion + G)     |
+-------------------------------------------------------+
        |
        v
Disjoint Batch Selection: [Up-ready keys] + [Down-ready keys]
        |
        v
Win32 SendInput (single syscall, Up-before-Down within batch)
```

### Obligation Lifecycle

1. **Down Completion:** When a Down chord completes transport successfully:
   ```text
   authored_up[key]    = epoch + paired_up_offset[key]
   release_due[key]    = max(authored_up[key], down_completion + H)
   next_down_due[key]  = up_completion + G
   ```
2. **Release Execution:** When the current QPC ticks reach or exceed `release_due[key]`,
   the key slot becomes Up-ready.
3. **Down Admission:** A pending Down chord can only proceed when all its member keys
   have completed their previous Up releases and satisfied `now >= next_down_due[key]`.
4. **Independent Up selection:** Each slot's Up is considered independently. A
   pending Down chord retains authored cursor order; an Up for one slot does not
   block a different chord unless their physical masks conflict. Mixed packet Up
   and Down masks are disjoint.
5. **Coalesced Transport:** Eligible disjoint Up keys and a ready Down chord may
   share one packet with Up events first. A packet contains at most 15 `INPUT` records.

## Timing Floors: H and G

The engine enforces physical and musical timing floors derived from the target
game frame rate and user-configured timing margin:

| Parameter | Definition | Formula |
| --- | --- | --- |
| Frame Interval ($T$) | Single frame duration | $\lceil 1{,}000{,}000 / \text{FPS} \rceil\ \mu\text{s}$ |
| Hold Floor ($H$) | Minimum key-down duration | $\lceil \text{hold\_frames} \times T \rceil + \text{margin}\ \mu\text{s}$ |
| Release Gap ($G$) | Authored and physical same-key retrigger floor | $T + \text{margin}\ \mu\text{s}$ |
| Authored Gap | Minimum authored spacing between repeated notes | $G\ \mu\text{s}$ |
| Structural Cycle Floor | Minimum same-key cycle | $H + G$ |

Here $T = \lceil 1{,}000{,}000 / \text{FPS} \rceil$ microseconds,
$H = \lceil \text{hold\_frames} \times T \rceil + \text{margin}$, and
$G = T + \text{margin}$.

- **Hold floor ($H$):** Sender policy: Up is not eligible before the later of
  authored Up time and successful Down completion plus $H$.
- **Release floor ($G$):** Authored repeat spacing and sender policy use the same
  value. A same-key Down is not eligible before successful Up completion plus
  $G$. These sender-side floors do not guarantee which frame the game samples;
  Windows and game sampling are independent.
- **Timing margin:** Extends both $H$ and $G$ at the sender boundary. Sender
  setup, per-input cost, and post-send cost remain separate advisory estimates.
  For example, at 60 FPS with 1.0 hold frame and a 500 µs margin, $T$ is
  16,667 µs and both $H$ and $G$ are 17,167 µs. At 120 FPS they are each
  8,834 µs. The game does not acknowledge `SendInput`; sender completion alone
  cannot attribute a missing game note. Without an operator-controlled real
  sheet and mapped note position, gameplay evidence remains inconclusive.

## Final Musical Packet Admission and Safety Boundary

Every musical packet, including Up-only packets, must pass a fresh final
admission against its staged HWND/generation and retained owner identity before
calling `SendInput`. Down-bearing packets also check modifiers:

```text
Worker Loop -> Wait Deadline Reached
    -> Early Control / Power Check
    -> Check Immutable Target and Focus Hint
    -> For Down: Check Modifier Keys (Ctrl, Alt, Shift, Win = unpressed)
    -> Verify Supervisor Lease with Required QPC Sample
    -> Zero-time Wait and Check Retained Process Creation Identity
    -> Sample Fresh Foreground HWND and its Owner PID
    -> Recheck Target / Focus Latches
    -> Read Controls and Power Once in Fixed Precedence
    -> Pass: Dispatch SendInput
    -> Reject: Defer, Replan, or Terminate
```

The worker stages a bounded packet before its deadline wait. Its final gate
uses one process-liveness/creation proof and one fresh foreground HWND/PID
query. Focus or modifier rejection, and a target already invalid at the initial
gate check, make the worker replan without attempting or advancing that authored
Down. Target invalidation observed during the final identity proof terminates
the session before the sender. Terminal Stop, Skip, Quit, Panic,
lease expiry, ownership failure, and system suspend remain fail-closed. When a
mixed packet's Down gate is temporarily rejected while its independent Up is
still eligible, the worker stages and admits that Up-only packet through the
same fresh musical final gate; the rejected Down remains pending.

When final Down admission rejects a held modifier, the worker uses a bounded
interruptible backoff. Its wake deadline is the earlier of the next modifier
poll and the earliest active-slot Up deadline. A due Up returns directly to the
normal scheduler for fresh selection and final admission; no zero-duration
wait is armed. The active-slot deadline includes the authored Up and H floor,
and the selector continues to enforce the completion-based G floor before a
same-key Down. The deadline wait retains its typed wake and timer evidence and
attributes the wake to the next packet only when that packet contains an Up at
the same release deadline. Maintenance polls stay out of the musical wait
observer stream; their interruptions and backend failures remain counted.

The fresh foreground-owner query is authoritative even when the published
focus hint still says focused. A `NotForeground` result enters the existing
Focus pause transition, clears the verified-target stamp and restoration
anchor, and publishes paused progress before returning `Replan`. This applies
to Down-bearing and musical Up-only packets. Owner mismatch, unavailable
queries, identity drift, lease rejection, target drift, and modifier rejection
keep their separate fail-closed outcomes and do not become Focus pauses.

After a staged candidate's deadline wait returns `Due`, the worker preserves
and sends that candidate after final admission, even if the wake is late enough
for another packet to become eligible. The next outer scheduler evaluation
then selects newly eligible work and applies the normal Up-before-Down
coalescing rule. `Interrupted` discards the staged candidate and returns to
the worker loop for control/focus handling and fresh selection. A
rejected final admission returns through the ordinary control-aware replan
path; it does not wait for or retry a target writer/owner permit.

The verified-target cache records only a successful all-instrument-keys-up
preflight for the session's immutable HWND/generation. It is not a cached
foreground or process-owner proof. Every packet still repeats fresh
foreground, owner PID, process-identity, lease, power, and control admission.
Focus recovery continues through its normal cleanup and preflight path.
Invalidating the session target is terminal and cannot create or transfer a
preflight stamp.

The focused production target is immutable. A target-stamp failure while
staging or before a packet wait therefore fails closed; the worker does not
wait for target publication to recover. The separate bounded resume wait is
retained for live foreground/focus recovery with the same still-valid target.

Musical packets and safety Ups use one raw `SendInput` primitive, with each
invocation making at most one syscall. The worker materializes a musical packet
before its precision wait and journals its Down/Up duties before entering the
sender. If the sender unwinds, that in-flight journal remains available to
terminal cleanup. Each authored Down onset crosses the syscall boundary at
most once and is never replayed. The first anomaly—zero or short insertion,
Win32 error, contradictory count/evidence, or a pre/post-call QPC failure—ends
musical playback. The worker preserves the raw count, error, attempt, and timing
evidence, does not commit an anomalous packet as a musical completion, and
never invents a sender completion QPC. The one narrow journal exception is a
provable one-attempt Down-only raw zero: after matching the attempted onset to
its scheduler chord and validating consistent count and QPC evidence, the
worker clears only that new Down obligation. Prior active or uncertain duties
remain intact, no safety Up is sent for the proven-zero Down, and no H/G
completion floor is created. Every other attempted anomaly keeps its uncertain
transport duties in the journal and enters ownership-verified cleanup; no later
musical onset is dispatched. Cleanup may resolve the physical residual but
does not convert the anomalous packet into success. A session with an attempted
anomaly ends with a typed error result that retains its cause and remaining
release evidence.

Attempted-anomaly diagnostics have bounded storage: the worker counts every
attempt, counts omitted detail records, and retains at most 16 records (the
first eight and rolling last eight). Each retained record is capped at 512
bytes and preserves the available source action, raw insertion count, Win32
error, attempt count, and QPC fields. The bounded sample is stored inline and
does not add heap allocation to the healthy dispatch path. The terminal error
latch depends on the total counter, not on which detail records remain. This is
internal diagnostic evidence; telemetry schema 25 and public transport DTOs
do not change.

Every musical Up bit, including an Up-only packet and a mixed-to-Up-only
replan, must match the immutable owner proof retained once by its worker
session. The final gate proves the session target and checks its retained owner
identity immediately before the syscall. Missing, invalidated, or drifted proof
fails closed without calling the sender or clearing the obligation. Terminal
and pause safety Up use the same tracked cleanup reducer and originating owner
proof. The reducer separates `needs_up` (transport Up still owed) from
`needs_probe` (Up transport completed, physical state still unverified). Only
`needs_up` can produce another physical Up; Held or Inconclusive probe results
retain `needs_probe` without sending another Up. Startup abort, Pause, Stop,
focus return, Suspend, sender anomaly, and panic all use this bounded cleanup
policy. A drain makes at most four raw Up calls, with 15/50/100 ms delays
between retries, and revalidates the origin after each delay and at the final
send boundary. A background Stop may make at most one best-effort Up if the
origin process is still live; that send is not proof that the game released the
key. QPC errors still enter finite terminal cleanup. Healthy dispatch uses
fixed buffers and does not allocate. Packets contain at most 15 inputs, with
Up before Down. H/G decisions use real sender completion QPC on healthy packets
or verified cleanup completion QPC after recovery; missing evidence is never
replaced by a later clock read. Terminal safety Up remains separate from
musical packet metrics.

Tracked terminal and resumable cleanup repeat the immutable target, origin,
owner, and process-continuity admission after each retry delay and immediately
before entering the physical sender. If admission is missing or fails, the
remaining Up sends are withheld, their obligations stay quarantined, and the
result retains only the actual prior sender calls and their transport, probe,
and QPC evidence. A withheld retry does not increase the sender-attempt count.
Zero-obligation cleanup performs no proof query or syscall. Terminal QPC
fallback does not bypass per-attempt admission. The typed terminal result
retains the remaining masks, session owner proof, and physical key profile for
the later recovery layer; playback UI retirement remains owned by its current
lifecycle owner.

Latency, envelope, and hold timing samples are qualified only when transport is
complete and sender QPC evidence is ordered with no timing error. Unqualified
syscalls still contribute their raw attempt, missing-QPC, and transport-error
counters without entering those timing distributions.

Unmatched-Up diagnostic metadata is captured during Prepare and its existing
telemetry rows are published during worker boot before Arm. Test-only metadata
wake instrumentation is not a production path. Metadata has no physical
dispatch boundary and does not issue `SendInput`.

### Focus and Process Identity
- The target window must be the exact foreground window (`GetForegroundWindow() == target_hwnd`).
- The process owning the foreground window must match the retained target PID and
  image verified at startup.
- Focus loss pauses musical dispatch without proactively sending musical Up or
  starting cleanup while the target is in the background. On return, the exact
  target must remain stable through the configured grace period. The worker then
  reconciles and verifies cleanup, and requires a Down barrier at verified
  cleanup completion plus $G$.
- Stop, Skip, Panic, and Suspend use terminal safety cleanup. A background Stop
  may send at most one Up to a still-live origin; because the target is not
  foreground, this best-effort send cannot prove that the game released the key.

### Session Target Binding and Lifetime

`SessionTarget` receives one HWND/generation and one retained owner identity in
the production session constructor, before Arm or worker creation. The retained
process handle requests only
`PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE`; a zero-time wait proves
liveness while the startup creation time and image path bind the PID
incarnation. Production has no target-rebind API. Target or owner failure is a
one-way session latch, so numeric HWND/PID reuse cannot publish a replacement.
Public DTOs and telemetry schema do not change.

For production sessions with `require_focus`, `Arm` revalidates that the
nonzero HWND/generation, bound PID, and retained process authority all match
and that the authority reports continuous identity before it launches a
worker. Missing, mismatched, failed, or late proof irreversibly closes
admission; proof arriving afterward cannot start or rebind that session.
Focus-free sessions and the explicit mock test path keep their existing
admission behavior. Desktop and native acceptance pass the verified authority
to the session constructor before calling `Arm`; target invalidation exists
only as a test/acceptance seam.

The supervisor polls the retained process object without a writer or owner
mutex. Continuous identity leaves the binding usable. Any continuity failure
sets a one-way terminal failure latch, cancels pre-Arm startup when applicable,
and wakes the worker. Target replacement and owner failure are terminal worker
conditions: the worker checks the latch before staging and before selecting its
next note, and an interruptible deadline wait returns promptly so the worker
can finish without a supervisor `quit` or `panic_release` call. Later polls
cannot clear the failure or bind a new owner. The desktop monitor may also
request panic-release after `CurrentInvalid`; that request does not provide the
worker's terminal-completion guarantee. Every packet still performs its fresh
foreground query and process-identity check. A focus-only loss with the same
live HWND/PID/process authority remains a recoverable Focus pause.
The process-authority `Arc` is not cloned for each packet. Per-key release
proofs share a small identity object that owns it. The authority remains alive
through worker/watchdog cleanup and drops with the session target after the
session and its worker references are retired.

For each focus-required musical packet, the worker checks modifiers for Down,
proves the supervisor lease, checks the retained process handle with a
zero-time wait and creation-time query, then samples the current foreground
HWND and that exact window's owner PID once. After the fresh Win32 reads, it
checks the target failure latch and latest focus hint, then reads controls and
power once in fixed precedence immediately before the sender. No foreground
callback or cached PID substitutes for that fresh query. The process check is
constant-time and allocation-free. The supervisor continues to check
`IsWindow`, owner PID, process creation time, and image path on its control
cadence.

Each tracked key slot freezes a release-owner proof before a Down can create an
in-flight obligation. The proof includes the originating HWND and generation,
PID when required, and retained process-authority object identity. A changed
target cannot transfer A's release obligation to another window. Terminal and
resumable safety cleanup read the proof from the tracked backend and recheck
the exact origin HWND/process continuity without requiring foreground focus.
Because a session target cannot be replaced, there is no publication/binding/
close permit to contend for during the safety syscall. If validation observes
drift or cannot prove the origin, cleanup sends no Up, preserves the obligation,
and finalizes with an error. Zero-obligation cleanup needs no owner proof or
syscall. The existing release and quarantine policy remains in force.

The target binding is stored under the same shared session state used by worker
and watchdog, so an owner object unwind does not close the process handle while
either thread can still perform terminal cleanup. The watchdog lease and
desktop supervisor remain active; presentation events and progress snapshots
are not worker authority. Presentation delivery failure still requests
terminal cleanup, while a blocked presentation consumer cannot hold the
supervisor's command, lease, or worker-join path.

### Resume Barrier (cleanup completion + $G$)
After verified pause or focus-restoration cleanup completes at QPC $Q$:
- All active key obligations are cleared or verified released.
- A mandatory delay barrier of $Q + G$ is imposed before any subsequent Down chord
  may be transmitted. This sender-side barrier does not control game sampling.

## Production Waiter: Frozen Guard Mode

The production wait strategy is the **Frozen waiter** implemented in
[`sky_dispatch_win32::wait::HybridWaiter`](../rust/crates/sky_dispatch_win32/src/wait/hybrid.rs).

1. **Pre-Sleep Wake Guard:** The worker arms a high-resolution Windows waitable timer
   to wake slightly before the exact target QPC deadline:
   $$\text{timer\_target} = \text{deadline\_qpc} - \text{wake\_guard\_ticks}$$
2. **Robust Wake Guard:** The wake guard duration is derived during host calibration
   as $\text{median} + 6 \times \text{MAD}$ of measured timer return errors. It remains
   **frozen** throughout playback to avoid oscillation or feedback loops during transient
   system load spikes.
3. **QPC Spin Loop:** Upon kernel timer wake, the thread enters a tight QPC busy-spin
   loop until `qpc_now >= deadline_qpc`, achieving microsecond-accurate physical dispatch.
4. **Strict Conversions:** All time and QPC tick conversions fail closed. Arithmetic
   overflows or invalid timer returns immediately abort rather than masking errors.

## Observation and memory boundaries

Valid waits are captured once and published after a successful physical packet or
before a replan. Packet traces carry the associated wait sequence and spin ticks;
missing observers and bounded-queue drops are counted separately. A missing QPC
sample remains unavailable rather than being reconstructed from another timestamp.

Schema 25 keeps one packet trace per physical send. Per-slot Up evidence is stored
in a separate bounded sidecar, linked by `trace_record_index`; it retains each
source action, slot, scan code, authored offset, release floor and residual. Its
4,096-entry capacity has separate loss counts for observer-queue, observer-abort,
pool-storage, telemetry-ring and sidecar-capacity loss. Capture-off records are
counted separately as not captured by mode. Final accounting checks
`expected = captured + dropped + not captured`; overflow, mismatch or incomplete
observer accounting is exported as invalid. Residual is measured from the later
of the authored target, slot release floor and packet not-before floor to the
sampled pre-call QPC; it is unavailable when that sample is missing. Packet-level
summaries do not provide per-key p95/p99.

Worker staging always keeps a scalar Up summary with the active-slot count,
first source action, authored target, release deadline, and hold-floor mask. It
materializes the fixed-capacity per-slot records only when the observer queue
is enabled. Observer-off mixed-packet replans use the same summary to retain the
Up-only timing window; observer-on traces retain each slot's source and timing
fields. Expected Up-slot accounting still partitions into captured, dropped,
and not captured by mode.

Live dispatch snapshots remain on the existing 50 ms cadence. Backend counters
are projected only when that cadence is due and the snapshot writer slot is
available. Focus/manual-pause lifecycle snapshots and terminal/finalization
snapshots still refresh backend counters at their existing publication points.

### M4 diagnostics compatibility and offline guard comparison

M4 makes the existing evidence contract easier to audit without changing its
projection. Telemetry schema 25, the `PlaybackStatusDto`, progress snapshots,
fingerprints, and serialized field meanings remain unchanged. A pinned fixture
checks the serialized key sets for `NativeTelemetryOutput`,
`NativeTelemetrySummary`, `TimingSemantics`, and `RtTraceRecord`. The regression
also covers attempted Up-only dispatch with a missing completion QPC: the raw
attempt/status/count and source identity remain visible, the completion field
retains the schema's unavailable encoding and provenance, and no sender-envelope
sample is added. A reversed QPC interval is rejected rather than clamped or
reconstructed. The current wire encoding uses a numeric zero sentinel for some
unavailable timestamps; it is meaningful only together with the corresponding
`available = false` and `provenance = unavailable` fields, and must not be read
as a measured QPC value.

An attempted post-call anomaly keeps its terminal-error latch even when
origin-owned cleanup is verified and later unattempted events continue. The
bounded session history separately preserves total/omitted counts and first/last
evidence. Finalization projects the anomaly summary, primary/secondary errors,
terminal trigger, release outcome, and remaining obligation through the
existing snapshot and terminal DTO fields. Missing timing does not qualify a
sender-duration sample, and a verified cleanup does not convert the anomalous
packet to success.

The opt-in Windows A/B test compares an experimental fixed 1,000 µs guard with
the current startup-calibrated selector in Quiet and a declared Contention
condition. It uses the production waiter and selector on one test binary, with
the same six-sample startup probe, 60 FPS deadline cadence, warm-ups, and
alternating AB/BA pairs. It stops at waiter return: it does not measure the
worker's final gate, SendInput entry/completion, H/G eligibility, note delivery,
cleanup, Prepare/Arm, or gameplay. The raw return-to-observation QPC gap is not
a sender pre-call measurement. These results are an offline waiter experiment,
not gameplay or native dispatch qualification, and they do not change the
production guard.

The healthy real-time dispatch path is checked for zero heap allocations by
[`rt_dispatch_no_alloc.rs`](../rust/crates/sky_player/tests/rt_dispatch_no_alloc.rs).
The Prepare UI also shows an offline, read-only estimate from the authored song,
timing policy, compiled schedule, and bounded simulator. It cannot block admission,
change note timing, or control realtime dispatch. Computation is capped at 50,000
actions and 64 MiB; exceeding either bound returns `unavailable`. Its illustrative
sender model assumes 10 µs setup, 3 µs per input, and 5 µs post-send; these values
are diagnostic assumptions, not measurements or promises. The game does not
acknowledge the estimate. The desktop consumer is
[`TimingAdvisory.tsx`](../apps/desktop/src/components/player/TimingAdvisory.tsx);
the simulator is in `apps/desktop/src-tauri/src/native_runtime/playback/timing_advisory.rs`.

The bounded observation producer does not sample queue occupancy; the observer
consumer owns queue high-watermark sampling and per-slot sidecar accounting.
The producer keeps only bounded scalar expected/loss totals.

The live `DispatchExecutor` stores compact chord headers and their intents in a
contiguous arena. `MAX_EXECUTOR_BYTES` bounds the calculated capacities of
those two executor vectors. Production reads a borrowed `DownChordView` and
retains only `DownChordMetadata` while staging. `DownChordTicks` is a
self-contained fixture type with an owned `SmallVec`; it has no production
caller. Schedule compilation has its own phase-capacity checks. There is one
scheduler implementation in the crate: `DispatchExecutor`, including when
`test-support` is enabled.

`MAX_RETAINED_DISPATCH_BYTES` is the 512 MiB calculated-capacity limit used by
the compiler's phase checks, prepared-plan materialization, and the desktop
million-action construction test. That test,
`valid_one_million_single_key_actions_fit_prepared_runtime_budget`, builds and
compiles one million single-key actions, creates the QPC-domain prepared plan,
and moves it into a fresh production executor. Its report includes the source,
compiled-schedule, prepared-plan, and overlapping peak capacities. It does not
run a million-action playback or measure process RSS, Windows memory peak, or
real-time dispatch. The historical
[FINAL-RT-R5 record](https://github.com/pumni/Sky-Auto-Player/blob/fcf8fcf87bd3215c110039185ece535516191aaf/evidence/dispatch-core-646/final-rt-r5.md)
describes a coordinator fixture at its pinned revision only; it is not evidence
for the current executor. Allocation failures are not universally recoverable:
in particular, constructing shared reason strings still uses infallible
allocation.

## Implementation Map

- [`rust/crates/sky_dispatch_core/src/executor.rs`](../rust/crates/sky_dispatch_core/src/executor.rs):
  15-slot dispatch executor, chord stream compilation, and slot state management.
- [`rust/crates/sky_player/src/engine/worker/dispatch_loop.rs`](../rust/crates/sky_player/src/engine/worker/dispatch_loop.rs):
  Realtime worker loop, event interleaving, and admission control.
- [`rust/crates/sky_player/src/engine/worker/dispatch_loop_scheduler.rs`](../rust/crates/sky_player/src/engine/worker/dispatch_loop_scheduler.rs):
  Production executor evaluation, candidate staging, and completion-bound timing floors.
- [`rust/crates/sky_dispatch_win32/src/wait/hybrid.rs`](../rust/crates/sky_dispatch_win32/src/wait/hybrid.rs):
  High-resolution timer and QPC spin hybrid waiter.
- [`rust/crates/sky_dispatch_win32/src/input/packet.rs`](../rust/crates/sky_dispatch_win32/src/input/packet.rs):
  Win32 `SendInput` batch packet assembly and execution.
