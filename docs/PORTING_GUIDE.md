# `PORTING_GUIDE.md` — Python → Rust Core Engine Conversion Standard

> **Purpose.** Define the technical standard, architectural layout, and translation matrix for the incremental migration of CPU-heavy modules from Python into a Rust Core Engine exposed through PyO3/Maturin. This guide is the reference rulebook for engineers and Multi-Agent AI workflows when restructuring Sky Auto Player for performance, without crossing any security boundary.

This document is independent of `AGENTS.md`. Where instructions conflict, `AGENTS.md` wins (Priority Stack P0–P2). See §6 for the Sky Auto Player-specific constraints that bind this guide.

---

## 1. Architectural Principles

Porting is not a 1-to-1 syntax translation. It is a migration from a **dynamic, GC-based** memory model to a **static, ownership-based** one.

### 1.1. Separation of Concerns — Strangler Fig Pattern

The system keeps three layers with clear responsibility boundaries:

* **Presentation / Routing Layer — Python.** HTTP request entry points, edge validation, authentication, routing. **No heavy compute.**
* **Application / Business Logic Layer — Rust Core.** All CPU-bound tasks, large-data structure processing, algorithms, complex encode/decode, high-performance I/O.
* **FFI Boundary — PyO3.** The thinnest possible layer. Only type conversion between Python objects and Rust native values. **No business logic at the binding layer.**

### 1.2. Production-Ready Standards

* **Zero-Cost Abstractions.** Prefer static dispatch (generics / trait bounds) over dynamic dispatch (`Box<dyn Trait>`) unless runtime polymorphism is genuinely required.
* **Memory By Design.** Minimize heap allocations. Reuse references (`&str`, `&[u8]`) and zero-copy into Rust from Python through `PyBuffer` or `numpy` slices.
* **Security & Safety.** **`unsafe` is forbidden** in business logic. `unsafe` may only appear when calling third-party C-FFI and must be encapsulated inside a safe API with a comment explaining why the memory layout is sound.

---

## 2. Translation Matrix

### 2.1. Data Types & Ownership

Core rule: **Do not use `.clone()` to placate the borrow checker.** Every `.clone()` must have an architectural justification (e.g., truly independent ownership of the value).

| Python (legacy) | Rust native (core logic) | PyO3 binding (FFI) | Strategy |
| --- | --- | --- | --- |
| `str` | `&str` (input), `String` (output) | `&str` / `PyString` | Avoid copies for read-only paths. Rust `String` returned to Python is copied into Python's heap. |
| `int` / `float` | `i64` / `f64` (or `u32` when sizing is fixed) | `i64` / `f64` | Pin bit width (32/64-bit) to align with the CPU cache line. |
| `list[T]` | `&[T]` (read-only slice), `Vec<T>` | `Vec<T>` / `Bound<'_, PyList>` | For large numeric arrays, the Python side **must** use `numpy`, and Rust receives it zero-copy via `rust-numpy`. |
| `dict[K, V]` | `std::collections::HashMap<K, V>` | `HashMap<K, V>` / `Bound<'_, PyDict>` | `HashMap` requires `K: Hash + Eq`. Prefer `FxHashMap` or `AHash` for higher throughput under predictable hashing. |
| `None` / Optional | `Option<T>` | `Option<T>` | No null pointers. Handle exhaustively via `match` / `if let`. |
| Class instance | `struct` + `impl` | `#[pyclass]` struct | Drop inheritance. Convert shared attributes to composition (embed one struct inside another). |

### 2.2. Error Handling Strategy

Python uses `try/except`: control flow is implicit. Rust requires explicit `Result<T, E>` propagation.

**Rule.** Define Domain Errors with `thiserror` inside the Core Engine. At the PyO3 Boundary, map each Domain Error variant to the appropriate `PyErr`.

