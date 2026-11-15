# Issue 666 — M1 prepared playback evidence

This record covers M1 on base `96e35b6c7b9946289e2bd6b02a658a83811e80ec`
(`c4629f87301243a8b3408c9990620ad78873b21c` tree), on Windows 11 x64 with
Rust 1.99.1. Measurements are local offline single runs and are descriptive,
not performance gates.

## Reproduction

Short desktop Prepare and prepared-executor construction:

```powershell
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --features tauri-test --lib prepare_publishes_a_plan_that_moves_into_fresh_runtime_state -- --nocapture
```

Observed for two source actions: one runtime schedule compile checkpoint;
baseline-equivalent raw compile, H/G validation, and executor construction
took 322 µs; candidate desktop Prepare took 670 µs; moving the prepared plan
and initializing fresh executor state took 500 ns. Calculated capacities were
1,008 bytes for the baseline executor, 2,136 bytes for baseline retained and
peak storage, 352 bytes for the prepared plan, and 3,027 bytes for the
candidate Prepare overlap peak.

One-million-action (500,000 single-key Down/Up pairs) capacity and timing run:

```powershell
cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --features tauri-test --lib valid_one_million_single_key_actions_fit_prepared_runtime_budget -- --nocapture
```

Observed baseline-equivalent compile, validation, and executor construction
took 3,378,660 µs. Candidate Prepare compile, validation, and QPC plan
materialization took 3,548,180 µs. Moving the plan and initializing executor
state took 17,600 ns. The baseline retained capacity was 445,001,240 bytes,
with a 481,000,758-byte phase peak. Candidate source schedule capacity was
73,000,000 bytes; compiled schedule capacity was 328,000,320 bytes; prepared
plan capacity was 44,000,264 bytes; and runtime executor capacity was
44,000,920 bytes. Candidate Prepare overlap, compiler phase, and modeled
startup peak were each 481,000,758 bytes, within the 536,870,912-byte policy
limit.

The short test asserts one desktop runtime schedule compilation, consumes the
published physical plan, and verifies fresh executor state. The million-action
fixture validates one million authored actions, materializes the QPC plan,
then moves it into the production executor. The native Start path test
`physical_startup_target_failure_retires_without_reentrant_active_lock`
asserts that Start does not increase the Prepare compiler checkpoint count.
`native_prepare_binds_advisories_to_variant_fingerprints_without_gating_model_pressure`
checks that advisories remain tied to variant fingerprints without gating
preparation, and
`unavailable_offline_advisory_does_not_block_physical_preparation` verifies a
valid physical plan remains selectable when the offline model reaches its
bound. These are exercised by `cargo xtask check desktop-native`.

## Limits and comparison basis

The baseline-equivalent path uses the raw compiler, timing validator, and
legacy executor constructor in the same candidate-source test binary; this is
not a separate build of the exact base commit. The candidate Start measurement
covers only moving the prepared executor arrays and initializing fixed-size
session state. It does not measure Tauri/native process setup, a real worker
Arm, Windows process RSS/peak, or actual gameplay. Capacity values are computed
from allocated vector capacities and struct sizes, not from process memory
sampling. The million-action test is a construction test; it does not dispatch
one million actions through `SendInput`.

Schema 25 unmatched-Up diagnostic rows are retained. Worker boot iterates only
their precomputed metadata to emit those existing rows, so boot cost remains
proportional to the number of unmatched-Up batches. The behavior/schema choice
for that exceptional diagnostic path and omission of an optional recommended
variant when its separate capacity admission fails have been raised to Leader
on issue 666 for decision.
