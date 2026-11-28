# WP4 Source Organization Evidence

**Disposition:** WP4A and WP4B implementation and local validation are complete for leader review. Hosted checks at the evidence HEAD are reported separately in the issue handoff; this report is not merge or release approval.

## Source identity and commits

- Authorized base: `f89baf898ab0db98fc2c49c785958e0525014a45`.
- WP4A: `f31ce1b137b2456f38e07dd40175e851a6ed0e5b` — `refactor: organize app core and native adapter sources`.
- WP4B: `03e85b1aa87dc014e11381442f7873e148f8f8a4` — `refactor: organize desktop IPC and runtime sources`.
- Exact final implementation tested: commit `03e85b1aa87dc014e11381442f7873e148f8f8a4`, tree `1780eb8e631cfa8a69e7d8cd325159fb189af4b7`. Evidence is committed separately, so its commit does not change the tested implementation tree.
- Integration branch: `refactor/source-architecture`; no additional branch was created.

## Ownership map and organization

| Existing owner | New physical owners | Retained facade or authority |
| --- | --- | --- |
| `crates/sky_app_core/src/catalog.rs` | `catalog/identity.rs`, `catalog/ranking.rs` | Catalog index, state, search, public API and compatibility paths stay in `catalog.rs`; existing `search_order.rs` is reused. |
| `crates/sky_app_core/src/song.rs` | `song/legacy.rs`, `song/schedule.rs`, `song/risk.rs`, `song/tests.rs` | Shared song types and stable public paths remain at the facade; decode/planner/numeric modules stay in place. |
| `crates/sky_native_adapters/src/lib.rs` | `atomic_file.rs`, `settings_store.rs`, `settings_store_tests.rs`, `library_store.rs`, `catalog.rs`, `catalog/{builtin_source.rs,file_source.rs,composer.rs}`, `catalog_tests.rs`, `tests.rs` | Adapter traits, catalog cache and public exports remain owned by the existing crate/facades. Existing atomic replace semantics are in one shared implementation. |
| `apps/desktop/src-tauri/src/commands.rs` | `commands/dto/{bootstrap,catalog,diagnostics,library,playback,settings,update}.rs`, `commands/tests.rs` | Tauri command functions, dispatch helpers, ownership matrix and handler macro remain in `commands.rs`; DTOs preserve explicit `commands::Type` re-exports. |
| `apps/desktop/src-tauri/src/lib.rs` | `lib_tests/{ipc,update_smoke}.rs`, `selftest.rs` | Entrypoint, startup composition, plugins, invoke-handler registry, window and shutdown order remain in `lib.rs`. Existing selftest entry points are re-exported. |
| `apps/desktop/src-tauri/src/native_runtime.rs` | `native_runtime/{bootstrap,settings,update,events}.rs` | Runtime struct/state/locks/reservation, dispatch match and runtime lifecycle authority remain in the facade; event hub ownership and synchronization stay cohesive in `events.rs`. |
| `apps/desktop/src-tauri/src/native_runtime/catalog.rs` | `native_runtime/catalog/{library,viewport,analysis}.rs` | Load/claim/cache/composition/reload lifecycle remains in `catalog.rs`; existing cache/reconcile/search/sheet owners remain intact. |

Actual functions and impl blocks were physically moved to these owners. Tauri `#[tauri::command]` functions and generated handler registration were not extracted. No playback/session engine, updater, frontend store or release implementation was changed.

### Informational LOC before/after

Counts are `physical lines / nonblank lines`, measured from the WP4 starting source (`f89baf8`) and accepted implementation source (`03e85b1`). The before group is the monolithic source owner; the after group is its facade plus extracted implementation and test owners. Existing sibling modules that were not part of those moved owners are excluded consistently. Physical lines include blank lines; nonblank lines exclude lines containing only whitespace. Formatting, imports and test ownership can change counts, so LOC is informational rather than a complexity target.

