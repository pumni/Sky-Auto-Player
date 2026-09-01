# Wave 5 — Legacy Python Product Retirement

Wave 4 established the native desktop as the sole supported product runtime.
Wave 5 removes the retired Python product surface and its compatibility
binding/build graph while deliberately retaining a small Python repository
tooling boundary until Wave 6.

## Current boundary

The supported product is:

```text
React/TypeScript -> Tauri -> Native Rust application services
```

The production portable artifact contains no Python interpreter, Python
extension, Python Core sidecar, or frozen Python runtime. All 21 desktop
commands are Native-owned and the production router has no Python fallback.

The Rust player graph is direct:

```text
sky_desktop_shell -> sky_player -> sky_dispatch_core/sky_dispatch_win32
```

The calibration measurement remains in the isolated native calibration
executable, and `sky_updater` remains the transaction/security boundary.

## Retired production surfaces

The following are retired and must not be reintroduced as supported paths:

- Textual/TUI and the Python CLI;
- Python application entrypoints and the Python product package;
- Python Core/desktop IPC;
- the `sky_player_rs` extension and PyO3 bridge;
- maturin wheel production builds;
- PyInstaller product packaging;
- Python runtime or `.pyd` files in the portable artifact.

## Retained repository material

Python may temporarily remain only as explicitly classified repository
material:

- release and verification scripts still executed by `uv`;
- small tooling tests;
- frozen oracle-generation evidence where the committed fixture is the test
  input;
- updater migration fixtures representing an old sidecar-containing release;
- historical documentation and rollback evidence.

These files are not installable product code and are not runtime dependencies.
Wave 6 will replace the remaining release/CI orchestration with `cargo xtask`.

The complete per-file retirement record is maintained in
`docs/migration/wave5-python-retirement-ledger.md` and its JSON companion.
Classifications are `MIGRATED`, `OBSOLETE`, `TRANSPORT_ONLY`, `DUPLICATE`,
`FIXTURE_FROZEN`, and `TOOLING_RETAINED`; each deleted test or source surface
must have a ledger entry and concrete evidence.

## Evidence labels

- **PROVEN PARITY** — direct Rust/TypeScript tests or frozen independent
  contract fixtures cover the behavior.
- **NATIVE IMPLEMENTATION** — the native owner exists, with behavior covered
  in the applicable native tests.
- **RETIRED PRODUCTION PATH** — removed from the product graph and package.
- **RETAINED ORACLE/ROLLBACK** — repository-only material retained for Wave 6
  migration or old-install update compatibility.

## Phase status

```text
Phase 9:                         CLOSED
Phase 10 product/runtime/binding: CLOSED
Phase 10 Python tooling cleanup: PENDING Wave 6
Phase 11 xtask/build orchestration: PENDING Wave 6
Phase 12 optimization:           PENDING later
```

The temporary canonical release assembler during Wave 5 was
`scripts/build_portable_release.py`, supported by
`scripts/release_common.py`. Those scripts invoked native Cargo/Bun builds
directly and did not build a wheel, invoke maturin or PyInstaller, import the
old product package, or ship Python runtime files. Wave 6 retired these
historical canonical scripts in favor of `cargo xtask`.
