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

Do not preload historical plans or archives. Existing documentation describes the current system and
should normally be preserved, but an intentional contribution may change a documented architecture
or contract; update the active documentation in the same change when that happens.

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
  mechanism. `scripts/audit_security_mandates.py` enforces this boundary.
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
2. During development, run the smallest relevant repository verification group:

   ```powershell
   uv run python scripts/check.py static
   uv run python scripts/check.py tests
   uv run python scripts/check.py rust
   ```

3. Before completion, run `uv run python scripts/check.py` when your environment supports the full
   normal gate. Run specialized Windows timing, package, updater, release, or benchmark evidence when
   the changed boundary requires it.
4. Keep secrets and `.env` out of commits. Do not bypass CI or weaken a security/release gate merely
   to make a change pass.
5. In the PR, report the checks/evidence actually run and any relevant residual risk.

A refactor and a behavior change may live in the same PR when they form one inseparable, reviewable
outcome and the tests make the behavior change explicit. Avoid unrelated cleanup.

## Dependency management

Use `uv sync`, `uv add`, or `uv add --dev`; do not use ad-hoc `pip install` for project dependency
changes. The repository's pinned free-threaded Python/toolchain configuration remains the executable
source of truth for supported development versions.

## Commit messages

Conventional commit subjects are preferred: `type(scope): summary` (for example `fix(ui): ...` or
`refactor(dispatch): ...`). Optimize for a reviewable history rather than forcing implementation
work into a predetermined phase choreography.

## Licensing

By contributing you agree that your contributions are licensed under the GNU GPL v3.0, the same
license as the rest of the repository.