```rust
// 1. Core logic: stdandardized, type-safe errors
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DataProcessingError {
    #[error("input field is empty: {0}")]
    EmptyField(String),
    #[error("parse failure: {0}")]
    ParseError(#[from] std::num::ParseIntError),
}

// 2. FFI boundary: map errors into the Python ecosystem
use pyo3::exceptions::{PyTypeError, PyValueError};
use pyo3::prelude::*;

impl From<DataProcessingError> for PyErr {
    fn from(err: DataProcessingError) -> PyErr {
        match err {
            DataProcessingError::EmptyField(msg) => PyValueError::new_err(msg),
            DataProcessingError::ParseError(e) => PyTypeError::new_err(e.to_string()),
        }
    }
}
```

### 2.3. Concurrency & GIL Bypass

The single largest reason to write Rust is to use multiple CPU cores. Whenever a CPU-bound task runs in Rust, **GIL must be released** so other Python threads (e.g., an HTTP request handler) are not blocked.

```rust
use pyo3::prelude::*;

#[pyfunction]
pub fn heavy_matrix_computation(py: Python<'_>, data: Vec<f64>) -> PyResult<f64> {
    // GIL RELEASE: everything inside this block runs independently of the Python runtime
    let result = py.allow_threads(|| {
        // Pure compute, e.g. via rayon
        // Absolutely no PyObject access inside this closure
        data.iter().map(|x| x.powf(2.0)).sum::<f64>()
    });

    Ok(result)
}
```

---

## 3. Anti-Patterns (Strict Prohibitions)

### 3.1. Forbidden — "Pythonic Rust"

* **Forbidden:** Cyclic data structures wrapped in `Rc<RefCell<T>>` or `Arc<Mutex<T>>` only to dodge a redesign of the data flow.
* **Solution:** Redesign toward **Data-Oriented Design**, or use arena / index-based referencing when graph-style structures are required.

### 3.2. Forbidden — `PyObject` Deep Inside Core

* **Forbidden:** Pass `PyAny`, `PyDict`, `PyList` deep into the Rust business logic. It ties Core to Python's ABI and removes `cargo test` independence.
* **Solution:** Strictly layered. The `#[pyfunction]` binding parses `PyObject` into a native Rust struct first; only that struct is handed to the Core.

### 3.3. Forbidden — Mutex Inside Async Runtime

* **Forbidden:** `std::sync::Mutex` inside a `tokio` `async fn`. It blocks the executor OS thread and hangs the system (deadlock / starvation).
* **Solution:** Use `tokio::sync::Mutex` only when a lock must be held across an `.await`. Otherwise prefer message passing (`tokio::sync::mpsc`, Actor Model).

---

## 4. Verification Pipeline

A porting PR may be merged **only** when every gate below passes.

### 4.1. Ground Truth — Behavioral Parity (E2E)

The entire existing Python test suite (e.g. written with `pytest`) **must pass 100%** when run against the newly compiled Rust module, with **no test source edits** (timing assertions excluded).

### 4.2. Memory & Linter Check

Before packaging, the Rust module must clear the strictest CI linters:

```bash
# 1. Linter at max strictness; reject every clippy warning
cargo clippy --all-targets --all-features -- -D warnings

# 2. Memory safety / access errors (when C-FFI is involved)
cargo test --target x86_64-unknown-linux-gnu
valgrind --leak-check=full --error-exitcode=1 python -m pytest tests/
```

### 4.3. Performance Sign-off

A `pytest-benchmark` comparison between the original Python module and the new Rust module is mandatory. A PR is accepted only when both criteria are met:

1. **Latency:** at least **5x–10x** reduction for CPU-bound tasks.
2. **Memory Peak:** at least **50%** reduction in RAM (measured via `memray`).

---

## 5. Multi-Agent AI Workflow Integration

When dispatching a porting task to an AI Agent (Claude Code / Cursor / Windsurf), the architect must inject system context per the 4-agent flow below:

