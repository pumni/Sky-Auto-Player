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

## Final Down Admission and Safety Boundary

Every physical Down chord must pass strict final admission checks immediately
before calling `SendInput`:

```text
Worker Loop -> Wait Deadline Reached
    -> Sample Foreground HWND
    -> Query Process Identity (PID, creation time)
    -> Verify Target HWND & Generation Match
    -> Check Modifier Keys (Ctrl, Alt, Shift, Win = unpressed)
    -> Verify Supervisor Lease Active
    -> Check System Power & Session Controls (Pause, Skip, Stop)
    -> Pass: Dispatch SendInput
    -> Reject: Defer, Replan, or Terminate
```

The physical path stages a bounded packet, waits to its deadline, checks the
final focus/identity/modifier/lease/control gates, then calls the prepared
`SendInput` sender. Healthy dispatch uses fixed buffers and does not allocate.
Packets contain no more than 15 inputs, with Up before Down. Full Down ownership
commits only after complete insertion and valid completion QPC. Zero, partial,
or invalid-QPC outcomes preserve possibly-active release obligations for cleanup.
Cleanup succeeds only after physical state is verified resolved; pause/focus
resume then observes the absolute cleanup-completion plus $G$ barrier.

Unmatched-Up diagnostic metadata is committed by the normal authored-frame
Prepare path. Test-only metadata wake instrumentation is not a production path.
Metadata has no physical dispatch boundary and does not issue `SendInput`.

### Focus and Process Identity
- The target window must be the exact foreground window (`GetForegroundWindow() == target_hwnd`).
- The process owning the foreground window must match the retained target PID and
  image verified at startup.
- If focus is lost, Down transmission halts immediately, and any active keys are
  safely released.

### Resume Barrier (cleanup completion + $G$)
After verified pause or focus cleanup completes at QPC $Q$:
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
consumer owns queue high-watermark sampling. Queue diagnostics and Up-slot
accounting remain off the real-time producer suffix.

The live `DispatchExecutor` stores compact chord headers and their intents in a
contiguous arena. `MAX_EXECUTOR_BYTES` bounds the calculated capacities of
those two executor vectors. Production reads a borrowed `DownChordView` and
retains only `DownChordMetadata` while staging. `DownChordTicks` is a
self-contained fixture type with an owned `SmallVec`; it has no production
caller. Schedule compilation has its own phase-capacity checks. There is one
scheduler implementation in the crate: `DispatchExecutor`, including when
`test-support` is enabled.

`MAX_RETAINED_DISPATCH_BYTES` is the 512 MiB calculated-capacity limit used by
the compiler's phase checks and the desktop million-action construction test.
That test, `valid_one_million_single_key_actions_fit_compact_executor_budget`,
builds and compiles one million single-key actions, constructs the production
executor, and sums the measured source, compiled-schedule, and executor
capacities across the overlapping construction phases. It does not run a
million-action playback or measure process RSS, Windows memory peak, or
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
