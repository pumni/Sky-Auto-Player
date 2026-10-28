# Issue #560 Package D qualification

This record qualifies the accepted Package C implementation at
`e15491a0653501499c4ccd51c1aa3ec147781825` and the Package D test/evidence additions
on the same implementation branch. It does not change the catalog architecture, page size,
resource manifest, release workflow, or gameplay input path.

## Qualification matrix

| Area | Evidence | Result |
|---|---|---|
| Frontend paging, active-page and index-map bounds | `desktop/src/state/store.test.ts`: 96-song single-page case; explicit built-in membership and 0/1/96/99/100/101/199/200/201/2,438/10,000 paging; 10,000-song eviction/refetch/selection/playback sweep; query and source invalidation; latest-intent navigation races | PASS |
| Native cache and worker limits | `desktop/src-tauri/src/native_runtime/tests/catalog_package_b.rs`: 8-entry search-order LRU, 100,000-index cap, 128-entry analysis LRU, 2 active workers and 64 queued IDs, source invalidation and safe recomputation | PASS |
| Compact viewport response and stale-state rejection | `desktop/src-tauri/src/native_runtime/tests/catalog_viewport_projection.rs`: exact response reuse, explicit ordered IDs, first/last/outside selection, 20/40/200 rows, shift/back parity, selected detail independence, Like/playlist/settings/generation invalidation and superseded intent | PASS |
| WRatio ordering and source membership | `cached_search_orders_match_the_wratio_reference_across_scopes_and_pages` covers 10,000 rows and All/Liked/playlist membership; `builtin_smart_source_uses_native_builtin_membership_only` and the frontend explicit-membership test protect Built-in isolation | PASS |
| Search, navigation, selection and playback | `desktop/tests/e2e/library.spec.ts`: A-B-A search, Built-in page navigation, selection and Show now playing; new test traverses Song 100 → 101 → 100 with Next/Previous while selection remains stable. Store tests cover Built-in Auto Play across index 99 → 100 and full-result Shuffle traversal | PASS (mock bridge/playback; no physical input) |
| Accessibility and supported layout | The Built-in pager E2E checks keyboard page jump, focus return, ARIA row/page state, axe serious/critical violations, forced colors, reduced motion, 800×560 and a narrow browser container | PASS |
| Built-in source and installed-layout resources | `cargo xtask builtin-catalog verify`; `cargo xtask builtin-catalog verify-installed --root rust/target/debug/builtin-songs`; adapter all-or-nothing fallback test | PASS for source and local Tauri debug resource tree; NSIS installer not available |
| Windows/Tauri performance and memory profile | `windows-tauri-profile.json`: actual packaged-assets debug shell, WebView2 native bridge, 96/1,000/10,000 disposable song fixtures, direct search and viewport calls, virtualized scroll sweep/back and memory snapshots | PASS with stated measurement limits below |
| v4.2.0 → candidate installed upgrade | Expected local NSIS bundle directories are absent; no historical v4.2.0 installer/candidate pair is available in this checkout | **NOT RUN / RELEASE BLOCKER** |

The frontend playback cases use the repository mock bridge and mock playback timer. Native
playback safety remains covered by the existing Rust suite, but this qualification sent no
physical gameplay input and does not claim live-game playback acceptance.

## Deterministic fixtures and Windows/Tauri profile

The profile harness creates one deterministic, one-note JSON user song per row, with stable
`Package D fixture song NNNNN` filenames. It uses 96, 1,000, and 10,000 user songs; the verified
immutable Built-in Songs source remains at 96. Observed All totals were therefore 192, 1,096, and
10,096. The shell was built with `packaged-assets` and launched only after confirming no other
Sky shell was active. Each run used fresh `SKY_APP_DATA_ROOT` and WebView2 user-data folders under
a unique temp directory, set `update.auto_check=false`, and removed that directory afterward
(`cleanup_issues=[]`). No installed app, real AppData, release metadata, updater, or signer was
used.

