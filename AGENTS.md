# Sky Auto Player — Repository Guide

Windows 11 music-sheet player for Sky: Children of the Light. The desktop application reads JSON,
skysheet, and TXT sheets and emits gameplay keyboard input through Windows `SendInput`.

## Scope and external actions

Keep work within the requested task. Do not create, publish, close, merge, retarget, or otherwise
mutate remote branches, pull requests, issues, releases, or other externally visible project state
unless the user's current request explicitly authorizes that action. Ordinary local work within the
requested scope does not need separate authorization.

## Security and input boundary

- Never modify game files, access another process's memory, bypass anti-cheat, inject code, attach a
  debugger, or install process/input hooks.
- Gameplay keystroke simulation uses Windows `SendInput` only. Do not introduce other injection
  mechanisms.
- Validate external and user inputs; fail closed on ambiguous or malformed security-sensitive data.
- [`SECURITY.md`](SECURITY.md) is canonical for the security contract; `cargo xtask check static`
  enforces it.

## Architecture and current behavior

- Domain and orchestration crates stay independent of Win32 details and direct `SendInput`; see
  [`docs/architecture.md`](docs/architecture.md) and boundary tests for exact ownership.
- The supported product and repository tooling are Rust/Bun native. Do not reintroduce Python into
  the product runtime or repository tooling.
- Native updater and release integrity contracts follow
  [`docs/distribution-and-update.md`](docs/distribution-and-update.md).
- Current source, tests, and configuration at the checked-out SHA establish implementation
  behavior. Use [`docs/INDEX.md`](docs/INDEX.md) to find relevant current contracts and scoped
  historical evidence.
- Active names describe lasting behavior, not implementation sequence; schema/version identifiers
  and immutable historical or external contracts are exceptions. `cargo xtask check static` audits
  this policy.

## Validation

- Run focused checks while editing. Before review, run `cargo xtask check all` once against the final
  relevant source state; it includes static, Rust, desktop, and native desktop checks.
- Record exact commands, HEAD and relevant input identity, exit code, and coverage. Report only
  checks that actually ran and identify skipped checks.
- Do not repeat full performance baselines or native acceptance without a new behavior or
  performance change. Run packaging, Windows timing/latency acceptance, release, and benchmark
  workflows only when the changed boundary requires them.
