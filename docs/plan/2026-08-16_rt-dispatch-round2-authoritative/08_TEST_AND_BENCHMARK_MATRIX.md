# 08 — Mandatory Test and Benchmark Matrix

The coding agent does not get to declare success from unit tests alone. This file defines the proof required for each phase.

## 1. Baseline rule

Before the first hot-path implementation PR, capture a reproducible baseline from:

```text
commit: bd5542dd01b6612eea0ebb48c0f6e7a27d8e690e
```

If implementation starts after `main` advances, record both:

```text
plan audit baseline
implementation branch base
```

and rerun the relevant baseline on the actual implementation base before comparing performance.

Record:

- Windows build;
- CPU/model and logical processor count;
- power mode;
- Rust version;
- build profile;
- target architecture;
- focus mode;
- telemetry mode;
- QPC frequency;
- acquired MMCSS label;
- HighQoS success;
- waiter mode;
- spin threshold;
- game FPS/config used by scenario.

Do not compare results from different machines/build profiles as if they were an A/B.

---

# 2. Mandatory repository gates after every implementation phase

Run from repository root with the project's pinned environment/toolchain:

```powershell
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --all-features
cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings
cargo test --manifest-path rust/Cargo.toml --workspace --all-features
uv run pytest -m "not slow"
```

If repository CI defines stricter canonical commands, run those too. Do not weaken lints/test features to make a phase pass.

---

# 3. P0 per-key release scheduler unit tests

Required exact semantic cases from `02_P0_RELEASE_SCHEDULER_AND_FEASIBILITY.md`:

- ordinary Down/Up;
- unrelated delayed Up + independent Down chord;
- feasible same-key Mixed retrigger;
- dynamically infeasible same-key retrigger;
- Up-only metadata boundary with deferred release;
- two pending releases with distinct due times;
- multiple pending releases at identical due time;
- pending release coalesced with next authored Down boundary;
- pending release before a later same-key Down;
- stale Up remains nonphysical;
- no duplicate pending release per slot;
- pending generation always matches active owner.

Assertions must inspect:

```text
selected target
physical masks
number of SendInput calls
commit cursor movement
active generation state
pending release state
release timestamp ordering
```

Do not settle for final “session finished” assertions.

---

# 4. Native hold-feasibility tests

At native/session construction boundary:

1. interval `< effective_min_hold_us` -> reject;
2. interval `== effective_min_hold_us` -> accept;
3. interval `> effective_min_hold_us` -> accept;
4. multiple keys with only one invalid lane -> reject with correct scan code/source evidence;
5. unmatched stale Up does not create a false generation validation failure;
6. timestamp arithmetic overflow -> reject;
7. direct native caller cannot bypass the validator even if Python scheduler normally prevents the case.

Add property tests over randomized valid/invalid key lanes.

---

# 5. P0 stall/backlog tests

Use deterministic test-support clock/emitter seams; do not rely on wall-clock sleeps for correctness assertions.

Required:

- two overdue physical boundaries -> at most first physical send, then abort before second;
- three overdue boundaries -> never drain backlog;
- one isolated overdue boundary -> still allowed when no prior physical send requires a future boundary;
- next target one tick in future -> wait path clears guard and allows send;
- slow first SendInput causes next target to expire -> second send blocked;
- metadata-only frame does not clear guard;
- pending-release Up-only path obeys same guard;
- Mixed/coalesced path obeys same guard;
- interrupt/replan cannot accidentally clear guard;
- supervisor lease expiry remains independently fatal.

Global invariant assertion:

```text
physical SendInput count between two completed future-deadline waits <= 1
```

except multiple logical actions deliberately coalesced into one single `SendInput` transaction at the same target.

---

# 6. Fault-injection matrix

For every DownOnly, UpOnly, Mixed, and coalesced plan shape, inject transport outcomes:

```text
0 inserted
1..N-1 inserted
N inserted
QPC failure before send
QPC failure after send
Win32 short-return error
```

For Down-bearing partial results:

- no logical success commit;
- uncertain physical mask is preserved for cleanup;
- no authored retry;
- terminal cleanup is invoked;
- chord integrity fault recorded.

For Up-only authored/pending release failure:

- no false `Released` generation state;
- cleanup decides physical final state;
- no normal playback retry.

Test every insertion prefix for maximum 30-event Mixed packet in test-support. This is correctness testing; production still performs one syscall attempt.

---

# 7. No-allocation gates

Expand `rust/crates/sky_player_rs/tests/rt_dispatch_no_alloc.rs`.

Counting window must include the new critical operations but exclude harness setup.

Required zero-allocation windows:

- frozen Down plan -> commit -> observation enqueue;
- frozen 15-key Down chord;
- Mixed transaction;
- pending Up-only release;
- coalesced pending Up + authored Down;
- metadata-only deferred-release commit;
- observer queue full/drop-new;
- backlog classifier terminal decision up to error construction boundary if practical (healthy path is the primary zero-allocation requirement).

A failure-path String allocation is acceptable after the decision to terminate; do not contort error reporting to remove rare terminal allocations.

---

# 8. Deterministic target-identity tests

Instrument test seams to count/record target calculations.

Prove for every physical plan:

```text
planner_target == waiter_target == dispatch_plan_target == observation_target
```

No second `epoch + deadline` reconstruction is allowed.

Tests must cover:

- startup target;
- normal Down;
- pending release;
- coalesced boundary;
- pause/resume epoch shift followed by a newly built plan;
- interrupt/replan (old plan target discarded, new plan frozen once).

---

# 9. QPC-read-count tests

Using test-support probes, verify the healthy deadline path does not regress into redundant sampling.

Expected conceptual sequence after the wait refactor:

