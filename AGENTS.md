# Sky Auto Player — Repository Instructions

Sky Auto Player is a Windows 11 music-sheet player for Sky: Children of the Light.
The Tauri 2 / React desktop uses Rust-native playback and sends gameplay input
only through Windows `SendInput`. Supported sheet imports are JSON, skysheet,
and JSON-compatible TXT.

## Scope and evidence

- Work within the requested outcome. Do not create, publish, merge, close, retarget,
  or otherwise mutate remote branches, pull requests, issues, releases, or other
  externally visible project state unless the current request authorizes it.
- Inspect the relevant implementation, direct tests, and configuration before
  changing behavior. At the checked-out SHA, those establish current behavior;
  historical plans, ADRs, and qualification reports establish only their recorded
  revisions and conditions.
- Use [docs/INDEX.md](docs/INDEX.md) to find task-specific contracts. Read only
  the documents needed for the task; do not preload the entire documentation tree.
  Consult [docs/agent-context-maintenance.md](docs/agent-context-maintenance.md)
  when auditing or changing agent instructions or documentation authority.

## Security and architecture

- Do not access game or other process memory, modify game files, attach debuggers,
  inject DLLs, install process/input hooks, or bypass anti-cheat mechanisms.
- Gameplay input is dispatched through the existing Windows `SendInput` boundary
  only. Validate untrusted inputs and fail closed when security-sensitive data
  or target identity is ambiguous. [SECURITY.md](SECURITY.md) is authoritative
  for this boundary; `cargo xtask check static` enforces repository checks.
- Domain and orchestration crates remain independent of Win32/input effects.
  Follow [docs/architecture.md](docs/architecture.md) for ownership and
  [docs/rt-dispatch-architecture.md](docs/rt-dispatch-architecture.md) for
  real-time dispatch constraints.
- The supported product and repository tooling are Rust/Bun native. Do not add
  a Python product runtime or repository toolchain.
- Release and updater integrity follow
  [docs/distribution-and-update.md](docs/distribution-and-update.md) and the
  applicable release runbooks. Do not weaken their checks for convenience.

## Changes and validation

- Prefer durable responsibility-based names for maintained code, tests,
  configuration, and scripts. Phase numbers are appropriate only for actual
  version/schema IDs or immutable historical/external references; the static
  audit enforces this distinction.
- Update the relevant *current* documentation when implementation contracts
  change. Preserve revision-bound evidence and release records as history.
  Documentation work under `docs/` also follows [docs/AGENTS.md](docs/AGENTS.md).
- During development, run focused checks for affected boundaries. Before review,
  run `cargo xtask check all` once on the final relevant source state; it covers
  static, Rust, desktop, and native desktop checks. Do not repeatedly run whole
  groups or native performance/packaging qualification without a changed boundary.
- Record the exact commands, HEAD/input identity, exit codes, and coverage for
  checks actually run. Report skipped or unavailable checks explicitly; do not
  present source inspection, mocks, or historical measurements as current native
  Windows validation.
