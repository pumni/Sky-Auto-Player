# 04 — P1: Real-Time Path Minimization

## 1. Goal

After P0 semantics are correct, reduce work between a physical deadline wake and readiness for the next physical deadline.

This phase is not “make everything low-level”. It removes work that does not authorize, send, or minimally commit the current physical transaction.

Target healthy path:

```text
frozen physical plan
    ↓
wait deadline handoff
    ↓
final bounded control/target gates
    ↓
final_admission_qpc
    ↓
SendInput
    ↓
sendinput_completion_qpc
    ↓
minimal commit token application
    ↓
fixed observation enqueue
    ↓
return
```

---

# 2. Make plan states unrepresentable instead of re-validating them

Current `NextDispatchPlan` carries a correlated set of `Option` fields and uses `plan_structure_is_valid()` at dispatch time.

Replace it after P0 with a typed enum.

Recommended shape:

```rust
enum NextDispatchPlan {
    NoWork,
    Metadata(MetadataBoundaryPlan),
    Physical(PhysicalDispatchPlan),
}
```

Recommended `PhysicalDispatchPlan` concept:

```rust
struct PhysicalDispatchPlan {
    target_qpc: QpcTicks,
    authored_target_ticks: TimelineTicks,
    packet: PhysicalPacket,
    prepared_packet: PreparedPhysicalPacket,
    commit: PhysicalCommitToken,
    target_proof: TargetProof,
    path: DispatchPath,
}

enum TargetProof {
    NotRequired,          // Up-only
    Down(TargetStamp),   // Down-bearing
}
```

Exact module ownership may differ, but these properties are mandatory:

- no optional prepared packet in a physical variant;
- no optional physical target in a physical variant;
- no runtime branch proving the packet and prepared packet co-exist;
- no health budget required for physical validity;
- no schedule lookup after deadline wake to rediscover packet identity.

`MetadataBoundaryPlan` must likewise contain everything its bounded commit needs.

---

# 3. Remove `FrozenDispatchBudget` from authoritative planning

Current plan freezes health thresholds/path evidence together with timing work. Health history does not authorize input.

Change:

- physical plan keeps only path/mask data required to send/commit;
- observer thread owns warning thresholds and health-window policy;
- raw observation contains path/event count if needed for classification;
- `build_dispatch_budget()` can be removed from planning or reduced to observer-only code.

Delete dispatch-time failures such as “physical plan has no health budget”. A missing diagnostic budget must never be capable of blocking a correct note.

---

# 4. Reuse the deadline QPC evidence already returned by the waiter

Current deadline flow:

```text
HybridWaiter gets final QPC proving deadline
returns WaitResult.wake_qpc
worker calls QPC again immediately
projects effective_now
calls dispatch
admission later calls QPC again for final_admission
```

Remove the middle QPC read.

For `WaitBoundary::Due`, carry the exact `WaitResult.wake_qpc` into the dispatch handoff.

Use it for:

- proving `target_qpc <= wake_qpc`;
- wake telemetry;
- any already-required deadline classification.

Do not use a fresh QPC solely to create an `effective_now_ticks` diagnostic value before final admission.

The next fresh healthy-path QPC sample should be `final_admission_qpc`.

For already-overdue work selected before entering the waiter, use the scheduler's existing current-QPC sample for due/backlog classification; do not invent a special replacement target.

---

# 5. Wait accepts absolute frozen target

Refactor `wait_for_next_boundary()` so the production physical path receives:

```rust
target_qpc: QpcTicks
```

not merely a logical deadline that it maps again through `PlaybackClockState`.

Lease wake bounding remains:

```text
bounded_wait_target = min(physical_target_qpc, supervisor_lease_deadline)
```

If the lease boundary wins, replan as today. If the physical target wins, the returned deadline belongs to that exact frozen plan.

Timeline deadline remains attached as observation metadata but is not used to reconstruct the physical target.

---

# 6. Preflight stays before waiting and becomes part of the plan proof

For Down-bearing work:

1. build/freeze authored/physical plan;
2. load target stamp;
3. run physical-key preflight if the target generation is not already verified;
4. ensure stamp still current;
5. store `TargetProof::Down(stamp)` in the plan;
6. enter timed wait.

For Up-only work:

```text
TargetProof::NotRequired
```

Final admission still verifies current target generation and fresh foreground window when focus gating is enabled.

Do not run `GetAsyncKeyState`, keyboard-layout mapping, or full physical preflight after the deadline wake.

---

# 7. Shrink the healthy post-SendInput commit

After full transport success, production needs only enough state to make the next plan correct.

Required work:

- validate transport success/timing boundary existence;
- map start/completion QPC to typed playback ticks with checked arithmetic where coordinator state requires it;
- apply the frozen commit token to touched generations/slots;
- compute new `release_not_before` for Down generations;
- update bounded backend masks;
- update one or two essential local structural counters if necessary;
- enqueue raw observation.

Move out of the healthy producer path:

- health-window calculations;
- warning threshold derivation;
- string formatting;
- rich telemetry record construction;
- report compatibility calculations;
- full-state scans;
- queue occupancy/high-watermark sampling;
- CPU/process metrics;
- repeated microsecond conversions.

---

# 8. Remove release-build full-slot validation per transaction

Current commit calls a full `validate_local_slot_masks()` loop over all 15 slots after each packet.

Replace with:

### Release healthy path

Validate only touched identities while applying transitions:

