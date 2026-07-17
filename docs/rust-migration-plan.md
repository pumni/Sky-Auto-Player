# Rust Dispatch Worker — Migration Plan

> **Status:** Implementation plan, pre-scaffold
> **Last updated:** 2026-07-17 (Phase 5 alignment: emit no-retry §8, single-interval pause §5, drop-oldest §10, focus ownership §Phase 2, invariant coverage §1.3)
> **Decisions recorded:** See [debrief](#d-decision-log) section

**Sections:** [§0 Background](#0-background) · [§1 Goals & Non-Goals](#1-goals--non-goals) · [§2 High-Level Architecture](#2-high-level-architecture) · [§3 Crate Layout](#3-crate-layout) · [§4 PyO3 Module Surface](#4-pyo3-module-surface-pymodule-sky_player_rs) · [§5 RuntimeKernel](#5-runtimekernel-rust-side-hot-state) · [§6 Dispatch Loop](#6-dispatch-loop-rust--worker-thread) · [§7 Wait Strategy](#7-wait-strategy-rust) · [§8 SendInput Wrapper](#8-sendinput-wrapper-rust) · [§9 Adaptive Lead Estimator](#9-adaptive-lead-estimator-rust-port) · [§10 Telemetry Cadence](#10-telemetry-cadence) · [§11 Prewarm Pipeline](#11-prewarm-pipeline) · [§12 Diagnostics Counters](#12-diagnostics-counters-atomicu64) · [§13 Test Strategy & Determinism](#13-test-strategy--determinism) · [§14 Phase Plan](#14-phase-plan-7-phases-810-calendar-days) · [§15 Validation Gates](#15-validation-gates-altitude-table) · [§16 Risks & Mitigations](#16-risks--mitigations) · [§17 Worker Panic Recovery](#17-worker-panic-recovery-pattern) · [§18 File Changes Summary](#18-file-changes-summary) · [§D Decision Log](#d-decision-log).

---

## 0. Background

Sky Player currently runs its real-time dispatch loop entirely in Python (with `ctypes` wrapping `user32.SendInput`, `SetWaitableTimer`, and other Win32 APIs). The Python-side overhead — ctypes argument marshalling, dict-based input-PRNG cache, `_CACHE_LOCK` acquisition, `time.perf_counter_ns()` spin loops, and per-event bookkeeping — accounts for **~5–15 µs/event** of pure send-path latency. While adaptive lead compensates, the cumulative latency sits at **~1–3 ms/song-second** for dense songs, raising the dispatch jitter floor.

> **Baseline numbers:** the figures above are order-of-magnitude estimates. The authoritative
> before/after measurements — dispatch-thread syscalls per send, spin-threshold trajectory,
> telemetry-enabled tail latency — live in `docs/perf-baselines/2026-07-refactor-baseline.md`,
> captured by Phase 0 of the Python core-dispatch refactor (see
> `docs/2026-07_core-dispatch-refactor-and-isolation-plan.md`). That refactor already removed
> several of the per-event costs listed here (mid-play CSV flush, per-send `OpenProcess` focus
> revalidation); re-baseline against that doc before quoting a speedup for the Rust port.

**This plan migrates the entire real-time hot path into a dedicated Rust dispatch worker** (via PyO3 `cdylib`), leaving Python responsible only for:
- Song parsing, AOT scheduling, and intent compilation (`domain/`)
- UI, telemetry rendering, CLI, update checks, watchdog (`ui/`, `cli/`, `watchdog.py`)
- High-level orchestration: focus guard, `RealtimeProcessScope`, gc timing windows
- Profiling and calibration (`calibration.py`)

---

## 1. Goals & Non-Goals

### 1.1 Goals

- **Move the dispatch loop + wait + send to Rust**: only 1 dedicated OS thread handles the hot path.
- **Sub-5 µs/event Rust-side overhead** (from intent fetch → `SendInput` return → telemetry tick).
- **Preserve all existing diagnostics**: partial-send counters, chord-split events, min-same-key-gap, unfocused detection, lead estimator persistence.
- **Achieve parity with golden timelines** (`tests/golden_schedules/*.json`).
- **Maintain free-thread compatibility** (Python 3.14 free-threaded, GIL disabled at runtime).
- **Pass existing tests unchanged** (with bridge fakes replacing Python-only dispatch).

### 1.2 Non-Goals

> [!IMPORTANT]
> **P0 Security Mandate:** Rust code uses **only** `user32.SendInput` for input simulation. No injection, no memory read/write of the game process, no hooks. This is enforced at CI gate; any `windows-sys` import outside `Win32_UI_Input` must be explicitly whitelisted in review.

- **Not** modifying the game window detection logic (`focus.py`, `inputs.get_sky_window`).
- **Not** altering the song parser, scheduler, or note data model.
- **Not** adding new dependencies through `pip` — only `cargo`-managed deps.
- **Not** changing the frontend CLI, Textual UI, hotkey hook, or update workflow.

### 1.3 Core-invariant coverage

The Rust port replaces the seam defined by `docs/2026-07_core-dispatch-refactor-and-isolation-plan.md` §1. Every invariant there must survive the port; this table cites where each is honored (or why it is N/A on the Rust side):

| Inv | Where honored in this plan |
|---|---|
| **I1** SendInput-only, no game-memory/hooks | §1.2 P0 Security Mandate (CI-gated `windows-sys` whitelist). |
| **I2** Completion anchor (`release_not_before = down_completed + min_hold`) | §5 `ActiveGeneration.down_completed_ns` + `PendingRelease.effective_release_us`; never re-anchored to dispatch start. |
| **I3** Musical no *late* retry (note-on immediate same-frame retry; note-off completes) | §8 `emit(.., complete_remainder)` + parity test; §6 call sites. |
| **I4** No-early-conflict guard | §5/§6 `confirm_down_intents` (unchanged coordinator logic ported from `core/coordinator.py`). |
| **I5** Single sender thread | §2 one dedicated worker OS thread; counters are worker-owned atomics. |
| **I6** Adaptive-lead semantics (bucketed EMA + positive residual, `disp_lead>0` overrides) | §9 estimator port (1:1) + §5 `disp_lead_us`/`max_lead_us`. |
| **I7** Watchdog & release-all ladder | N/A to worker — stays in Python (`watchdog.py`, `backend.release_all_full_instrument`); the Rust worker exposes panic/release entry points the Python ladder drives. |
| **I8** Per-thread MMCSS/priority only, always reverted | §7 `realtime/{mmcss,priority}.rs` scoped to the worker thread; no process-class change. |
| **I9** CLI/config/telemetry key stability | §10 `TelemetryRecord` keys mirror Python; §12 counters mirror `get_send_diagnostics()` names. |

---

## 2. High-Level Architecture

```
┌─────────────────────── Python process ─────────────────────────────────────┐
│                                                                             │
│   domain/ (parser, scheduler, runtime_intents)                              │
│        │                                                                    │
│        ▼  compile_runtime_intents(actions) → Vec<IntentDTO>                │
│   orchestration/engine.py — PlaybackEngine.play()                          │
│        │   (prepares bridge, prewarms input cache)                         │
│        ▼                                                                    │
│   orchestration/runtime_dispatch.py — RustBridge class                     │
│        │  ├─ prepare(&mut intents, min_hold_us, config)                    │
│        │  ├─ prewarm(chord_shapes, single_keys)                            │
│        │  ├─ start(telemetry_sink)                                         │
│        │  └─ wait_finished() → str                                         │
│        │                                                                    │
│        └───► sky_player_rs (Rust #[pymodule])                              │
│                    │                                                        │
│                    ▼                                                        │
│                Rust Dispatch Worker                                         │
│                    │  (dedicated OS thread)                                 │
│        ┌───────────┼───────────────┐                                       │
│        ▼           ▼               ▼                                        │
│   WaitStrategy  RuntimeKernel   SendInput                                  │
│   (spin+timer)  (intent_heap)   (cached INPUT array)                       │
│                    │                                                        │
│                    ▼  (push only)                                            │
│                crossbeam::bounded(1024)                                      │
│                    │                                                        │
│                    ▼  (pull — separate Teleporter thread)                   │
│                Python::with_gil(|py| sink.__call__(records))                 │
│                    │                                                        │
│                    ▼                                                         │
│                TelemetryLogger.record_batch                                 │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

The Rust worker is the **sole** sender of `SendInput`. After `play()` the Python main thread blocks on `wait_finished()` (listening for commands on a crossbeam channel and forwarding commands to the worker). No Python threads participant in the dispatch hot path.

---

## 3. Crate Layout

```
rust/
├── Cargo.toml                     # workspace root
├── rust-toolchain.toml            # stable (1.83+), targets = ["x86_64-pc-windows-msvc"]
└── crates/
    ├── sky_player_sys/            # thin safe wrappers around windows-sys
    │   ├── Cargo.toml
    │   └── src/
    │       ├── lib.rs
    │       ├── sendinput.rs       # safe SendInput binding
    │       ├── timer.rs           # CreateWaitableTimerExW + Set + Wait
    │       ├── event.rs           # CreateEventW + Set + WaitForMultiple
    │       ├── thread.rs          # GetCurrentThread, AvSetMm*, priority
    │       └── perf.rs            # QueryPerformanceCounter wrapper
    │
    └── sky_player_rs/             # PyO3 module, owns dispatch worker
        ├── Cargo.toml
        ├── pyproject.toml         # maturin config
        └── src/
            ├── lib.rs             # pymodule init
            ├── py_bridge.rs       # #[pyclass] DispatchHandle + TelemetrySink
            ├── py_types.rs        # #[pyclass] IntentDTO, ConfigDTO, Snapshot
            ├── dispatch/
            │   ├── mod.rs
            │   ├── worker.rs      # thread-fn, command loop, lifecycle
            │   ├── kernel.rs      # RuntimeKernel (state, queue, conflict)
            │   ├── deadline.rs    # next_deadline_us computation
            │   └── intent.rs      # IntentDTO → internal intent decode
            ├── send/
            │   ├── mod.rs
            │   ├── input_cache.rs # prewarmed INPUT[][] keyed on (scan_codes, flags)
            │   ├── emit.rs        # SendInput + partial retry + zero-progress guard
            │   └── diagnostic.rs  # AtomicU64 counters + snapshot
            ├── wait/
            │   ├── mod.rs
            │   ├── strategy.rs    # WaitStrategy trait + implementations
            │   ├── mm_timer.rs    # high-res waitable timer integration
            │   ├── spin.rs        # QPC busy-wait
            │   └── event.rs       # Win32 event + WaitForMultipleObjects
            ├── realtime/
            │   ├── mod.rs
            │   ├── mmcss.rs       # AvSetMmThreadCharacteristics("Pro Audio")
            │   ├── period.rs      # timeBeginPeriod/EndPeriod OnceCell refcount
            │   └── priority.rs    # SetThreadPriority(ABOVE_NORMAL)
            ├── focus/
            │   ├── mod.rs
            │   └── guard.rs       # foreground check, TTL 2 ms cache
            ├── estimator/
            │   ├── mod.rs
            │   ├── ema.rs         # SendLatencyEstimator port (per-bucket EMA + linear RLS)
            │   └── state.rs       # export/import for lead cache
            ├── telemetry/
            │   ├── mod.rs
            │   └── sink.rs        # bounded(1024) MPSC + Teleporter thread → PyO3 sink
            └── common/
                ├── error.rs       # thiserror → PyOSError mapping
                └── debug.rs       # OutputDebugStringW + optional Python log sink
```

**Dependencies:**

| Crate | Version | Purpose |
|---|---|---|
| `pyo3` | 0.23+ (abi3-py310 with conditional abi3-py314 upgrade — see R4) | Python extension bridge |
| `windows-sys` | 0.59+ | Bindings: `Win32_UI_Input`, `Win32_System_Threading`, `Win32_Media`, `Win32_System_Realtime` |
| `crossbeam-channel` | 0.5+ | MPSC command channel, telemetry batch |
| `smallvec` | 1.15+ | Stack-allocated chord batches |
| `once_cell` | 1.20+ | `timeBeginPeriod` refcount |
| `parking_lot` | 0.12+ | `Mutex` + `RwLock` for `InputCache` |
| `serde` / `serde_json` | 1+ | Lead estimator state export |
| `thiserror` | 2+ | Error → PyErr mapping |
| `hashlink` | 0.10+ | LruHashMap for bounded input cache |

**Dev deps:**

| Crate | Purpose |
|---|---|
| `mockall` | Mock SendInput + QueryPerformanceCounter for unit tests |
| `criterion` | micro-benchmarks for emit + deadline + estimator |
| `prop-test` | property-based testing for conflict/resolution logic |

---

## 4. PyO3 Module Surface (`#[pymodule] sky_player_rs`)

### 4.1 `#[pyclass] DispatchHandle`

The single object Python creates and holds per `PlaybackEngine.play()` session.

| Method | Signature | Description |
|---|---|---|
| `prewarm` | `(shapes: list[tuple[list[int], bool]], single: list[int]) -> None` | Warm `InputCache` with forthcoming chords + individual keys |
| `prepare` | `(intents: list[IntentDTO], min_hold_us: int, config: ConfigDTO) -> None` | Build `RuntimeKernel` with sorted intent heap |
| `start` | `(sink: TelemetrySink) -> None` | Spawn worker thread; sets MMCSS + priority |
| `poll_command` | `(cmd: str \| None) -> None` | Send command via crossbeam (non-blocking) |
| `wait_finished` | `() -> str` | Block until worker exits; returns `"finished"\|"quit"\|"skipped"\|"panic"` |
| `cancel` | `() -> None` | Signal early termination |
| `release_all_stuck` | `() -> list[int]` | Sync 3-pass panic release (call after `cancel`) |
| `diagnostics_snapshot` | `() -> dict[str, int]` | AtomicU64 counter snapshot |
| `last_health` | `() -> dict` | Final health state (active, possibly_active, failed_release) |
| `estimator_export` | `() -> str \| None` | JSON lead estimator state for persistence |

### 4.2 `#[pyclass] IntentDTO`

```python
@dataclass
class IntentDTO:
    idx: int                 # global index in action list
    kind: str                # "down" | "up"
    scan_codes: list[int]    # 1..N scan codes in this chord
    at_us: int               # absolute scheduled µs
    source_action_index: int # for telemetry correlation
    reason: str              # action categorization
    generation_id: int       # ties down↔up pair
```

### 4.3 `#[pyclass] TelemetrySink`

```python
class TelemetrySink:
    """Python callable: Rust pushes batched telemetry here ~4 Hz.
    
    Signature: __call__(records: list[TelemetryRecord]) -> None
    """
    def __call__(self, records): ...
```

`TelemetryRecord` is a dict shape with typed keys (lateness_us, send_duration_us, applied_lead_us, etc.). Rust serializes into `Bound<PyDict>` using PyO3 before calling the callback on the Python `executor` thread (see §10).

### 4.4 Module-level free functions

```python
def set_debug_log(callable_or_none) -> None ...  # best-effort log sink
def enable_high_resolution_timers() -> None ...   # timeBeginPeriod refcount
def disable_high_resolution_timers() -> None ...
def reset_window_cache() -> None ...              # cached sky HWND reset
def set_dispatch_lead_us(us: int) -> None ...
def set_onset_bias_us(us: int) -> None ...
```

---

## 5. RuntimeKernel (Rust-side Hot State)

```rust
pub(crate) struct RuntimeKernel {
    // ----- config (set once in prepare) -----
    pub min_hold_us: u64,
    pub total_time_us: u64,

    // ----- timeline state -----
    // Single-interval pause state machine (see the Python core plan §4.2 fix A2). The former
    // dual-anchor (`pause_started_ns` + `focus_pause_started_ns`) double-counted overlapping
    // manual+focus pauses and made elapsed run backwards; DO NOT reintroduce it. One reason
    // set + one anchor + one accumulator: entering pause from an empty set captures the anchor;
    // a second concurrent reason does not move it; only the last exiting reason accumulates the
    // contiguous interval into `pause_accumulated_us` exactly once (attributed to the first
    // reason that opened it).
    pub start_perf_ns: u64,
    pub pause_reasons: PauseReasons,         // bitflags { MANUAL, FOCUS }; nonempty ⇒ paused
    pub pause_interval_started_ns: Option<u64>,  // anchor of the CURRENT contiguous paused interval
    pub pause_open_reason: Option<PauseReason>,  // first reason that opened it (telemetry attribution)
    pub pause_accumulated_us: u64,

    // ----- key tracking -----
    pub active_keys: fxhash::FxHashMap<u16, ActiveGeneration>,
    pub possibly_active: fxhash::FxHashSet<u16>,
    pub failed_release: fxhash::FxHashSet<u16>,

    // ----- intent graph -----
    pub intents: Box<[InternalIntent]>,                 // sorted by at_us
    pub pending_releases: BinaryHeap<PendingRelease>,   // min-heap on eff_release_us
    pub cursor: usize,                                  // next authored intent index

    // ----- adaptive lead -----
    pub estimator: SendLatencyEstimatorPort,
    pub disp_lead_us: u32,
    pub onset_bias_us: u32,
    pub max_lead_us: u32,

    // ----- diagnostics (Ordering::Relaxed atomics) -----
    pub counters: DispatchCounters,
}

struct ActiveGeneration {
    scan_code: u16,
    generation_id: u64,
    down_dispatched_ns: u64,
    down_completed_ns: u64,
}

struct PendingRelease {
    scan_code: u16,
    generation_id: u64,
    down_dispatch_started_us: u64,
    scheduled_release_us: u64,
    effective_release_us: u64,
    source_action_index: u32,
    reason: String,
}

struct DispatchCounters {
    partial_sends: AtomicU64,
    chord_splits: AtomicU64,
    keys_deferred: AtomicU64,
    zero_progress_retries: AtomicU64,
    unfocused_sends: AtomicU64,
    improbable_same_key_repeats: AtomicU64,
    min_same_key_gap_us: AtomicU64,    // sentinel u64::MAX = None
}
```

---

## 6. Dispatch Loop (Rust — Worker Thread)

```
worker_thread(config, kernel, cmd_rx, telemetry_sink):
    // 1. setup
    mmcss::register("Pro Audio")
    // Default MMCSS-managed scheduling is sufficient; do NOT also boost CRITICAL priority class —
    // that would risk priority inversion against the telemetry callback (which must acquire
    // GIL to deliver batches). If profiling shows interference, raise ONLY after Phase 7 sign-off.
    priority::set(THREAD_PRIORITY_NORMAL)
    timer::create_high_res()
    event::create_auto_reset(interrupt_event)
    let mut kernel = kernel

    // 2. outer loop
    loop {
        // 2a. service control state
        match cmd_rx.try_recv() {
            Some("quit")  | Some("skip")  => record telemetry; break;
            Some("pause")                 => enter pause state;
            Some("panic")                 => send::release_all(&mut kernel); enter pause;
            Some("refocus")               => focus::bring_to_foreground();
            None                          => {}
        }

        let now = perf_now_ns();
        let elapsed = elapsed_us(now, &kernel);
        if pause_state_active() {
            sleeper::sleep_1ms();
            continue;
        }

        // 2b. next deadline
        let deadline = next_deadline(&kernel)
            .unwrap_or(kernel.total_time_us)
            .saturating_sub(max_lead(&kernel));
        if elapsed >= deadline {
            drain_due(&mut kernel, elapsed);
            if kernel.cursor >= kernel.intents.len() && kernel.pending_releases.is_empty() {
                record telemetry; break;
            }
            continue;
        }

        // 2c. wait until deadline
        let remaining = deadline - elapsed;
        wait::until(target_us, remaining, &timer_handle, &interrupt_event);
    }

    // 3. teardown
    send::release_all(&mut kernel)
    telemetry::flush()
    event::close()
    timer::close()
    mmcss::revert()
```

`drain_due()`:
```
drain_due(&mut kernel, elapsed) {
    // pending releases first
    while let Some(next) = kernel.pending_releases.peek() {
        if next.effective_release_us > elapsed { break; }
        kernel.pending_releases.pop();
        // Release: key_up=true, complete_remainder=true (finish so no key sticks).
        let res = send::emit(&input_cache, &[next.scan_code], true, true);
        kernel.counters.accumulate(&res);
        kernel.deactivate_key(next.scan_code);
    }

    // authored downs
    while kernel.cursor < kernel.intents.len() {
        let intent = &kernel.intents[kernel.cursor];
        let playable = kernel.confirm_down_intents(intent);
        if playable.is_empty() { kernel.cursor += 1; continue; }
        if playable.scheduled_us > elapsed + kernel.max_lead { break; }

        // Note-on: key_up=false, complete_remainder=false (musical no-retry — drop the tail).
        // Only res.sent (the landed prefix) is activated; res.dropped gens → DROPPED_BACKEND.
        let res = send::emit(&input_cache, &playable.scan_codes, false, false);
        kernel.counters.accumulate(&res);
        kernel.activate_downs(playable, res.sent, now, elapsed);
        kernel.cursor += 1;
    }
}
```

---

## 7. Wait Strategy (Rust)

```rust
pub(crate) trait WaitStrategy: Send {
    fn wait_until_us(
        &self,
        target_us: u64,
        remaining_us: u64,
        elapsed_us: u64,
        timer_handle: HANDLE,
        interrupt_event: HANDLE,
    ) -> bool;  // true = interrupted by command event
}

pub(crate) struct HybridWaitStrategy {
    spin_threshold_us: u64,
}

impl WaitStrategy for HybridWaitStrategy {
    fn wait_until_us(...) -> bool {
        if remaining_us <= spin_threshold_us {
            spin::busy_wait(target_us * 1000);   // QPC nsec loop
            return false;
        }
        let guard = spin_threshold_us;
        let sleep_us = remaining_us - guard;
        timer::set_relative_us(sleep_us as i64);
        let handles = [timer_handle, interrupt_event];
        match event::wait_for_multiple(&handles, INFINITE) {
            0 => { /* timer fired */ },
            1 => { return true; },  // command event
            _ => { /* fallback spin */ },
        }
        spin::busy_wait(target_us * 1000);
        false
    }
}
```

- `spin::busy_wait(target_ns)` uses `while perf_counter_ns() < target {}` with `Relaxed` fence. The hot path is QPC read from kernel32 — no Python GIL interaction at all.
- `event::wait_for_multiple` is a thin safe wrapper over `WaitForMultipleObjects`.
- `timer::set_relative_us` wraps `SetWaitableTimer` with negative (relative) due time.

**For tests / determinism:** a `ManualWaitStrategy` increments a fake QPC counter by `remaining_us + 1` each call, so schedule always finishes without waiting on wall clock.

---

## 8. SendInput Wrapper (Rust)

```rust
pub(crate) struct InputCache {
    map: parking_lot::Mutex<LruHashMap<(u64, u32), Box<[INPUT]>>>,
    // (scan_codes_token, flags) → pre-built INPUT array. Token = 64-bit FNV-1a of scan_codes.
}

pub(crate) struct EmitResult {
    pub sent: Vec<u16>,              // the scan codes that ACTUALLY landed (atomic prefix), NOT the request
    pub skipped_duplicates: Vec<u16>,
    pub dropped: Vec<u16>,           // note-on tail dropped by musical no-retry (invariant I3); empty on releases
    pub partial_send: bool,          // sent < requested
    pub zero_progress_retries: u8,   // 0..3
    pub start_perf_ns: u64,
    pub completed_perf_ns: u64,      // right after the final SendInput returned
}

/// Build KEYBDINPUT struct matching the current Win32 layout.
fn build_key_input(sc: u16, flags: u32) -> KEYBDINPUT {
    KEYBDINPUT {
        wVk: 0,
        wScan: sc,
        dwFlags: KEYEVENTF_SCANCODE | flags,
        time: 0,
        dwExtraInfo: SKY_PLAYER_SIGNATURE,
    }
}

const SKY_PLAYER_SIGNATURE: usize = 0x5C1B9111usize;
```

**Invariant I3 (musical no *late* retry) governs this function.** A partial note-on SendInput is
NEVER completed by a *late* sleeping call. It is retried exactly once immediately. Whatever still fails is DROPPED (an incomplete chord beats a
staggered wrong-timing chord). Only note-off / panic (`complete_remainder = true`) loops to
finish the remainder so keys cannot stick. This mirrors Python `_send_scan_code_batch_impl`;
the coordinator promotes the dropped gens to `DROPPED_BACKEND`.

```rust
pub(crate) fn emit(
    cache: &InputCache,
    scans: &[u16],
    key_up: bool,
    complete_remainder: bool,   // false = musical note-on (drop tail); true = release/panic (finish)
) -> EmitResult {
    if scans.is_empty() { return EmitResult::empty(); }
    let flags = if key_up { KEYEVENTF_KEYUP } else { 0 };
    let arr = cache.lookup(scans, flags)
        .unwrap_or_else(|| build_and_cache(scans, flags));
    let n = arr.len();
    let started_ns = perf_counter_ns();

    // First (and, for note-on, ONLY) SendInput. `landed` is the atomic prefix count.
    let first = unsafe { SendInput(n as u32, arr.as_ptr(), mem::size_of::<INPUT>() as i32) };
    let mut landed = (first as usize).min(n);
    let mut zero_retries = 0u8;

    if landed < n && complete_remainder {
        // Safety path (releases only): finish the remainder — a split release beats a stuck key.
        let mut remaining: &[INPUT] = &arr[landed..];
        loop {
            let sent = unsafe {
                SendInput(remaining.len() as u32, remaining.as_ptr(),
                          mem::size_of::<INPUT>() as i32)
            };
            landed += (sent as usize).min(remaining.len());
            if sent as usize >= remaining.len() { break; }
            if sent > 0 {
                remaining = &remaining[sent as usize..];
                zero_retries = 0;
            } else {
                zero_retries += 1;
                if zero_retries >= 3 { return EmitResult::zero_progress_error(started_ns); }
                spin::busy_wait_ns(2_000);
            }
        }
    }
    // Musical path (note-on): if `landed < n` we STOP here — the tail is dropped, never retried.

    let completed_ns = perf_counter_ns();
    let sent_prefix: Vec<u16> = scans[..landed].to_vec();
    let dropped: Vec<u16> =
        if !complete_remainder { scans[landed..].to_vec() } else { vec![] };
    EmitResult {
        sent: sent_prefix,                 // actually-landed prefix, NOT scans.to_vec()
        skipped_duplicates: vec![],
        dropped,                           // note-on tail dropped by I3; empty on releases
        partial_send: landed != n,
        zero_progress_retries: zero_retries,
        start_perf_ns: started_ns,
        completed_perf_ns: completed_ns,
    }
}
```

> [!IMPORTANT]
> - **No *late* retry (invariant I3):** with `complete_remainder = false`, a partial send is retried immediately exactly once; what still fails is dropped. It issues at most two `SendInput` calls. Applying late sleeping retry to note-on would stagger the chord and is forbidden. The old draft's unconditional `remaining = &remaining[sent..]` loop is deleted.
> - **`sent` is the landed prefix**, not `scans.to_vec()`. The stray post-loop `sent` reference is gone (`landed` is the single source of truth). Callers/telemetry read `EmitResult.sent` as the keys that truly landed and `EmitResult.dropped` as the musical drops.
> - `dwExtraInfo = SKY_PLAYER_SIGNATURE` is constant-backed (same `0x5C1B9111` as Python ctypes). This is required by the game-side dedup logic.
> - **Layout dependency:** `KEYBDINPUT` is a packed C struct with `ULONG_PTR` fields whose width depends on the target arch (4 bytes on i686, 8 bytes on x86_64). Pinning target exclusively to `x86_64-pc-windows-msvc` (via `rust/rust-toolchain.toml` + Cargo target config) prevents accidental cross-compilation that would shift the struct layout and silently break the `dwExtraInfo` alignment. Adding i686 would require a parallel magic-constant strategy (separate `SKY_PLAYER_SIGNATURE` carrier) and is explicitly out of scope.
> - No dedup/duplicate check at the Rust send layer: that's `RuntimeKernel::confirm_down_intents`'s job. The send layer only asserts `scans.dedup()` in debug builds.
> - `Spin::busy_wait_ns(2_000)` in zero-progress matches the Python `_retry_wait_seconds(0.002)` — critical for UIPI elevation-error timing parity.
>
> **Parity test (required):** a note-on whose first `SendInput` lands a strict prefix must DROP the tail AFTER one immediate retry — `emit(.., key_up=false, complete_remainder=false)` returns `sent == total landed prefix`, `dropped == tail`, issues exactly two `SendInput` calls; the coordinator marks the unsent gens `DROPPED_BACKEND` and diagnostics count `keys_dropped`. A note-off with the same partial first send must instead finish the remainder (`keys_retried`), never dropping. This mirrors the Python `tests/test_send_diagnostics.py` coverage of the split-chord path.

---

## 9. Adaptive Lead Estimator (Rust Port)

Port `engine.SendLatencyEstimator` 1:1 into Rust `estimator/`:

```rust
pub(crate) struct SendLatencyEstimator {
    alpha: f64,
    max_lead_us: u32,
    max_poly: u8,
    seed_samples: u8,     // = 5

    // per-bucket EMA
    count: Vec<u32>,
    sum: Vec<u64>,
    ema: Vec<f64>,
    warm: Vec<bool>,

    // total fallback
    count_total: u64,
    sum_total: u64,
    ema_total: f64,

    // linear RLS with exponential forgetting (lin_forget = 0.999)
    lin_count: u64,
    lin_w: f64,
    lin_sx: f64,
    lin_sxx: f64,
    lin_sy: f64,
    lin_sxy: f64,

    // up path (single scalar EMA)
    count_up: u64,
    sum_up: u64,
    ema_up: f64,
}
```

Algorithm identical to Python version:
- `update(kind, duration_us, n_keys)`: folds into per-bucket EMA, total fallback, and linear RLS accumulators.
- `get_lead_us(kind, n_keys) → u32`: bucket chain (exact → linear → nearest ≤ n → total → 0).
- `export_state() → serde_json::Value`: same JSON schema as `engine.SaveLatencyEstimator.export_state()` (version 1, same field names).
- `import_state(&mut self, json)`: best-effort seed (same validation + range checking).

**Seeding strategy for parity tests (deterministic warm-sample phase):**

The Python reference uses `statistics.mean` over a fixed-size sample window when `n_samples < seed_samples` (= 5). To get 1-µs tolerance:

1. Both implementations receive the **same** `(kind, duration_us, n_keys)` tuples in identical order.
2. The samples used in tests are recorded from an actual `SendInput`-instrumented run, captured by Python (`SendLatencyEstimator.export_state()`) into `tests/golden/estimator_seed_v1.json`.
3. Rust unit tests deserialize that JSON and replay it via `update(...)`; this guarantees both sides start with identical accumulator state.
4. Floating-point equality is asserted bitwise: `f64.to_bits()` match. `f64` arithmetic is deterministic across `x86_64-pc-windows-msvc`; we do not compare `1 µs` rounded, but bit-exact. Tests skip on different target triples.
5. **Numerical jitter caveat:** when both sides process RLS linear regression accumulators (`lin_sx`, `lin_sxx`, ...) the order of additions matters. Tests pre-allocate 2000 input tuples and feed them in linear order. Future splitter-path fuzzing is explicitly out of scope (R3 below).

**Test parity:** Golden test feeds same `(kind, duration_us, n_keys)` samples to both Python and Rust estimators; asserts `get_lead_us` output matches **bitwise** (within f64 ULP = 0).

---

## 10. Telemetry Cadence

The Rust worker publishes telemetry **~4 Hz** (every ~250 ms of *elapsed playback time*, not wall time) to a bounded crossbeam channel (capacity 1024). The sink flushes:

```
Worker thread ┌── push(TelemetryBatch) ──► crossbeam::bounded(1024) ─► Teleporter thread
              │                                                     │
              │                                                     ▼
              │                              Python::with_gil(|py| sink.__call__(records))
              │                                                     │
              │                                                     ▼
              │                                            TelemetryLogger.record_batch
              │
              └─ (counter reads on demand via diagnostics_snapshot())
```

- **Threading model:** the **worker thread owns** `push()` (non-blocking). A **dedicated Teleporter thread** spawned by `DispatchHandle.start()` owns `pull()` + the PyO3 GIL callback. This isolation keeps the dispatch hot path free of any GIL acquisition, even if Python is rendering Textual at 60 Hz.
- **Cadence:** the worker invokes `channel_push()` from its spin loop whenever `now - last_push >= 250 ms` (elapsed playback time). The Teleporter pulls opportunistically — it does not block.
- **Bounded channel:** prevents Rust writer from unbounded heap growth if Python telemetry consumer is slow. Capacity 1024. **Drop policy: drop the *oldest* pending batch on overflow (drop-oldest = FIFO drop).** The earlier "LIFO drop" wording was self-contradictory — dropping the oldest entry is FIFO by definition. Rationale: on overflow the freshest/late-tail batches carry more diagnostic value than stale mid-playback batches that the consumer already fell behind on, so we evict from the head.
- **TelemetryRecord:** lightweight dict with keys `event_index`, `kind`, `lateness_us`, `send_duration_us`, `applied_lead_us`, `sent_scan_codes`, `runtime_outcome`, `elapsed_us`.
- **Budget per batch flush:** ≤ 200 µs measured end-to-end inside the Teleporter thread (`GIL acquire + serialize records + sink.__call__`). Verified in Phase 3 §10 via `criterion`.
- **4 Hz cadence:** drives ~4 renders/s for the UI progress bar + diagnostics dashboard.

**Diagnostic counters** (partial send events, chord splits, etc.) are **not** sent per-batch; they are read on-demand via `DispatchHandle.diagnostics_snapshot()` (which does `load(Relaxed)` on each `AtomicU64`). The engine calls this snapshot once at the end, and optionally mid-play for the dashboard.

---

## 11. Prewarm Pipeline

```rust
impl InputCache {
    pub fn prewarm(&self, shapes: &[(Vec<u16>, bool)]) {
        for (scans, key_up) in shapes {
            let flags = if *key_up { KEYEVENTF_KEYUP } else { 0 };
            let arr = build_input_array(scans, flags);
            let token = hash(scans);
            let mut map = self.map.lock();
            if map.len() >= MAX_CACHE_SIZE { map.pop(); }  // LRU eviction
            map.insert((token, flags), arr);
        }
    }
}
```

Python calls `bridge.prewarm(...)` once, before `bridge.start(...)`. The Rust worker reuses the cache without any lock for *reads* (the cache is populated before the worker thread starts). This design avoids runtime mutex contention on the hot path: the `InputCache` is owned by the `Worker` Arc and is never mutated after `start()`. If a cache miss occurs during playback (unexpected batch shape), it is built on-the-fly under a `Mutex`.

---

## 12. Diagnostics Counters (AtomicU64)

Six counters replace the Python module-level globals:

| Python global | Rust `DispatchCounters` field | Description |
|---|---|---|
| `_PARTIAL_SEND_EVENTS` | `partial_sends` | `SendInput` returned sent < requested |
| `_CHORD_SPLIT_EVENTS` | `chord_splits` | Partial send with n > 1 |
| `_SEND_KEYS_DEFERRED` | `keys_deferred` | Total keys deferred across all partial sends |
| `_ZERO_PROGRESS_RETRIES` | `zero_progress_retries` | `SendInput` returned 0 (count of retry cycles) |
| `_SEND_WHILE_UNFOCUSED` | `unfocused_sends` | Keys sent while foreground ≠ Sky |
| `_IMPOSSIBLE_SAME_KEY_REPEATS` | `improbable_same_key_repeats` | Same-key UP→DOWN interval < min_hold |
| `_MIN_SAME_KEY_UP_GAP_US` | `min_same_key_gap_us` | Minimum observed gap (u64::MAX = None sentinel) |

All operations use `Ordering::Relaxed` — the counters are diagnostic-only and do not gate any correctness path. `diagnostics_snapshot()` collects them with `load(Relaxed)` and returns a Python dict.

---

## 13. Test Strategy & Determinism

### 13.1 Rust Unit Tests (cargo test)

- **`send/tests/`**: mock `SendInput` via a `MockSendInput` trait (behind `#[cfg(test)]`). Test zero-progress path (3 retries → OsError), partial send, empty batch, duplicate detection.
- **`wait/tests/`**: mock `QueryPerformanceCounter` to advance a fake clock. Test spin-then-sleep, event wake, timer-only wake.
- **`estimator/tests/`**: bit-exact (f64 `.to_bits()`) parity vs golden data generated by Python `SendLatencyEstimator.export_state()` (see §9 seeding strategy).
- **`dispatch/tests/`**: inject a small schedule (e.g., 2 chords + 3 single notes); run with `ManualWaitStrategy`; verify that `emit()` is called with correct scan codes at correct times.
- **`kernel/tests/`**: test conflict resolution, pending release sorting, same-key generation tracking.

### 13.2 Python Integration Tests (pytest)

All existing tests remain. A new fixture `rust_bridge` provides `DispatchHandle` with `ManualWaitStrategy`. The test suite does NOT import `sky_player_rs` directly; instead, `conftest.py` has:

```python
# In conftest.py — hard ImportError from phase 0
try:
    import sky_player_rs as _rs
except ImportError:
    raise ImportError(
        "sky_player_rs wheel not found. Run `scripts/build_rust_wheel.py` first."
    )
```

- `test_rust_bridge.py`: minimal lifecycle test (prepare → prewarm → start → wait_finished).
- `test_rust_estimator_parity.py`: verifies Rust estimator matches Python golden output.
- `test_engine_rust_equivalence.py`: runs full `PlaybackEngine.play()` with Rust bridge and asserts `final_result` + `diagnostics_snapshot()` match Python baseline (within tolerance).
- `test_golden_regression.py`: updated to run with Rust bridge; golden files unchanged.
- Legacy tests (`test_threaded_dispatch.py`, `test_runtime_dispatch.py`): refactored to use the same `RustBridge` path (Pythons `Engine` calls straight into Rust `DispatchHandle`; no separate Python runtime path.)

### 13.3 Determinism Guard

`ManualWaitStrategy` in Rust:
```rust
#[cfg(test)]
pub(crate) struct ManualWaitStrategy {
    pub fake_perf_ns: Arc<AtomicU64>,
}

impl WaitStrategy for ManualWaitStrategy {
    fn wait_until_us(...) -> bool {
        self.fake_perf_ns.fetch_add(remaining_us * 1000, Relaxed);
        false
    }
}

#[cfg(test)]
pub(crate) fn current_fake_perf_ns() -> u64 {
    // returned by perf_counter_ns() when ManualWaitStrategy is installed
}
```

All Rust tests compile with `cfg(test)` and link against mock `SendInput`/`QPC`. The Python-side bridge starts tests by passing `ManualWaitStrategy` config.

---

## 14. Phase Plan (7 Phases, 8–10 Calendar Days)

Each phase has a **Gate** (command-level pass/fail) AND a **Behavioral Exit Criterion** (observable property of the system that must hold after the phase). A phase is complete only when both are met.

| Phase | Gate | Behavioral Exit Criterion |
|---|---|---|
| 0 | `cargo check && uv run --with maturin maturin develop && uv run python -c "import sky_player_rs"` | `import sky_player_rs` succeeds under free-threaded 3.14. `maturin develop` produces a `.pyd` that PyInstaller can locate (verified later in Phase 5). |
| 1 | `cargo test && uv run pytest tests/test_rust_send.py && uv run ruff check .` | A round-trip `bridge.send_scan_code_batch_trusted(scans=[...])` from Python returns identical `EmitResult` shape to the legacy ctypes path within 5 µs. Mock `SendInput` test asserts 3-zero-progress → error, partial-send continuation, LRU eviction. |
| 2 | `cargo test && uv run pytest tests/test_realtime_scope.py && uv run ruff check .` | `enable_high_resolution_timers()` increments refcount → `QueryPerformanceFrequency` reports ≥ 1 MHz. `set_debug_log(callable)` captures worker `OutputDebugStringW` redirects when Python callback is installed. |
| 3 | `cargo test && uv run pytest tests/test_rust_bridge.py && uv run pytest -x tests/test_rust_estimator_parity.py` | A miniature 3 s schedule completes deterministically under `ManualWaitStrategy`. `get_lead_us` returns bitwise-equal f64 vs Python golden. Teleporter flush budget ≤ 200 µs/batch. |
| 4 | `uv run pytest tests/test_engine_rust_equivalence.py && uv run python -m app --selftest-textual` | `PlaybackEngine.play()` end-to-end with `RustBridge`; `final_result` and `diagnostics_snapshot()` parity within tolerance of Python baseline. `--selftest-textual` exits 0. |
| 5 | `uv run python scripts/build_rust_wheel.py && uv run --env-file .env python scripts/audit_free_threaded_wheels.py && uv run python -m build_app` | `dist/Sky-Player.exe --selftest-textual` passes against the bundled `.pyd`. `audit_free_threaded_wheels.py` reports `cp314t` wheel importable. |
| 6 | `uv run pytest && uv run ruff check .` | All legacy diagnostic callers (`get_send_diagnostics`, `reset_send_diagnostics`, module globals) are gone. CI search `grep -r 'CACHE_LOCK\|InputCache\|partial_sends' src/` returns 0 hits except in `runtime_dispatch.py` RustBridge and deprecation warning docs. |
| 7 | `uv run ruff check . && uv run pyright && uv run pytest && uv run python scripts/build_rust_wheel.py && uv run --env-file .env python scripts/audit_free_threaded_wheels.py && uv run python -m build_app` | `scripts/measure_dispatch_tail.py` shows tail latency ≤ 50 µs/event for the longest song in `songs/`. Golden regression (5 schedules) all pass. `docs/INDEX.md` lists rust-migration-plan.md as the canonical migration doc. |

**Rollback rule:** if a phase fails its behavioral criterion, do NOT start the next phase. Re-plan the failed phase or insert a sub-phase.

### Phase 0 — Scaffold Rust Workspace (0.5 day)

**Toolchain setup (decided up-front, not later):**

* Rust toolchain pinned via `rust/rust-toolchain.toml`: `channel = "stable"`, `targets = ["x86_64-pc-windows-msvc"]`. No i686 / GNU profile — `KEYBDINPUT.dwExtraInfo` layout depends on this (see V14 rationale inline in §8).
* `maturin` is added to `[dependency-groups.dev]` in `pyproject.toml` as `maturin>=1.7,<2.0`. Install flows through `uv sync`, never `pip install maturin` and never `cargo install maturin` (both bypass uv-managed deps and violate AGENTS.md "Dependency management — use only `uv sync` / `uv add` / `uv add --dev`").
* Rust toolchain itself is installed **once per dev machine** via `rustup-init.exe` per the team onboarding doc. It is not a workspace dep. CI installs it via `rust-lang/rustup` GitHub Action.

**Actions:**

1. Create `rust/Cargo.toml` (workspace with `members = ["crates/*"]`).
2. Create `rust/rust-toolchain.toml` with `targets = ["x86_64-pc-windows-msvc"]`.
3. Create `rust/crates/sky_player_sys/` (safe wrappers around `windows-sys` items — `SendInput`, timer, event, QPC, MMCSS).
4. Create `rust/crates/sky_player_rs/` (PyO3 `#[pymodule]` with no-op `DispatchHandle`).
5. Add `maturin>=1.7,<2.0` to `[dependency-groups.dev]` in `pyproject.toml`. Run `uv sync` once to install it into `.venv`.
6. Verify `cargo check` → clean.
7. Verify `uv run --with maturin maturin build` (pre-release minor wheel) → installs into `.venv`.
8. Verify `import sky_player_rs` → succeeds in free-threaded Python 3.14.

**Gate:** `cargo check && uv run --with maturin maturin develop && uv run python -c "import sky_player_rs" && uv run pytest --selectable terse`

### Phase 1 — Port SendInput + Cache (1.5 days)

**Actions:**
1. Implement `send/input_cache.rs` (LRU `HashMap<(u64,u32), Box<[INPUT]>>`).
2. Implement `send/emit.rs` per §8: single note-on `SendInput` with **immediate same-frame retry** (invariant I3 — one retry, then drop unsent tail); note-off/panic completes the remainder with the zero-progress guard. Not a blanket sleeping "partial-retry" loop.
3. Implement `send/diagnostic.rs` (AtomicU64 counters + snapshot).
4. Implement `py_types.rs` + `py_bridge.rs` — export `send_scan_code_batch_trusted(_)` as a module-level function for backward compat.
5. **Add Python-side `try: import sky_player_rs`** + hard ImportError to `platform/win32/inputs.py`. Keep existing ctypes functions as dead code (not removed yet — removal in Phase 6).
6. Write Rust unit tests: mock SendInput, test partial/zero paths, test cache LRU eviction.
7. Write `test_rust_send.py` (Python-side smoke: import bridge, call `send_scan_code_batch_trusted`, verify return signature; mock user32.dll for full isolation).

**Gate:** `cargo test && uv run pytest tests/test_rust_send.py && uv run ruff check .`

### Phase 2 — Port Wait + Realtime (1 day)

**Actions:**
1. Implement `wait/strategy.rs`, `wait/mm_timer.rs`, `wait/spin.rs`, `wait/event.rs`.
2. Implement `realtime/mmcss.rs`, `period.rs`, `priority.rs`.
3. Implement `perf.rs` in `sky_player_sys` (safe `QueryPerformanceCounter` + `QueryPerformanceFrequency`).
4. Implement `focus/guard.rs` as a **cheap foreground-HWND compare only** (`GetForegroundWindow() == sky_hwnd`) with the 2 ms cache — the same TTL as Python `DispatchHealthMonitor._focus_cache_ttl_us`. **Do NOT port process-name revalidation** (`GetWindowThreadProcessId` + `OpenProcess` + `QueryFullProcessImageNameW`) into the worker — see the focus-ownership note below.
5. Implement `common/debug.rs` (forward to Python `debug_log` callback if set).
6. Export new functions to Python: `enable_high_resolution_timers()`, `disable_high_resolution_timers()`, `set_debug_log()`.
7. Rust unit tests for wait strategy, timer events, focus cache.

> **Focus ownership (single source of truth — decided here).** There is exactly one focus
> *policy* owner: the **Python supervisor-side guard** (`infrastructure/focus.FocusGuard` +
> `PlaybackSupervisor`). It runs the full process-name validation at the human-facing 20–50 ms
> sampling cadence and decides pause/resume. The **Rust worker does not duplicate that guard**;
> it consumes a shared focus flag (the Rust equivalent of `SharedFocusSignal`, published by the
> supervisor's sample) plus the cheap foreground-HWND compare from the Python core plan §5.2 for
> the per-send pre-gate. This deletes the two-sources-of-truth problem the earlier draft had
> (`focus/guard.rs` re-running process-name checks in parallel with Python `focus.py`). The cheap
> HWND compare is safe because a live HWND cannot change the process behind it; staleness is
> bounded by the supervisor's full-check cadence.

**Gate:** `cargo test && uv run pytest tests/test_realtime_scope.py && uv run ruff check .`

### Phase 3 — Port RuntimeKernel + Loop (2.5 days — largest phase)

**Actions:**
1. Implement `dispatch/kernel.rs`: `RuntimeKernel`, `ActiveGeneration`, `PendingRelease`, `InternalIntent`.
2. Implement `dispatch/intent.rs`: decode `IntentDTO` list into sorted `Box<[InternalIntent]>`.
3. Implement `dispatch/deadline.rs`: `next_deadline(kernel, lead_down, lead_up) → Option<u64>`.
4. Implement `dispatch/worker.rs`: the full thread function with command channel, pause/skip/quit/panic handling.
5. Implement `dispatch/worker.rs` — the outer loop from §6.
6. Implement `estimator/` (full port of Python `SendLatencyEstimator`).
7. Wire `DispatchHandle.prepare()`, `.start()`, `.poll_command()`, `.wait_finished()`, `.cancel()`, `.release_all_stuck()`, `.estimator_export()`.
8. Implement `TelemetrySink` — bounded crossbeam channel, 4 Hz flush, PyO3 callback with GIL.
9. Rust integration tests: run a miniature 3-second schedule end-to-end with `ManualWaitStrategy`; verify emit history matches expected.
10. Python `test_rust_bridge.py`: full lifecycle test with dry-run schedule.

**Gate:** `cargo test && uv run pytest tests/test_rust_bridge.py && uv run pytest -x tests/test_rust_estimator_parity.py`

### Phase 4 — Wire Python Orchestration (1 day)

**Actions:**
1. Add `RustBridge` class in `orchestration/runtime_dispatch.py` (facade over `DispatchHandle`).
2. In `engine.py`, replace the `DispatchLoop.run()` → supervisor path with:
   ```python
   bridge = RustBridge()
   bridge.prepare(intents_dto, min_hold_us, config)
   bridge.prewarm(shapes, single_keys)
   bridge.start(telemetry_sink)
   result = bridge.wait_finished()
   diag = bridge.diagnostics_snapshot()
   # read estimator state for caching
   ```
3. Optionally keep `DispatchLoop` Python-only for debug (`use_rust=True` default, never fallback).
4. Refactor `PlaybackSupervisor` — it no longer spawns `DispatchLoop` on a thread. Its job is now the pause/focus command handling **in the Python main thread** (forwarding commands via `bridge.poll_command()`).
5. Refactor `RealtimeProcessScope`: keep `gc.pause()`, but remove MMCSS thread setting for the Python dispatch thread (no longer exists). MMCSS is now set by the Rust worker.

**Gate:** `uv run pytest tests/test_engine_rust_equivalence.py && uv run python -m app --selftest-textual`
### Phase 5 — Build Pipeline Hook (0.5 day)

> **AGENTS.md compliance note.** This phase inserts a Rust build step **without renumbering** the existing Build Environment steps in `AGENTS.md`. The new step is a *precheck* that runs only when a `rust/` directory exists; on pure-Python branches of the pipeline it is a no-op. AGENTS.md P1 (`pyproject.toml`, CI commands) remains authoritative on numbering.

**Actions:**

1. Create `scripts/build_rust_wheel.py`:
   ```python
   #!/usr/bin/env python3
   """Build Rust wheel and install into the active uv-managed venv.

   Idempotent: skips if `rust/` directory does not exist (pure-Python branch).
   Called as a precheck before `audit_free_threaded_wheels.py` in the release pipeline.
   """
   import subprocess
   import sys
   import pathlib

   ROOT = pathlib.Path(__file__).resolve().parent.parent
   RUST_DIR = ROOT / "rust"
   if not RUST_DIR.exists():
       print("[build_rust_wheel] no rust/ directory; skipping (pure-Python branch).")
       sys.exit(0)

   MANIFEST = RUST_DIR / "crates" / "sky_player_rs" / "Cargo.toml"

   subprocess.run(
       [sys.executable, "-m", "maturin", "build", "--release",
        "--out", str(ROOT / "dist"),
        "--manifest-path", str(MANIFEST)],
       cwd=ROOT, check=True,
   )

   wheels = sorted((ROOT / "dist").glob("sky_player_rs-*.whl"))
   wheel_path = wheels[-1]
   if wheel_path.suffix == ".whl":
       # Use `uv pip install` so the wheel is visible to all uv-managed resolution paths.
       subprocess.run(
           ["uv", "pip", "install", "--reinstall", str(wheel_path)],
           cwd=ROOT, check=True,
       )
   elif wheel_path.suffix == ".pyd":
       # Dev build via `maturin develop` (editable link) is preferred for that flow; omitted here.
       raise SystemExit(f"Unexpected artifact: {wheel_path}")
   ```

2. Update `pyproject.toml`:
   * Add `maturin>=1.7,<2.0` to `[dependency-groups.dev]`.
   * Add a `[tool.uv.sources]` section pointing to local wheel for reproducible resolution:
     ```toml
     [tool.uv.sources]
     sky_player_rs = { path = "dist/sky_player_rs-0.1.0-*.whl" }
     ```
   * Note: the source glob is best-effort; rebuild in CI uses `maturin build` then `uv pip install` (not the source path).

3. Update `Sky-Player.spec`:
   * Add `sky_player_rs*.pyd` to `binaries` list (matches PyInstaller's automatic path pattern).
   * Keep `onedir` COLLECT strategy unchanged.

4. Update `AGENTS.md` Build Environment section:
   * Add a **precheck step** (do NOT rename existing steps):
     > *Before step 1 (cache location note), if the workspace contains a `rust/` subdirectory, run:*
     > ```powershell
     > uv run python scripts/build_rust_wheel.py
     > ```
     > *This precheck is a no-op on pure-Python branches. Step 1 (`audit_free_threaded_wheels.py`) stays as step 1.*
   * The smoke test gate remains the final gate (unchanged).
   * Add note under precheck: *"Requires Rust toolchain matching `rust/rust-toolchain.toml`. See onboarding doc for `rustup-init.exe` instructions."*

**Gate:** `uv run python scripts/build_rust_wheel.py && uv run --env-file .env python scripts/audit_free_threaded_wheels.py && uv run python -m build_app`

### Phase 6 — Telemetry Snapshot + Cleanup (1 day)

**Actions:**
1. Replace calls to `inputs.get_send_diagnostics()` with `bridge.diagnostics_snapshot()` in:
   - `orchestration/dispatch_loop.py` — removed entirely (this module stays as test-only artifact).
   - `orchestration/engine.py` — `play()` endpoint now reads Rust counters.
2. Remove Python module globals in `platform/win32/inputs.py`:
   - `_PARTIAL_SEND_EVENTS`, `_CHORD_SPLIT_EVENTS`, `_SEND_KEYS_DEFERRED`, `_ZERO_PROGRESS_RETRIES`, `_SEND_WHILE_UNFOCUSED`, `_IMPOSSIBLE_SAME_KEY_REPEATS`, `_MIN_SAME_KEY_UP_GAP_US`.
   - `_CACHE_LOCK`, `_INPUT_CACHE`, `_ARRAY_CACHE`.
   - `reset_send_diagnostics()`, `get_send_diagnostics()`, `set_schedule_diagnostics()`, `note_send_while_unfocused()`.
3. Remove `send_scan_code_batch_trusted()` and `send_scan_code_batch()` — these are exclusively called from Rust now.
4. Keep `get_sky_window()`, `is_sky_active()`, `focusWindow()`, `is_virtual_key_down()`, `describe_input_target()`, and the debug_log infrastructure — they are called from Python focus guard and CLI, not from the hot path.
5. Rename `platform/win32/inputs.py` to `platform/win32/window.py`? No — keep the file but strip send-input functions.

**Gate:** `uv run pytest && uv run ruff check .`

### Phase 7 — Final Validation (1 day)

**Actions:**
1. Run full pipeline: build wheel → audit → build_app → smoke test.
2. Measure tail latency with `scripts/measure_dispatch_tail.py` (compare vs baseline).
3. Run `tests/test_golden_regression.py` with all 5 golden schedules.
4. Run CLI playback of longest song in `songs/` — verify diagnostics diff.
5. Remove all dead Python dispatch code:
   - `orchestration/dispatch_loop.py` (or archive to `docs/archive/`).
   - `dispatch_loop.py` imports removed from `engine.py` and `tests/conftest.py`.
   - `orchestration/playback_supervisor.py` — strip `DispatchLoop`, `CommandEvent`, `thread.join()` logic (Python side only forwards commands now).
6. Update `docs/INDEX.md` to list `rust-migration-plan.md`.
7. Final commit.

**Gate:** `uv run ruff check . && uv run pyright && uv run pytest && uv run python scripts/build_rust_wheel.py && uv run --env-file .env python scripts/audit_free_threaded_wheels.py && uv run python -m build_app`

*Note:* `scripts/build_rust_wheel.py` is a precheck (skips when `rust/` is absent on pure-Python branches). On Rust-enabled branches, AGENTS.md Build Environment precheck step runs it before step 1 (audit) — see Phase 5 §4.

## 15. Validation Gates (Altitude Table)

| Change scope | Command |
|---|---|
| Rust only (no Python change) | `cargo check && cargo test --all-features` |
| Rust + Python bridge | `uv run pytest tests/test_rust_bridge.py tests/test_rust_estimator_parity.py` |
| Full playback end-to-end | `uv run pytest tests/test_engine_rust_equivalence.py tests/test_golden_regression.py` |
| Full pre-flight | `uv run ruff check . && uv run pyright && uv run pytest` |
| Release pipeline | `uv run python scripts/build_rust_wheel.py` (precheck, no-op if no `rust/`) → then `uv run --env-file .env python scripts/audit_free_threaded_wheels.py` (AGENTS.md step 1) → `uv run python -m build_app` (AGENTS.md step 3) |

---

## 16. Risks & Mitigations

| # | Risk | Likely-hood | Impact | Mitigation |
|---|---|---|---|---|
| R1 | Rust worker `panic!` during playback → thread death, keys stuck | Low (all error paths use `Result`) | Medium (keys stuck, user must restart) | **Recovery block pattern** in `worker.rs`: `let result = std::panic::catch_unwind(AssertUnwindSafe(|| { /* spin + drain_due */ }))`. The `catch_unwind` boundary is drawn **outside** any `unsafe`/extern call, **before** `SendInput`. On `Err`: set `interrupt_event` (wakes `wait_finished`), push `WorkerPanicked` to channel, attempt worst-effort `release_all` via a fresh Rust thread (not the panicked thread — DOH!). Python `cancel()` then routes to watchdog subprocess for full re-sync. Never let panic propagate through `extern "system"` (UB). |
| R2 | `QueryPerformanceCounter` vs Python `time.perf_counter_ns()` drift over long song | Low (< 1 µs drift per 10 min) | Low (single song < 10 min) | Sync baseline at `start()`. Enable telemetry report on final mismatch. |
| R3 | PyO3 GIL acquire in telemetry callback causes 2–5 µs spike on main thread | Medium | Low | GIL lock is every 250 ms only; the callback does a fast `TelemetryLogger.record_batch()` that's < 10 µs. Acceptable. |
| R4 | `maturin` build fails on free-threaded 3.14 due to PyO3 ABI3 wheel | Medium | High | Phase 0 prototype: `cargo build --release` with `abi3-py310` against target free-threaded 3.14. PyO3 ≥ 0.23 supports free-threaded when built against `pyo3-ffi` matching the host Python's GIL status (CPython 3.13t+ requires the `Py_IsFinalizing` / `Py_mod_gil` shim). **Verified upstream:** PyO3 0.23 release notes (2024-10) state free-threaded is supported as experimental and requires `extension-module` + `pyo3` cfg. If `abi3-py310` extension-module fails to import under 3.14t, **fallback path** is to build a per-minor `cp314t-py314` wheel via `maturin build --target-dir` (NOT `abi3`). Document the chosen path in Phase 0 §Actions step 7. |
| R5 | `PyInstaller` cannot relocate the Rust `.pyd` with `onedir` | Low | High | Test bundling in Phase 3 (before full pipeline); adjust `Sky-Player.spec` `binaries` list. `pyi_rth_pkgutil` may need registration. |
| R6 | Legacy Python dispatch code paths diverge and are never tested | Medium | Low (no legacy) | Decision: **no fallback** at all. `import sky_player_rs` is required. Python dispatch code is removed in Phase 6. |
| R7 | Lead estimator golden test parity fails due to float rounding differences | Low | Low | Accept 1 µs tolerance; round-to-nearest matches Python `round()` (banker's rounding = round half even). |
| R8 | `timeBeginPeriod` refcount needs thread-safe + lazy lifecycle (Begin on first call, End on last), not a single-shot `OnceCell` | Low | Medium | `realtime/period.rs` uses `AtomicUsize` refcount guarded by a `parking_lot::Mutex` for the `Begin→End` transition. On enter: `fetch_add(1, AcqRel)`; if `prev == 0`, call `timeBeginPeriod`. On exit: `fetch_sub(1, AcqRel)`; if `new == 0`, call `timeEndPeriod`. The `Mutex` is held only during the winmm call (<< 1 µs), uncontended in steady state. |
| R9 | Crossbeam channel capacity 1024 too small for dense 10-min song at 200 events/s → all batches dropped | Low | Low | 4 Hz flush × 250 s = 1000 pushes; 1024 exactly fits. If exceeded, oldest drop is acceptable (telemetry still shows `dropped: N` in final snapshot). |
| R10 | `InputCache` LRU evictions during playback cause on-the-fly build | Low | Low (single build per batch shape = ~3 µs) | Prewarm covers every distinct chord shape via `bridge.prewarm()` + single keys. Miss only happens on corrupt/unexpected data. |

---

## 17. Worker Panic Recovery Pattern

`catch_unwind` across FFI boundaries is **undefined behavior** if a panic propagates through an `extern "system"` frame. The recovery block must be drawn such that:

1. The unwind boundary lives entirely in safe Rust: the closure passed to `catch_unwind` does **not** call `SendInput` while unwinding.
2. After `UnwindSafe` boundary, the cleanup path uses a **newly-spawned** OS thread (calling `release_all`), so a poisoned `parking_lot::Mutex` does not lock up the cleanup.
3. The crossbeam command channel receives a `WorkerPanicked` event so `wait_finished` returns `"panic"`.

```rust
use std::panic::{catch_unwind, AssertUnwindSafe};

let outcome = catch_unwind(AssertUnwindSafe(|| {
    worker_main_loop(&mut kernel, &timer, &interrupt, cmd_rx, telemetry_sink)
}));

match outcome {
    Ok(reason) => emit_finished(reason),
    Err(_) => {
        // Spawn a fresh thread for cleanup — the panicked thread is poisoned.
        let panic_sender = panic_sender.clone();
        std::thread::Builder::new()
            .name("sky-player-panic-cleanup".into())
            .spawn(move || {
                let _ = panic_sender.send(WorkerExit::Panicked);
                let _ = release_all_stuck_keys();
            })?;
        // Wake any waiters.
        set_event(&interrupt_event);
    }
}
```

`AssertUnwindSafe` is correct here because we do not access aliased mutable state across the boundary — `kernel`'s `&mut` does not cross.

---

## 18. File Changes Summary

### Created

| File | Phase |
|---|---|
| `rust/Cargo.toml` | 0 |
| `rust/rust-toolchain.toml` | 0 |
| `rust/crates/sky_player_sys/Cargo.toml` | 0 |
| `rust/crates/sky_player_sys/src/lib.rs` | 0 |
| `rust/crates/sky_player_sys/src/sendinput.rs` | 0 |
| `rust/crates/sky_player_sys/src/timer.rs` | 0 |
| `rust/crates/sky_player_sys/src/event.rs` | 0 |
| `rust/crates/sky_player_sys/src/thread.rs` | 0 |
| `rust/crates/sky_player_sys/src/perf.rs` | 0 |
| `rust/crates/sky_player_rs/Cargo.toml` | 0 |
| `rust/crates/sky_player_rs/pyproject.toml` | 0 |
| `rust/crates/sky_player_rs/src/lib.rs` | 0 |
| `rust/crates/sky_player_rs/src/py_bridge.rs` | 1 |
| `rust/crates/sky_player_rs/src/py_types.rs` | 1 |
| `rust/crates/sky_player_rs/src/send/input_cache.rs` | 1 |
| `rust/crates/sky_player_rs/src/send/emit.rs` | 1 |
| `rust/crates/sky_player_rs/src/send/diagnostic.rs` | 1 |
| `rust/crates/sky_player_rs/src/wait/strategy.rs` | 2 |
| `rust/crates/sky_player_rs/src/wait/mm_timer.rs` | 2 |
| `rust/crates/sky_player_rs/src/wait/spin.rs` | 2 |
| `rust/crates/sky_player_rs/src/wait/event.rs` | 2 |
| `rust/crates/sky_player_rs/src/dispatch/kernel.rs` | 3 |
| `rust/crates/sky_player_rs/src/dispatch/intent.rs` | 3 |
| `rust/crates/sky_player_rs/src/dispatch/deadline.rs` | 3 |
| `rust/crates/sky_player_rs/src/dispatch/worker.rs` | 3 |
| `rust/crates/sky_player_rs/src/estimator/ema.rs` | 3 |
| `rust/crates/sky_player_rs/src/estimator/state.rs` | 3 |
| `rust/crates/sky_player_rs/src/telemetry/sink.rs` | 3 | Changed: adds Teleporter thread (separate from worker). Worker only `push`es; Teleporter pulls + calls PyO3 sink. |
| `rust/crates/sky_player_rs/src/realtime/mmcss.rs` | 2 |
| `rust/crates/sky_player_rs/src/realtime/period.rs` | 2 |
| `rust/crates/sky_player_rs/src/realtime/priority.rs` | 2 |
| `rust/crates/sky_player_rs/src/focus/guard.rs` | 2 |
| `rust/crates/sky_player_rs/src/common/error.rs` | 0 |
| `rust/crates/sky_player_rs/src/common/debug.rs` | 0 |
| `scripts/build_rust_wheel.py` | 5 |
| `tests/test_rust_bridge.py` | 3 |
| `tests/test_rust_estimator_parity.py` | 3 |
| `tests/test_rust_send.py` | 1 |
| `tests/test_engine_rust_equivalence.py` | 4 |
| `.gitignore` — add `rust/target/` | 0 |

### Modified

| File | Phase | Change |
|---|---|---|
| `src/sky_music/platform/win32/inputs.py` | 1 | Add `try: import sky_player_rs` + hard ImportError; keep functions |
| `src/sky_music/platform/win32/inputs.py` | 6 | Remove send functions, cache, diagnostics globals |
| `src/sky_music/orchestration/dispatch_loop.py` | 4 | Deprecate (mark only test usage) |
| `src/sky_music/orchestration/dispatch_loop.py` | 7 | Remove file (or archive) |
| `src/sky_music/orchestration/engine.py` | 4 | Wire `RustBridge` into `play()` |
| `src/sky_music/orchestration/engine.py` | 7 | Remove `_compat_*` shims, `_build_dispatch_loop` |
| `src/sky_music/orchestration/playback_supervisor.py` | 4 | Strip thread spawning; keep command forwarding only |
| `src/sky_music/orchestration/runtime_dispatch.py` | 4 | Add `RustBridge` class |
| `src/sky_music/infrastructure/backend.py` | 4 | **`WinSendInputBackend` is frozen**: its public surface stays intact (`is_dry_run` parity, panic release, debug log forwarding, `_TrackedKeyState` filtering), but the actual `SendInput` call inside `_emit` becomes a forwarding shim to `RustBridge.send_now(...)`. New tests cover both `DryRunBackend` (pure in-process) and the no-op path; hot-path tests use `RustBridge` directly. This prevents divergence between two abstraction layers (avoid V9 — see Phase 6 cleanup window). |
| `src/sky_music/infrastructure/wait_strategy.py` | 4 | Add `RustHybridWaitStrategy` thin wrapper |
| `src/sky_music/infrastructure/realtime.py` | 4 | Remove MMCSS thread setting (Rust worker owns it) |
| `src/sky_music/orchestration/telemetry.py` | 4 | Add `TelemetrySink` handler |
| `pyproject.toml` | 5 | Add `[tool.uv.sources]` for Rust wheel |
| `Sky-Player.spec` | 5 | Add `sky_player_rs.pyd` to binaries |
| `AGENTS.md` | 5 | Add pre-step `build_rust_wheel.py` to Build Environment |
| `docs/INDEX.md` | 7 | Add `rust-migration-plan.md` reference |
| `.gitignore` | 0 | Add `rust/target/` |

### Removed

| File | Phase |
|---|---|
| `src/sky_music/orchestration/dispatch_loop.py` | 7 (or archive to `docs/archive/`) |

### Preserved (no changes to core logic)

- `domain/` (analyzer, parser, scheduler, scheduler_types, domain models) — all pure Python, no Rust dependency.
- `ui/` (Textual app, HUD, pickers) — telemetry ingress unchanged; `TelemetrySink` replaces direct dispatch loop call.
- `cli/` (console playback, calibration, doctor) — no hot path.
- `infrastructure/hotkeys.py`, `hotkey_hook.py` — no change.
- `watchdog.py` — no change.
- `scripts/audit_*`, `scripts/mem_*` — no change.
- All tests except the 3 new ones remain structurally unchanged (some refactored in Phase 4 to use `RustBridge` fixture).

---

## D. Decision Log

| Decision | Choice | Rationale |
|---|---|---|
| Migration scope | **C — Full Rust dispatch worker** | Heat (send + wait + runtime) moves to Rust. Python keeps orchestration/UI. |
| Build stack | **PyO3 + maturin** (`abi3-py310` first; `cp314t-py314` fallback verified in Phase 0) | Most community support, abi3 reduces wheel count, free-thread compatible. |
| Legacy fallback | **None** — hard `ImportError` from phase 0 | Cleaner architecture; no dual path to maintain; Rust is mandatory. |
| Wheel strategy | **cdylib (`.pyd`) bundled via PyInstaller `onedir` COLLECT** | Compatible with current build env; no external exe; same smoke test gate. |
| Telemetry cadence | **Batched 4 Hz via bounded MPSC (1024)** | Sufficient for UI dashboard; minimal GIL contention; LIFO drop prevents unbounded growth. |
| Diagnostics counters | **AtomicU64 (Relaxed) + snapshot accessor** | No locks in hot path; race-safe; diagnostics are best-effort. |
| Pre-scaffold gate | **Write plan first, then scaffold** | Ensuring decisions are documented before implementation. |
| Rust edition | **2024** (stable) | Modern Rust, `impl Trait`, `let-else` |
| Test determinism | **`ManualWaitStrategy` with fake QPC** | All Rust tests are deterministic; Python tests use `RustBridge` with manual strategy fixture. |
| Error mapping | **`thiserror` → `PyOSError`** | Every Rust error path maps 1:1 to Python `OSError` or subclass; same error messages as ctypes version. |
