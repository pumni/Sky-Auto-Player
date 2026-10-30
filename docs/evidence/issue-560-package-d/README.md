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
| Search, navigation, selection and playback | `desktop/tests/e2e/library.spec.ts`: A-B-A search, Built-in page navigation, selection and Show now playing; new test plays Song 100 → 101 across a page boundary, changes pages, and confirms selection/playback identity remain independent. Store tests cover Built-in Auto Play across index 99 → 100 and full-result Shuffle traversal | PASS (mock bridge/playback; no physical input) |
| Accessibility and supported layout | The Built-in pager E2E checks keyboard page jump, focus return, ARIA row/page state, axe serious/critical violations, forced colors, reduced motion, 800×560 and a narrow browser container | PASS |
| Built-in source and installed-layout resources | `cargo xtask builtin-catalog verify`; `cargo xtask builtin-catalog verify-installed --root rust/target/debug/builtin-songs`; adapter all-or-nothing fallback test | PASS for source and local Tauri debug resource tree; NSIS installer not available |
| Windows/Tauri performance and memory profile | `windows-tauri-profile.json`: corrected committed harness profile with explicit ordered-ID alternating viewport shifts, 96/1,000/10,000 disposable fixtures, accepted-response checks, virtualized scroll sweep/back and memory snapshots | PASS with stated measurement limits below |
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
The qualifying profile records source/harness SHA `5b195359830ac6904b7b85f536b6f46556c35151`,
branch `feat/builtin-library-paging`, and a clean worktree at launch. It ran this build command at
that SHA:

```powershell
cargo +1.99.1 build --manifest-path desktop/src-tauri/Cargo.toml --locked --features packaged-assets --bin sky_desktop_shell
```

The executable SHA-256 is `a56c61bbbb9eca75719015e106da0d24f6df4cd521767ab3484054c60fa1144e`;
the capture time and source metadata are in the JSON. The evidence/docs commit is later than the
profile source SHA and changes evidence/documentation only.

The prior `windows-tauri-profile-original-c-head.json` is preserved byte-for-byte (SHA-256
`AF6314C5B86BBA7E0807B6D71CFD215A1D7C1DBF9813442C4FB5FB399804A686`). It records Package C
`source_head=e15491a...` and did not record harness cleanliness/revision; it remains historical
only and is excluded from corrected shift timing and D provenance claims.

| User fixtures | All total | UI scroll jumps | JS heap before → after sweep (bytes) | Shell private bytes before → after | Mounted rows after |
|---:|---:|---:|---:|---:|---:|
| 96 | 192 | 1 | 17,101,676 → 17,151,588 | 8,028,160 → 8,028,160 | 26 |
| 1,000 | 1,096 | 10 | 14,528,428 → 18,760,004 | 9,129,984 → 9,216,000 | 26 |
| 10,000 | 10,096 | 100 | 14,197,192 → 10,432,328 | 20,131,840 → 20,418,560 | 26 |

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
| 96 | empty / short / fuzzy | 6.2/6.5 · 6.4/7.2 · 6.9/7.1 (first only; fixture fits one window) |
| 1,000 | empty | 6.2/6.4 · 6.1/7.0 · 6.8/7.5 |
| 1,000 | short | 6.9/7.2 · 6.8/7.2 · 6.5/7.1 |
| 1,000 | fuzzy | 6.8/7.0 · 6.8/7.4 · 6.7/8.0 |
| 10,000 | empty | 6.6/7.1 · 6.6/7.1 · 6.7/8.2 |
| 10,000 | short | 6.7/7.5 · 6.9/8.2 · 6.5/7.1 |
| 10,000 | fuzzy | 6.5/6.9 · 6.5/7.0 · 6.4/6.6 |

The separate release-profile catalog-index harness includes ranking work for every sample. At
10,000 rows, the All-source WRatio query measured 133.872–133.999 ms p50 and 134.521–138.185 ms
p90 across first/middle/late windows; Liked measured 35.468–35.615 / 35.775–35.895 ms and playlist
56.310–56.371 / 56.453–57.782 ms. One-character All search measured 131.975–135.903 µs p50 and
132.432–137.031 µs p90. These values retain the WRatio ranking contract; repeated order reuse is
covered by the native cache tests. Complete All/Liked/playlist, empty/one-character/fuzzy
first/middle/late p50/p90 records are in
[release-core-search-isolated.txt](release-core-search-isolated.txt).

## Corrected Windows/WebView2 viewport transitions

For each available query window, the harness first accepts the original explicit-ID window, then
measures 21 alternating transitions: original → shifted +1 → original → shifted +1 → original.
Every response must be accepted and match generation, requested first/last index, selected ID,
equal row width, and the full ordered expected ID list. Each shifted list is fetched independently
and its ordered overlap with the original (`original[1..] == shifted[..-1]`) is verified before
timing. Thus each `shift_plus_one` and `shift_back` sample is an actual adjacent transition; the
21-request exact-repeat sample remains separately labeled `exact_warm`.

A constant-width +1 window is marked unavailable when the result set ends before the final shifted
row. That occurred for the 96-row short/fuzzy results and the last short/fuzzy windows at 1,000 and
10,000 rows. The raw JSON records each unavailable case and reason. No partial window was timed as
a full-width shift.