| Owner group | Before physical/nonblank | After physical/nonblank | Final grouping (nonblank LOC) |
| --- | ---: | ---: | --- |
| App core catalog | 826/764 | 841/778 | catalog facade 498; identity 73; ranking 207 |
| App core song | 857/818 | 882/843 | facade 88; legacy 89; schedule 211; risk 235; tests 220 |
| Native adapters | 2,111/1,982 | 2,162/2,036 | crate facade 29; atomic 47; library 50; settings 577; settings-store tests 19; catalog facade 9; built-in 127; file 162; composer 319; catalog tests 290; adapter tests 407 |
| Desktop commands | 1,513/1,391 | 1,531/1,413 | facade 392; tests 279; DTO modules: bootstrap 39, catalog 239, diagnostics 14, library 53, playback 259, settings 106, update 32 |
| Desktop lib and extracted tests | 1,474/1,400 | 1,472/1,400 | facade 421; IPC tests 574; update smoke tests 89; selftest 316 |
| Native runtime | 1,833/1,746 | 1,903/1,816 | facade 789; bootstrap 239; settings 154; update 103; events 531 |
| Native runtime catalog | 1,605/1,557 | 1,642/1,598 | facade 687; library 257; viewport 333; analysis 321 |

## Regression coverage and selector mapping

The exact-final all-features `--list` inventory contains 1,514 unique Rust selectors and no benchmarks, versus 1,513 at the accepted baseline. The one added selector is `checks::tauri_metadata::tests::v4_native_adapter_retirement_guards_scan_extracted_owners_and_required_drops`, which guards the moved updater-retirement source owners and required drop keys. No existing test was removed.

- 62 generated DTO export selectors moved from `commands::export_bindings_<type>` to `commands::dto::<capability>::export_bindings_<type>`: bootstrap 2, catalog 20, diagnostics 2, library 7, playback 22, settings 6, update 3.
- Five adapter catalog selectors moved from `tests::<name>` to `catalog_tests::<name>`: `builtin_source_is_manifest_driven_and_fails_as_one_source`, `catalog_observer_composition_matches_production_composition`, `catalog_source_handles_missing_directory_as_empty`, `catalog_source_rejects_non_directory_and_ignores_unsupported_files`, and `library_manifest_round_trips_and_imported_folders_are_composed`.
- `tests::persisted_config_coercions_keep_distinct_integer_and_text_rules` moved to `settings_store::tests::...`.
- Existing `commands::tests::*`, IPC tests, update smoke tests, core catalog/song tests, and runtime selectors were kept stable using named modules/paths. Exact final inventory command: `node scripts/toolchain-pin.mjs exec -- cargo test --workspace --all-features --locked -- --list` (repository root; exit 0; 52.5s).
- Focused desktop `tauri-test` suite: 349 passed, 0 failed, 4 ignored (353 total). Focused app-core and adapter suites: 127 passed, 0 failed, 2 ignored. The final canonical aggregate below reran workspace tests at the exact final implementation SHA.

## Equivalence, guards and cleanup

- Against accepted WP3 source `1f70882a87851eeb935399b8fdc8c827d0d79941`, normalized locked Cargo metadata is identical: 8 workspace members, 73 direct edges, 544 resolved nodes; semantic SHA-256 `8359fcd5034a6e64c7d927dbadad1df06245ed476e2bd8bc1c0f83ae8fb43e6c`.
- Normalized Cargo tree projections are identical: production normal/build 1,018 lines (`c23a486c1034067cc8b5fb304e4b23dbcbc6c36bdb14cc81e82b7aa2af562e68`); default production and updater-fixture features 2,977 lines (`a16c58dd14c862141a187981fa88dbc9227066cf4cc94d22b05beb1aaaba23fa`); qualification features 2,793 lines (`beeaece1c43c3341aaf777b04febc57131db8e85dab93be71800e3e552fc9d5a`). Package identities, feature boundaries, Cargo manifests, `Cargo.lock`, compiler pin and profiles are unchanged from accepted WP3.
- All 93 checked-in generated binding paths and blobs are unchanged from the accepted source. `cargo xtask bindings check` passed; final canonical validation also passed its binding checks. The Cargo.lock blob remains `4993318c716321328f606b039051799151a29dbe` (SHA-256 `a30234510d815f56e1ca49c9168e14e3189166b9c401216c10e288a1f145e3ca`).
- Unsafe inventory is unchanged at 19 production and 7 test-only blocks. The existing allowlist entry moved one-to-one to the same existing unsafe block in `atomic_file.rs`, retaining its reason and scope. No new unsafe code or broader exception was added.
- Existing source-text consumers now inspect actual extracted owners: updater retirement checks cover moved adapter modules; terminal-release checks inspect `native_runtime/bootstrap.rs`; architecture scans recurse through runtime modules. Command ownership continues to inspect the actual handler registry and runtime dispatch in their retained owners. Path-dependent tests/selectors and documentation links were reconciled; the small core facades intentionally preserve existing public paths.
- Normalized architecture warning identities changed from 70 to 68: only `regular_module_lines|crates/sky_app_core/src/catalog.rs` and `regular_module_lines|crates/sky_app_core/src/song.rs` were removed. No warning identity was added; thresholds and suppressions are unchanged. The previously accepted `rust_toolchain.rs` advisory remains.
- Removal ledger: core ranking/identity, legacy/schedule/risk and adapter store/source/composition implementations moved from monolithic owners to the listed modules; IPC DTOs, desktop tests/selftest, runtime bootstrap/settings/update/events, and catalog library/viewport/analysis implementations moved to their listed owners. Their old inline definitions were removed and callers/tests migrated. No duplicate implementation, stale active `rust/` path, compatibility wrapper around duplicated logic, unfinished placeholder or generated file was left in the repository. Intentional retained references are facade exports, the actual command registry/dispatch owners, and source-text tests that now target those real owners.

