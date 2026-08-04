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

Worker orchestration, estimators, coordinators, and Python boundaries must remain 100% safe Rust. All `unsafe` blocks must be prefixed with a `// SAFETY: ...` explanation.

## Test-Support Boundary

- Test-support APIs (mock senders, fault injection, command timing inspection) must be strictly feature-gated (`#[cfg(any(test, feature = "test-support"))]`).
- Do not include test-support logic, telemetry serialization, or mock state in production structures when the feature is disabled.
