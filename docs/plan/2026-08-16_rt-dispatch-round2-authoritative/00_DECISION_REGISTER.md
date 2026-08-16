# 00 — Authoritative Decision Register

This file records decisions, not suggestions. A coding agent may choose local naming/layout details only when they do not alter these decisions.

Legend:

- **KEEP** — current idea is correct; refactor only for clarity if needed.
- **REFACTOR** — behavior/structure must change as specified.
- **REMOVE** — remove from production path/API internals when phase allows.
- **REJECT** — explicitly prohibited in this overhaul.

---

## D-001 — Authored time remains immutable

**Decision: KEEP**

An authored timestamp is musical input data. Runtime sender cost, wake history, observer metrics, or previous dispatch error may not rewrite it.

Allowed runtime changes are only physical feasibility constraints on releases and explicit pause-time exclusion. They do not become a learned lead or a global timeline controller.

---

## D-002 — One authored Down action is one atomic chord

**Decision: KEEP / STRENGTHEN**

All keys in one authored Down action must enter one `SendInput` transaction. Never split a chord into an early subset and late subset to satisfy a per-key dependency.

If one member makes the chord physically impossible at its authored boundary, fail closed rather than substitute a different chord.

This corrects the tempting but wrong solution of sending an unrelated key from `[A, B]` early while waiting to retrigger `A`: doing so changes the authored chord.

---

## D-003 — Same-timestamp compiler packet is metadata, not an inseparable runtime deadline unit

**Decision: REFACTOR**

The compiler may continue grouping same-timestamp Up actions and one Down chord for compact immutable metadata. Runtime must stop treating the whole compiled packet as having one release-floor-shifted physical deadline.

A compiled packet/frame can yield:

- immediate releases,
- deferred releases,
- one atomic Down chord,
- stale nonphysical Up metadata,
- or a combination.

---

## D-004 — Release floors are per generation/key

**Decision: REFACTOR**

Maintain pending release state in a fixed slot table bounded by `MAX_KEYS = 15`.

Conceptual shape:

```rust
struct PendingRelease {
    generation_id: GenerationId,
    key_slot: KeySlot,
    authored_release_ticks: TimelineTicks,
    due_ticks: TimelineTicks,
    source_action_index: u32,
}

pending_release_by_slot: [Option<PendingRelease>; MAX_KEYS]
pending_release_mask: u16
```

Exact visibility/naming may vary, but the bounded fixed-slot model is mandatory.

---

## D-005 — Unrelated deferred releases may not block a Down chord

**Decision: REFACTOR / P0**

For an authored frame at `T`:

```text
owned_up_mask
immediate_up_mask = releases whose release_not_before <= T
deferred_up_mask  = owned_up_mask - immediate_up_mask
down_mask
```

If `deferred_up_mask & down_mask == 0`, the Down chord remains eligible at `T`. Deferred releases are registered for their own future due boundaries.

This removes packet-wide head-of-line blocking.

---

## D-006 — A deferred release that is required by the current Down chord is a fidelity fault

**Decision: FAIL CLOSED**

If:

```text
deferred_up_mask & down_mask != 0
```

then the same-key release cannot legally precede the new Down at the authored target. Do not delay the chord and do not split it. Terminate playback through full cleanup with a structured `physical_deadline_infeasible` reason.

---

## D-007 — Deterministic hold infeasibility is rejected before worker start

**Decision: REFACTOR / P0**

Native admission must validate same-key authored Down→Up intervals against `effective_min_hold_us`.

Current `validate_schedule_timing()` only checks arithmetic range even though the hold model documentation says authored validation rejects intervals below the floor. Fix the implementation, not the documentation.

Python scheduler validation remains useful but is not the native safety boundary.

---

## D-008 — Runtime completion anchoring remains

**Decision: KEEP**

For a successful Down:

```text
release_not_before = sendinput_completion + effective_min_hold
```

Do not change this to authored Down time or admission time merely to avoid release deferral. Completion anchoring protects the configured sender-side minimum hold from the sender's own call duration.

---

## D-009 — No overdue catch-up burst

**Decision: REFACTOR / P0**

Production must never respond to a large OS/scheduler stall by issuing several overdue physical notes back-to-back.

When more than one distinct physical boundary is already overdue, treat it as a fidelity fault and terminate/cleanup. See `03_P0_OVERDUE_STALL_POLICY.md` for the exact classifier.

---

## D-010 — Do not introduce active-playback timeline rebasing in this refactor

**Decision: REJECT**

