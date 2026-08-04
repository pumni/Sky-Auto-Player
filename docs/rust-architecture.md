# Rust Core Architecture

## Crate Dependency Direction

The workspace consists of three core crates that follow a strict one-way dependency graph:

```text
sky_dispatch_core
        ↑
sky_dispatch_win32
        ↑
sky_player_rs
        ↑
Python application
```

- `sky_dispatch_core`: Pure scheduling/domain logic. Must not import Win32 or PyO3.
- `sky_dispatch_win32`: Windows/QPC/SendInput platform adapter. Must not import PyO3.
- `sky_player_rs`: Runtime orchestration and Python FFI boundary. Only this crate may import PyO3.

## Module Ownership

- Each state must have exactly one module owner.
- A module should be extracted when it owns a distinct state, protects a distinct invariant, acts as a platform/test boundary, has a distinct lifecycle, or represents a cohesive capability with a small internal API.
- Do not create utility catch-all files (`helpers.rs`, `utils.rs`, `common.rs`).
- Use facade modules (e.g., `engine.rs` exposing `engine/worker.rs`) to keep public paths stable.

Current stable facades and ownership boundaries:

- `sky_player_rs::engine.rs` owns only module declarations and stable
  re-exports; session lifecycle is in `engine/session.rs`.
- `engine/shared.rs` owns the cross-thread command, target, lifecycle, metrics,
  telemetry, and completion resources shared by a session and its worker.
- `SessionShared` groups those resources by capability (`commands`, `target`,
  `lifecycle`, and `publication`); it does not add nested `Arc` ownership for
  fields that already live for the session.
- `engine/worker/{admission,cleanup,control,estimator,health,startup,timing,wait}.rs`
  own focused invariant and phase boundaries. Terminal cleanup, command
  control, and wait-boundary orchestration currently use narrowly scoped
  capability inputs; the remaining orchestration loop is still being reduced
  to owner methods. `WorkerCore` owns mutable metrics, runtime, health, timing,
  error, and dispatch resources outside the panic boundary. Down/Up
  transaction extraction remains gated on a complete worker-state owner so
  operation order cannot change.
- `sky_player_rs::lib.rs` registers the Python module; Python-facing conversion,
  session, and telemetry code live under `python/`.
- `sky_dispatch_win32::input.rs` and `wait.rs` are facades for their platform
  submodules. `input/raw.rs` owns packet/syscall results while
  `input/{down_transaction,up_transaction}.rs` own bounded transaction policy;
  tracked masks remain in `input/tracked.rs`. Raw SendInput and timer unsafe
  boundaries remain platform-owned.

### State ownership table

| State/capability | Owner | Readers | Mutation boundary |
| --- | --- | --- | --- |
| Session commands | `SessionShared::commands` | session, worker | atomics and command-timing guard |
| Target stamp | `SessionShared::target` | session, admission | generation publication and acquire loads |
| Lifecycle/completion | `SessionShared::lifecycle` | session, worker | lifecycle atomics and completion condvar |
| Published metrics/telemetry | `SessionShared::publication` | session, worker | publication locks and atomics |
| Playback clock | `WorkerCore::resources.playback` | worker phases | worker thread only |
| Backend masks | `TrackedKeyState` | dispatch, cleanup | worker thread only |
| Coordinator | `WorkerCore::resources.coordinator` | dispatch, cleanup | worker thread only |
| Telemetry ring | `WorkerCore::resources.telemetry` | worker | worker thread; serialized after finish |
| Estimator | `WorkerCore::resources.estimator` | dispatch, cleanup | worker thread only |
| Local health/timing/error state | `WorkerCore::{health,timing,errors}` | worker phases | worker thread only |

Adding worker state requires assigning it to one `WorkerCore` capability before
it is read by a phase. Adding a phase requires documenting the invariant it
owns, its allowed inputs, and its exact position relative to focus checks,
preflight, SendInput, coordinator commit, telemetry, and cleanup. Timing-boundary
changes require a dedicated regression test and a baseline/candidate benchmark.

## Public API Boundaries

- `pub`: Only for types strictly crossing the crate boundary.
- `pub(crate)`: Only when multiple high-level module trees require access.
- `pub(super)`: Preferred for child/sibling interactions.
- Private: Default visibility. Do not globally `#![allow(unreachable_pub)]`.

## Hot-Path Constraints

The healthy Down/Up dispatch path must not introduce:
- Heap allocations (`Box`, `Vec`, `String` formatting).
- Lock acquisitions (`Mutex`, `RwLock`).
- Additional QPC calls or system time requests.
- Dynamic dispatch (`Box<dyn Trait>`).
- Cross-thread channel communication on the latency-sensitive path.
- Reference counting clones (`Arc::clone`, `Rc::clone`).

## Unsafe Boundary

`unsafe` blocks are strictly forbidden outside explicitly approved Win32 platform modules. Approved locations:
- `sky_dispatch_win32/input/raw.rs`
- `sky_dispatch_win32/input/physical.rs`
- `sky_dispatch_win32/wait/timer.rs`
- `sky_dispatch_win32/wait/hybrid.rs`

Worker orchestration, estimators, coordinators, and Python boundaries must remain 100% safe Rust. All `unsafe` blocks must be prefixed with a `// SAFETY: ...` explanation.

## Test-Support Boundary

- Test-support APIs (mock senders, fault injection, command timing inspection) must be strictly feature-gated (`#[cfg(any(test, feature = "test-support"))]`).
- Do not include test-support logic, telemetry serialization, or mock state in production structures when the feature is disabled.

## Architecture Checker

Run `uv run python scripts/check_rust_architecture.py --enforce` from the
repository root. The checker reports file sizes, public-item counts,
unsafe-boundary violations, PyO3-boundary violations, forbidden lower-layer
imports, and temporary architecture debt. Enforced violations fail CI; a
temporary allowlist entry must name its reason and expiry phase.
