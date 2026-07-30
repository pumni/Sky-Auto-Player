# Version baseline — reviewed 2026-07-30

- Rust 1.97.1: official release dated 2026-07-16. Pin the point release because it fixes an LLVM miscompilation present in 1.97.0.
- PyO3 0.29.0: released 2026-06-11; supports CPython 3.14t and defaults to free-thread-compatible modules from 0.28 onward. Keep `#[pymodule(gil_used = false)]` explicit as an audit marker.
- PyO3 free-threaded rule: long native work and joins use `Python::detach`; do not use the old `allow_threads` example.
- CPython 3.14t packaging: do not enable `abi3`. Build a version-specific wheel with the exact free-threaded interpreter.
- Maturin 1.13.3: current reviewed release; use `>=1.13.3,<2` in build-system and lock the build environment.
- windows-sys 0.61.2: reviewed current release; enable only the Win32 feature groups actually needed.

Official references:

- https://blog.rust-lang.org/2026/07/16/Rust-1.97.1/
- https://pyo3.rs/v0.29.0/free-threading.html
- https://pyo3.rs/v0.29.0/changelog
- https://pyo3.rs/v0.29.0/parallelism.html
- https://www.maturin.rs/bindings.html
- https://github.com/PyO3/maturin/releases/tag/v1.13.3
- https://docs.rs/crate/windows-sys/0.61.2
