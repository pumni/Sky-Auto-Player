# Source architecture WP6 final acceptance

## Disposition

**WP5 corrections are complete. WP6 local acceptance is in progress; Leader and Owner acceptance remain pending.** This record is tied to the exact source SHAs below. It does not authorize merge, release, deployment, physical-input testing, or production signing.

| Identity | SHA | Role |
| --- | --- | --- |
| Authorized start | `a2a0dd39a560e8f54b34981ec17c7f6d2b9d4eb0` | Binding Section 17 dispatch and WP5 starting point |
| WP5 correction source | `a81d94dc182753407f0dde378d8214926bd84325` | Source-contract, adapter scanner, and workflow newline corrections |
| Current local source before this report | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | Durable documentation, immutable archive index, and removal of temporary plan/report files |
| Final pushed source | Pending | Will be recorded after validation and push |

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

The previously reported 341 and 349 counts correspond to different valid feature sets; the earlier narrative omitted `--no-default-features`. The selector inventory is **1,523** from `cargo test --workspace --all-features --locked -- --list`. The default desktop library test command without `tauri-test` is not the regression configuration: it fails compilation because test-only helpers `TrackedKeyState::set_emitter` and `WindowIdentityAuthority::synthetic_for_test` are unavailable. It is recorded as a configuration error, not a test pass.

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

Locked `cargo metadata --format-version 1` was compared across original main, accepted WP3, and WP6 correction source. After normalizing only the old nested `rust/` workspace-root relocation, all three metadata graphs have SHA-256 `0D9D4DD3E47062D65D49EAFB5B82F6D363A27963375921976109ACB43C2FB0FE`, with the same 544 resolved nodes and 1,746 dependency edges. The root lockfile has the same Git blob as original main. No dependency version upgrade was made.

The four locked Windows-target dependency projections also match original main and accepted WP3 exactly:

| Projection | Lines | SHA-256 |
| --- | ---: | --- |
| `--target x86_64-pc-windows-msvc -e normal,build` | 1,018 | `c23a486c1034067cc8b5fb304e4b23dbcbc6c36bdb14cc81e82b7aa2af562e68` |
| `--target x86_64-pc-windows-msvc -e normal,build,features` | 2,977 | `a16c58dd14c862141a187981fa88dbc9227066cf4cc94d22b05beb1aaaba23fa` |
| `--no-default-features --features desktop-runtime,packaged-assets,tauri-update-fixture -e normal,build,features` | 2,977 | `a16c58dd14c862141a187981fa88dbc9227066cf4cc94d22b05beb1aaaba23fa` |
| `--no-default-features --features tauri-test -e normal,build,features` | 2,793 | `beeaece1c43c3341aaf777b04febc57131db8e85dab93be71800e3e552fc9d5a` |

The compiler pin remains Rust `1.99.1` (`minimal`, target `x86_64-pc-windows-msvc`); its blob matches original main. `.cargo/config.toml` changed only the xtask alias from the old nested manifest to `--package sky_xtask --bin xtask`. Release and dist profile definitions match the original baseline: release uses LTO off, 16 codegen units, opt-level 2, unwind, symbol stripping; dist inherits release with debug off, thin LTO, one codegen unit, opt-level 3. The final clean locked dist build remains pending below.

All 93 generated bridge files under `apps/desktop/src/bridge/generated` have identical path/blob pairs to original main. Current architecture owners are documented in [architecture.md](../../architecture.md): pure application policy in `sky_app_core`; playback and target authority in `sky_player`; pure scheduling policy in `sky_dispatch_core`; direct Win32 timing, foreground, power, and input effects in `sky_dispatch_win32`; filesystem/process adapters in `sky_native_adapters`; Windows security primitives in `sky_windows_security`; Tauri delivery and service orchestration in `sky_desktop_shell`; and repository checks/release orchestration in `sky_xtask`.

### Extraction, consumer, and dead-code audit

The accepted source moves leave no tracked legacy `rust/` tree or duplicate workspace/lockfile. The desktop command and DTO, native runtime catalog/events/playback/session, updater state/progress, and xtask classifier, updater fixture, and release self-test have explicit physical owners. Test owners are separated under `cfg(test)`; runnable release self-test code remains production compiled. Public IPC identifiers and generated bindings are unchanged. The adapter scanner now checks all production owners and only the three declared test owners listed in R02. Current architecture and source-organization conventions are consolidated in `docs/architecture.md`; temporary SPEC, plan, and WP1–WP5 reports were removed after immutable links, source commits, byte counts, and SHA-256 digests were committed in `docs/HISTORY.md`.

Unsafe effects remain in the existing Windows/runtime owners. `sky_app_core` and xtask entrypoints retain `forbid(unsafe_code)`; Win32 code is subject to the existing architecture unsafe inventory and allowlist. Final exact production/test inventory and hard-fail audit result will be added from canonical validation below. No physical input or ReceiveOnly qualification was performed or inferred from mocks.

## Ten-gate acceptance matrix

Statuses are source-bound. A pending or failed hosted proof is not represented as a local pass.

