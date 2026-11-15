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
proportional to the number of unmatched-Up batches. Leader decision
`6898174730` accepts this M1 exception only for emitting existing rows from
precomputed unmatched-Up metadata; it does not permit another whole-schedule
walk or revalidation during Arm. The same decision requires an optional
recommended variant that exceeds capacity to be omitted while a valid Proceed
and offered DryRun remain selectable, and requires cancellation/supersession to
fail the entire Prepare.

## M1 review corrections

The review corrections are applied to the same branch and reviewed tree. No
public DTO, fingerprint encoding, telemetry schema, sender, focus/identity,
lease, power, cleanup, H/G, packet ordering, error policy, or pre-roll contract
is changed.

- Recommended advisory analysis now receives
  `recommended_schedule.actions.len()`. A regression uses 50,002 base actions
  and the distinct two-action recommended schedule. Proceed is unavailable at
  its advisory bound while the recommended advisory remains available and
  retains its own fingerprint. The advisory's action/work/memory admission
  calculation is fed the selected variant's authored action count.
- `sky_dispatch_core` exercises QPC frequency 3,579,545 Hz. It checks ceiling
  conversions (H = 36 ticks for 10 µs, G = 26 ticks for 7 µs, and a 50 µs
  paired-Up offset = 179 ticks), compares prepared and raw executor decisions,
  masks, ordering, timing windows, deadlines, and actual completion-QPC floors.
  At G, tick 225 waits for 226, and 226/227 are eligible; at H, tick 240 waits
  for 241, and 241/242 are eligible. The fixture includes a mixed Up/Down
  packet and independent keys.
- The successful service harness selects `UseRecommended`, enters session
  admission, and reaches worker `ArmedPublished` with a mock backend. Test-only
  counters show one prepared-plan executor consumption, zero raw admission
  validations, zero raw worker materializations, and no compiler checkpoint
  after Prepare. Core coverage checks that executor construction preserves the
  prebuilt vector allocations while starting with a fresh cursor and slots.
  Raw-caller coverage records admission validation and enters cancellable worker
  materialization at `StreamMaterialization`, then quits before any send.
- Recommendation capacity exhaustion is injected at its dispatch-plan
  materialization boundary. The test asserts that `UseRecommended` has no DTO
  decision, DTO variant, fingerprint reference, or native plan; retained cache
  capacity stays within policy; `Proceed` reaches successful mock Arm; and a
  valid `DryRun` remains offered and startable without a physical plan.
  Separate regressions cancel and supersede during recommendation
  materialization; both fail the whole Prepare and do not publish partial
  variants.

These session tests use only a mock sender. They prove variant selection,
admission, executor consumption, and successful Arm publication; they do not
call real `SendInput` or qualify playback in Sky. Candidate-binary
baseline-equivalent timings above are structural, single-run descriptions of
the raw compile/validate/executor path and the prepared plan move. They are not
an exact-base A/B, gameplay-speed result, or end-to-end native Arm measurement.
Capacity numbers remain calculated vector capacities and struct sizes, not
Windows process-memory measurements.

## Review-correction verification

Final-source checks ran on Windows 11 x64 MSVC with `rustc 1.99.1`
(`x86_64-pc-windows-msvc`) and Cargo 1.99.1. Exit codes below are from the
listed commands; PASS means exit 0.

| Command | Exit | Result |
| --- | ---: | --- |
| `git diff --check` | 0 | PASS |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | 0 | PASS |
| `cargo fmt --manifest-path apps/desktop/src-tauri/Cargo.toml --all -- --check` | 0 | PASS |
| `cargo clippy --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core -p sky_dispatch_win32 -p sky_player --all-targets --all-features -- -D warnings` | 0 | PASS |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_dispatch_core --features test-support` | 0 | PASS: 78 unit and 4 integration tests |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support` | 0 | PASS: 255 unit and 4 real-time no-allocation tests |
| `cargo test --locked --manifest-path rust/Cargo.toml -p sky_player --features test-support --test rt_dispatch_no_alloc` | 0 | PASS: 4 tests |
| `cargo test --locked --manifest-path apps/desktop/src-tauri/Cargo.toml --features tauri-test --lib preparation_cancellation_tests::` | 0 | PASS: 15 tests |
| `cargo xtask check desktop-native` | 0 | PASS |
| `cargo xtask check all` (single serial run) | 0 | PASS: static, Rust, Bun checks, Playwright E2E, and desktop checks; architecture check reported 61 accepted warnings |
| `cargo xtask check all` (earlier overlapping run) | 1 | FAIL: two simultaneous invocations collided on E2E port 4173; rerun after both exited passed |

The overlapping failure was an invocation collision: one Playwright run
reported `Port 4173 is already in use`, followed by connection-refused errors.
The single-run verification above completed all gates successfully. No test was
skipped by the final `check all` run. Real `SendInput` and in-game playback
qualification were not performed; all successful session Arm coverage uses a
mock sender as described above. The final `check all` architecture warnings
are existing allowlisted/advisory findings; that gate reported PASS.
