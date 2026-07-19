# Contributing to Sky Player

Thanks for considering a contribution to Sky Player. This document explains the
expectations, the architecture boundaries you must respect, and how to get a change merged.

## Read these first

Before writing any code, read:

- [`SECURITY.md`](SECURITY.md) - the P0 security mandates are immutable.
- [`AGENTS.md`](AGENTS.md) - the priority stack, architecture invariants, and boundaries.
  Treat it as the single source of truth for engineering rules in this repo.
- [`docs/architecture.md`](docs/architecture.md) and
  [`docs/rt-dispatch-architecture.md`](docs/rt-dispatch-architecture.md) - the layering and
  purity contracts you must keep intact.

If any of these documents and your change disagree, the documents win. If you think a document
itself is wrong, open an issue first - do not fix it silently in a code PR.

## Scope of contributions we welcome

- Bug fixes with a minimal failing test that goes green.
- Performance work that is backed by a `perf-baselines/` measurement before and after.
- New sheet-format readers (JSON, skysheet, JSON-compatible TXT are the supported set today).
- Documentation improvements that match the existing `docs/INDEX.md` hierarchy.
- Windows platform integrations that respect the `platform/`-only `ctypes` boundary.
- Translation improvements for the landing page (`docs/index.html`, `docs/vi/index.html`).

## Out of scope (will be closed)

- Anything on the P0 never-list: game tampering, memory reads, hooks, injection, debugger
  attach, anti-cheat evasion, or any input mechanism other than Windows `SendInput`.
- Ports to macOS or Linux. Sky Player is Windows-only by design.
- Dependencies on `python-keyboard`, `pynput`, `SetWindowsHookEx`, or any third-party keyboard
  module. The security audit (`scripts/audit_security_mandates.py`) will fail any such PR.
- Broad rewrites without tests. Keep diffs focused and reviewable.

## Architecture contracts you must keep

- `src/sky_music/domain/` and `src/sky_music/orchestration/` stay pure: no `ctypes`, no
  `SendInput`, no wall-clock, no Windows-specific imports. Timing edges are unit-tested
  against a controlled clock.
- `src/sky_music/platform/` is the only place Win32, `ctypes`, or `SendInput` may live.
- `src/sky_music/infrastructure/` may import `platform/` but must not be imported by `domain/`
  or `orchestration/`.
- Domain models use `@dataclass(frozen=True, slots=True)`.

## Workflow

1. Open an issue describing the change before opening a PR for anything non-trivial.
2. Branch from `main`. Use a conventional-commit prefix on the branch name and the commit
   subjects (`feat(scope):`, `fix(scope):`, `docs(scope):`, `refactor(scope):`, etc.).
3. Run the narrowest validation gate that covers your change scope:

   ```powershell
   uv run ruff check .
   uv run pyright
   uv run pytest
   ```

   For any security-touch surface (`platform/`, `SendInput` seams, the audit script, the
   updater script, the scheduler-purity boundary), also run:

   ```powershell
   uv run --env-file .env python scripts/audit_security_mandates.py
   ```

4. Keep one logical change per PR. Never mix a refactor with a behavior change in one PR.
5. Do not commit secrets, do not push `.env`, do not skip hooks. Never bypass a CI gate to
   reach "done".

## Dependency management

Always use `uv sync` / `uv add` / `uv add --dev`. Never `pip install`, never manually activate
`.venv`. Free-threaded interpreter (`3.14+freethreaded`) is mandatory.

## Commit message format

Conventional commits: `type(scope): summary`. Frequent scopes: `scheduler`, `windows`, `ui`,
`cli`, `build`, `docs`, `deps`, `tooling`, `sec`. One logical change per commit.

## Licensing

By contributing you agree that your contributions are licensed under the GNU GPL v3.0, the
same license as the rest of the repository.
