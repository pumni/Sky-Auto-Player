# 10 — Files / Functions Touch Map

This map tells the coding agent where the planned changes belong. It is not permission for unrelated cleanup.

Paths are based on audit baseline `bd5542dd01b6612eea0ebb48c0f6e7a27d8e690e`.

---

# 1. `rust/crates/sky_dispatch_core/src/compile.rs`

## Keep

- one open generation per physical slot;
- reject overlapping same-key Down;
- same timestamp: 0..N Up actions + at most one Down chord;
- canonical Up metadata before Down metadata;
- compact masks/intents.

## Change only if needed by validator metadata

If native hold validation needs a reliable Up source action index for each owned Up intent, add the minimum compact metadata necessary.

Do not change authored chord grouping or introduce runtime timing policy into compiler.

---

# 2. `rust/crates/sky_dispatch_core/src/model.rs`

Potential additions:

- compact authored-frame metadata needed by validator;
- fixed-size/source-index fields if compile output lacks sufficient identity.

Do not put Win32 `INPUT`, HWND, SendInput result, or QPC API bindings in core model.

---

# 3. New/appropriate `sky_dispatch_core` schedule-validation module

Preferred location:

```text
rust/crates/sky_dispatch_core/src/validation.rs
```

or an existing timing-validation module if one already owns this responsibility.

Add:

```text
validate_min_hold_feasibility(schedule, effective_min_hold_us)
ScheduleTimingError::SameKeyHoldTooShort { ... }
```

Keep pure, deterministic, Windows-free.

---

# 4. `rust/crates/sky_dispatch_core/src/coordinator/mod.rs`

Add coordinator state:

```text
pending_release_by_slot: [Option<PendingRelease>; MAX_KEYS]
pending_release_mask: u16
```

Add `PendingRelease`/commit-token core types in the most cohesive coordinator submodule.

Do not use HashMap/BinaryHeap for pending releases.

---

# 5. `rust/crates/sky_dispatch_core/src/coordinator/timeline.rs`

## Remove as production concept

Packet-wide deadline calculation based on max release floor:

```text
packet_release_floor_ticks()
packet_effective_deadline_ticks_uncompensated()
```

or refactor their remaining compatibility callers so authored frame deadline is immutable authored time.

## Add/query

- authored frame target;
- earliest pending release due target;
- fixed-slot due mask helper if timeline ownership fits here.

Do not create a global learned/rebased target.

---

# 6. `rust/crates/sky_dispatch_core/src/coordinator/authored.rs`

Major P0 work:

- replace `prepare_current_authored_packet()` semantics with prepared authored-frame classification;
- classify immediate/deferred/stale Up;
- detect `deferred_up_mask & down_mask` infeasibility;
- register deferred releases on frame commit;
- add metadata-only frame commit;
- add pending release prepare/commit;
- support coalesced commit token;
- advance authored cursor once per authored frame;
- remove full 15-slot release-build validation from each successful commit;
- preserve touched-slot generation checks.

Keep full invariant validation accessible for tests/finalization.

Temporary old helpers may exist only during migration and must be removed in Phase 10.

---

# 7. `rust/crates/sky_dispatch_core/src/coordinator/tests/authored.rs`

Replace/add tests for:

- unrelated deferred Up no longer shifts Down;
- feasible/infeasible retrigger;
- pending release lifecycle;
- metadata-only deferred frame;
- equal/distinct release due targets;
- coalesced commit;
- cursor advancement;
- touched-slot masks.

Any old test asserting packet-wide release-floor delay for unrelated Down must be changed to the new invariant, not preserved as compatibility behavior.

---

# 8. `rust/crates/sky_dispatch_core/tests/properties.rs`

Add property tests:

- pending release identity equals active generation;
- no physical Up before due;
- unrelated Down target remains authored;
- authored Down chord mask is never split;
- every successful generation ends in exactly one release;
- native hold validator accepts/rejects correct boundary conditions.

Preserve existing lifecycle/canonical order properties.

---

# 9. `rust/crates/sky_player_rs/src/python/conversion.rs`

Current `validate_schedule_timing()` only checks final timestamp arithmetic.

Refactor it to call the new pure core validator after effective native hold is known, or move validation call to `python/session.rs` if that is the cleaner ownership point.

Do not duplicate a second same-key validator in Python-facing Rust code.

---

# 10. `rust/crates/sky_player_rs/src/python/session.rs`

Production construction changes:

- invoke native hold feasibility validation using final `effective_min_hold_us`;
- production wait policy no longer sets adaptive-spin controller booleans;
- fixed 700 µs policy is internal Rust production policy, not a Python tuning knob;
- preserve stable public config where practical.

Do not expose low-level thread priority or spin tuning to ordinary Python API as part of this refactor.

---

# 11. `rust/crates/sky_player_rs/src/engine/config.rs`

Refactor:

