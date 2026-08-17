# 02 — P0: Per-Key Release Scheduler and Physical Feasibility

## 0. Why this is P0

Current runtime makes the effective deadline of a whole compiled same-timestamp packet:

```text
max(authored_packet_timestamp, latest release_not_before among packet Up intents)
```

That is safe for minimum hold but wrong for timestamp fidelity when a delayed Up is unrelated to the packet's Down chord.

Example:

```text
A is currently held
T = authored timestamp
packet at T:
    Up A
    Down [B, C]

release_due(A) = T + 300 µs
```

Current behavior delays `Down [B, C]` by 300 µs even though B/C do not depend on A.

The opposite naive fix is also wrong:

```text
Up A + Down [A, B]
```

Sending B early while waiting for A would split one authored chord. Therefore the correct solution is **dependency-aware per-key release scheduling**, not arbitrary packet splitting.

---

# 1. Required semantic model

The compiler's `CompiledPacket` remains a compact **authored frame**: all Up metadata at one authored timestamp plus at most one atomic Down chord.

Runtime owns two sources of upcoming boundaries:

1. the next authored frame timestamp;
2. pending per-key physical release deadlines created by earlier authored frames.

The coordinator chooses the earliest boundary. Equal boundaries may be coalesced into one physical `SendInput` transaction.

```text
next_boundary = min(
    next_authored_frame.authored_ticks,
    earliest_pending_release.due_ticks,
)
```

No packet-wide release floor is allowed to rewrite the authored frame timestamp.

---

# 2. New fixed-slot pending-release state

Add bounded state to `RuntimeDispatchCoordinator`.

Recommended shape:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingRelease {
    pub generation_id: GenerationId,
    pub key_slot: KeySlot,
    pub authored_release_ticks: TimelineTicks,
    pub due_ticks: TimelineTicks,
}

pub struct RuntimeDispatchCoordinator {
    // existing fields...
    pending_release_by_slot: [Option<PendingRelease>; MAX_KEYS],
    pending_release_mask: u16,
}
```

Requirements:

- no `HashMap`/heap collection for pending releases;
- at most one pending release per physical slot;
- pending release generation must equal the active generation on that slot;
- `pending_release_mask` must agree with `pending_release_by_slot`;
- state is mutated only by coordinator commit methods, never by observer code.

If additional diagnostic metadata is genuinely needed, add only fixed-size scalar fields. Do not store strings/reasons in pending state.

---

# 3. Remove packet-wide release-floor deadline calculation

Current coordinator functions around:

```text
packet_release_floor_ticks()
packet_effective_deadline_ticks_uncompensated()
```

must no longer define the authored frame's physical deadline as a max across Up releases.

Replace the semantic split with:

```text
authored_frame_ticks(packet) = immutable authored timestamp
release_due(generation) = max(authored Up timestamp, active.release_not_before_ticks)
```

The next authored frame always retains `authored_frame_ticks`.

The release scheduler separately exposes the earliest pending release deadline.

---

# 4. Prepare one authored frame without mutation

Introduce a pure/bounded prepare operation in `sky_dispatch_core`, conceptually:

```rust
pub struct PreparedAuthoredFrame {
    pub packet_index: usize,
    pub first_batch_index: usize,
    pub packet_batch_count: usize,
    pub authored_ticks: TimelineTicks,
    pub immediate_up_mask: u16,
    pub deferred_up_mask: u16,
    pub down_mask: u16,
    pub stale_up_count: u8,
    // exact compact intent evidence needed by commit
}
```

Exact field visibility can vary. The following semantics cannot.

For each owned Up intent in the current frame at authored target `T`:

```text
due = max(T, active.release_not_before_ticks)

if due <= T:
    immediate Up
else:
    deferred Up