Host: Windows 11 build 10.0.26300, AMD Ryzen 5 5500U, 12 logical processors; Rust 1.99.1
(`58cf8f9c3`, `x86_64-pc-windows-msvc`). The two installed WebView2 runtime directories were
157.0.4341.16 and 157.0.4341.24; this harness did not record which version the WebView selected.
The executable SHA-256 and capture time are in the JSON evidence.

| User fixtures | All total | UI scroll jumps | JS heap before → after sweep (bytes) | Shell private bytes before → after | Mounted rows after |
|---:|---:|---:|---:|---:|---:|
| 96 | 192 | 1 | 18,310,992 → 18,363,460 | 8,228,864 → 8,228,864 | 26 |
| 1,000 | 1,096 | 10 | 13,670,800 → 17,980,276 | 8,945,664 → 8,949,760 | 26 |
| 10,000 | 10,096 | 100 | 13,771,932 → 10,098,680 | 20,824,064 → 20,901,888 | 26 |

The profile measures WebView V8 heap and the Tauri shell process; it does not sum memory for the
separate WebView2 renderer/GPU processes, does not capture peak heap, and is not a memory cap
assertion. The 10,000-row heap fell after the sweep, consistent with collection during sampling;
cache-count limits are established by deterministic state/native tests, not inferred from these
process snapshots. The virtualized DOM held 26 track rows after each sweep. Timings are diagnostic
only and have no CI threshold.

Repeated exact Tauri-bridge search p50/p90 (milliseconds) and viewport metadata p50/p90 are
retained in the JSON. Representative query results:

| User fixtures | Query | First / middle / last p50/p90 (ms) |
|---:|---|---|
| 96 | empty / short / fuzzy | 6.4/6.8 · 6.1/6.4 · 6.2/6.6 (first only; fixture fits one window) |
| 1,000 | empty | 6.4/6.8 · 6.5/6.8 · 6.3/6.9 |
| 1,000 | short | 6.6/7.1 · 6.6/7.0 · 6.7/7.3 |
| 1,000 | fuzzy | 6.6/7.2 · 6.6/6.9 · 6.6/7.0 |
| 10,000 | empty | 6.6/7.9 · 6.7/8.0 · 6.3/7.4 |
| 10,000 | short | 6.8/7.6 · 6.9/7.7 · 6.4/6.7 |
| 10,000 | fuzzy | 6.4/6.8 · 6.4/6.9 · 6.6/7.0 |

The separate release-profile catalog-index harness includes ranking work for every sample. At
10,000 rows, the All-source WRatio query measured 133.872–133.999 ms p50 and 134.521–138.185 ms
p90 across first/middle/late windows; Liked measured 35.468–35.615 / 35.775–35.895 ms and playlist
56.310–56.371 / 56.453–57.782 ms. One-character All search measured 131.975–135.903 µs p50 and
132.432–137.031 µs p90. These values retain the WRatio ranking contract; repeated order reuse is
covered by the native cache tests. Complete All/Liked/playlist, empty/one-character/fuzzy
first/middle/late p50/p90 records are in
[release-core-search-isolated.txt](release-core-search-isolated.txt).

## Release-profile native measurements

The explicit-ID benchmark uses the same production-shaped 40-row window for 96 songs and
200-row windows at 1,000/10,000 songs. It reports selection at none/first/last/outside. The table
shows p50 ranges across those placements; full p50/p90 values for every placement and cold,
exact-warm, shift +1, and shift-back phase are retained in
[release-native-viewport-isolated.txt](release-native-viewport-isolated.txt).

| Songs / viewport | Cold p50 / p90 range | Exact-warm p50 / p90 range | Shift +1 p50 / p90 range | Shift-back p50 / p90 range |
|---:|---:|---:|---:|---:|
| 96 / 40 | 2.49–5.84 / 2.96–9.18 ms | 79–91 / 84–167 µs | 90–95 / 339–404 µs | 91–98 / 100–121 µs |
| 1,000 / 200 | 13.21–17.08 / 14.25–24.40 ms | 388–521 / 418–713 µs | 451–909 / 729–1,666 µs | 424–1,249 / 471–1,943 µs |
| 10,000 / 200 | 12.80–15.81 / 13.13–18.81 ms | 392–463 / 420–522 µs | 438–458 / 672–676 µs | 431–449 / 450–666 µs |

