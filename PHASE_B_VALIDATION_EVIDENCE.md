# Phase B validation execution evidence

Validation date: 2026-08-19

Validation tree: `de0caeb46e4b3f501a8bcee8e5ed842a06bc3300`

All logs below are raw command transcripts. Each ends with an explicit
`EXIT_CODE` line.

| Command | Exit | Relevant result | Warnings | Raw log |
| --- | ---: | --- | --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | format clean | none | [01-cargo-fmt.log](phase-b-validation/01-cargo-fmt.log) |
| `cargo check --manifest-path rust/Cargo.toml --workspace --all-targets --all-features` | 0 | completed | none | [02-cargo-check-workspace.log](phase-b-validation/02-cargo-check-workspace.log) |
| `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings` | 0 | completed with `-D warnings` | none | [03-cargo-clippy-workspace.log](phase-b-validation/03-cargo-clippy-workspace.log) |
| `cargo test --manifest-path rust/Cargo.toml --workspace --all-features` | 0 | 459 passed, 0 failed | none | [04-cargo-test-workspace.log](phase-b-validation/04-cargo-test-workspace.log) |
| `uv sync` with `UV_ENV_FILE=.env`, repo-local cache | 0 | 39 packages resolved | none | [05-uv-sync.log](phase-b-validation/05-uv-sync.log) |
| `uv run --env-file .env python scripts/build_rust_wheel.py` | 0 | exact `cp314t-win_amd64` wheel built, installed, imported, provenance/GIL verified | UV hardlink fallback because cache/target volumes differ | [06-rust-wheel-build.log](phase-b-validation/06-rust-wheel-build.log) |
| `uv run --env-file .env python scripts/audit_free_threaded_wheels.py` | 0 | runtime GIL disabled; dependency audit PASS | none | [07-free-threaded-audit.log](phase-b-validation/07-free-threaded-audit.log) |
| `uv run --env-file .env ruff check .` | 0 | all checks passed | global cache `Access is denied`; no lint failure | [08-ruff.log](phase-b-validation/08-ruff.log) |
| `uv run --env-file .env pyright` | 0 | 0 errors, 0 warnings, 0 informations | none | [09-pyright.log](phase-b-validation/09-pyright.log) |
| `uv run --env-file .env pytest -q -m "not slow"` | 0 | 824 passed, 6 skipped, 1 xfailed | one `PytestCacheWarning`; skipped native acceptance cases are explicitly marked | [10-pytest-nonslow.log](phase-b-validation/10-pytest-nonslow.log) |
| `uv run --env-file .env python scripts/audit_security_mandates.py` | 0 | no forbidden Windows API references | none | [11-security-audit.log](phase-b-validation/11-security-audit.log) |
| `git diff --check` | 0 | whitespace clean | none | [12-git-diff-check.log](phase-b-validation/12-git-diff-check.log) |

## Exact Phase-A unlock record

The exact governance token was recorded by the user during the
post-implementation Phase-B acceptance exchange:

```text
PHASE_A_ACCEPTED: proceed to Phase B calibration vNext
```

This records the late Phase-A sequencing authorization/owner confirmation. It
is distinct from human real-host calibration evidence, which remains pending.

## Other evidence artifacts

- [Phase-B completion report](PHASE_B_COMPLETION_REPORT.md)
- [Phase-A regression benchmark evidence](phase-b-evidence/)
- Precision/provenance correction commit: `de0caeb46e4b3f501a8bcee8e5ed842a06bc3300`
