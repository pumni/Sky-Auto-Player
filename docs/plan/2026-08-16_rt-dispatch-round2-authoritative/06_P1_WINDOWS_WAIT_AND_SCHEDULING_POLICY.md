# 06 — P1: Windows 11 Wait and Scheduling Policy

## 1. Production policy

The shipping Windows physical scheduler for this refactor is fixed:

```text
clock:              QueryPerformanceCounter
kernel wake:        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION
interrupt:          auto-reset Win32 event
final wait:         bounded QPC busy-spin
spin threshold:     700 µs fixed
thread scheduling:  MMCSS Games + AVRT_PRIORITY_HIGH when available
power QoS:          HighQoS / execution-speed throttling disabled
```

None of these mechanisms may change the authored/physical target. They only affect the probability of the dispatch thread being runnable close to that target.

---

# 2. Replace adaptive production spin with fixed 700 µs

Current startup behavior samples wake error 32 times and derives a spin threshold from p95 plus a guard, clamped by a 700 µs floor and 3 ms cap.

Remove that startup sample from production control flow.

Set one production constant:

```rust
pub const PRODUCTION_SPIN_THRESHOLD_US: u64 = 700;
```

Convert it once to `DurationTicks` during worker admission.

Why 700 µs for this refactor:

- it is already the current production minimum after adaptive policy;
- therefore fixing at 700 does not introduce a new larger minimum spin budget;
- it removes host-startup randomness and weak 32-sample p95 control;
- future fixed-threshold tuning can be decided from real p99/p99.9 A/B data.

Do not replace the removed controller with a more sophisticated controller.

---

# 3. Wake probes become diagnostics only

`probe_wake_error_stats()` and robust statistics may remain for:

- explicit benchmark tools;
- diagnostic profile output;
- test-support.

Normal `DispatchSession(profile="production")` must not use wake probe output to set its spin threshold.

If retaining probe code complicates production build substantially, move it behind an appropriate diagnostic/test feature rather than deleting useful benchmark capability.

---

# 4. High-resolution waitable timer is mandatory

Production constructor should make the requirement explicit rather than encoding it as booleans that Python always sets true.

Preferred direction:

```rust
HybridWaiter::new_production() -> Result<HybridWaiter, WaitFailure>
```

or equivalent startup creation that guarantees:

```text
timer.is_some()
event_wait_enabled == true
```

A production worker that cannot create/arm/wait the required timer terminates admission/runtime safely.

Do not silently fall back to:

- `std::thread::sleep`;
- normal-resolution timer;
- busy-spin for the entire remaining interval.

Test-support may keep configurable modes needed by wait tests and A/B benchmarks.

---

# 5. Remove `timeBeginPeriod` from production

`TimerResolutionGuard` is not required by the accepted production architecture.

Current fallback behavior acquires `timeBeginPeriod(1)` only when high-resolution waitable-timer creation fails, while production startup subsequently rejects the missing high-resolution timer anyway.

Refactor so production timer creation failure goes directly to explicit failure without first changing multimedia timer resolution.

If `TimerResolutionGuard` has no remaining legitimate nonproduction user, delete it and remove the WinMM dependency/imports associated with it.

If a benchmark helper still needs it for an explicit comparison, keep it isolated from `new_production()` and label the mode nonproduction.

---

# 6. Keep the hybrid wait algorithm shape

Healthy future physical target:

```text
remaining > spin_threshold
    -> arm high-res relative timer for remaining - spin_threshold
    -> wait on timer + interrupt event

remaining <= spin_threshold
    -> QPC spin until target or interrupt-generation change
```

Keep these good properties:

- target is absolute QPC even though timer arm is relative;
- timer guard is only wake-early budget, not a lead applied to target;
- final QPC comparison decides deadline;
- interrupt can invalidate a future plan;
- once deadline has been reached, deadline result wins at wait layer and final admission still checks commands/target/lease;
- spin loop uses `std::hint::spin_loop()`;
- Win32 event object is not polled on every spin iteration.

The current relaxed interrupt-generation polling every bounded group of iterations is acceptable. Do not change the group size in this refactor unless a dedicated benchmark proves it matters.

---

# 7. Keep QPC as the only physical clock

Do not add:

