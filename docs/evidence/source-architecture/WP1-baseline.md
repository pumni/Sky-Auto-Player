# WP1 — Foundation and baseline

**Status:** Ready for leader review

**Observed:** 2026-11-28
**Scope:** Read-only repository investigation and baseline validation. No product, dependency, build, runtime, security, or release source was changed.

This report records the repository state and validation for WP1, and maps the active consumers that a root-workspace migration would affect. It does not authorize WP2 or choose unresolved topology.

## 1. Source identity and coordination

| Item | Identity / result |
|---|---|
| Authorized branch | `refactor/source-architecture` |
| Accepted PR base (`main`) | `5c79c8cf604072f331820126223188161d17d7b1` |
| Leader planning / tested source commit | `367b2031125048c4eb72b2a4f3c25fa21de0794a` |
| Tested source tree | `abd5183fd23ea50aabbdd0851c00e2b56a70068e` |
| Branch remote at investigation start | `origin/refactor/source-architecture` at the planning commit above |
| Starting/final pre-report worktree | Clean; no pre-existing local work was overwritten |
| Draft PR | [#690](https://github.com/pumni/Sky-Auto-Player/pull/690), cumulative draft, base `main`; at investigation start its head was the planning commit |
| Coordination authority | [Issue #689, authorized WP1 comment](https://github.com/pumni/Sky-Auto-Player/issues/689#issuecomment-7225255813) |

The remote branch was fetched without force to its remote-tracking ref, then checked out as the authorized local integration branch. The tested commit is planning-only. The final evidence commit is also documentation-only; it must not be described as a new product-code qualification.

Relevant open-PR overlap observed, without merging or editing any PR:

| PR | Relevant overlap |
|---|---|
| [#673](https://github.com/pumni/Sky-Auto-Player/pull/673) | `.github/workflows/ci.yml`, `release-v4.yml`, and `v4-metadata-recovery.yml`; intersects WP2 CI/release path work. |
| [#291](https://github.com/pumni/Sky-Auto-Player/pull/291) | Root `AGENTS.md`; intersects repository policy/documentation. |
| [#683](https://github.com/pumni/Sky-Auto-Player/pull/683) | Desktop `package.json` and `bun.lock`; intersects independent frontend dependency state. |
| [#63](https://github.com/pumni/Sky-Auto-Player/pull/63) | Windows updater E2E script plus historical release material; inspect before touching the updater harness. |

## 2. Host and toolchain baseline

| Component | Observed |
|---|---|
| OS / architecture | Windows 11 Home, build `10.0.26300`, x64 |
| Rustup | `rustup 1.29.1`; global active selector is `stable-x86_64-pc-windows-msvc` |
| Pinned Rust | `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, host `x86_64-pc-windows-msvc`, LLVM `23.1.1`; Cargo `1.99.1 (5f94df478 2026-08-27)` |
| Node / Bun | Node `v24.21.0`; Bun `1.4.4`, matching root `.bun-version` |
| MSVC / SDK | VS Build Tools 18.8.2; MSVC toolset files `14.51.36231`; Windows SDK `10.0.26100.0`. `cl.exe` was not on the shell PATH. |
| WebView2 | Evergreen runtime registry version `159.0.4402.13` (an older `158.0.4370.26` application directory is also present). |
| NSIS | `makensis.exe` was not found on PATH; no current-source NSIS candidate was built. |
| Relevant environment overrides | `RUSTUP_TOOLCHAIN`, `CARGO_TARGET_DIR`, `RUSTC`, `RUSTFLAGS`, `TS_RS_EXPORT_DIR`, `TAURI_CONFIG`, and related build overrides were unset at baseline. The isolated test-list command later set only its documented `CARGO_TARGET_DIR`. |

`node scripts/toolchain-pin.mjs read rust` passed and returned `1.99.1`. Direct `node scripts/toolchain-pin.mjs verify rust` exited 1 because this machine's global active selector is the `stable` alias, while the repository requires the exact pinned selector. The scoped command `RUSTUP_TOOLCHAIN=1.99.1 node scripts/toolchain-pin.mjs verify rust` passed. Canonical commands through `node scripts/toolchain-pin.mjs exec -- ...` passed and used Rust 1.99.1. This is a workstation selector mismatch, not a repository validation failure; no toolchain was installed or upgraded.

## 3. Current Cargo workspace, package identities, and graphs

The current workspace is `rust/Cargo.toml`, resolver 3, with eight members and no `default-members`: six libraries, `sky_xtask`, and the external desktop shell at `../apps/desktop/src-tauri`. `rust/Cargo.lock` is lockfile format v4, 140,647 bytes, SHA-256 `a30234510d815f56e1ca49c9168e14e3189166b9c401216c10e288a1f145e3ca`. Cargo metadata resolved 544 package/resolve nodes.

| Workspace package | Declared features | Direct dependencies (normal / build / dev / target-qualified) | Targets |
|---|---|---:|---|
| `sky_app_core` | none | 6 / 0 / 0 / 0 | library; 5 integration-test targets |
| `sky_desktop_shell` | `default`, `desktop-runtime`, `packaged-assets`, `tauri-test`, `tauri-update-fixture` | 20 / 1 / 0 / 2 | lib (`rlib`, `cdylib`, `staticlib`), binary, build script |
| `sky_dispatch_core` | `default`, `test-support` | 3 / 0 / 2 / 0 | library; 3 integration tests; 2 benches |
| `sky_dispatch_win32` | `default`, `qpc-read-counting`, `test-support` | 5 / 0 / 0 / 1 | library; build script |
| `sky_native_adapters` | none | 6 / 0 / 0 / 2 | library; 2 integration tests |
| `sky_player` | `default`, `qpc-read-counting`, `real-input-acceptance`, `test-support` | 8 / 0 / 4 / 0 | library; `rt-native-acceptance` binary; example; integration test |
| `sky_windows_security` | none | 2 / 0 / 0 / 1 | library; integration test |
| `sky_xtask` | none | 16 / 0 / 0 / 1 | `xtask` binary; integration test |

Profiles in `rust/Cargo.toml` are part of the compatibility baseline. `[profile.release]` sets `lto = false`, `codegen-units = 16`, `opt-level = 2`, `panic = "unwind"`, and `strip = "symbols"`. `[profile.dist]` inherits the release profile and overrides `lto = "thin"`, `codegen-units = 1`, and `opt-level = 3`. Preserve these values and the package/member identities during any cutover.

Locked metadata and target-specific graph commands ran through the pinned wrapper from the repository root:

| Purpose | Command arguments | Result |
|---|---|---|
| Workspace metadata | `cargo metadata --locked --manifest-path rust/Cargo.toml --format-version 1` | Exit 0, 0.88 s; 8 members / 544 resolved package nodes. |
| Shipping shell dependency graph | `cargo tree --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --target x86_64-pc-windows-msvc -e normal,build` | Exit 0, 0.59 s; 1,018 lines; SHA-256 `65e19b049f7381a4d4d472b73189b17047e4c3d0997839f537fb578739816383`. Uses the actual default shipping feature set. |
| Shipping shell feature graph | Same manifest/package/target, `-e features` | Exit 0, 0.64 s; 2,977 lines; SHA-256 `91a1a48125869ec999103f92a48fcfa5ea6921f2ac316bb6a4053a0c5e63147d`. |
| Updater fixture graph | `cargo tree --locked --manifest-path apps/desktop/src-tauri/Cargo.toml -p sky_desktop_shell --target x86_64-pc-windows-msvc --no-default-features --features desktop-runtime,packaged-assets,tauri-update-fixture -e normal,build,features` | Exit 0, 0.61 s; 2,977 lines and the same hash as the default feature graph. `tauri-update-fixture` adds no dependency edge. |
| Tauri qualification graph | Same manifest/package/target, `--no-default-features --features tauri-test -e normal,build,features` | Exit 0, 0.59 s; 2,793 lines; SHA-256 `37b56a65e87a05917864f2a40fdd94a6066e85fbc1932eb17bef240897ebcffe`. |

The graph outputs were written to a local temporary directory and summarized here by line count and hash; no large graph blob is committed or externally linked. Before/after comparisons should normalize only first-party package/path identities, not versions, registry source, or features.

The Tauri shell declares default features `desktop-runtime` and `packaged-assets`. Its separate qualification feature is `tauri-test`; the updater bridge uses `desktop-runtime,packaged-assets,tauri-update-fixture`. These switches are not interchangeable with `--all-features` when comparing a shipping graph.

## 4. Validation evidence

### Local baseline

The canonical pinned command ran once from the repository root on tested commit `367b2031125048c4eb72b2a4f3c25fa21de0794a`:

```text
node scripts/toolchain-pin.mjs exec -- cargo xtask check all
exit: 0
duration: 807.77 s
```

The aggregate reported PASS for repository/app boundaries, docs integrity, architecture and security audits, Tauri feature contract, cargo-vet supply-chain validation, branding, generated/built-in catalog freshness, v4 trust material, Bun pin, Rust pin, CI/release contract, release metadata/authority, hosted-signing and malware-proof boundaries, canonical packaged-CI Tauri checks, legacy updater validation, Rust format, production and all-target/all-feature Clippy, workspace Rust tests, desktop shell runtime/packaged/qualification checks, generated TypeScript binding comparison, Bun checks, and Playwright E2E. The audit reported **69 existing architecture warnings/advisories** while passing; existing allowlists include a glob import and an over-limit dispatch function. They were not changed or suppressed.

The xtask aggregate suppresses successful child-test output, so its transcript did not retain exact counts for each Rust, shell, frontend, or browser suite. To record Rust discovery without running tests, this command used a separate ignored target directory:

```text
$env:CARGO_TARGET_DIR = 'rust/target/check-all-rust-tests'
node scripts/toolchain-pin.mjs exec -- cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked -- --list
exit: 0
duration: 52.83 s
result: 30 harnesses, 1,502 listed test entries, 0 benchmarks
```

The standard workspace command excludes the seven source-marked `#[ignore]` tests; the list result is discovery, not an executed-test count. The canonical aggregate did execute the normal workspace test gates and passed. An initial exploratory list attempt against the shared target was interrupted with Ctrl+C (exit 1, no tests executed); the isolated list above is the completed count capture. JavaScript/Vitest/Playwright and separate `tauri-test` exact per-suite counts were not recoverable from the aggregate output.

After adding this evidence file, the owning docs-integrity test was run against the new Markdown from the repository root:

```text
node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path rust/Cargo.toml --package sky_xtask audits::docs_integrity::tests::current_repository_docs_integrity_passes -- --exact
exit: 0; duration: <1.1 s; 1 passed, 0 failed; 336 filtered from the xtask unit target
```

The package script and hosting lanes do not establish native physical-input qualification. No game was launched, no physical input was generated, no production credential or signing route was invoked, and no installer was installed or run.

### Hosted CI on the tested source

At tested source `367b2031125048c4eb72b2a4f3c25fa21de0794a`, GitHub run [46047775641](https://github.com/pumni/Sky-Auto-Player/actions/runs/46047775641) reported `Sky Auto Player — required CI gate` successful and `Classify validation layers` successful. Platform, Rust/native, package, updater, site, and supply-chain validation lanes were skipped because this commit was docs-only. The combined legacy commit-status API showed pending/zero contexts; the named workflow check-runs are the evidence above. This result is bound to the tested planning commit, not automatically to the later evidence commit.

### Package and artifact measurement

No package or executable was built from the tested source, so current-source package size, build wall time, installer identity, and artifact hash are **not measured**. The generated target tree contained a stale 2026-11-25 version 4.5.0 pair, while this source declares version 4.6.0. It has no recorded source-SHA provenance and was excluded from the baseline:

| Existing file (not a WP1 build) | Version / size | SHA-256 |
|---|---:|---|
| `rust/target/dist/sky_desktop_shell.exe` | 4.5.0 / 16,677,376 bytes | `626bffc5f0340075f9bec6cc6e132ec408de48f2f1ee5cdae0f41e8df614a563` |
| `rust/target/dist/bundle/nsis/Sky Auto Player_4.5.0_x64-setup.exe` | 4.5.0 / 4,702,942 bytes | `7dc66a492f43bb9e43f9ba5f3d276ccbbef204a97b9146d4bf005bec7dab595f` |

These ignored generated files predate the baseline run and are not migration evidence. The temporary `rust/target/check-all-rust-tests` listing cache is disposable and will be removed; the pre-existing `rust/target/dist` artifacts are retained untouched.

## 5. Consumer inventory and cutover map

The entries below group related active consumers. References are to tracked files at the tested HEAD; branch-/SHA-frozen historical evidence is called out separately. This is the WP2 migration map and required regression surface, not a change list already applied.

The tracked-file inventory used `git ls-files rust` (259 tracked paths), followed by literal searches equivalent to:

```text
git grep -n -I -F -e 'rust/' -e 'rust\' -e 'rust-toolchain.toml' -e 'Cargo.lock' -e 'CARGO_MANIFEST_DIR' -e 'CARGO_TARGET_DIR' -e '--manifest-path' -e 'target/dist' -e 'target/release' -- .
git grep -n -I -E 'CARGO_MANIFEST_DIR|CARGO_TARGET_DIR|include!|include_str!|include_bytes!|#\[path|\.\./\.\.' -- rust apps/desktop .github scripts .cargo docs
```

Literal sweep results (matches overlap; these are discovery counts, not unique consumers):

| Search term | Matches | Files |
|---|---:|---:|
| `rust/` | 553 | 76 |
| `rust\` | 32 | 13 |
| `rust-toolchain.toml` | 35 | 11 |
| `Cargo.lock` | 93 | 23 |
| `CARGO_MANIFEST_DIR` | 36 | 21 |
| `CARGO_TARGET_DIR` | 27 | 13 |
| `--manifest-path` | 84 | 27 |
| `target/dist` | 29 | 6 |
| `target/release` | 16 | 9 |
| Relative path/include semantic sweep | 382 | 150 |

The literal and semantic results were inspected in context. The inventory below groups related findings by owning contract; it does not treat historical strings, sibling-relative includes, or fixture expectations as automatic replacements.

| Consumer / current owner | Current location and dependency on nested layout | Expected cutover treatment and required proof |
|---|---|---|
| Workspace, lock, compiler pin, profiles | `rust/Cargo.toml`, `rust/Cargo.lock`, `rust/rust-toolchain.toml`; `rust/Cargo.toml:1-44`; external shell member/dependencies in `apps/desktop/src-tauri/Cargo.toml:4,46-51` | Proposed move of workspace manifest, lock, and pin to repository root; preserve resolver 3, members, package identities, lock resolution, profiles, features, and target predicates. Compare locked metadata and normalized graphs. |
| Workspace source tree | Six packages under `rust/crates/{sky_app_core,sky_dispatch_core,sky_dispatch_win32,sky_native_adapters,sky_player,sky_windows_security}`, plus `rust/xtask` and `rust/build_support` | Proposed atomic moves to root `crates/`, `xtask/`, and `build_support/`; no copies, duplicate workspaces, or old-layout fallback. Preserve all internal relative relationships and public IDs. |
| Cargo alias and generated binding destination | `.cargo/config.toml:2,6`; alias currently runs nested xtask manifest; `TS_RS_EXPORT_DIR` points to `apps/desktop/src/bridge/generated` | Change only alias routing as authorized by the plan; preserve app-owned generated output path. Verify `cargo xtask`, CWD independence, and byte-for-byte generated bindings. |
| Root discovery and process CWD | `rust/xtask/src/repo.rs:14-17,83`; `root()` derives the repo by taking two parents from `CARGO_MANIFEST_DIR` and asserts `rust/Cargo.toml` | Replace with validated repository-root discovery independent of caller CWD, with root discovery exercised from root, moved crates, desktop shell, and xtask. Do not add dual-layout detection. |
| Rust pin resolver and tests | `scripts/toolchain-pin.mjs:7-11,134-155`; tests in `scripts/toolchain-pin.test.mjs:19-25,80-92`; `rust/xtask/src/checks/rust_toolchain.rs:8,91-110,527-530` and its path tests; `.github/actions/setup-rust-pinned/action.yml:2,21,45` | Update the one authoritative pin path and all test fixtures/consumers. Preserve exact-version verification, conflicting/legacy pin rejection, environment override rejection, and action behavior. Run positive and negative pin tests. |
| Rust pin fan-out and cache keys | `.github/workflows/ci.yml` pin checks and cache inputs around `548-603`; `rust_toolchain.rs` reports 22 workflow consumers; CI setup action above | Update paths and cache inputs without weakening pin checks. Verify workflow contract tests enumerate all consumers and deliberate override failures. |
| Tauri workspace/path dependencies | `apps/desktop/src-tauri/Cargo.toml:4,46-51`, feature declarations `:11-23` | Shell stays app-owned and outside the new source subtree; update workspace/path dependency relations while retaining package name/version, dependency kinds and feature behavior. Verify runtime, packaged, and `tauri-test` modes. |
| Tauri build support and watch set | `apps/desktop/src-tauri/build.rs:1,22` includes `rust/build_support/git_metadata.rs`; `apps/desktop/src-tauri/tauri.conf.json:8-14` watches `../../../rust/crates`, nested manifest and lock; `frontendDist` is `../dist` | Redirect only Rust source/manifest/lock/build-support paths. Preserve `frontendDist`, resources, capabilities, permissions, and app config. Run Tauri metadata and package-contract checks. |
| Tauri actual Cargo mode | `apps/desktop/package.json:25`; `.github/workflows/ci.yml:868,1043`; fixture builder `rust/xtask/src/updater_fixture.rs:2310-2326` | Preserve the real invocation/feature/profile behavior while resolving the missing lock-mode assurance described in §6 via explicit leader decision. Verify the inner Cargo command rather than assuming xtask or CLI lock behavior. |
| Generated IPC bindings | `apps/desktop/src-tauri/src/bindings.rs`; export path in `.cargo/config.toml:2`; generator/check in `rust/xtask/src/checks/bindings.rs:11-28,126-153,187` and `checks.rs:707-744`; IPC ownership check `command_ownership.rs:110-124` | Preserve the 93 checked-in generated TypeScript files, command names, DTOs, and export ordering/location. Generate into isolated output and compare; retain IPC ownership and Tauri test checks. |
| Xtask validation manifest/target routing | `rust/xtask/src/checks.rs:260-354,716-764`; `checks/bindings.rs:141-188`; target currently `rust/target/check-all-rust-tests` for isolated workspace tests | Update manifest and default target-root references to the selected root layout. Preserve externally supplied `CARGO_TARGET_DIR` and containment/isolation rules. Run focused check selectors, then the canonical aggregate once on migrated source. |
| Source scanners and architecture allowlist | `rust/xtask/src/checks/{tauri_metadata,ci_contracts}.rs`, `audits/{app_boundaries,architecture,security}`, `assembly_audit.rs`; `.config/rust_architecture_allowlist.json` has 18 `rust/crates/...` paths | Update active source roots and allowlist path keys mechanically with identical exceptions. Keep scanner fixtures and negative assertions; do not expand allowlists or relax audit limits. |
| CI source classifier and required check | `.github/workflows/ci.yml:1634-1714` defines exact required check `Sky Auto Player — required CI gate`; classifier `rust/xtask/src/ci_classifier.rs:380-505,580-583`; path tests `:1688-1752,1818-1835,2047-2070` | Preserve required aggregate name. Root `Cargo.toml` is already recognized but dormant; test new `crates/`, `xtask/`, and `build_support/` paths, modifications, renames, deletions, malformed/unknown paths, and full-dispatch fallback. Retain fail-closed coverage and prove new paths cannot skip required lanes. |
| CI Cargo manifests, target caches, and artifacts | `.github/workflows/ci.yml` Cargo/cache/target references at `67,154,255,321,383-395,505-517,548-603,666-695,801-870,995,1020,1043,1059,1142,1204-1215,1303,1313-1442`; also Tauri package lanes | Update active workspace, lock, pin, cache, `xtask.exe`, target, and artifact paths. Preserve platform matrix, cache isolation, output names, SHA/source identity checks, artifact retention, and required aggregation. Re-run CI contract and classifier tests on positive and negative paths. |
| Release builder and recovery | `.github/workflows/release-v4.yml:225-271`; `.github/workflows/v4-metadata-recovery.yml:94-109,153-230`; `rust/xtask/src/release_v4.rs:1371`; `rust/target/release/xtask.exe` and `rust/target/dist` are current output locations | Move paths while keeping release authority, detached candidate identity, exact `xtask.exe` bytes/SHA, permission order, recovery boundaries, and output discovery unchanged. Do not execute a release/signing transaction in WP1/WP2. |
| SBOM and dependency audits | `rust/xtask/src/sbom.rs:69,290,422` hashes/loads `rust/Cargo.lock`; `rust/xtask/src/supply_chain.rs:8-10,43-45,73,105-107`; CI `cargo audit` and `cargo vet` at `ci.yml:569,596`; data under `rust/supply-chain/{audits.toml,config.toml,imports.lock}` | Update workspace/lock and vet/audit/SBOM references while preserving inputs and trust checks. The destination of `rust/supply-chain/` is not specified by the implementation plan; leader disposition is required (see §6). `.github/CODEOWNERS:6` and `.gitattributes:8-9` also own this path. |
| Synthetic updater fixture | `rust/xtask/src/updater_fixture.rs:2027-2057,2132-2154,2207-2216,2310-2340`; CI invocation `ci.yml:1204-1215` | Keep same-source SHA validation, detached temporary Git worktree, external validated temp root, isolated `CARGO_TARGET_DIR`, package-version lock rewrite, exact bridge feature set, fixture outputs, and cleanup. Update root manifest/lock path without historic-layout autodetection. Test same-source and mismatch/cleanup failures. |
| Build support include and Tauri rerun paths | `apps/desktop/src-tauri/build.rs:1,22`; `rust/crates/sky_dispatch_win32/build.rs:1` references `../../build_support` | Update cross-tree include and rerun paths. Keep the in-tree sibling build-support relation after the move. Exercise build scripts with correct CWD and Git metadata. |
| Relative root traversals and cross-tree fixtures | `rust/xtask/src/updater_fixture.rs:3308`; checks in `ci_contracts.rs:1637,1690,1781,1809`, `hosted_malware.rs:275`, `hosted_signing.rs:219`, `release_runner.rs:823,947,1046,1055,1074`, `ci_contracts/website_tests.rs:20,74,119`, `release_pipeline.rs:672`, `rust_toolchain.rs:766` derive paths with `../..` or equivalent. `checks/trust.rs:685-686` includes the app NSIS hook. | Review each depth semantically after moving `xtask`; update paths and test from varied CWDs. Do not bulk replace. `apps/desktop/src-tauri` remains in place, so its frontend/resource tree does not follow the Rust move. |
| Moved library fixture paths | `sky_app_core/tests/legacy_song_corpus.rs:157-230` and song corpus includes; `settings_catalog_update_fixtures.rs:11-13`; `sky_native_adapters/tests/settings_fixtures.rs:7`; `sky_native_adapters/src/paths.rs:363`; `sky_player/src/bin/acceptance/benchmark_schedule_tests.rs:405-408` | Recompute paths to repository-owned `songs/` and fixtures after crate roots move. Keep test data and acceptance binary behavior unchanged. `sky_player` sibling references and Win32 sibling includes remain structurally relative and should be checked rather than rewritten blindly. |
| App-owned resources and catalog | `apps/desktop/src-tauri/tauri.conf.json:43-46` maps songs and built-in manifest; `lib.rs:815` reaches repository songs; `apps/desktop/src/bridge/generated` remains shell-owned | Preserve resource mapping and app layout; prove packaged asset freshness and catalog identity after the Rust tree moves. |
| Frontend binary helpers and independent Bun projects | `apps/desktop/scripts/profile-builtin-library.mjs:54,254`, `profile-workbench-resize.mjs:18` use `rust/target/debug/sky_desktop_shell.exe`; root `.bun-version`; desktop and website have independent manifests/locks | Update only current Rust binary paths and build invocation. Keep separate Bun lockfiles and app version, no new root Bun workspace. `apps/website` source guides/FAQs (EN/VI) link live `rust/crates/sky_app_core` and `rust/xtask` pages; update active links after cutover. |
| Scripts and script contracts | Current references include `scripts/build_dispatch_core_harness.ps1`, `dispatch_harness_manifest.psm1`, `dispatch_qualification_profiles.psm1`, `export_startup_calibration_evidence.ps1`, `prepare_chord_width_pilot_v1.ps1`, `qualify_dispatch_core.ps1`, `run_dispatch_lifecycle_evidence.ps1`, `run_issue666_m4_guard_ab_offline.ps1`, `run_native_acceptance_case.ps1`, `test_build_provenance_switch.ps1`, `test_chord_width_pilot_v1_preflight.ps1`, `test_qualify_dispatch_core_contracts.ps1`, `test_v4_production_signing_contract.ps1`, `test_v4_updater_credential_broker.ps1`, plus toolchain-pin tests | Update generic current build/test defaults and their contract tests. Preserve exact path/hash assertions in revision-bound qualification scripts; inspect every match before modifying. Do not execute physical-input qualification. |
| Live documentation, repository automation, ignore rules | `docs/architecture.md:7-21`; `docs/rust-toolchain-policy.md:3-23,56`; `docs/desktop-tooling.md:66`; `docs/ci-validation-topology.md:97-102,212-218,254`; `docs/v4-tauri-packaging.md:229`; `docs/v4-release-authority.md:407`; `docs/v4-release-execution-topology.md:312`; `docs/v4-updater-key-custody.md:32,283`; playback/song docs; `.github/dependabot.yml:3-5`; `.gitignore:69` | Update live path guidance and Cargo automation. Keep docs and website aligned with the chosen topology. The current `docs/desktop-tooling.md:66` says generated IPC bindings are under `rust/crates/sky_player/bindings`, but this directory is absent; actual generated output is app-owned under `apps/desktop/src/bridge/generated`. Record/fix as doc drift when its owner is touched. |
| Rust target/default output | `.gitignore:69` ignores `rust/target/`; CI and xtask also set explicit target roots | Make the approved root target output ignored and default to `target/`; do not move or commit generated `rust/target` files. Preserve caller-supplied isolated target overrides and output identity. |

### Historical and intentional retained paths

Do not rewrite revision-bound evidence as if it described the current source tree. In particular, `docs/playback-verification.md` contains both live command guidance and frozen branch/SHA/hash evidence; `docs/work-orders/pr-687-catalog-and-global-search.md` is historical. `prepare_chord_width_pilot_v1.ps1` and related dispatch qualification/evidence scripts contain explicit old branch or lock identities. Keep those historical records intact and classify each match; update only current execution defaults. Internal `#[path]`, `include!`, `include_str!`, and `include_bytes!` relations within a moved subtree generally retain their relative layout, while cross-tree includes and repository-root parent traversals listed above need explicit review.

## 6. Baseline defects, risks, and leader decisions

### Confirmed shipping lock-mode gap

The owning Rust policy says native shipping builds use locked mode (`docs/rust-toolchain-policy.md:13`), and the source-architecture specification explicitly requires checking the actual Cargo subprocess. Three Tauri build entry points do not currently pass `--locked` to that subprocess:

- `apps/desktop/package.json:25`: `tauri build --ci -- --profile dist`.
- `.github/workflows/ci.yml:868,1043`: production candidate/package builds use `--profile dist` without `--locked`.
- `rust/xtask/src/updater_fixture.rs:2310-2326`: bridge Tauri args include `--profile dist` and `desktop-runtime,packaged-assets,tauri-update-fixture`, without `--locked`.

The pinned Tauri CLI is 2.12.3. Its [official v2.12.3 desktop Cargo command construction](https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.3/crates/tauri-cli/src/interface/rust/desktop.rs#L233-L279) appends caller arguments and feature/profile flags but does not inject `--locked`. Therefore, the absence is not corrected implicitly by the Tauri CLI. This is a pre-existing build reproducibility defect, not introduced by WP1. The implementation plan says to propose a separately reviewed fix; leader decision is needed on owner and sequencing. No source change was made.

### Topology decision: supply-chain directory

The planned atomic cutover names `Cargo.toml`, `Cargo.lock`, the toolchain pin, six crate directories, `xtask`, and `build_support`; it does not give a destination for `rust/supply-chain/`. Active `cargo vet`, xtask, CI, CODEOWNERS, and `.gitattributes` contracts depend on this data. Leader must decide whether it remains under `rust/` intentionally or moves to an approved root path; WP2 must update every consumer and ownership rule consistently. No destination is selected here.

### Other recorded drift / limits

- `docs/desktop-tooling.md:66` points at a nonexistent old bindings directory; actual generated TypeScript lives in the desktop bridge.
- `verify rust` fails only when invoked with this machine's global `stable` alias; the pinned wrapper and scoped exact override pass.
- Architecture audit passes with 69 existing advisories. Retain existing allowlists and limits; no warning was “fixed” by weakening checks.
- The only available installer/executable is version 4.5.0 with no source-SHA proof. No comparable 4.6.0 package, installer timing, or current executable measurement exists.
- Successful xtask child output hides exact Rust/frontend sub-suite execution totals. Rust discovery count and known ignored tests are recorded above; exact successful child test counts and raw child logs are unavailable.
- A pin-only configuration change would avoid moving the workspace and therefore avoid much of the manifest/path/cache/classifier/build-support fan-out, but would leave Rust ownership, root Cargo invocation, and current path consumers nested under `rust/`. A root-workspace cutover aligns those paths but has a materially broader coordinated surface, especially CI, release, updater, toolchain, source scanners, and docs. This is a scope/risk comparison, not a recommendation; leader selects the intended option.

## 7. Atomic migration map and WP2 regression plan

The specification's proposed atomic move set is:

| Current | Proposed destination | Boundary |
|---|---|---|
| `rust/Cargo.toml`, `rust/Cargo.lock`, `rust/rust-toolchain.toml` | repository-root `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` | Same workspace identities/resolution/profile/pin; no compiler or dependency update. |
| `rust/crates/*` (all six packages) | `crates/*` | No library behavior, API, target, or feature change. |
| `rust/xtask` | `xtask` | Preserve `sky_xtask`, validators, subprocess safety, and root-independent CWD behavior. |
| `rust/build_support` | `build_support` | Update Tauri cross-tree includes and rerun paths. |
| generated `rust/target` default | root `target` default | Do not move/build-commit old output; honor explicit isolated target roots. |
| `apps/desktop/src-tauri`, frontend, generated TS | stay in place | Keep the shell as app-owned package; update Rust paths only. |
| `rust/supply-chain` | **leader disposition required** | No proposed destination until approved. |

Before source edits, WP2 should preserve and prove these checks:

1. From repository root, each moved crate, desktop shell, and xtask, verify repository root and run locked metadata through both ordinary Cargo and the trusted wrapper. Compare member/package IDs, target predicates, dependency kinds, feature declarations, production/qualification graphs, lock hash/content, and profile values.
2. Run positive and negative pin tests: root exact pin accepted; missing/duplicate/conflicting/legacy pins and unauthorized `RUSTUP_TOOLCHAIN`, `RUSTC`, and related compiler overrides rejected before trusted build work. Verify the setup action and all workflow consumers.
3. Exercise `cargo xtask` from varied CWDs. Run format, Clippy, workspace tests, shell default runtime, packaged `dist`, `tauri-test`, generated binding compare, architecture/security/app-boundary checks, and all source-scanner tests. Preserve nonzero exact test selectors and ignored-only hosted tests.
4. Test classifier rules for root manifest/lock/pin, each moved root directory, modified/added/deleted/renamed paths, malformed names, unknown paths, no-change and full-dispatch behavior. Keep `Sky Auto Player — required CI gate` unchanged and prove moved paths cannot avoid required lanes.
5. Validate Tauri watch folders/build support, real inner Cargo lock mode, frontend output path, resources, updater fixture exact feature switch, generated IPC output, and disposable same-source updater worktree/lock rewrite/cleanup. Verify candidate and bridge builds use separate validated target roots.
6. Verify release and recovery path consumers, exact xtask executable SHA/source binding, SBOM/vet/audit inputs, target/artifact discovery, signer and permission order, CI artifact identities, and `.gitignore`/cache changes. Do not run signing or a production release transaction.
7. Update generic scripts, live docs, website source links, Dependabot, CODEOWNERS, `.gitattributes`, and current ignore entries. Search the tracked repository for old paths and classify each residual as a deliberately frozen historical identity or a missed consumer. Do not leave a compatibility workspace, duplicate lock, fallback layout, stale active path, or generated output.
8. Build an authorized nonproduction package only under the owner-approved route if needed for comparable current artifact evidence; use disposable paths, record source SHA/toolchain/profile/target/cache, package and executable hashes, sizes and duration. Do not treat browser/mock checks as native package proof or perform physical-input qualification without owner authorization.

## 8. Cleanup and evidence boundary

No product source or manifest was edited in WP1. The evidence report is the only intended tracked addition. Existing ignored build output under `rust/target/dist` was inspected and left untouched. The isolated test-list cache at `rust/target/check-all-rust-tests` remains ignored local build output; the execution policy rejected its recursive removal, so it was not changed. Local temporary graph/command evidence is summarized by exact commands, outcomes, counts, and hashes above; there are no externally accessible raw-log links. No secrets, generated graphs, or binary artifacts are included in this report.
