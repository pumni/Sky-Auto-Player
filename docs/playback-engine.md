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
walk, or validate the authored schedule. Worker boot checks the plan's QPC and
H/G compatibility, moves its arrays into a new `DispatchExecutor`, and creates
fresh cursor, slot, epoch, barrier, and transport state for that playback. A
replay therefore begins with empty session state while reusing only immutable
schedule data. Dry-run variants do not require a physical QPC plan; their
advisory remains offline and cannot block a valid dry-run.

`NativeScheduleInput::Raw` remains the safe admission path for Rust callers
that still provide a `RuntimeSchedule`: session admission validates its timing
policy, and worker boot validates and materializes it again with cancellation
checkpoints before creating the executor. Desktop production entry points
provide `NativeScheduleInput::Prepared`. A prepared
plan whose QPC frequency or H/G policy no longer matches the worker is rejected
before dispatch.

Schedule compilation, timing validation, QPC conversion, and arena creation
run during cancellable Prepare. The remaining prepared worker construction
moves the already allocated arrays and initializes fixed-size state. Startup
still emits existing per-batch telemetry rows for unmatched Up metadata to
preserve schema 25; that diagnostics-only loop is proportional to the number
of unmatched-Up batches, which is zero for ordinary valid songs. See the
[M1 evidence record](evidence/issue-666-m1.md) for modeled-capacity and timing
scope.

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
    -> Sample Foreground HWND
    -> Query Process Identity (PID, creation time)
    -> Verify Target HWND & Generation Match
    -> For Down: Check Modifier Keys (Ctrl, Alt, Shift, Win = unpressed)
    -> Verify Supervisor Lease Active
    -> Check System Power & Session Controls (Pause, Skip, Stop)
    -> Pass: Dispatch SendInput
    -> Reject: Defer, Replan, or Terminate