- replace `DEFAULT_SPIN_THRESHOLD_US=150` + adaptive production floor/controller with clear `PRODUCTION_SPIN_THRESHOLD_US=700`;
- remove production adaptive-spin option;
- simplify wait options so mandatory production timer/event are not contradictory booleans;
- keep test-support wait tuning separately;
- keep strict diagnostic profile distinct from structural-fault policy.

Avoid creating a large config matrix.

---

# 12. `rust/crates/sky_player_rs/src/engine/worker/planning.rs`

Major P0/P1 work:

- select earliest of authored frame and pending release;
- equal-target coalescing;
- map selected logical target to one absolute QPC target;
- introduce typed `NextDispatchPlan` variants;
- physical plan has nonoptional prepared packet and commit token;
- metadata plan has nonoptional metadata token;
- remove `authored_budget`/health validity;
- remove correlated Option validity checker after migration;
- prepare Win32 packet before wait.

Planning may scan 15 pending slots. It must not perform I/O/Win32 focus queries except through the existing preflight layer after the logical/physical plan is built.

---

# 13. `rust/crates/sky_player_rs/src/engine/worker/admission.rs`

Keep core ordering:

- final command precheck;
- final target generation/focus proof for Down;
- one final-admission QPC sample;
- lease classification from that sample.

Rename/comment semantics to `final_admission_qpc` consistently.

Do not add a second production pre-SendInput QPC read here.

Keep focus query before final-admission QPC.

---

# 14. `rust/crates/sky_player_rs/src/engine/worker/wait.rs`

Refactor API:

- take exact frozen `physical_target_qpc` for physical wait;
- lease bounds the wait wake only;
- stop reconstructing target from playback epoch + deadline;
- preserve deadline-vs-interrupt ordering;
- return exact same target identity to dispatch.

Metadata boundaries may use their own frozen target variant but must follow the same single-target rule.

---

# 15. `rust/crates/sky_player_rs/src/engine/worker/dispatch_loop.rs`

Major integration work:

- consume typed plan variants;
- process metadata-only boundary without SendInput;
- wire pending release/coalesced physical plans;
- add `awaiting_future_physical_boundary` guard;
- reject second overdue physical send;
- clear guard only after future target deadline wait;
- remove redundant post-deadline QPC read;
- use `WaitResult::wake_qpc` handoff;
- do not recreate physical target.

Keep outer control/focus/pause/lease orchestration outside precision region.

---

# 16. `rust/crates/sky_player_rs/src/engine/worker.rs`

Add worker runtime state only if owned here:

```text
awaiting_future_physical_boundary
last_successful_physical_completion_qpc (optional scalar diagnostic)
```

Refactor worker timing state:

- fixed production spin ticks;
- remove adaptive production controller state;
- remove retry timing state if proven dead in Phase 10.

Do not add shared locks.

---

# 17. `rust/crates/sky_player_rs/src/engine/worker/dispatch/mod.rs`

Refactor public/internal dispatch structs:

- physical plan/commit token integration;
- nonoptional prepared packet path;
- compact raw observation exports for test-support;
- remove old AuthoredBatchView Option fields after migration.

Keep platform/core separation.

---

# 18. `rust/crates/sky_player_rs/src/engine/worker/dispatch/authored.rs`

Refactor to a thin physical executor:

```text
consume frozen physical plan
final admission
backend SendInput
validate transport result
minimal commit
strict-only terminal predicate if enabled
raw observation push
return
```

Remove:

- schedule re-query;
- packet Option checks that types can eliminate;
- health-budget validity;
- nonessential timing derivation;
- legacy authored retry logic if any remains reachable.

Pending Up-only physical plans may share the same executor if that reduces duplication without creating path-specific ambiguity.

---

# 19. `rust/crates/sky_player_rs/src/engine/worker/dispatch/timing.rs`

Move pure derived telemetry calculations toward observer side.

Keep only producer-side tick arithmetic required by:

- coordinator commit;
- release floor;
- structural ordering check;
- strict diagnostic terminal predicate.

Rename `result_started_ticks`/similar internals to admission semantics.

Remove unused `_health` parameters and legacy helpers when callers are migrated.

---

# 20. `rust/crates/sky_player_rs/src/engine/worker/dispatch/observer.rs`

Refactor:

- consume compact raw observation;
- own health threshold lookup/windows;
- derive start/completion/wake metrics;
- materialize compatibility records;
- compute approximate queue depth/high watermark consumer-side if retained;
- no effect on physical authorization.

Keep observer isolated on its own thread.

---

# 21. `rust/crates/sky_player_rs/src/engine/worker/dispatch/observation.rs`

Expected to change significantly or be simplified into raw observation definitions.

Remove duplicated derived fields and static threshold copies from producer payload.

Keep only immutable evidence the observer cannot safely reconstruct.

---

# 22. `rust/crates/sky_player_rs/src/engine/worker/health.rs`

Remove `FrozenDispatchBudget` from physical planning.

