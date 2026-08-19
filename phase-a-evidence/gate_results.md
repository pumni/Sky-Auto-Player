# Phase A correction validation gates

All commands ran from the repository root.

- `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` — PASS
- `cargo clippy --manifest-path rust/Cargo.toml --workspace --all-targets --all-features -- -D warnings` — PASS
- `cargo check --manifest-path rust/Cargo.toml -p sky_player_rs` — PASS
- `cargo test --manifest-path rust/Cargo.toml -p sky_dispatch_win32 input::packet::tests::target_crossing_inside_down_cutoff_is_allowed -- --exact` — PASS, 1 test
- `cargo test --manifest-path rust/Cargo.toml -p sky_dispatch_win32 input::packet::tests::target_crossing_sample_is_reused_without_a_second_pre_call_read -- --exact` — PASS, 1 test
- `cargo test --manifest-path rust/Cargo.toml -p sky_dispatch_win32` — PASS, 142 tests
- `cargo test --manifest-path rust/Cargo.toml --workspace --all-features` — PASS; core 47, golden 1, properties 3, win32 142, player 186, no-alloc 20, updater 43, updater E2E 1, safety 5, corpus 1, all doc tests pass
- `git diff --check` — PASS
- `uv run --env-file .env python scripts/audit_security_mandates.py` — PASS; no forbidden Windows API references

The benchmark harness now records wait failures/outliers instead of aborting a
whole run. The official A/B artifacts use the explicitly labelled
`phase_a_sender_only` scope; the full real-wait scope remains the default mode.