```

Because `due >= T`, the test is equivalent to checking whether the completion-anchored release floor is already satisfied at the authored target.

Stale `NO_GENERATION_ID` Up intents remain nonphysical metadata and are neither immediate nor pending physical releases.

---

# 5. Feasibility test for the Down chord

Before a physical plan is accepted:

```text
blocked_retrigger_mask = deferred_up_mask & down_mask
```

### If zero

The Down chord does not depend on a delayed release. It remains eligible at the authored timestamp.

### If nonzero

The authored Down chord is physically impossible at its timestamp.

Required behavior:

- do not wait until the release floor and then send the chord late;
- do not remove blocked chord members;
- do not send unblocked chord members early;
- do not convert this into a warning;
- return a structural fidelity error before calling `SendInput`.

Recommended error variant:

```rust
CoordinatorError::PhysicalDeadlineInfeasible {
    authored_ticks: TimelineTicks,
    blocked_mask: u16,
    latest_required_release_ticks: TimelineTicks,
}
```

The public Python error text may wrap this variant but must retain the key facts.

---

# 6. Commit semantics for an authored frame

Preparing a frame must not mutate coordinator state. Mutation happens only when the selected boundary is committed.

There are two authored-frame commit cases.

## 6.1 Frame has immediate physical work

Physical work is:

```text
up_mask   = immediate_up_mask (+ pending releases coalesced at same boundary)
down_mask = authored down_mask
```

On full `SendInput` success:

1. Commit coalesced older pending releases.
2. Commit current-frame immediate Up transitions.
3. Register current-frame deferred Up intents into `pending_release_by_slot`.
4. Commit the atomic Down chord using the one transport start/completion pair.
5. Advance the authored cursor by the frame's `packet_batch_count` exactly once.
6. Validate only touched slots in release production code.

The logical Up-before-Down transition order remains mandatory for same-key retrigger.

## 6.2 Frame has no immediate physical work

Example: an Up-only authored frame where every owned release is still below its minimum-hold floor.

At the authored boundary:

1. register its deferred releases;
2. consume stale Up metadata;
3. advance authored cursor;
4. perform no `SendInput` call.

This is a metadata boundary, not a physical send. It must remain bounded and allocation-free after preparation.

This step is necessary: leaving the authored cursor parked until the release floor would block unrelated later authored frames.

---

# 7. Pending release planning

Add coordinator queries that are O(`MAX_KEYS`) and allocation-free:

```rust
fn earliest_pending_release_ticks(&self) -> Option<TimelineTicks>;
fn pending_release_mask_due_at(&self, target: TimelineTicks) -> u16;
```

Scanning 15 fixed slots during **planning before timed wait** is acceptable. Do not maintain a heap/priority queue for fifteen keys.

For the earliest due target `R`, group all pending releases whose `due_ticks == R` into one Up-only physical mask.

Do not send a release before its own `due_ticks` merely because another release is due.

---

# 8. Boundary selection and coalescing

The planner compares:

```text
A = next authored frame target (if any)
R = earliest pending release target (if any)
```

## R < A

Plan pending Up-only physical work at `R`.

## A < R

Plan the authored frame at `A`.

Its immediate Up + Down masks are determined by the prepare logic. Any deferred unrelated Up is registered on commit and does not move A.

## A == R

Build one coalesced physical transaction when the authored frame has physical work:

```text
up_mask = pending_due_mask | authored.immediate_up_mask
down_mask = authored.down_mask
```

The packet builder already emits all Up entries before all Down entries.

Commit token must record both ownership sources so one full success can commit them exactly once.

If the authored frame is metadata-only, send the pending Up mask and commit the authored metadata in the same boundary completion step.

---

# 9. Pending release and later same-key Down

A later authored Down chord may reuse a slot whose earlier authored Up was deferred.

The planner must enforce:

```text
pending release due <= new Down authored target
```

Normal scheduler operation will select and send the pending release at its earlier due boundary before reaching the later Down.

If, because of a stall, the pending release and the later Down are both overdue, the stall/backlog policy in `03_P0_OVERDUE_STALL_POLICY.md` decides whether the state is still coherent. Do not silently collapse distinct missed boundaries into one transaction.

If a new Down target is earlier than the pending release due, this is a structural physical-deadline fault and the chord must not be moved.

---

# 10. Native deterministic hold validation

The current PyO3 `validate_schedule_timing()` checks only arithmetic overflow. Add a pure validator in `sky_dispatch_core` and call it during native session construction after `effective_min_hold_us` is materialized.

Recommended API:

```rust
pub fn validate_min_hold_feasibility(
    schedule: &RuntimeSchedule,
    effective_min_hold_us: u64,
) -> Result<(), ScheduleTimingError>
```

Walk the compiled schedule in authored order using fixed per-slot state. For every owned generation with authored Down `Td` and authored Up `Tu`:

```text
Tu - Td >= effective_min_hold_us
```

Reject otherwise.

Recommended structured error data:

```rust
ScheduleTimingError::SameKeyHoldTooShort {
    scan_code: u16,
    down_source_action_index: u32,
    up_source_action_index: u32,
    down_scheduled_us: u64,
    up_scheduled_us: u64,
    interval_us: u64,
    required_min_hold_us: u64,
}
```

If current compiled structures do not retain a direct Up source index per compact intent, derive it while walking batch ranges; do not weaken the validator just to avoid adding a bounded lookup.

Also keep the existing overall timestamp-overflow validation.

Why validate again natively:

- Python scheduler is not the only possible caller of the native API.
- native `effective_min_hold_us` is the actual runtime contract;
- documentation currently promises this validation;
- rejecting deterministic impossibility before spawning the RT worker is cheaper and clearer than runtime conflict recovery.

---

# 11. Coordinator commit-token design

Do not let the post-`SendInput` path re-query large schedule structures to discover what was just sent.

Before waiting, freeze exact bounded commit evidence.

Recommended conceptual enum:

```rust
enum PhysicalCommitToken {
    PendingReleases(PendingReleaseCommit),
    AuthoredFrame(AuthoredFrameCommit),
    Coalesced {
        pending: PendingReleaseCommit,
        authored: AuthoredFrameCommit,
    },
}
```

Each token contains only fixed-size masks/scalars and `SmallVec<[CompactIntent; MAX_KEYS]>` where identity lists are required.

On full success, commit is deterministic and bounded by 15 keys.

On any ambiguous send result, do not consume the token; terminate and cleanup.

---

# 12. `PreparedPhysicalPacket` construction

`sky_player_rs` converts the core plan masks into `sky_dispatch_win32::input::PhysicalPacket`, then `PreparedPhysicalPacket`, **before** timed waiting.

No Win32 type is added to `sky_dispatch_core`.

Maximum physical transaction remains 30 INPUT entries:

```text
15 Up + 15 Down
```

Builder order remains:

```text
all Up entries first
all Down entries second
```

---

# 13. What to delete/retire after this phase

Once the new scheduler is covered by tests:

- remove packet-wide `packet_release_floor_ticks()` as a physical deadline source;
- remove any production API whose meaning is “next authored packet deadline after max release floor”;
- replace tests that expect unrelated Down to inherit an Up floor;
- update `docs/timing-principles.md` and `docs/hold-frame-model.md` to describe per-key pending release scheduling;
- keep compatibility helpers only if a test-only public seam still needs them, and mark them nonproduction.

---

# 14. Required regression scenarios

At minimum implement these exact cases.

## Case A — ordinary Down/Up

```text
Down A @ 0
Up A   @ 50 ms
min_hold 20 ms
```

Up remains authored at 50 ms. No pending deferral.

## Case B — release floor defers only its own key

```text
Down A completes late
Up A @ T, release_due(A) = T + 300 µs
Down [B, C] @ T
```

Expected:

- Down `[B, C]` remains target T;
- A becomes pending release for T+300 µs;
- no packet-wide shift.

## Case C — same-key retrigger is still feasible

```text
Up A @ T
Down [A, B] @ T
release_due(A) <= T
```

Expected one Mixed `SendInput`: `Up A` before `Down A,B`.

## Case D — same-key retrigger becomes dynamically impossible

```text
Up A @ T
Down [A, B] @ T
release_due(A) > T
```

Expected:

- no Down send;
- structural fidelity error;
- cleanup;
- B is not sent alone.

## Case E — authored Up-only all deferred

```text
Down A earlier
Up A @ T
release_due(A) > T
next Down B @ T + 1 ms
```

Expected:

- metadata frame consumed at T;
- A pending release registered;
- Down B is not blocked by the old authored packet cursor;
- whichever of A's pending due vs B's authored target is earlier is selected next.

## Case F — two pending releases with different floors

A due at T1, B due at T2, T1 < T2.

Expected two distinct release boundaries. Do not send B at T1.

## Case G — equal pending release floors

A and B both due at T.

Expected one Up-only packet containing both.

## Case H — pending release equals next authored chord target

Pending Up A due at T; Down `[A, B]` authored at T.

Expected one transaction Up A then Down A/B, provided no other invariant is violated.

## Case I — deterministic native hold rejection

Authored same-key Down→Up interval `< effective_min_hold_us`.

Expected session construction error before worker start.

## Case J — exact-floor authored interval

Interval `== effective_min_hold_us` is valid at authored-validation time. Runtime completion anchoring may still defer the release if the Down completion occurred after its authored target. The runtime scheduler must handle that without shifting unrelated keys.

---

# 15. Performance constraints for this P0 refactor

Correctness comes first, but the new design must remain RT-suitable:

- no heap/priority queue for releases;
- no locks in coordinator prepare/commit;
- O(15) release scan allowed during pre-wait planning;
- post-send commit touches only the sent/deferred masks/intents, not the whole song;
- `PreparedPhysicalPacket` still built pre-wait;
- no extra `SendInput` call when due releases and a Down chord can safely share one boundary;
- separate sends occur only when physical deadlines are genuinely different.

Do not optimize away these semantics for syscall-count aesthetics. One fewer syscall is not worth moving an unrelated note.