Each cell gives the min–max across no selection/first/last/outside selection placements; all
per-placement p50/p90 values, including the 96-song 9.18 ms cold p90 and 1,000-song shifted p90
tails, remain in [release-native-viewport-isolated.txt](release-native-viewport-isolated.txt).
The accepted Package B warm p50 target was within 2× Package A for all measured placements;
current p50s also remain below that threshold. Approved A/B/C comparison data is available in the
[Agent Lead Package B handoff](https://github.com/pumni/Sky-Auto-Player/issues/560#issuecomment-6472735431).

Implicit-ID results are separate in
[release-native-implicit-viewport.txt](release-native-implicit-viewport.txt); the 96-song shift
case is unavailable because the viewport covers that whole fixture. This run also measures
selected-detail-after-hydration separately: 10,000-song p50 is 13.2–14.3 µs across first/last/
outside placement. Reconciliation p50/p90 was 1.140/1.181 ms at 96, 12.200/12.290 ms at 1,000,
and 132.050/139.430 ms at 10,000; see
[release-native-reconcile-isolated.txt](release-native-reconcile-isolated.txt).

The first native release command against the preexisting default Cargo target exited 101 because
its cached `sky_app_core` artifact lacked the current `CatalogSearchOrder` API. No cache was
cleaned or overwritten. The benchmark was rerun successfully with a fresh ignored target at
`rust/target/package-d-bench`; the failed compile log remains in
[release-native-viewport.txt](release-native-viewport.txt). The first default-target core query
run exited 0 but was excluded for the same cache-provenance reason; the clean-target result is the
qualifying record above.

## Resource and upgrade qualification

`cargo +1.99.1 xtask builtin-catalog verify` passed with schema 1, 96 active and 0 retired songs.
The generated Tauri debug resource tree at `rust/target/debug/builtin-songs` passed
`verify-installed` with the same count and hashes. `tauri.conf.json` maps source sheets and the
committed manifest to `builtin-songs/sheets` and `builtin-songs/manifest.json`; the adapter test
`builtin_source_is_manifest_driven_and_fails_as_one_source` passed. This is local packaged-resource
evidence, not proof against an NSIS candidate.

Upgrade qualification is **NOT RUN / RELEASE BLOCKER**. None of the expected local
`desktop/src-tauri/{debug,release}/bundle/nsis` or `rust/target/{debug,release}/bundle/nsis`
directories contains a package. This checkout therefore lacks the exact v4.2.0 previous installer
and candidate installer needed by the isolated test. No app was installed or launched for upgrade,
no real user state was seeded or read, and no release metadata/signing path was touched. Release
promotion must wait for isolated disposable-runner qualification with both exact artifacts, using
the governed NSIS/updater-fixture path described in [V4 Tauri packaging](../../v4-tauri-packaging.md).

## Commands and exit codes

| Command | Exit | Result |
|---|---:|---|
| From `desktop`: `bun run test -- src/state/store.test.ts` | 0 | 144 passed |
| From `desktop`: `bun run test:e2e -- tests/e2e/library.spec.ts --workers=1` | 0 | 21 passed |
| From `desktop`: `bun run typecheck` | 0 | TypeScript check passed |
| From `desktop`: `node --check scripts/profile-builtin-library.mjs` | 0 | Profile script syntax passed |
| From `desktop`: `bunx eslint scripts/profile-builtin-library.mjs --max-warnings 0` | 0 | Profile script lint passed after the first full-check finding |
| From `desktop`: `bunx prettier --check scripts/profile-builtin-library.mjs` | 0 | Profile script formatting passed after the second full-check finding |
| `cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib --features tauri-test catalog_ -- --test-threads=1` | 0 | 25 passed, 4 opt-in benchmarks ignored |
| `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked -p sky_app_core catalog_ -- --test-threads=1` | 0 | 4 passed, 2 opt-in benchmarks ignored |
| `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked -p sky_app_core cached_search_orders_match_the_wratio_reference_across_scopes_and_pages` | 0 | 1 passed; exercises 10,000-row All/Liked/playlist cases |
| `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked -p sky_native_adapters builtin_source_is_manifest_driven_and_fails_as_one_source` | 0 | 1 passed |
| `cargo +1.99.1 xtask builtin-catalog verify` | 0 | 96 active, 0 retired; source hashes and parseability verified |
| `cargo +1.99.1 xtask builtin-catalog verify-installed --root rust/target/debug/builtin-songs` | 0 | 96 active, 0 retired; local Tauri resource tree verified |
| `cargo +1.99.1 build --manifest-path desktop/src-tauri/Cargo.toml --locked --features packaged-assets --bin sky_desktop_shell` | 0 | Debug shell built with embedded frontend/resources; no installer/signing |
| From `desktop`: `node scripts/profile-builtin-library.mjs --output=../docs/evidence/issue-560-package-d/windows-tauri-profile.json` | 0 | Actual Windows/Tauri profile, three fixture sizes, temp cleanup clear |
| Release query, viewport, viewport/detail, and reconciliation commands (exact invocations below; isolated target) | 0 each | Full p50/p90 logs retained |
| From repository root: `cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --release --lib --features tauri-test native_runtime::tests::catalog_viewport_benchmark::opt_in_package_b_production_shape_viewport_benchmark -- --ignored --nocapture` using preexisting default target | 101 | Stale dependency artifact; superseded by isolated-target pass, log retained |
| From repository root: `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked --release -p sky_app_core opt_in_package_b_catalog_query_benchmark -- --ignored --nocapture` using preexisting default target | 0 | Excluded because cached-artifact provenance was uncertain; isolated-target pass retained |
| `cargo +1.99.1 xtask check all` | 1, 1, 1, 0 | The first runs caught profile-script lint and formatting issues, both corrected. The third run passed Rust and desktop unit stages but its six-worker E2E phase reported 106 passed, 2 skipped, 1 failed: the new test assumed Previous would traverse after mock playback crossed the three-second restart threshold. A focused rerun then exposed that Song 100 was virtualized after returning to page 1. The test now scrolls it back into view and checks page navigation/Show now playing while preserving selected Song 100 and playing Song 101. Final run: full check passed, including all desktop tests, configured six-worker E2E, and native checks. |

The profile harness passed `node --check scripts/profile-builtin-library.mjs` and
`bun run typecheck` (both from `desktop`). Preliminary profile launches were not counted: one
preflight correctly refused while an existing Sky shell held the singleton; after the user closed
that shell, an early launch hit a missing Vite URL, and the first fixture query exposed that the
native catalog indexes filename stems. The harness was corrected to launch the embedded-assets
debug shell and to use matching filename-stem queries. Only the successful final profile above is
qualification evidence; it reports zero cleanup issues.

The four release benchmark invocations used an isolated, previously absent Cargo target so they
could not overwrite the preexisting target artifacts. From the repository root, the target was set
once and each command below was run sequentially:

```powershell
$env:CARGO_TARGET_DIR = Join-Path (Get-Location) 'rust/target/package-d-bench'
cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked --release -p sky_app_core opt_in_package_b_catalog_query_benchmark -- --ignored --nocapture
cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --release --lib --features tauri-test native_runtime::tests::catalog_viewport_benchmark::opt_in_package_b_production_shape_viewport_benchmark -- --ignored --nocapture
cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --release --lib --features tauri-test native_runtime::tests::catalog_viewport_benchmark::opt_in_package_b_viewport_projection_benchmark -- --ignored --nocapture
cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --release --lib --features tauri-test native_runtime::catalog_reconcile::package_b_benchmark_tests::opt_in_package_b_catalog_reconciliation_benchmark -- --ignored --nocapture
```

All four exited 0. The first two failed/default-target attempts are documented above with their
logs; the failed native viewport attempt exited 101, while the default-target core query exited 0
but was excluded due to uncertain cached-artifact provenance. Neither default target was cleaned.
The upgrade attempt was skipped because the required v4.2.0 and candidate installers were absent;
this is the explicit release blocker described above.

No CI, worker count, timeout, resource, release, updater, or physical-input behavior was changed.
