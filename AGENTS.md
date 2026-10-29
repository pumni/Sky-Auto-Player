# Sky Auto Player — Agent Context

Windows 11 desktop app (Tauri 2, React/TypeScript) with a Rust-native
music-sheet player for Sky: Children of the Light.

## Project boundaries

- Gameplay input uses Windows `SendInput` only. No game-memory access,
  file tampering, process hooks, DLL injection, debugger attachment, or
  anti-cheat bypass. See [SECURITY.md](SECURITY.md).
- Keep domain/orchestration separate from Win32 effects. The native timing
  and focus contracts matter; see [architecture](docs/architecture.md) and
  [dispatch](docs/rt-dispatch-architecture.md) for changes to those boundaries.
- Rust and Bun are the supported product/repository toolchains; do not
  introduce a Python product runtime or repository toolchain.
- Preserve updater and release trust requirements in
  [distribution-and-update.md](docs/distribution-and-update.md).

## Working in this repository

- Use code, tests, and configuration at the checked-out commit for current
  implementation behavior. Old ADRs, benchmarks, and release evidence describe
  their recorded revisions, not current qualification.
- [docs/INDEX.md](docs/INDEX.md) locates task-specific contracts. Open only
  the references relevant to the change; update a current contract when its
  behavior changes.
- Local editing and testing are within scope. Remote branch/PR/issue/release
  changes need authorization for the requested action.
- Run focused checks as appropriate. The pre-review full gate for implementation
  changes is `cargo xtask check all`; native timing, packaging, and release
  qualification are task-specific. Report what actually ran and what did not.
- Maintained names describe responsibilities rather than past project phases;
  genuine version/schema IDs and immutable historical names are exceptions.