| # | SPEC §10.1 gate | Status | Evidence / remaining proof |
| ---: | --- | --- | --- |
| 1 | One root virtual workspace, one live lockfile, eight preserved members/package identities and no active stale topology paths | PASS | Metadata graph comparison above; zero tracked `rust/**`; one root lockfile. |
| 2 | Compiler discovery and trusted verification pass, including override-negative tests; isolated output/fixture paths remain correct | PENDING | Pin blob and pinned wrapper verified; final canonical toolchain/override checks pending. |
| 3 | Locked dependency resolution, relevant effective features and actual distribution build invocation preserved; `dist` unchanged; no test-feature leakage | PENDING | Locked graph/tree fingerprints and profiles match; clean locked dist build and canonical feature checks pending. |
| 4 | Existing architecture, unsafe, security and hard safety rules remain effective; inherited/aliased/target-specific checks covered | PENDING | Correction suites pass; final canonical architecture/security/unsafe output pending. |
| 5 | Accepted extractions have clear ownership and no unauthorized public API, state, lock, lifetime or wire changes; justified residual large modules allowed | PASS | Existing architecture owner map, source guards, unchanged 93 generated IPC bindings, and preserved package graph. |
| 6 | Relevant tests and selectors reconciled, actual intended execution verified, bindings and generated contracts correct | PASS | Focus suites, 1,523 selector inventory, three exact desktop feature counts, 93 unchanged binding files; full canonical binding check pending. |
| 7 | Canonical full validation and exact-source required hosted status complete; classifier remains conservative | PENDING | `cargo xtask check all` rerun pending after this report exists; exact-final-source hosted CI status will be queried after push and not awaited. |
| 8 | Exact Windows NSIS candidate and applicable install/launch/uninstall, resource inventory, app-data preservation and synthetic updater qualification complete; actual historical-release qualification only where explicitly required | PENDING | No exact-final-source Windows package/NSIS hosted proof yet. Earlier ClamAV qualification failed freshness at source `4b1d4dd`; no physical/package proof inferred. |
| 9 | Existing release authority, trust, permissions, exact-byte handoff, signing/malware/SBOM/attestation and recovery contracts remain intact with applicable nonproduction proof; no unauthorized publication | PENDING | 34-scenario local mock self-test passed. Exact-final-source hosted signing/malware/SBOM/attestation/recovery proofs remain pending; older ClamAV job failed closed. No credentials used. |
| 10 | Live documents reflect accepted structure; deferred findings and exceptions recorded; owner accepts baseline | PENDING | Architecture/HISTORY/playback docs updated and temporary plans archived; this final report is in progress. Leader and Owner acceptance remain pending. |

## Validation ledger

| Check | Exact command / conditions | Source | Result |
| --- | --- | --- | --- |
| Updater bridge and related source guards | Focused suites listed above | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS |
| Desktop feature counts | Three exact commands listed under R03; pinned wrapper; locked; isolated `TS_RS_EXPORT_DIR` in ignored `target` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS |
| Selector inventory | `cargo test --workspace --all-features --locked -- --list` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS, 1,523 selectors |
| Release CLI state-machine self-test | `cargo xtask release-v4 self-test` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | PASS, 34 scenarios |
| First canonical attempt | `cargo xtask check all` | `a14efa9b682f3cae8240a49381f3c8ff1217f946` | FAIL at docs-integrity because this required report target did not yet exist. Guard was not weakened; rerun is pending. |
| Final pinned canonical validation | `node scripts/toolchain-pin.mjs exec -- cargo xtask check all`, repository root | Pending | Pending |
| Clean locked dist build | Frozen worktree at exact tested source; `bun install --frozen-lockfile`; `node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist` from `apps/desktop` | Pending | Pending |
| Hosted CI on exact final pushed SHA | Workflow/status query after push | Pending | Pending; will be recorded without waiting for hosted completion. |

## Archives, cleanup ledger, and deferred scope

The immutable SPEC, plan (including Section 17), and WP1–WP5 source commits, byte counts, and SHA-256 digests are listed in [HISTORY.md](../../HISTORY.md). The history index was committed before deleting the eight working documents. The active PR #690 remains a draft; its old body still contains links to archived branch paths and is not being updated as part of this handoff.

Cleanup completed: removed the two temporary planning documents and six WP1–WP5 evidence reports from the active tree; confirmed no tracked files remain under `rust/`; restored test-generated `BootstrapDto.ts` and confirmed all 93 generated binding blobs match original main; left source, dependencies, runtime behavior, safety limits, release authority, and CI gates unchanged. Ignored build/test outputs remain under the repository `target` directory. No external temporary directory was manually deleted, and no execution restriction was bypassed. A previously created disposable external worktree is left untouched because its cleanup was blocked by path-length/shell restrictions.

Deferred: exact-final-source hosted Windows NSIS packaging and installation flows; ClamAV freshness qualification (the older exact job failed closed); hosted signing, SBOM, attestation, recovery and aggregate required status; any owner-required physical-input/ReceiveOnly qualification; Leader review and Owner merge decision. No additional optimization or discretionary refactoring is authorized by this report.

## Handoff

Final pushed branch SHA, exact hosted status at handoff, cleanup path inventory, and the issue #689 handoff URL will be appended after local gates and the push. Freeze the final pushed HEAD after handoff; do not mark PR #690 ready or enable auto-merge.

**WP5 CORRECTED / WP6 READY FOR LEADER REVIEW**
