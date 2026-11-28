# Contributing to Sky Auto Player

Keep contributions focused on one reviewable outcome, with evidence proportional
to its risk. [AGENTS.md](AGENTS.md) gives repository working agreements;
[SECURITY.md](SECURITY.md) owns security and trust boundaries.

## Find the owning code and contract

Use the actual source, direct tests and configuration at the checkout's revision.
[docs/INDEX.md](docs/INDEX.md) routes to current contracts by task; historical
records are provenance rather than current qualification. Update the owning
contract with behavior changes, not a second instruction or summary layer.

Prefer names that describe current responsibility. Old phase, sprint and
work-order labels are historical; retain real version/schema identifiers and
immutable external identifiers with live consumers. Migrate callers and remove
replaced private paths. A refactor and behavior change may share a PR when they
form one inseparable, reviewable outcome. Avoid unrelated rewrites.

## Scope

Contributions welcome: regression-backed bug fixes, measured performance work,
supported sheet-format improvements, documentation, Windows/native improvements
within the security contract, and site translations in
`apps/website/src/content/` and `apps/website/src/data/`.

The current product scope is Windows; macOS/Linux ports and input mechanisms
outside the [security boundary](SECURITY.md) are out of scope. Keep domain and
orchestration independent of direct Win32/input effects. Architecture changes
require the corresponding executable boundary checks and owning documentation.

## Toolchains and commands

Use repository pins, not versions copied into instructions. The
[Rust toolchain policy](docs/rust-toolchain-policy.md) owns the compiler,
locked dependencies and production lint rules. At the repository root use:

```powershell
node scripts/toolchain-pin.mjs exec -- cargo xtask check all
```

For local dependency checks install Cargo Audit 0.22.2 and Cargo Vet 0.10.2
with locked installs (see the Rust toolchain policy). Do not automatically
ignore advisory warnings or add vet exemptions to pass validation.

The root `.bun-version` is the Bun pin. Desktop and site retain independent
manifests and lockfiles; install from the matching app with
`bun install --frozen-lockfile`. See [desktop tooling](docs/desktop-tooling.md)
for app checks and the blocking ESLint policy. Rust and Bun are the canonical
product/development toolchains.

## Review and evidence

1. Branch from main or use the authorized existing branch; an issue or
   intermediate handoff is useful for material decisions, not required ceremony.
2. Iterate with focused tests and relevant format/lint. Demonstrate the
   production path for production behavior claims; explicitly bound simulation,
   browser and native evidence. See [playback verification](docs/playback-verification.md).
3. Before claiming merge readiness, run the pinned canonical check on final
   relevant source and verify required CI at exact PR HEAD. The
   [CI contract](docs/ci-validation-topology.md) owns lane scope and the
   merge gate. Do not run each broad subgroup and then repeat the same checks
   through `check all` unless inputs or coverage changed.
4. Run specialized package, updater, release, Windows timing or benchmark
   evidence when the changed boundary warrants it and authorization permits.
   Avoid repeating full cold/warm baselines or native acceptance without a
   relevant change.
5. Report exact commands, source/input identity, exit status, coverage, skips
   and residual risk. Preserve raw evidence access. Passing a command does not
   establish a gate or capability it did not exercise.
6. Keep secrets and `.env` out of commits. Never bypass required CI or weaken
   security/release checks. Changes to assertions, goldens, skips and tolerances
   require independent behavioral justification. Turn repeatable failures into
   regression tests or improvements to the owning tool.

## Commits and license

Prefer conventional commit subjects (`type(scope): summary`) and reviewable
history rather than predetermined phase choreography. By contributing you
agree that contributions are licensed under GNU GPL v3.0.
