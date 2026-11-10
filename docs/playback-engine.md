# Playback Engine Architecture

The playback engine is the realtime subsystem of Sky Auto Player responsible
for executing music sheets as timed Windows keyboard events. It is implemented
in Rust (`sky_player`, `sky_dispatch_core`, `sky_dispatch_win32`) and runs on
a dedicated worker thread with MMCSS scheduling.

## Core Boundaries and Invariants

- **SendInput only:** All physical gameplay input uses the public Win32
  `SendInput` API. The engine never accesses game memory, injects DLLs,
  installs hooks, or bypasses anti-cheat boundaries. See [SECURITY.md](../SECURITY.md).
- **Single 15-slot executor:** Musical playback is driven by a single
  15-slot executor ([`DispatchExecutor`](../rust/crates/sky_dispatch_core/src/executor.rs)).
  There are no independent dual cursors for Down and Up events.
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
|  - Session Resume Barrier (Q + T)                     |
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
   release_due[key]    = max(epoch + paired_up_offset[key], down_completion + H)
   next_down_due[key]  = up_completion + T
   ```
2. **Release Execution:** When the current QPC ticks reach or exceed `release_due[key]`,
   the key slot becomes Up-ready.
3. **Down Admission:** A pending Down chord can only proceed when all its member keys
   have completed their previous Up releases and satisfied `now >= next_down_due[key]`.
4. **Coalesced Transport:** If disjoint Up-ready keys and a Down-ready chord mature at
   the same boundary, they are dispatched in a single `SendInput` call with Up events
   ordered before Down events.

## Timing Floors: H and G

The engine enforces physical and musical timing floors derived from the target
game frame rate and user-configured timing margin:

| Parameter | Definition | Formula |
| --- | --- | --- |
| Frame Interval ($T$) | Single frame duration | $\lceil 1{,}000{,}000 / \text{FPS} \rceil\ \mu\text{s}$ |
| Hold Floor ($H$) | Minimum key-down duration | $\lceil \text{hold\_frames} \times T \rceil + \text{margin}\ \mu\text{s}$ |
| Release Gap ($G$) | Physical same-key retrigger floor | $T\ \mu\text{s}$ |
| Authored Gap | Authored spacing between notes | $T + \text{margin}\ \mu\text{s}$ |
| Structural Cycle Floor | Minimum same-key cycle | $H + G$ |

- **Hold floor ($H$):** Guarantees the game client samples the key-down state
  across at least the configured number of frame boundaries.
- **Release floor ($G$):** Guarantees the game client registers a key-up frame
  before re-triggering the same key.
- **Timing margin:** Protects against host scheduling jitter and Windows timer
  granularity variations.

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

### Focus and Process Identity
- The target window must be the exact foreground window (`GetForegroundWindow() == target_hwnd`).
- The process owning the foreground window must match the retained target PID and
  image verified at startup.
- If focus is lost, Down transmission halts immediately, and any active keys are
  safely released.

### Resume Barrier ($Q + T$)
When resuming from pause, focus recovery, or emergency cleanup at time $Q$:
- All active key obligations are cleared or verified released.
- A mandatory delay barrier of $Q + T$ is imposed before any subsequent Down chord
  may be transmitted, ensuring the game processes the release state cleanly.

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

Schema 23 keeps one packet trace per physical send. Per-slot Up evidence is stored
in a separate bounded sidecar, linked by `trace_record_index`; it retains each
source action, slot, scan code, authored offset, release floor and residual. Its
4,096-entry capacity has a separate dropped-record count. Residual is measured from
the later of the authored target, slot release floor and packet not-before floor to
the sampled pre-call QPC; it is unavailable when that sample is missing.

`MAX_EXECUTOR_BYTES` limits the estimated `DownChordTicks` backing array to
512 MiB. It does not limit the combined schedule, coordinator, input, telemetry
or process peak. Allocation failures are not universally recoverable: in
particular, constructing shared reason strings still uses infallible allocation.

## Implementation Map

- [`rust/crates/sky_dispatch_core/src/executor.rs`](../rust/crates/sky_dispatch_core/src/executor.rs):
  15-slot dispatch executor, chord stream compilation, and slot state management.
- [`rust/crates/sky_player/src/engine/worker/dispatch_loop.rs`](../rust/crates/sky_player/src/engine/worker/dispatch_loop.rs):
  Realtime worker loop, event interleaving, and admission control.
- [`rust/crates/sky_player/src/engine/worker/physical_timing_guard.rs`](../rust/crates/sky_player/src/engine/worker/physical_timing_guard.rs):
  Realtime enforcement of hold floors ($H$) and release gaps ($G$).
- [`rust/crates/sky_dispatch_win32/src/wait/hybrid.rs`](../rust/crates/sky_dispatch_win32/src/wait/hybrid.rs):
  High-resolution timer and QPC spin hybrid waiter.
- [`rust/crates/sky_dispatch_win32/src/input/packet.rs`](../rust/crates/sky_dispatch_win32/src/input/packet.rs):
  Win32 `SendInput` batch packet assembly and execution.
