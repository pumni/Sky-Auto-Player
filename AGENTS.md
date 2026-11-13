# Sky Auto Player — Agent Context

Windows 11 desktop app (Tauri 2, React/TypeScript) with a Rust-native
music-sheet player for Sky: Children of the Light.

## Project boundaries

- The desktop UI and website live in `apps/desktop/` and `apps/website/`;
  each keeps its independent Bun manifest and lockfile. The Rust workspace
  remains under `rust/`.
- Gameplay input uses Windows `SendInput` only. No game-memory access,
  file tampering, process hooks, DLL injection, debugger attachment, or
  anti-cheat bypass. See [SECURITY.md](SECURITY.md).
- Keep domain/orchestration separate from Win32 effects. The native timing
  and focus contracts matter; see [architecture](docs/architecture.md) and
  [playback engine](docs/playback-engine.md) for changes to those boundaries.
- Rust and Bun are the canonical product/development toolchains; do not
  introduce a Python product runtime or general-purpose repository toolchain.
  The existing Python ClamAV scanner is a narrowly scoped hosted-release
  adapter governed by [release authority](docs/v4-release-authority.md).
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
- Run focused checks and report what ran before intermediate review. The full
  local gate `cargo xtask check all` and required CI must pass at the exact PR
  head for final merge readiness.
- Prefer names that describe current responsibilities. Historical labels remain
  human review context; retain genuine version/schema IDs and live immutable
  identifiers when their consumers require them.