- `RDTSC`/`RDTSCP`;
- `Instant` as the Windows physical scheduler clock;
- UTC/system-time scheduling;
- core affinity to “stabilize QPC”.

Cache QPC frequency once per worker and perform checked tick arithmetic as today.

QPC includes sleep/standby elapsed time; supervisor lease and overdue-backlog logic must therefore remain prepared to see large jumps after host sleep.

---

# 8. MMCSS policy

Keep default `PriorityMode::Auto` behavior conceptually:

1. try MMCSS task `Games`;
2. set relative `AVRT_PRIORITY_HIGH`;
3. if MMCSS acquisition fails, use the existing bounded `THREAD_PRIORITY_HIGHEST` fallback;
4. restore on guard drop.

Do not add a chain of speculative MMCSS task names without evidence and documentation.

Do not change default to `AVRT_PRIORITY_CRITICAL` in this refactor.

Priority acquisition failure remains observable but should not cause unsafe input state.

---

# 9. Do not default to TimeCritical/realtime scheduling

Keep `PriorityMode::TimeCritical` only if it remains an explicit diagnostic/manual option required by existing tests or tooling.

It must not be selected by production profile.

Do not set process `REALTIME_PRIORITY_CLASS`.

Do not pin the dispatch thread to one core.

The dispatch thread's desirable behavior is:

```text
blocked most of the time
runnable shortly before a deadline
brief bounded execution around SendInput
blocked again
```

not “run at the highest possible priority continuously”.

---

# 10. Keep HighQoS thread power policy

Current `PowerThrottlingGuard` disables `THREAD_POWER_THROTTLING_EXECUTION_SPEED`, which requests HighQoS behavior for the performance-critical dispatch thread.

Keep this behavior and RAII restoration.

Do not extend the refactor into process-wide power-plan manipulation, CPU frequency locking, or vendor-specific performance APIs.

---

# 11. Focus-safe mode is a separate latency profile

When `require_focus=true`, the final target proof may call `GetForegroundWindow()` before final-admission QPC.

This is intentional correctness work.

Benchmark and report two modes separately:

```text
minimum-jitter mode: require_focus=false
focus-safe mode:      require_focus=true
```

Do not compare one mode's p99 against the other's and attribute the difference to timer/spin changes.

---

# 12. Startup behavior

Production startup order should be deterministic:

1. initialize QPC/frequency;
2. initialize backend;
3. acquire HighQoS/MMCSS guards;
4. create mandatory high-resolution waiter/event resources;
5. convert fixed timing constants to ticks;
6. construct coordinator/observer;
7. perform startup/preflight contracts;
8. enter worker loop.

No repeated timing calibration loop is required before playback.

A small future startup anchor/guard may remain if required to ensure the worker is fully admitted before the first authored target; it is not an adaptive lead and must be documented separately from spin threshold.

---

# 13. Tests

Required unit/integration tests:

1. production waiter creation explicitly reports high-res timer failure;
2. production configuration cannot disable event wait;
3. production code does not acquire `TimerResolutionGuard`;
4. production effective spin threshold is exactly 700 µs converted to ticks;
5. wake probe result does not modify production threshold;
6. interrupt during kernel wait replans;
7. interrupt during spin replans while target remains future;
8. deadline wins at wait layer after target is reached;
9. wait runtime failure is terminal;
10. MMCSS guard restoration remains correct;
11. HighQoS guard restoration remains correct;
12. explicit TimeCritical test mode never becomes production default.

---

# 14. Real Windows benchmark

Compare at least these fixed spin thresholds in benchmark tooling:

```text
250 µs
400 µs
700 µs  <- shipping decision
1000 µs
```

Optional: include the old adaptive policy as a nonshipping baseline.

Record:

```text
wake error p50/p95/p99/p99.9/max
start error p50/p95/p99/p99.9/max
spin duty cycle
worker/process CPU
interrupt correctness
```

Do not let benchmark results automatically choose the runtime threshold. A future human/architectural decision may change the constant after evidence review.

---

# 15. Acceptance

This phase is accepted when production wait behavior is deterministic from configuration/build state, has no multimedia timer-resolution fallback dependency, and still uses the documented Windows high-resolution/QPC/MMCSS/HighQoS mechanisms without introducing aggressive priority or affinity hacks.
