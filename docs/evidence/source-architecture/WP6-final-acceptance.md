# Source architecture WP6 final acceptance

## Disposition

**WP5 corrections are complete. WP6 local acceptance is complete; exact-source hosted proofs and Leader/Owner acceptance remain pending.** This record is tied to the exact source SHAs below. It does not authorize merge, release, deployment, physical-input testing, or production signing.

| Identity | SHA | Role |
| --- | --- | --- |
| Authorized start | `a2a0dd39a560e8f54b34981ec17c7f6d2b9d4eb0` | Binding Section 17 dispatch and WP5 starting point |
| WP5 correction source | `a81d94dc182753407f0dde378d8214926bd84325` | Source-contract, adapter scanner, and workflow newline corrections |
| Current local source before this report | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | Durable documentation, immutable archive index, and removal of temporary plan/report files |
| Canonical and clean-build source | `6e6bd96d7d31032800ced8f710becdcb851c08d7` | Acceptance report present; clean canonical and dist source |
| Final pushed source | This report's containing commit | Exact SHA and branch state are recorded in the linked issue #689 handoff comment. |

The WP5 correction and WP6 dispatch are bound by [issue #689, comment 7236865015](https://github.com/pumni/Sky-Auto-Player/issues/689#issuecomment-7236865015). The original SPEC and full implementation plan are preserved through immutable references in [HISTORY.md](../../HISTORY.md). The original package comparison base is `5c79c8cf604072f331820126223188161d17d7b1`.

## WP5 correction results

### R01 — updater bridge line endings

Reproduced the LF/CRLF source-contract mismatch and normalized fixture and test-owner text at the contract boundary. The existing positive and negative assertions remain, including source ownership, required implementation and test selectors, wrong-owner rejection, missing-build rejection, and Tauri build-argument placement. The corrected regression exercises LF and CRLF inputs for both source owners.

### R02 — adapter retirement scanner

The scanner now excludes only the declared `tests.rs`, `catalog_tests.rs`, and `settings_store_tests.rs` owners, and confirms each is declared under `#[cfg(test)]` in its physical Rust owner. It scans the full production source, including text after a test module declaration, and normalizes line endings before source checks. Regressions cover test-like production filenames and forbidden production markers after CRLF test declarations. Existing positive and negative retirement checks remain.

### R03 — desktop counts and feature graphs

All counts below were reproduced at `a14efa9b682f3cae8240a49381f3c8ff1217f946` with the pinned wrapper and locked dependencies. `TS_RS_EXPORT_DIR` pointed to an ignored directory under `target` so test output could not rewrite checked-in bindings.

| Exact command after the pinned wrapper | Configuration | Result |
| --- | --- | --- |
| `cargo test -p sky_desktop_shell --lib --features tauri-test --locked -- --quiet` | Defaults plus `tauri-test`; 345 total | 341 passed, 0 failed, 4 ignored |
| `cargo test -p sky_desktop_shell --lib --no-default-features --features tauri-test --locked -- --quiet` | Qualification-only `tauri-test`; 353 total | 349 passed, 0 failed, 4 ignored |
| `cargo test -p sky_desktop_shell --lib --all-features --locked -- --quiet` | All features; 346 total | 342 passed, 0 failed, 4 ignored |

The previously reported 341 and 349 counts correspond to different valid feature sets; the earlier narrative omitted `--no-default-features`. The selector inventory is **1,523** from `cargo test --workspace --all-features --locked -- --list`: WP4 recorded 1,514, WP5 recorded 1,521, and this correction adds exactly two selectors with no removals. The two new selectors are `native_adapter_retirement_excludes_only_declared_test_owners_and_scans_full_production_sources` and `active_release_surface_source_normalizes_crlf_before_separating_tests`; R01 expands an existing selector. The default desktop library test command without `tauri-test` is not the regression configuration: it fails compilation because test-only helpers `TrackedKeyState::set_emitter` and `WindowIdentityAuthority::synthetic_for_test` are unavailable. It is recorded as a configuration error, not a test pass.

### R04 — ClamAV freshness proof

The failed hosted run was [CI run 46127181869](https://github.com/pumni/Sky-Auto-Player/actions/runs/46127181869), job `140924465329`, on source `4b1d4dd2e10b7dda6ab3d54ca2a7fe4e5156b471`. The hosted Ubuntu runner installed ClamAV `1.5.4+dfsg-0ubuntu0.24.04.1`. `freshclam` reported daily database 28194 up to date, then downloaded and verified main database 63 and bytecode 339. The nested EICAR-in-NSIS positive control passed. Qualification then failed closed with: `ClamAV virus signatures are stale or dated implausibly in the future`.

The failed log does not provide enough timestamp evidence to distinguish stale data from an implausible future date. The `NotifyClamd` configuration warning did not prevent bootstrap or the positive control. No freshness limit, clock, scanner evidence, or security gate was changed. A new exact-final-source hosted proof remains pending; the earlier failed run is not treated as final-source evidence.

### Other newline-sensitive guards

Reviewed source guards in `ci_contracts`, `tauri_metadata`, `release_runner`, `release_pipeline`, `trust`, and `recovery_auth`. The affected updater bridge, adapter-retirement, active release-surface, and metadata-workflow guards normalize line endings at their source-contract boundary. Existing negative assertions are retained.

Focused correction tests at the current source checkpoint:

| Command after the pinned wrapper | Result |
| --- | --- |
| `cargo test --locked -p sky_xtask --bin xtask checks::ci_contracts::tests` | 15 passed, 0 failed |
| `cargo test --locked -p sky_xtask --bin xtask checks::tauri_metadata::tests` | 18 passed, 0 failed |
| `cargo test --locked -p sky_xtask --bin xtask checks::release_runner::tests::metadata_app_tokens_are_separate_and_only_reach_probe_or_promotion -- --exact` | 1 passed, 0 failed |
| `cargo fmt --all -- --check` | Passed |
| `cargo xtask release-v4 self-test` | Passed, all 34 scenarios |

## Cumulative architecture and compatibility audit

### Workspace and package graph

The root is one virtual workspace using resolver 3, one root `Cargo.lock`, and eight package identities: `sky_app_core@0.1.0`, `sky_desktop_shell@4.6.0`, `sky_dispatch_core@0.1.0`, `sky_dispatch_win32@0.1.0`, `sky_native_adapters@0.1.0`, `sky_player@0.1.0`, `sky_windows_security@0.1.0`, and `sky_xtask@0.1.0`. `git ls-files 'rust/**'` returns zero tracked paths.

Locked `cargo metadata --format-version 1` was compared across original main, accepted WP3, and WP6 correction source. After normalizing only the old nested `rust/` workspace-root relocation, all three metadata graphs have SHA-256 `0D9D4DD3E47062D65D49EAFB5B82F6D363A27963375921976109ACB43C2FB0FE`, with the same 544 resolved nodes and 1,746 dependency edges. `Cargo.lock` is byte-identical to original main (Git blob `4993318c716321328f606b039051799151a29dbe`; SHA-256 `A30234510D815F56E1CA49C9168E14E3189166B9C401216C10E288A1F145E3CA`). No dependency version upgrade was made.

The four locked Windows-target dependency projections also match original main and accepted WP3 exactly:

| Projection | Lines | SHA-256 |
| --- | ---: | --- |
| `--target x86_64-pc-windows-msvc -e normal,build` | 1,018 | `c23a486c1034067cc8b5fb304e4b23dbcbc6c36bdb14cc81e82b7aa2af562e68` |
| `--target x86_64-pc-windows-msvc -e normal,build,features` | 2,977 | `a16c58dd14c862141a187981fa88dbc9227066cf4cc94d22b05beb1aaaba23fa` |
| `--no-default-features --features desktop-runtime,packaged-assets,tauri-update-fixture -e normal,build,features` | 2,977 | `a16c58dd14c862141a187981fa88dbc9227066cf4cc94d22b05beb1aaaba23fa` |
| `--no-default-features --features tauri-test -e normal,build,features` | 2,793 | `beeaece1c43c3341aaf777b04febc57131db8e85dab93be71800e3e552fc9d5a` |

The compiler pin remains Rust `1.99.1` (`minimal`, target `x86_64-pc-windows-msvc`); its blob matches original main. The measured compiler is `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, host `x86_64-pc-windows-msvc`. `.cargo/config.toml` changed only the xtask alias from the old nested manifest to `--package sky_xtask --bin xtask`. Release and dist profile definitions match the original baseline: release uses LTO off, 16 codegen units, opt-level 2, unwind, symbol stripping; dist inherits release with debug off, thin LTO, one codegen unit, opt-level 3. Canonical pin/override checks and the clean locked dist build passed below.

All 93 generated bridge files under `apps/desktop/src/bridge/generated` have identical path/blob pairs to original main. Current architecture owners are documented in [architecture.md](../../architecture.md): pure application policy in `sky_app_core`; playback and target authority in `sky_player`; pure scheduling policy in `sky_dispatch_core`; direct Win32 timing, foreground, power, and input effects in `sky_dispatch_win32`; filesystem/process adapters in `sky_native_adapters`; Windows security primitives in `sky_windows_security`; Tauri delivery and service orchestration in `sky_desktop_shell`; and repository checks/release orchestration in `sky_xtask`.

### Extraction, consumer, and dead-code audit

The accepted source moves leave no tracked legacy `rust/` tree or duplicate workspace/lockfile. The desktop command and DTO, native runtime catalog/events/playback/session, updater state/progress, and xtask classifier, updater fixture, and release self-test have explicit physical owners. Test owners are separated under `cfg(test)`; runnable release self-test code remains production compiled. Public IPC identifiers and generated bindings are unchanged. The adapter scanner now checks all production owners and only the three declared test owners listed in R02. Current architecture and source-organization conventions are consolidated in `docs/architecture.md`; temporary SPEC, plan, and WP1–WP5 reports were removed after immutable links, source commits, byte counts, and SHA-256 digests were committed in `docs/HISTORY.md`.

Unsafe effects remain in the existing Windows/runtime owners. `sky_app_core` and xtask entrypoints retain `forbid(unsafe_code)`; Win32 code is subject to the existing architecture unsafe inventory and allowlist. Canonical validation passed with 19 production unsafe owner modules and 7 files containing `cfg(test)`-only unsafe sites. Original main also had 19 `unsafe_boundary` allowlist entries; 18 paths are identical after workspace relocation and the remaining `crates/sky_native_adapters/src/lib.rs` entry moved with its implementation to `crates/sky_native_adapters/src/atomic_file.rs`. No unsafe owner was added or removed.

The 19 production owners are `apps/desktop/src-tauri/src/{power_lifecycle,single_instance,windows_caption,windows_icon}.rs`; `crates/sky_dispatch_win32/src/{clock,cpu,event,focus,mmcss,power,timer}.rs`; `crates/sky_dispatch_win32/src/focus/{foreground_watcher,window_identity}.rs`; `crates/sky_dispatch_win32/src/input/{packet,physical,raw}.rs`; `crates/sky_dispatch_win32/src/wait/hybrid.rs`; `crates/sky_native_adapters/src/atomic_file.rs`; and `crates/sky_windows_security/src/win32.rs`. Test-only sites remain in `sky_dispatch_win32/focus.rs`, `sky_dispatch_win32/input/{packet,physical}.rs`, `sky_player/engine/worker/dispatch_loop_safety_tests.rs`, `sky_native_adapters/paths.rs`, `sky_windows_security/win32.rs`, and `apps/desktop/src-tauri/power_lifecycle.rs`. The architecture hard-fail audit passed; this inventory does not authorize physical input or ReceiveOnly qualification.

The recorded architecture warning trend is 69 at WP1, 68 at WP4, 67 at WP5, and 67 in this final check. The WP6 delta is zero warning identities; all 67 are the existing advisory/allowlisted findings. Security checks passed with 0 baseline-covered findings. No threshold, assertion, or allowlist was weakened. The pre-existing `sky_player` session glob-import warning and large-module advisories remain for separate review.

The remaining `rust/Cargo.toml` documentation references are revision-bound playback baselines or the separate PR #687 work order. Its pinned baseline `2e335f152fd98a085668647186e41c7e94e1dcfe` contains `rust/Cargo.toml`, so those commands remain valid for that separate checkout and were left unchanged. No physical input or ReceiveOnly qualification was performed or inferred from mocks.

## Ten-gate acceptance matrix

Statuses are source-bound. A pending or failed hosted proof is not represented as a local pass.

| # | SPEC §10.1 gate | Status | Evidence / remaining proof |
| ---: | --- | --- | --- |
| 1 | One root virtual workspace, one live lockfile, eight preserved members/package identities and no active stale topology paths | PASS | Metadata graph comparison above; zero tracked `rust/**`; one root lockfile. |
| 2 | Compiler discovery and trusted verification pass, including override-negative tests; isolated output/fixture paths remain correct | PASS | Final canonical Rust pin contract passed for 22 workflow consumers; measured compiler is Rust 1.99.1. |
| 3 | Locked dependency resolution, relevant effective features and actual distribution build invocation preserved; `dist` unchanged; no test-feature leakage | PASS | Metadata and four feature projections match; the locked unbundled dist build passed and compiled only `tauri/custom-protocol`, with no test features. |
| 4 | Existing architecture, unsafe, security and hard safety rules remain effective; inherited/aliased/target-specific checks covered | PASS | Canonical architecture checks passed (67 existing warnings); unsafe inventory matched the 19-entry baseline allowlist with one extracted owner; security findings 0. |
| 5 | Accepted extractions have clear ownership and no unauthorized public API, state, lock, lifetime or wire changes; justified residual large modules allowed | PASS | Existing architecture owner map, source guards, unchanged 93 generated IPC bindings, and preserved package graph. |
| 6 | Relevant tests and selectors reconciled, actual intended execution verified, bindings and generated contracts correct | PASS | Focus suites, 1,523 selector inventory, three exact desktop feature counts, 93 unchanged binding files, and final canonical binding checks passed. |
| 7 | Canonical full validation and exact-source required hosted status complete; classifier remains conservative | PENDING | Local `cargo xtask check all` passed at `6e6bd96d`; exact-final-source hosted status will be queried after push and not awaited. |
| 8 | Exact Windows NSIS candidate and applicable install/launch/uninstall, resource inventory, app-data preservation and synthetic updater qualification complete; actual historical-release qualification only where explicitly required | PENDING | No exact-final-source Windows package/NSIS hosted proof yet. Earlier ClamAV qualification failed freshness at source `4b1d4dd`; no physical/package proof inferred. |
| 9 | Existing release authority, trust, permissions, exact-byte handoff, signing/malware/SBOM/attestation and recovery contracts remain intact with applicable nonproduction proof; no unauthorized publication | PENDING | 34-scenario local mock self-test passed. Exact-final-source hosted signing/malware/SBOM/attestation/recovery proofs remain pending; older ClamAV job failed closed. No credentials used. |
| 10 | Live documents reflect accepted structure; deferred findings and exceptions recorded; owner accepts baseline | PENDING | Architecture/HISTORY/playback docs and this report are complete; temporary plans are archived; Leader and Owner acceptance remain pending. |

## Validation ledger

| Check | Exact command / conditions | Source | Result |
| --- | --- | --- | --- |
| Updater bridge and related source guards | Focused suites listed above | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS |
| Desktop feature counts | Three exact commands listed under R03; pinned wrapper; locked; isolated `TS_RS_EXPORT_DIR` in ignored `target` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS |
| Selector inventory | `cargo test --workspace --all-features --locked -- --list` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS, 1,523 selectors |
| Release CLI state-machine self-test | `cargo xtask release-v4 self-test` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS, 34 scenarios |
| First canonical attempt | `cargo xtask check all` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | Initial pre-report attempt stopped at docs-integrity because the required report target did not yet exist. The guard was not weakened; the report was added and the complete rerun passed below. |
| Final pinned canonical validation | `node scripts/toolchain-pin.mjs exec -- cargo xtask check all`, repository root | `6e6bd96d7d31032800ced8f710becdcb851c08d7` | PASS (exit 0): Rust, frontend checks/E2E, desktop checks/tests, bindings, architecture, security, toolchain, supply chain, release and source guards. Architecture reported 67 warnings; security reported 0 findings. |
| Clean locked dist build | Frozen worktree at exact tested source; `bun install --frozen-lockfile`; `node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist` from `apps/desktop` | `6e6bd96d7d31032800ced8f710becdcb851c08d7` | PASS (exit 0): Bun 1.4.4 installed 298 locked packages; build took 8m15s. `sky_desktop_shell.exe`: 16,670,208 bytes, SHA-256 `169CDE41760245285FF34A431727C3512F6085469E6F44D3735686EE857EAB2B`. No bundle or signing. |
| Docs integrity after final report edit | `cargo test --locked -p sky_xtask --bin xtask audits::docs_integrity::tests::current_repository_docs_integrity_passes -- --exact` | Parent `6e6bd96d` plus final report edits | PASS, 1 passed, 0 failed |
| Hosted CI on exact final pushed SHA | Workflow/status query after push | Pending | Pending; will be recorded without waiting for hosted completion. |

## Archives, cleanup ledger, and deferred scope

The immutable SPEC, plan (including Section 17), and WP1–WP5 source commits, byte counts, and SHA-256 digests are listed in [HISTORY.md](../../HISTORY.md). The history index was committed before deleting the eight working documents. PR #690 remains a draft. Its existing description still has branch-relative links to the archived plan; it was left unchanged, and the issue handoff links to the immutable archive references.

Cleanup completed: removed the two temporary planning documents and six WP1–WP5 evidence reports from the active tree; confirmed no tracked files remain under `rust/`; restored test-generated `BootstrapDto.ts` and confirmed all 93 generated binding blobs match original main; left source, dependencies, runtime behavior, safety limits, release authority, and CI gates unchanged. The disposable clean build worktree `D:\Dev\Sky-Auto-Player\target\wp6-dist`, graph comparison directory `D:\Dev\Sky-Auto-Player\target\wp6-graph-compare`, and `TS_RS_EXPORT_DIR` output `D:\Dev\Sky-Auto-Player\target\wp6-tsrs-export` remain under the repository target root. The targeted local cleanup command was rejected before execution with `blocked by policy`; no alternate deletion route was attempted. Four stale external worktree registrations under `%LOCALAPPDATA%\Temp` are reported by `git worktree list` as prunable because their `.git` pointer targets are missing: `sky-source-architecture-warning-baseline-ad8ca09-6c2d2de6c642470d8578e220cd935f16` (HEAD `ad8ca09`), `sky-source-architecture-wp2-correction-clean-912d2d05` (HEAD `1f70882`), `sky-source-architecture-wp4-clean-ba777594` (HEAD `03e85b1`), and `sky-wp5-warning-baseline-03e85` (HEAD `03e85b1`). They and their external paths were left untouched; no cleanup restriction was bypassed. Ignored Playwright artifacts and canonical logs remain in repository-local ignored output folders.

Deferred: exact-final-source hosted Windows NSIS packaging and installation flows; ClamAV freshness qualification (the older exact job failed closed); hosted signing, SBOM, attestation, recovery and aggregate required status; any owner-required physical-input/ReceiveOnly qualification; Leader review and Owner merge decision. No additional optimization or discretionary refactoring is authorized by this report.

## Handoff

The handoff with exact final pushed SHA, hosted status, test/build identities, archive links, cleanup ledger, and residual risks will be posted to [issue #689](https://github.com/pumni/Sky-Auto-Player/issues/689) after the push. This report is part of that frozen commit. Do not mark PR #690 ready or enable auto-merge.

**WP5 CORRECTED / WP6 READY FOR LEADER REVIEW**