```text
waiter final deadline QPC sample
(final target/focus checks, no QPC solely for telemetry)
final_admission QPC sample
SendInput
completion QPC sample
```

A focus/lease/control path may require its documented operations, but there must not be an immediate post-wake QPC sample whose only purpose is recomputing `effective_now`.

Diagnostic syscall-entry sampling must be absent in normal production configuration.

---

# 10. Producer queue A/B

Retain/extend `rt_handoff_bench` producer-only comparison.

Compare before/after:

```text
observation construction/copy cost
available queue push
saturated queue drop-new
completion-to-ready handoff
```

Report:

```text
p50
p95
p99
p99.9
max
samples
```

Use at least 10,000 iterations for p99.9 reporting.

The after version must not perform `queue.len()` in producer code.

---

# 11. Fixed-spin benchmark matrix

Benchmark nonshipping comparison modes:

```text
250 µs
400 µs
700 µs
1000 µs
old adaptive policy (optional baseline only)
```

Shipping result remains 700 µs per decision register; benchmark does not auto-select a value.

For each mode record:

```text
wake error p50/p95/p99/p99.9/max
final-admission start error p50/p95/p99/p99.9/max
spin duration/duty cycle
worker CPU
process CPU
interrupt count/correctness
```

Run enough samples to make p99.9 meaningful (>= 10,000 physical deadlines for final acceptance data).

---

# 12. Real Windows SendInput acceptance matrix

Final performance evidence must use the production `SendInput` path on Windows 11, targeting an application/window controlled by the project/test harness. Do not infer timing from mock emitter results.

Scenarios:

### Path shape

```text
DownOnly: 1, 5, 15 keys
UpOnly:   1, 5, 15 keys
Mixed:    representative small and max-size masks
coalesced pending release + Down
```

### Timing density

Use representative and stress gaps, including at least:

```text
10 ms
5 ms
2 ms
1 ms
500 µs synthetic stress where physically meaningful for the app-owned harness
```

The stress harness is for sender performance evidence, not a claim that a game observes sub-millisecond separate notes.

### Environment

Run:

- idle/quiet host;
- moderate CPU background load;
- game/normal application workload if available without modifying/injecting into the game;
- `require_focus=false` and `require_focus=true` as separate datasets.

---

# 13. Required real-host metrics

For each physical send:

```text
physical_target_qpc
wake_qpc
final_admission_qpc
sendinput_completion_qpc
```

Diagnostic A/B may additionally capture:

```text
sendinput_call_qpc
rt_ready_qpc
```

Derive distributions for:

```text
wake_error
wake_to_admission
dispatch_start_error
admission_to_completion
target_to_completion
completion_to_rt_ready (diagnostic)
admission_to_call (diagnostic)
call_to_completion (diagnostic)
```

Also record:

```text
early dispatch count
partial/zero insertion count
observer drops
backlog aborts
cleanup failures
CPU/spin duty cycle
```

---

# 14. Performance acceptance rule

Because Windows host latency is environment-dependent, final acceptance is paired A/B on the same host rather than an invented universal microsecond ceiling.

For five repeated runs per scenario, compare the median run-level statistics.

Mandatory:

1. **No early physical admission:** `final_admission_qpc < physical_target_qpc` count must be zero.
2. **No integrity regression:** no new partial/chord-split/stuck-key failures in clean acceptance runs.
3. **Start tail non-regression:** p99 and p99.9 `dispatch_start_error` may not regress by more than `max(50 µs, 10% of baseline statistic)`.
4. **Completion tail non-regression:** p99/p99.9 target-to-completion may not regress by more than `max(100 µs, 10%)`, unless the scenario intentionally changes release semantics and the difference is fully explained by a distinct physical deadline.
5. **RT-ready objective:** p99 `completion_to_rt_ready` should improve by at least 10% in the hot-path-minimization phase; if baseline values are below reliable measurement resolution, demonstrate non-regression and removal of the specified operations instead.
6. **Fixed spin CPU:** 700 µs fixed policy must not materially increase spin duty cycle versus the old production effective policy; since old policy floor is already 700 µs, an increase requires investigation.
7. **Observer isolation:** artificial slow observer must not change physical target ordering or dispatch-start distribution beyond normal run noise; queue drops are acceptable.

Do not average p99 values across unrelated scenarios.

---

# 15. Soak test

Run a long synthetic schedule with:

- high action count;
- repeated use of all 15 keys;
- pending releases;
- same-target coalescing;
- pause/resume cycles;
- focus transitions in focus-safe mode;
- observer saturation scenario.

Verify:

```text
no generation leak
no pending release leak
no active/possibly-active residue after clean completion
no cursor deadlock
no counter overflow/wrap
no increasing timing drift controller (none should exist)
```

Use existing core soak benchmark/tests where possible rather than introducing another framework.

---

# 16. Documentation/golden-schema tests

Update and test:

- timing semantic labels;
- deprecated compatibility fields;
- telemetry golden schema if field semantics require it;
- Python protocol/stub expectations;
- native acceptance parsing.

A compatibility field that becomes approximate/unavailable must be documented and tested as such.

---

# 17. Evidence artifacts

For every hot-path PR keep machine-readable before/after output outside source logic, with names containing:

```text
commit SHA
Windows build
scenario/profile
before/after label
```

Do not check large transient artifacts into the repo unless current repository policy explicitly expects them. At minimum the PR description/engineering note must record commands and summary statistics so the experiment can be repeated.

---

# 18. Final sign-off checklist

A human/architect reviewer signs off only after confirming:

- P0 semantic tests pass;
- fault matrix passes;
- no-allocation tests pass;
- full Rust/Python gates pass;
- real Windows A/B satisfies the acceptance rules;
- no code path reintroduced learned lead/retry/catch-up behavior;
- normative docs match implementation.