`PlaybackClockState::rebase_epoch()` is not the solution for OS stalls here. Runtime release floors and active generation state already live in the playback timeline; rebasing correctly would require a broader transformation of all outstanding temporal state.

A fail-closed stall policy is simpler and safer. Rebase may be researched later as a separate design with explicit state transformation proofs.

---

## D-011 — One immutable physical plan type; invalid states should be unrepresentable

**Decision: REFACTOR / P1**

Replace the correlated `Option` matrix in `NextDispatchPlan` with an enum/typed variant such as:

```rust
enum NextDispatchPlan {
    NoWork,
    Metadata(AuthoredMetadataPlan),
    Physical(PhysicalDispatchPlan),
}
```

A `PhysicalDispatchPlan` must contain non-optional:

- exact absolute QPC target,
- prepared Win32 packet,
- exact coordinator commit token,
- dispatch path/masks,
- any target/preflight proof required by a Down-bearing send.

Do not keep runtime `plan_structure_is_valid()` branches for states the type system can forbid.

---

## D-012 — Planner is the sole owner of `physical_target_qpc`

**Decision: REFACTOR / P1**

Wait code receives the frozen absolute target. It must not reconstruct the target from `epoch + deadline_ticks`.

Timeline ticks remain metadata/observation evidence, not a second physical-target calculator.

---

## D-013 — Prepared `INPUT[]` is mandatory before the precision region

**Decision: KEEP**

`PreparedPhysicalPacket` remains fixed-capacity and is built before timed waiting. No scan-code mapping, allocation, packet construction, sorting, or schedule lookup is allowed after the final deadline wake.

---

## D-014 — `final_admission_qpc` is not a syscall-entry timestamp

**Decision: RENAME/CLARIFY, NOT RESAMPLE BY DEFAULT**

The current supplied start sample is taken before the final lease classification and before the transport call stack. Its truthful semantic name is `final_admission_qpc`.

Compatibility fields such as `send_started_ticks` may remain aliases, but documentation must explicitly map them to final-admission time.

---

## D-015 — No second production QPC read immediately before `SendInput` unless benchmarked separately

**Decision: KEEP ONE AUTHORITATIVE PRODUCTION SAMPLE**

A diagnostic/benchmark build may add `sendinput_call_qpc` immediately before the syscall to measure admission→call cost. That sample is observation-only and must never feed a controller.

Do not pay an extra production QPC read merely to make a field name sound more exact.

---

## D-016 — Remove the redundant post-deadline-wake QPC sample

**Decision: REFACTOR / P1**

`WaitResult::wake_qpc` already contains the QPC sample that proved `now >= target`. The dispatch loop must reuse it for the deadline handoff instead of immediately querying QPC again before entering dispatch.

The next fresh production QPC sample should be the final-admission sample unless a correctness gate explicitly needs another one.

---

## D-017 — Observer/health state cannot authorize physical dispatch

**Decision: REFACTOR / P1**

Remove `FrozenDispatchBudget`/health threshold ownership from the authoritative physical plan. Health thresholds are diagnostic observer policy.

A telemetry or health subsystem failure may terminate at a defined safe boundary, but health history cannot move, lead, or authorize a note.

---

## D-018 — Producer queue path is one nonblocking push plus essential local failure accounting

**Decision: REFACTOR / P1**

Remove producer `ArrayQueue::len()` high-watermark sampling from every dispatch. Compute an approximate high watermark on the consumer side or deprecate the exact field.

No drain, mutex, formatting, conversion, shared publication, or queue backpressure on the sender.

---

## D-019 — Keep `crossbeam_queue::ArrayQueue`

**Decision: KEEP**

Its current bounded push is safe, allocation-free, and already tested. A custom SPSC ring would add unsafe/concurrency complexity without evidence that the queue primitive is the limiting tail-latency source.

Replacing it requires a future independent A/B benchmark and review.

---

## D-020 — Full 15-slot invariant scan leaves the production healthy path

**Decision: REFACTOR / P1**

`validate_local_slot_masks()` must not scan all slots after every healthy packet in release builds.

Keep:

- local checks on every touched slot/generation,
- full validation in tests/debug assertions,
- full validation before/after cleanup or explicit diagnostic validation.

Correctness must be maintained by bounded transitions, not by repeatedly proving untouched state.

---

## D-021 — Production spin threshold is fixed at 700 µs

**Decision: REFACTOR / P1**

Remove the 32-sample p95 startup measurement as a controller. Ship one deterministic 700 µs production threshold for this refactor.

