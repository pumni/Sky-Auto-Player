# Contributing to Sky Auto Player

Thanks for considering a contribution. The repository favors small, evidence-backed changes and
keeps process proportional to the risk of the boundary being changed.

## Start with the relevant evidence

- Read [`SECURITY.md`](SECURITY.md) for the non-negotiable security boundary.
- Use [`docs/INDEX.md`](docs/INDEX.md) to route to the smallest active document set relevant to your
  change.
- Inspect the production source and direct tests for the behavior you are changing.
- [`AGENTS.md`](AGENTS.md) is a concise repository/agent guide; it is not a replacement for source,
  tests, or task-specific investigation.

Use [`docs/INDEX.md`](docs/INDEX.md) for relevant active contracts; don't preload historical
records. Update the owning contract when behavior changes and preserve earlier evidence at its
recorded revision. Agent-context reviews can use
[`docs/agent-context-maintenance.md`](docs/agent-context-maintenance.md).

## Required naming rule

Give maintained paths, Rust and TypeScript identifiers, tests, scripts, CLI arguments, environment
variables, and configuration fields names that describe their continuing behavior or responsibility.
Do not use phase, sprint, work-order, or earlier optimization-wave numbers as implementation names
(for example `r3_*`, `p4_*`, `phase8_*`, `wo04_*`, or `h4_*`). Keep such a marker only for a genuine
version/schema identifier or immutable historical or external contract, and record the narrow reason
in the relevant documentation. Do not preserve an old alias without evidence of an external caller.
The durable-name audit in `cargo xtask check static` is required for changes to active source or
configuration.

## Contributions we welcome

- Bug fixes with direct regression coverage.
- Performance work backed by measurements on a comparable environment.
- New or improved supported sheet-format handling.
- Documentation improvements that make current behavior easier to navigate.
- Windows/native improvements that preserve the documented security and platform boundaries.
- Translation improvements for the site source under `site/src/content/` and `site/src/data/`.

## Out of scope

- Game/process tampering, memory reads/writes, hooks, injection, debugger attach, anti-cheat evasion,
  or a gameplay-input mechanism other than Windows `SendInput`.
- Dependencies such as `python-keyboard`, `pynput`, `SetWindowsHookEx`, or another keyboard-injection
  mechanism. `cargo xtask check static` enforces this boundary.
- macOS/Linux ports in the current product scope.
- Broad unrelated rewrites without evidence that they are necessary for the stated outcome.

## Architecture

The current architecture is documented in `docs/architecture.md` and
`docs/rt-dispatch-architecture.md`. In particular, keep domain/orchestration independent from direct
Win32/input effects unless the contribution explicitly changes the architecture and updates the
corresponding executable boundary checks and documentation.

Do not copy architecture rules into a new contribution-specific instruction layer. Prefer source,
direct tests, active docs, and repository checks as the evidence system.

## Workflow

1. Branch from `main` and keep the change focused on one outcome. An issue is useful for product
   discussion or genuinely ambiguous semantics, but it is not mandatory ceremony for ordinary
   implementation work.
2. During editing, run focused tests for the changed boundary. Avoid running every broad group after
   each edit.
3. Before review, run `cargo xtask check all` once against the final relevant source state. It
   includes the static, Rust, desktop, and native desktop groups; do not automatically run those
   groups individually and then repeat them through `check all`. Repeat a passed gate only if a
   later change invalidates its inputs or coverage.
4. Do not repeat full cold/warm performance baselines or native acceptance without a new behavior or
   performance change. Run specialized package, updater, release, Windows timing, or benchmark
   evidence when the changed boundary requires it.
5. Record the exact command, HEAD and relevant input identity, exit code, and coverage. Report only
   checks actually run, and explicitly identify checks that were skipped.
6. Keep secrets and `.env` out of commits. Do not bypass CI or weaken a security/release gate merely
   to make a change pass.
7. In the PR, report the checks/evidence actually run and any relevant residual risk.

A refactor and a behavior change may live in the same PR when they form one inseparable, reviewable
outcome and the tests make the behavior change explicit. Avoid unrelated cleanup.

## Dependency management

Use the pinned Rust toolchain and Bun lockfile for canonical product checks. Rust and Bun are the
only required repository toolchains.

## Commit messages

Conventional commit subjects are preferred: `type(scope): summary` (for example `fix(ui): ...` or
`refactor(dispatch): ...`). Optimize for a reviewable history rather than forcing implementation
work into a predetermined phase choreography.

## Licensing

By contributing you agree that your contributions are licensed under the GNU GPL v3.0, the same
license as the rest of the repository.