The values below are end-to-end Tauri/WebView2 bridge times in milliseconds (p50 / p90, min–max
across measured query windows):

| User fixtures | Paired windows | Exact repeat | Shift +1 | Shift back | Unavailable windows |
|---:|---:|---:|---:|---:|---|
| 96 | 1 / 3 | 6.7 / 7.3 | 7.5 / 8.3 | 7.4 / 8.4 | short/fuzzy first |
| 1,000 | 7 / 9 | 6.8–7.2 / 7.3–7.8 | 7.0–7.5 / 7.5–8.8 | 7.1–7.4 / 7.6–8.6 | short/fuzzy last |
| 10,000 | 7 / 9 | 6.9–7.1 / 7.5–8.2 | 7.1–7.9 / 7.7–8.8 | 7.2–7.9 / 8.0–8.9 | short/fuzzy last |

This is a negative end-to-end result: most measured transitions are modestly slower than exact
repeat requests, with bridge overhead around 6–9 ms. These numbers do not isolate native cache
work. Keep the paired native Rust release benchmark below as the native-layer measurement; it was
not changed or rerun for this evidence correction. All individual query/window p50/p90 samples
remain in [windows-tauri-profile.json](windows-tauri-profile.json).

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
| From `desktop`: `bunx eslint scripts/profile-builtin-library.mjs --max-warnings 0` after the shift correction | 0 | Alternating-shift harness lint passed |
| From `desktop`: `bunx prettier --check scripts/profile-builtin-library.mjs` after the shift correction | 0 | Alternating-shift harness formatting passed |
| `cargo +1.99.1 build --manifest-path desktop/src-tauri/Cargo.toml --locked --features packaged-assets --bin sky_desktop_shell` at `5b195359830ac6904b7b85f536b6f46556c35151` | 0 | Packaged-assets debug shell built for the corrected profile |
| From `desktop`: `node scripts/profile-builtin-library.mjs --output=../docs/evidence/issue-560-package-d/windows-tauri-profile-d-corrected.json` | 0 | Clean committed harness; 96/1,000/10,000 fixtures; 21 alternating pairs per available window; cleanup issues empty |
| `cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --lib --features tauri-test catalog_ -- --test-threads=1` | 0 | 25 passed, 4 opt-in benchmarks ignored |
| `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked -p sky_app_core catalog_ -- --test-threads=1` | 0 | 4 passed, 2 opt-in benchmarks ignored |
| `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked -p sky_app_core cached_search_orders_match_the_wratio_reference_across_scopes_and_pages` | 0 | 1 passed; exercises 10,000-row All/Liked/playlist cases |
| `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked -p sky_native_adapters builtin_source_is_manifest_driven_and_fails_as_one_source` | 0 | 1 passed |
| `cargo +1.99.1 xtask builtin-catalog verify` | 0 | 96 active, 0 retired; source hashes and parseability verified |
| `cargo +1.99.1 xtask builtin-catalog verify-installed --root rust/target/debug/builtin-songs` | 0 | 96 active, 0 retired; local Tauri resource tree verified |
| `cargo +1.99.1 xtask check all` at the previous code/test SHA `8763d9f76a6f5993d22335ceef365bbb818f66a0` | 0 | Historical prior D check; not the final evidence-correction SHA |
| `cargo +1.99.1 xtask check all` at the pushed final evidence-correction SHA | To be posted in PR conversation after push | Run on exact immutable HEAD; no source commits afterward |
| Release query, viewport, viewport/detail, and reconciliation commands (exact invocations below; isolated target) | 0 each | Full p50/p90 logs retained |
| From repository root: `cargo +1.99.1 test --manifest-path desktop/src-tauri/Cargo.toml --locked --release --lib --features tauri-test native_runtime::tests::catalog_viewport_benchmark::opt_in_package_b_production_shape_viewport_benchmark -- --ignored --nocapture` using preexisting default target | 101 | Stale dependency artifact; superseded by isolated-target pass, log retained |
| From repository root: `cargo +1.99.1 test --manifest-path rust/Cargo.toml --locked --release -p sky_app_core opt_in_package_b_catalog_query_benchmark -- --ignored --nocapture` using preexisting default target | 0 | Excluded because cached-artifact provenance was uncertain; isolated-target pass retained |
| Earlier `cargo +1.99.1 xtask check all` attempts at prior D source | 1, 1, 1, 0 | Historical: profile lint, profile formatting, then a timing-sensitive E2E assumption were corrected. The successful prior run was at `8763d9f...`, not the new final evidence SHA. |

The corrected harness passed `node --check scripts/profile-builtin-library.mjs`,
`bunx eslint scripts/profile-builtin-library.mjs --max-warnings 0`, and
`bunx prettier --check scripts/profile-builtin-library.mjs` from `desktop`. Preliminary profile
launches were not counted: one preflight correctly refused while an existing Sky shell held the
singleton; after the user closed that shell, an early launch hit a missing Vite URL, and the first
fixture query exposed that the native catalog indexes filename stems. The harness was corrected to
launch the embedded-assets debug shell and use matching filename-stem queries. The historical C-
head profile is not qualifying; only the corrected committed profile at harness SHA
`5b195359830ac6904b7b85f536b6f46556c35151` is used for the alternating-shift evidence. It reports
zero cleanup issues.

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