- old active generation matches Up intent;
- no Down overwrites an active/blocked slot unless its old generation is released in the same successful transaction;
- pending-release identity matches active owner;
- masks are updated directly from the same touched-slot transition.

### Test/debug path

Keep a full validator:

```rust
#[cfg(any(test, debug_assertions))]
fn validate_local_slot_masks_full(...)
```

and invoke it after commits in tests/debug builds.

### Cleanup/finalization

Keep full invariant checks before/after cleanup because those are not precision-path operations.

Do not remove invariant tests; move the cost to the appropriate boundary.

---

# 9. Replace large producer observation with compact raw evidence

Current `DownObservation`/`UpObservation` copy many derived/compatibility/health fields into `ArrayQueue`.

Create one compact raw observation representation sufficient for deferred derivation.

Conceptual example:

```rust
struct RawDispatchObservation {
    target_qpc: QpcTicks,
    final_admission_qpc: QpcTicks,
    completion_qpc: QpcTicks,
    authored_ticks: TimelineTicks,
    wake_qpc: Option<QpcTicks>,
    packet: PhysicalPacket,
    status: SendTransactionStatus,
    inserted: u8,
    flags: u16,
}
```

Add only fields that cannot be recovered safely by the observer.

Do not copy into every observation:

- static health thresholds;
- timeline-rebase counters that production no longer uses;
- duplicated values derivable from masks/status;
- formatted labels/strings.

Observer can derive:

```text
admission_to_completion
start error
completion residual
path counts
lateness buckets
health windows
compatibility telemetry aliases
```

from raw ticks and fixed policy.

---

# 10. Remove producer queue-length sampling

Current push path does:

```text
queue.push(observation)
queue.len()
update high watermark
```

Change healthy producer to:

```text
if queue.push(observation).is_err() {
    dropped += 1;
}
```

No exact producer high-watermark read.

Options for compatibility field, in preference order:

1. consumer-side approximate observed depth;
2. mark metric unavailable/deprecated while preserving schema field;
3. zero compatibility value with explicit semantics.

Do not add a new shared occupancy atomic just to preserve exact high watermark; that replaces one producer cost with another.

---

# 11. Keep `ArrayQueue`; do not write a custom SPSC

The queue is not currently proven to dominate tail latency. Existing tests already establish bounded/no-allocation push behavior.

This phase removes unnecessary work around the queue first.

Only reconsider queue implementation if real producer-only A/B data after these changes shows queue push itself is a material p99.9 contributor.

---

# 12. Separate normal production from strict diagnostic calculations

`strict_timing_diagnostic` may need a completion-late predicate before allowing another dispatch.

Keep that as one explicit post-commit diagnostic-policy branch.

Normal production must not calculate strict-only microsecond strings/threshold details on every send.

Suggested separation:

```rust
commit_success(...);

if config.timing.strict_timing {
    evaluate_strict_terminal_predicate(...)?;
}

push_raw_observation(...);
```

The strict branch may perform extra checked tick comparison but should still avoid formatting unless it actually terminates.

Structural faults remain fatal in all profiles.

---

# 13. Avoid function-argument refactors for performance theater

Several worker functions have many arguments. Reducing argument count can improve readability, but it is not itself a latency optimization.

Do not create deep trait hierarchies, boxed strategy objects, or dynamic-dispatch contexts to make signatures look smaller.

Use plain borrowed context structs only where they clarify ownership and compile away cleanly.

---

# 14. Preserve important existing fast-path choices

Keep:

- fixed `INPUT` arrays;
- scan-code templates;
- `SmallVec` bounded by 15 where copied identity evidence is needed;
- no Python/PyO3 work in worker;
- no healthy-path mutex on telemetry;
- nonblocking queue overflow policy;
- typed checked QPC/timeline arithmetic;
- `SetLastError(0)` + `GetLastError()` only on short insertion;
- one authored physical SendInput attempt.

---

# 15. No-allocation gate expansion

Existing `rt_dispatch_no_alloc` tests are good but must cover the refactored plan/commit path.

Add allocation-count windows for:

1. plan already prepared → deadline handoff → full Down commit → raw enqueue;
2. Mixed commit;
3. pending Up-only commit;
4. coalesced pending release + authored Down commit;
5. metadata-only deferred release registration;
6. saturated observer queue drop-new path.

The measured window must exclude test harness construction and consumer drain.

---

# 16. Benchmark points required by this phase

Add/retain direct QPC or benchmark evidence for:

```text
wake -> final_admission
final_admission -> SendInput completion
SendInput completion -> dispatch function return
producer observation enqueue
full plan/commit handoff
```

The key new metric is:

```text
completion_to_rt_ready
```

where `rt_ready` means coordinator/backend state required to plan the next physical operation has been committed and the producer is about to return to the outer scheduler.

A diagnostic build may sample this boundary. Production need not permanently pay an extra QPC read if A/B shows it is not worth retaining.

---

# 17. Acceptance

P1 RT minimization is complete when:

- no correlated-Option plan validation remains on healthy physical dispatch;
- wait no longer reconstructs the frozen physical target;
- deadline wake does not immediately trigger a redundant QPC sample;
- health budgets are absent from physical plan validity;
- full 15-slot scan is absent from release healthy commit;
- producer does not query queue length;
- raw observation is materially smaller/simpler than current rich observation;
- no-allocation tests pass for all new P0 physical plan variants;
- Windows A/B does not regress dispatch-start p99/p99.9 and improves or preserves completion-to-ready tail.