The wake probe may remain as diagnostics/benchmark evidence. Any future change to 700 µs requires real Windows A/B evidence.

---

## D-022 — High-resolution waitable timer + event wait is mandatory in production

**Decision: KEEP / SIMPLIFY**

Production must fail admission if either required primitive is unavailable. No `std::thread::sleep` or multimedia-timer fallback is permitted for production physical scheduling.

Test/degraded helpers can remain behind test-support boundaries.

---

## D-023 — `timeBeginPeriod` is removed from the production waiter

**Decision: REMOVE**

The shipping path already requires `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`; `timeBeginPeriod` is neither the QPC clock nor a reliable substitute for the required high-resolution timer, and Windows 11 can withhold elevated resolution for occluded/minimized window-owning processes.

Do not acquire a timer-resolution guard during a production timer-creation failure that will immediately be rejected anyway.

---

## D-024 — MMCSS + HighQoS remain; TimeCritical/affinity do not become defaults

**Decision: KEEP / REJECT ALTERNATIVES**

Keep MMCSS `Games` registration with `AVRT_PRIORITY_HIGH` when available and the current safe fallback behavior. Keep thread HighQoS by disabling execution-speed power throttling.

Do not:

- use `REALTIME_PRIORITY_CLASS`,
- default to `THREAD_PRIORITY_TIME_CRITICAL`,
- pin dispatch to a core,
- call RDTSC/RDTSCP directly.

---

## D-025 — No learned dispatch compensation

**Decision: REJECT**

No EMA, PID, adaptive lead, SendInput-duration subtraction, Raw Input delivery lead, or other history-driven advance of note-on targets.

Calibration may inform the hold margin only where already specified. It must not advance authored note-on timing.

---

## D-026 — Production authored sends are single-attempt

**Decision: KEEP / CLEAN UP LEGACY NAMES**

Down, Mixed, and authored Up physical transactions get one `SendInput` attempt. Zero/partial/ambiguous result is terminal for playback and enters cleanup.

Bounded retries belong only to the explicit cleanup/release FSM where physical state is already in recovery.

---

## D-027 — Structural fidelity faults are terminal independent of `strict_timing`

**Decision: STRENGTHEN**

`strict_timing` may continue controlling diagnostic lateness thresholds. It must not control whether these structural faults are fatal:

- partial Down/Mixed insertion,
- impossible same-key deadline,
- multiple-overdue-boundary catch-up condition,
- coordinator/physical ownership disagreement,
- QPC failure,
- required wait backend failure.

---

## D-028 — Focus-safe mode keeps a fresh foreground-window proof

**Decision: KEEP**

When `require_focus=true`, the fresh `GetForegroundWindow`-based final proof is a correctness cost and remains before final-admission QPC.

Do not cache it away to win microseconds. Benchmark focus-safe and minimum-jitter modes separately.

---

## D-029 — Physical-key preflight remains before timed waiting

**Decision: KEEP**

The preflight protects against pre-existing keyboard state and target changes. It stays outside the precision region and can be cached by target generation as today.

---

## D-030 — Release build keeps checked temporal arithmetic

**Decision: KEEP**

Do not remove typed tick wrappers or checked add/subtract/conversion for speculative speed. Arithmetic failure is terminal because a wrapped timestamp is worse than stopping.

---

## D-031 — `KEYBDINPUT.time` remains zero

**Decision: KEEP**

Do not use the millisecond `time` field as a future scheduler. Scheduling is QPC wait → immediate `SendInput`.

---

## D-032 — Release build keeps `panic = "unwind"`

**Decision: KEEP**

The worker's final cleanup boundary must survive an unexpected Rust panic where possible. Do not switch this core to `panic = "abort"` for code-size/performance speculation.

---

## D-033 — Compatibility fields may remain, compatibility behavior may not control production

**Decision: CLEAN UP**

Old estimator/retry/lead fields that external Python/reporting still expects may remain stable aliases/zeros. Internal production state machines must not retain dead adaptive/retry logic solely because the report schema contains old names.

---

## D-034 — No speculative optimization outside measured bottlenecks

**Decision: REJECT**

Not part of this overhaul unless a phase explicitly requests it:

- custom allocator,
- custom lock-free queue,
- PGO/target-cpu tuning,
- cache-line padding everywhere,
- command-bit atomics consolidation,
- manual prefetch,
- spin-loop unrolling,
- core isolation/affinity,
- kernel drivers or alternative input APIs.

The project already has enough complexity. First remove semantic errors and unnecessary RT work.