### Generated TypeScript output investigation

An earlier disposable WP3 checkout contains 89 untracked TS-RS `.ts` files under `apps/desktop/src-tauri/bindings/`. The prior test was launched from `%TEMP%` with an absolute manifest path, so Cargo did not discover the repository-root `.cargo/config.toml`; TS-RS used its crate-local default output directory. This output is outside the integration checkout and was left untouched. WP4 direct test commands used repository-root CWD and an explicit absolute external `TS_RS_EXPORT_DIR`; canonical tooling uses an isolated export directory. The current repo and clean build worktree had no generated drift.

## Validation on exact implementation source

| Command / CWD | Result |
| --- | --- |
| `node scripts/toolchain-pin.mjs exec -- cargo fmt --all -- --check` (repository root) | Exit 0 on final source. |
| `node scripts/toolchain-pin.mjs exec -- cargo xtask check static --skip-supply-chain` (repository root) | Exit 0; final canonical aggregate also reran static checks; 68 architecture warnings and 0 security findings. |
| `node scripts/toolchain-pin.mjs exec -- cargo test --workspace --all-features --locked -- --list` (repository root) | Exit 0; 1,514 unique selectors, 0 benchmarks; 52.5s. |
| `node scripts/toolchain-pin.mjs exec -- cargo xtask check all` (repository root) | Exit 0; 17m55.46s on exact commit/tree above. Includes pinned formatting, static/supply-chain, strict clippy, workspace all-feature tests, Bun checks, desktop E2E, desktop feature checks, and binding checks. The successful aggregate does not print a single total test count or per-case E2E count. E2E completed in the successful aggregate; no retry was needed for WP4. |
| `node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist` (fresh detached clean checkout, CWD `apps/desktop`) | Exit 0; 6m44.93s. Verbose child invocation was `cargo build --locked --profile dist --bins --features tauri/custom-protocol`. Build-info selftest reports commit `03e85b1aa87dc014e11381442f7873e148f8f8a4`, rustc 1.99.1, Win32 backend true. Pre/post status was clean; no legacy `rust/` directory. Binary: 16,670,208 bytes, SHA-256 `f232b6ace3c161b667d19e4fb60a7553d0bf846fb44922fb0757bfc6f437f15b`. Vite emitted its existing >500 KB chunk advisory; command succeeded. |

The first canonical attempt stopped at a formatting issue in the extracted test file; the next stopped at strict Clippy's `manual_inspect` finding in moved atomic cleanup. Both were corrected without suppressions, each in its owning implementation commit. The final canonical command above passed after those corrections. The earlier WP3 E2E timeouts are recorded in WP3 evidence; they passed their isolated retry and WP3 canonical retry. No timeout policy, worker count, assertion, or test gate was changed for WP4.

## Pending proof and limits

Hosted CI was not awaited. The source-bound hosted Windows/package/updater/qualification proofs must be checked at the pushed evidence HEAD. No production signing, release, deployment, merge, or physical-input qualification was run. No raw local aggregate log was committed; exact commands, source identities, durations, focused suite counts, and build artifact digest are captured above.
