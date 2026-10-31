# Song Numeric Compatibility and Maintenance Inventory

This document records the maintained numerical and data-compatibility rules
for legacy schedules and versioned Piano plans. The implementation baseline
for this cleanup is `6878b3816e7fed0b2e12c77b5d3056646271c40d`.

## Numeric contract

| Input or calculation | Maintained behavior |
| --- | --- |
| Legacy song timestamp | Accept the existing integer, finite-float-truncated, trimmed-integer-string, and Boolean forms; reject negative values and values that cannot be converted into source microseconds. |
| Scaled onset | Scale source microseconds once, round to the nearest integer microsecond with `f64::round_ties_even()`, then reject nonfinite, negative, or out-of-range results. A unit scale preserves the exact source integer. |
| Release and duration | Check every release addition and the published source duration before returning a schedule. No saturating timestamp is published. |
| Two-decimal tempo advice | Scale by 100, round with nearest-even, divide by 100, then clamp to 0.1–1.0 where the existing recommendation is bounded. An unrepresentable advisory value is omitted and diagnosed. |
| Composed source columns | Keep positive-millisecond `round()` behavior, the 100 ms source offset, and boundary rounding before conversion to microseconds. |
| Physical hold floor | Keep `ceil()` for frame-based holds and `div_ceil()` for frame duration. |
| Risk gaps | Convert a nonnegative millisecond gap with checked subtraction and multiplication. An unrepresentable optional gap is omitted. |

The Rust standard rounding method replaces duplicate nearest-even arithmetic
only where that is the existing contract. It does not replace source-column
rounding or physical hold rounding.

## Maintained occurrence inventory

| Area | Cleanup and retained responsibility |
| --- | --- |
| `rust/crates/sky_app_core/src/song.rs` | Replaced interpreter-themed timestamp and diagnostic formatters with legacy-song-specific names. Replaced manual nearest-even timestamp and hundredths code with checked Rust-native helpers. Added checked source, release, duration, gap, and advisory handling. |
| `rust/crates/sky_app_core/src/song/planner.rs` | Removed its second nearest-even implementation and reused the checked onset helper. Existing source provenance, tempo-collapse losses, and release preflight remain. |
| `rust/crates/sky_app_core/src/song/decode.rs`, `timing.rs` | Kept composed source `round()`, the source offset, microsecond conversion, physical hold `ceil()`, and frame `div_ceil()` unchanged. |
| `rust/crates/sky_native_adapters/src/lib.rs` | Renamed config formatting and numeric coercion helpers by their persisted-settings role. Kept the adapter's Boolean rejection for integer settings, Boolean acceptance for historical profile numbers, and legacy text formatting. |
| `rust/crates/sky_app_core/src/catalog.rs`, `settings.rs` | Removed migration-era explanatory comments and test names. Kept WRatio matching, normalized path hashing, settings values, and persisted layouts unchanged. |
| `desktop/src-tauri/src/native_runtime.rs` | Replaced the handwritten percentile index tie logic with `round_ties_even()` and retained the nearest-even percentile result. |
| Native playback, player, dispatch, and calibration comments | Reworded references to removed callers and migration stages as current ownership, admission, snapshot, and publication rules. Runtime comments and test labels changed; dispatch and Win32 behavior did not. |
| Rust and native integration tests | Renamed the current song contract fixture to `schedule_contract_cases.json`; updated test names to describe checked-in behavior and typed DTO contracts. Fixture values and schedule hashes remain unchanged. |
| Architecture and bilingual site guides | Rewrote current runtime descriptions around the native router and Rust services. The `SendInput`-only security limits and forbidden third-party input examples remain explicit. |

## Before and after evidence

Boundary tests record the numerical changes while existing fixture tests pin
normal behavior:

| Case | Before | After |
| --- | --- | --- |
| Timestamp ties `0.5, 1.5, 2.5, 3.5` | Two custom parity branches | `round_ties_even()` → `0, 2, 2, 4` |
| Hundredths `0.015, 0.025, 0.035` | Custom scaled parity branch | Scale, nearest-even, divide → `0.02, 0.02, 0.04` |
| Finite scaled onset beyond `u64` or nonfinite result | Legacy cast could saturate; planner used a separate manual range check | Typed schedule/planner error; no actions published |
| Legacy release near `u64::MAX` | Saturating release could place an `Up` at the bound after an invalid conversion | Checked release/duration addition rejects an unrepresentable schedule |
| Raw gap too large for microseconds | Unchecked multiply could panic in debug or wrap in release | Checked metric is omitted; valid schedule-derived risk remains available |
| Nonfinite tempo advice | Invalid intermediate could be turned into a bounded numeric suggestion | Numeric advice is omitted with a diagnostic; physical risk severity remains |
| Composed columns and physical hold | Source-specific `round()` and `ceil()` | Unchanged |

The `legacy_schedule_fingerprints.json`, `piano_planner_cases.json`,
`schedule_contract_cases.json`, settings compatibility fixtures, catalog search
fixtures, and native prepare/risk tests are the before/after behavioral record.
The baseline commit was `6878b3816e7fed0b2e12c77b5d3056646271c40d`.

| Regression surface | Before | After |
| --- | --- | --- |
| Core schedule, planner, catalog, and settings tests | `cargo test --manifest-path rust/Cargo.toml -p sky_app_core`: 73 passed, 2 ignored | `cargo test --manifest-path rust/Cargo.toml -p sky_app_core --locked --quiet`: 81 passed, 2 ignored |
| Settings adapter compatibility tests | `cargo test --manifest-path rust/Cargo.toml -p sky_native_adapters`: 40 passed | Same package suite: 41 passed, including retained Boolean and text coercion differences |
| Desktop shell native tests | `cargo test --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --all-features --locked`: 327 passed, 4 ignored | Same suite: 327 passed, 4 ignored |
| Legacy schedules and modern plans | All 106 legacy schedule fingerprints and planner golden cases passed | Same 106 fingerprints and planner golden cases pass; fixture values and schedule hashes are unchanged |
| Settings persistence and catalog IDs | Compatibility fixtures and catalog search tests passed | Same persisted layouts, values, search behavior, and IDs pass their fixtures |
| Physical risk and input/security boundary | Existing native prepare/risk tests passed; `SECURITY.md` defined `SendInput`-only gameplay input | Native prepare/risk tests pass with physical severity retained for invalid advisory values; security enforcement and `SECURITY.md` are unchanged |

The final repository gate, `cargo xtask check all`, exited 0. It covered the
static and security audits, supply-chain policy, catalog verification, Rust
formatting and Clippy, the locked all-features Rust workspace tests, desktop
checks, and native desktop tests. The maintained-name audit is also part of the
static gate. `bun run check` and `bun run lint` passed in `site`; Prettier passed
for all eight changed site Markdown files. The repository-wide `bun run
format:check` exited 1 on 20 pre-existing, unmodified Astro files. This table
does not claim a performance improvement.

## Historical and security exceptions

The maintained-content audit scans Rust and desktop source, tests, active docs,
and the full website source tree. Its historical path exceptions are
deliberately limited to:

- `CHANGELOG.md`, which records behavior at earlier releases;
- `docs/adr/`, `docs/releases/`, and `docs/evidence/`, which retain revision-
  bound decisions, release records, and measurements;
- `docs/maintenance-test-pipeline-report.md`, which records the earlier
  fixture rename and validation report as it existed at that revision.

Those files are left unchanged under `docs/AGENTS.md`. The current `SECURITY.md`
and its active contribution/site guidance retain the full prohibited-input
rules and concrete library examples. The audit checks removed helper and
fixture names without treating those valid security references as violations.