```

The worker stages a bounded packet before its deadline wait, then rechecks
controls, modifiers for Down, fresh foreground HWND/PID ownership, the retained
process creation-time proof, target generation, lease, power, and focus
immediately before the prepared sender. A
pre-call focus, target, or modifier rejection makes the worker replan without
attempting or advancing that authored Down. Terminal Stop, Skip, Quit, Panic,
lease expiry, ownership failure, and system suspend remain fail-closed. When a
mixed packet's Down gate is temporarily rejected while its independent Up is
still eligible, the worker stages and admits that Up-only packet through the
same fresh musical final gate; the rejected Down remains pending.

Each authored Down onset can cross the syscall boundary at most once. A
pre-call rejection has no attempt; once the sender reports a syscall attempt,
the onset is consumed even when insertion is zero, short, complete with missing
QPC evidence, or otherwise ambiguous. The worker preserves the raw count,
Win32 error, attempt, and timing evidence, does not commit the packet as a
successful musical completion, and never invents a sender completion QPC. The
one narrow exception is an internally consistent, one-attempt Down-only raw
zero: the attempted onset is consumed and its zero-insertion journal bits are
cleared without a safety Up or a new H/G completion floor. Existing active
slots and their physical floors remain intact, and later unattempted onsets
remain eligible at their authored times. Any Up-bearing or mixed packet, short
insertion, unavailable or contradictory transport evidence, invalid QPC
interval, control change, or target change follows the conservative
ownership-verified cleanup path. Attempted Down keys then remain tracked as a
release obligation until safety cleanup proves release. If cleanup succeeds,
the actual cleanup completion QPC starts the resume barrier at $Q + G$; only
later Down onsets that have never been attempted may continue, in authored
order, after all final gates pass again. If ownership, release, or timing cannot
be proved, playback stays blocked or exits fail-closed with the obligation
retained. Recovering the physical key state does not convert the anomalous
transport into a successful packet. The worker keeps an attempted-anomaly latch
during recovery, then promotes that latch to a terminal error during
finalization. A session with any attempted post-call anomaly therefore ends in
error even if later unattempted onsets ran. An already stronger terminal error
remains primary; a concurrent terminal control cause remains recorded alongside
the anomaly evidence.

Attempted-anomaly diagnostics have bounded storage: the worker counts every
attempt, counts omitted detail records, and retains at most 16 records (the
first eight and rolling last eight). Each retained record is capped at 512
bytes and preserves the available source action, raw insertion count, Win32
error, attempt count, and QPC fields. The bounded sample is stored inline and
does not add heap allocation to the healthy dispatch path. The terminal error
latch depends on the total counter, not on which detail records remain. This is
internal diagnostic evidence; telemetry schema 25 and public DTOs do not
change.

Every musical Up bit, including an Up-only packet and a mixed-to-Up-only
replan, must carry the frozen release-owner proof recorded when its Down
created the obligation. The final gate proves the current target and compares
that current proof with every Up origin under a generation-conditional permit
held through the syscall. Missing, conflicted, mixed-generation, or drifted
proof fails closed without calling the sender or clearing the obligation.
Independent Up bits with one matching owner remain eligible. Terminal and
pause safety Up use the tracked cleanup policy and its originating owner proof
without requiring current foreground focus. Healthy dispatch uses fixed
buffers and does not allocate. Packets contain at most 15 inputs, with Up
before Down. H/G decisions use real sender completion QPC on healthy packets or
verified cleanup completion QPC after recovery; missing evidence is never
replaced by a later clock read. Terminal safety Up remains separate from
musical packet metrics.

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
- Stop, Skip, Panic, and Suspend retain their terminal safety cleanup path,
  including an Up attempt when the target is not foreground.

### Session Target Binding and Lifetime

`SessionTarget` owns the target HWND/generation publication, its bound owner
PID and generation, and the retained `WindowIdentityAuthority` for that
generation. The authority retains a query-only process handle plus the startup
creation-time and image identity. Desktop startup binds it before Arm; a
generation publication invalidates and releases the old binding before a new
identity can bind. A worker that already cloned the old authority may finish
that bounded query, but its final target-generation check rejects the old
publication. A target publication cannot inherit the prior process proof, even
when the numeric HWND is reused.

Supervisor poll results and authority close are conditional on the exact
retained `Arc<WindowIdentityAuthority>` and target generation that were
snapshotted. Querying process continuity happens without holding the writer
lock; applying its result rechecks the target stamp and retained authority
under the same writer serialization used by publication and binding. A poll
returns `CurrentValid`, `CurrentInvalid`, or `StaleSuperseded`. The desktop
monitor only turns `CurrentInvalid` into panic-release; a stale result cannot
invalidate the replacement owner's live bit, failure reason, or pre-Arm
startup. A stale poll can wake the worker, but does not authorize a musical
syscall: each packet still performs fresh final admission. Late close uses the
same conditional rule and cannot clear a newly bound authority.

For each focus-required musical packet, the worker samples the current
foreground HWND and owner PID, then queries the retained process handle for its
live state and creation time after lease/control checks. It performs a final
atomic HWND/generation/owner check before entering the sender. The process
query is constant-time and uses no heap allocation; a concurrent control-plane
binding makes that packet replan before any syscall. The supervisor continues
to check `IsWindow`, owner PID, process creation time, and image path on its
control cadence. This final proof supplements the fresh OS window query; it
does not replace it with a cached PID or identity value.

The musical final permit uses nonblocking acquisition for both the target
writer and retained owner-authority mutex. If either is busy, the worker
returns to replan before SendInput and leaves the Down cursor and Up
obligations untouched. On success, the writer permit remains held through the
physical sender call. Terminal and resumable safety cleanup keep their
serialized blocking permit path after validating the frozen release origin.

Each tracked key slot freezes a release-owner proof before a Down can create an
in-flight obligation. The proof includes the originating HWND and generation,
PID when required, and retained process-authority object identity. An A-to-B
rebind or same-HWND generation change cannot transfer A's release obligation
to B. Terminal and resumable safety cleanup read the proof from the tracked
backend, recheck the exact origin HWND/process continuity without requiring
foreground focus, and acquire a generation-conditional release permit. The
permit serializes the final proof check with publication/binding/close and is
held through the safety syscall. If validation observes drift or cannot prove
the origin, cleanup sends no Up, preserves the obligation, and finalizes with
an error. Zero-obligation cleanup needs no owner proof or syscall. The existing
release and quarantine policy remains in force.

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
