# Phase A validation gates

All commands ran from the repository root.

- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — PASS
- `git diff --check` — PASS
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings` — PASS
- `cargo check --manifest-path rust/Cargo.toml -p sky_player_rs` — PASS
- `cargo test --manifest-path rust/Cargo.toml -p sky_dispatch_win32` — PASS, 141 tests
- `cargo test --manifest-path rust/Cargo.toml --workspace --all-features` — PASS; core 47, golden 1, properties 3, win32 141, player 186, no-alloc 20, updater 43, updater E2E 1, safety 5, corpus 1, all doc tests pass
- `uv run --env-file .env python scripts/audit_security_mandates.py` — PASS; no forbidden Windows API references

One earlier full player run exposed a timing-sensitive startup test failure;
the isolated rerun passed, and the final complete workspace run passed.