Keep health windows/threshold policy in observer domain if still useful.

Do not delete diagnostics wholesale.

---

# 23. `rust/crates/sky_player_rs/src/engine/worker/boot.rs`

Refactor startup:

- fixed 700 µs spin conversion;
- remove production adaptive wake-probe controller;
- mandatory production waiter constructor;
- preserve QPC/MMCSS/HighQoS initialization;
- preserve observer startup;
- no production `timeBeginPeriod` fallback.

Remove initialization of timing fields made dead by P2.

---

# 24. `rust/crates/sky_player_rs/src/engine/worker/startup.rs`

Keep RAII scheduling guards.

Update waiter construction to explicit production mode/result.

Do not add process-wide priority/power changes.

---

# 25. `rust/crates/sky_dispatch_win32/src/wait/hybrid.rs`

Refactor:

- production constructor requires high-res timer + event;
- no `TimerResolutionGuard` on production failure;
- fixed threshold supplied by worker;
- keep current timer-then-spin algorithm;
- keep interrupt generation optimization;
- keep test/benchmark configurable modes separately.

---

# 26. `rust/crates/sky_dispatch_win32/src/timer.rs`

Keep `WaitableTimer` high-resolution implementation.

Remove `TimerResolutionGuard` if no legitimate nonproduction caller remains after waiter refactor.

Do not replace waitable timer with multimedia timer.

---

# 27. `rust/crates/sky_dispatch_win32/src/wait.rs` and `wait/spin.rs`

Keep `WaitResult.wake_qpc` and bounded spin semantics.

Potentially add test-support counters for QPC read-count acceptance, but do not burden production state.

---

# 28. `rust/crates/sky_dispatch_win32/src/input/packet.rs`

Keep:

- prebuilt `[INPUT; 30]`;
- Up-before-Down order;
- scan-code keyboard input;
- `time=0`;
- one production SendInput call;
- QPC completion sample;
- GetLastError only for short result.

Rename supplied start parameter/comments to final-admission semantics.

Optional diagnostic syscall-entry QPC capture belongs behind test/diagnostic support, not default production.

---

# 29. `rust/crates/sky_dispatch_win32/src/input/tracked/*`

Update physical/backend state application to new commit/plan flow while preserving uncertain masks on partial/QPC-after-send failure.

Do not let backend independently mutate coordinator generation state.

Audit legacy retry fields in Phase 10.

---

# 30. `rust/crates/sky_dispatch_win32/src/mmcss.rs`

Expected semantic change: none.

Keep MMCSS Games/High + safe fallback + RAII restoration.

Only update comments/tests if production policy documentation needs clarification.

---

# 31. `rust/crates/sky_dispatch_win32/src/power.rs`

Expected semantic change: none.

Keep current HighQoS execution-speed throttling opt-out and restoration.

---

# 32. `rust/crates/sky_player_rs/tests/rt_dispatch_no_alloc.rs`

Expand with new plan variants and compact observation producer path.

This is a mandatory acceptance test, not optional cleanup.

---

# 33. `rust/crates/sky_player_rs/examples/rt_handoff_bench.rs`

Update to benchmark:

- fixed spin thresholds;
- new physical plan variants;
- compact observation push;
- completion-to-ready handoff;
- no producer queue-length read.

Keep it dependency-light unless current repository benchmark conventions require otherwise.

---

# 34. `scripts/bench_native_acceptance.py`

Update only as needed to expose new semantic/compatibility metrics and run fixed policy A/B.

Do not put runtime controller logic into benchmark script.

---

# 35. Python scheduler files

Relevant:

```text
src/sky_music/domain/scheduler.py
src/sky_music/domain/scheduler_types.py
src/sky_music/domain/hold_timing.py
```

Do not redesign scheduler generation in this overhaul unless a native-validator regression reveals an actual contract mismatch.

Python already performs same-key feasibility reasoning. Native validator is defense in depth and must agree with the final effective hold contract.

If tests reveal Python/native hold values differ, fix the materialization contract explicitly; do not weaken native validation.

---

# 36. Normative docs

Update after code is implemented:

```text
docs/timing-principles.md
docs/hold-frame-model.md
docs/rt-dispatch-architecture.md
docs/architecture.md
```

Key wording changes:

- release floor is per generation;
- compiled same-timestamp packet is not a packet-wide delayed deadline;
- pending releases may be physically separated when deadlines differ;
- atomic Down chord never splits;
- structural backlog abort prevents catch-up bursts;
- production spin is fixed 700 µs;
- final-admission timestamp semantics are explicit.

---

# 37. Explicit non-goal areas

Do not modify unless a required build fix is unavoidable:

- updater crate;
- unrelated UI rendering;
- song parsing formats;
- installer/release packaging;
- network code;
- game integration beyond existing SendInput/focus boundary;
- calibration provenance protocol except compatibility wording about what calibration can influence.

Any unavoidable cross-area change must be minimal and called out in the phase PR.