```
[Current Python source + PORTING_GUIDE.md]
                   │
                   ▼
       ┌───────────────────────┐
       │ 1. ARCHITECT AGENT    │ ──> Parse AST, extract types, design Rust Trait/Struct.
       └───────────────────────┘
                   │
                   ▼
       ┌───────────────────────┐
       │ 2. GENERATOR AGENT    │ ──> Write pure Rust core + PyO3 binding (zero-copy).
       └───────────────────────┘
                   │
                   ▼
       ┌───────────────────────┐
       │ 3. COMPILER & FIXER   │ ──> Run cargo check/clippy. Read errors, fix, loop.
       └───────────────────────┘
                   │
                   ▼
       ┌───────────────────────┐
       │ 4. ADVERSARIAL AGENT  │ ──> Audit anti-patterns (extra `.clone()`, GIL locks, `unsafe`).
       └───────────────────────┘
                   │
                   ▼
      [Pass PyTest E2E Suite & Benchmark]
```

**Mandatory Agent prompt template:**

> "You are the Generator Agent. Port module `legacy_math.py` into Rust per the rules in `PORTING_GUIDE.md`. Strictly separate Core Engine from FFI Boundary. Release GIL with `py.allow_threads` around the main compute loop. Do not introduce `unsafe` or `.clone()` without an explicit comment justifying the technical trade-off."

---

## 6. Sky Auto Player Specific Boundaries

This section binds the abstract guidance above to the concrete constraints of this repository. Where a rule conflicts with `AGENTS.md`, **`AGENTS.md` wins** (Priority Stack: P0 Security → P1 enforced config → P2 local evidence → P3 task intent).

### 6.1. P0 Security Mandates — Always Bound

* **NO GAME TAMPERING** (`AGENTS.md` P0.1). Applies unchanged.
* **SENDINPUT ONLY** (`AGENTS.md` P0.2). Applies unchanged; no exception is granted by this guide.
* **STRICT VALIDATION** (`AGENTS.md` P0.3). Every Python→Rust boundary must validate inputs strictly before they enter Core.

### 6.2. Rust is Allowed to Call `SendInput`

A Rust core **may** invoke the Windows API `SendInput` through the `windows` or `winapi` crate. This is a **pure FFI wrapper**, not a hook/inject/bypass:

* `SendInput` is a public, documented Windows API designed for legitimate input simulation.
* Calling it via Rust FFI does not change the security posture mandated by `AGENTS.md` P0.2.
* Any hot path that sends signals must:
  * Wrap the raw `extern "system"` call in a `safe fn` surface.
  * Document memory layout (`INPUT` struct, key codes, flags) why the call is sound.
  * Hold zero references to Python-side state during the call.

### 6.3. Absolute Prohibitions — Hard No

Even if a future Rust subsystem could technically perform them, the following are **forbidden by `AGENTS.md` P0**:

* ❌ Hooking (e.g. `SetWindowsHookEx`, low-level keyboard hooks) on the game or any other process.
* ❌ DLL injection (`LoadLibrary` remote, `CreateRemoteThread`).
* ❌ Reading/writing external process memory (`ReadProcessMemory`, `WriteProcessMemory`, `VirtualProtectEx`).
* ❌ Debugger attach / anti-cheat bypass (`NtQueryInformationProcess` for anti-cheat, `DebugActiveProcess`).
* ❌ Scanning memory of another process.
* ❌ Tampering with binary files (modifying `.exe`, `.dll` of any external app or game).

### 6.4. FFI Safety Rules

Every `unsafe` block inside the Rust core must:

* **Only** appear at the call site to a documented, official OS API.
* **Always** carry a comment justifying pointer ownership, alignment, and lifetime.
* **Always** be wrapped by a `safe` function at the boundary — `unsafe` must never bleed into business logic.
* **Always** clear `cargo clippy -- -D warnings` (see §4.2).

### 6.5. Allowed Porting Scope (Current Stage)

* ✅ Scheduler logic in isolation (timing, deduplication, ordering).
* ✅ Configuration parsers (YAML/JSON ingestion, validation).
* ✅ Validation pipelines that run before signal dispatch.
* ✅ CPU-bound loops with **no** Windows I/O.
* ❌ **Signal-dispatch hot path.** This path stays in Python for now, honoring the `AGENTS.md` Working Principle *"Isolate the Windows backend behind an interface"* — keeping it independently testable and behind an abstract boundary.

When the hot path is later allowed to move into Rust, that decision must update both `AGENTS.md` and this document, and must add the corresponding CI gate entry under §4.

---

*End of document.*
