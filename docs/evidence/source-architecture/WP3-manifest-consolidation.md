# WP3 Manifest Consolidation Evidence

**Disposition:** WP3 implementation is complete in a separate reviewable commit after the WP2 correction checkpoint. This evidence records local results for leader review; it is not WP2 or WP3 acceptance.

## Source identity and commits

- Authorized base: c9d782c1006e372f9eefada4ac5567bb0faae45b.
- WP2 correction commit: 912d2d054cb1feb152bdae3f57e0ea3c7f33ada3; tree 7dd84e7e5ebea2f61bcbf18080e34bf7e3c3da58.
- WP3 implementation commit: 1f70882a87851eeb935399b8fdc8c827d0d79941; tree 24c040d34e55444628dc57e06273273bd2ecf40b.
- WP2 and WP3 are separate commits on refactor/source-architecture. The evidence-only commit is not part of the tested implementation tree; its final pushed SHA is reported in the issue handoff.
- Final canonical validation ran in a detached checkout at the WP3 commit. Test products used the isolated target/check-all-rust-tests directory and the checkout had no rust/ directory.

## Manifest consolidation

Added only the seven approved registry requirements and six first-party path dependencies to root workspace.dependencies: serde 1, serde_json 1, sha2 0.11, thiserror 2, smallvec 1, getrandom 0.4, zeroize =1.9.1, sky_app_core, sky_dispatch_core, sky_dispatch_win32, sky_native_adapters, sky_player, and sky_windows_security.

Member manifests now inherit those roots while keeping consumer-specific features, dependency kinds, and target predicates at the consumer. serde derive, smallvec union/serde, sky_player test-support dev edges, and the adapter's cfg(windows) normal dispatch edge remain local. url remains 2.5.8 in xtask and 2.5.9 in the desktop shell. windows-sys constraints and feature sets remain separately declared. Single-consumer dependencies remain local. No test, runtime, or packaged feature was added to a shared dependency.

The shell's redundant explicit workspace pointer was removed. Workspace members, resolver, package names/versions, edition/MSRV, default features, target declarations, profiles, compiler pin, and runtime contracts are unchanged. No default-members, lint policy, crate, dependency, profile, or lockfile change was added.

## Before/after equivalence

Baseline is the corrected WP2 commit 912d2d0; after is WP3 commit 1f70882. Locked Cargo metadata was normalized over workspace package IDs, package versions/sources, target and dependency declarations, requirements, kinds, optional/default-feature flags, package feature sets, workspace members, and all resolved graph nodes/features. The normalized projections are equal: 8 workspace members, 73 direct member dependency edges, 544 resolved nodes. Semantic SHA-256 is 5e7fcb7e100030e960bcbcf24b335751cea3ac7b7a506e523e67491cadffac30 before and after.

| Locked graph | Lines before/after | SHA-256 before/after |
| --- | ---: | --- |
| Windows production normal/build: sky_desktop_shell, x86_64-pc-windows-msvc, -e normal,build | 1,018 / 1,018 | c72e6dcdb897cf58a0b79b703ffb8a441a895142271a930b47dc64cce4db2652 / identical |
| Default production feature graph, same target | 2,977 / 2,977 | dad617cd827590a7f9a84f0a79a75f165090f0e6e786a5415599212c9076b31d / identical |
| Updater fixture features desktop-runtime,packaged-assets,tauri-update-fixture, same target | 2,977 / 2,977 | dad617cd827590a7f9a84f0a79a75f165090f0e6e786a5415599212c9076b31d / identical |
| Qualification features --no-default-features --features tauri-test, same target | 2,793 / 2,793 | 3f2c27d272c643621e9e333ef583b468af34b01b79587c9542e219dda8cd7df7 / identical |

Cargo.lock Git blob is 4993318c716321328f606b039051799151a29dbe at both commits; SHA-256 is a30234510d815f56e1ca49c9168e14e3189166b9c401216c10e288a1f145e3ca. The root Cargo.toml diff from WP2 to WP3 contains only the workspace.dependencies table; resolver, members, package policy and profile values are unchanged. Pinned Rust remains 1.99.1; Bun remains 1.4.4.

The 93 checked-in generated binding files under apps/desktop/src/bridge/generated have identical path sets and blob IDs before/after. The pinned cargo xtask bindings check command exited 0 and generated/exported output compared equal to the checked-in binding contract.

## Dependency-boundary audit

Added xtask/src/audits/architecture/dependencies.rs as a small explicit identity resolver. It resolves direct names, package aliases, workspace inheritance and workspace alias identities over normal, build, dev, and target tables; malformed or absent inherited declarations fail closed. The app-core forbidden package set and source-token/unsafe guards remain hard failures. The adapter still forbids sky_player in every kind/target and permits/requires sky_dispatch_win32 only as a cfg(windows) normal dependency; unrelated dev dependencies remain allowed.

Replaced duplicated manifest scans only after equivalent enforcement was routed through the shared resolver and the full architecture run. Positive and negative regression cases cover direct rename, inherited alias, build/dev/target edges, missing/malformed inheritance, adapter Windows placement, forbidden player edges, app-core source tokens, and unsafe code.

- Focused pinned selector: node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path Cargo.toml -p sky_xtask audits::architecture::tests -- --test-threads=1 — exit 0; 22 passed, 0 failed, 326 filtered.
- Final static check reported 70 architecture advisories and no security findings. The normalized warning set is exactly the WP2 correction set: the sole accepted addition remains rust_toolchain.rs at threshold 860; WP3 introduced no threshold, suppression, added warning, or removed warning.
- Root discovery from repository root, crates/sky_app_core, apps/desktop/src-tauri, and xtask resolved the same root workspace and all 8 members.

## Locked Tauri build and contracts

From apps/desktop, the pinned command node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist exited 0. The verbose CLI trace confirms the actual Cargo child was cargo build --locked --profile dist --bins --features tauri/custom-protocol. It produced an unbundled 16,670,208-byte binary with SHA-256 a819427a80208f85e4b9fbcd4c1ac17a1e9bf2f6150ab7d1691aaae075f3c074; the embedded build identity is 1f70882a87851eeb935399b8fdc8c827d0d79941-dirty. The dirty marker comes from 89 untracked TypeScript files generated by earlier tests in this disposable worktree; no tracked source was modified. The build products are isolated under %TEMP%.

The final static and aggregate checks report the preserved Tauri feature contract (default desktop-runtime + packaged-assets; dev desktop-runtime), six active locked Cargo builders, cargo-vet pass, release and updater source contracts, and Rust/Bun pin contracts. These are static/local proofs, not final hosted installer or updater qualification.

The existing Tauri lock, CI contract, release pipeline/runner, and updater fixture test sources are unchanged by WP3. Their previously captured focused selector inventory remains Tauri lock 15, CI contracts 16, release pipeline 7, release runner 8, and updater fixture 19 passed / 1 intentionally ignored; the final canonical aggregate reran the workspace Rust tests and passed.

## Final validation

| Command | CWD / source | Result |
| --- | --- | --- |
| node scripts/toolchain-pin.mjs exec -- cargo fmt --all -- --check | WP3 detached checkout | Exit 0. |
| node scripts/toolchain-pin.mjs exec -- cargo metadata --locked --manifest-path Cargo.toml --format-version 1 | WP3 detached checkout | Exit 0; 8 workspace members, 544 resolved nodes. |
| node scripts/toolchain-pin.mjs exec -- cargo xtask check static --skip-supply-chain | WP3 detached checkout | Exit 0; architecture 70 warnings, security 0 findings; Tauri and CI/release contracts pass. Dedicated cargo-vet gate passed in canonical aggregate. |
| node scripts/toolchain-pin.mjs exec -- cargo xtask check all | WP3 detached checkout; CARGO_TARGET_DIR outside checkout | Final retry exit 0 in approximately 13m14s. Rust, desktop, E2E, formatting, lint, binding drift, CI/release/security, and supply-chain stages report PASS. |

The first canonical attempt on the same source exited 1 in desktop E2E: 171 passed, 3 timed out after 30 seconds, and 1 was skipped out of 175 selected. No frontend source changed in WP2 or WP3. The three exact cases then passed in isolation with one worker (3/3, 39.4 seconds); after the concurrent browser workload ended, the same canonical aggregate completed with exit 0. The aggregate does not emit a successful per-test E2E count, so this report records its exit and the observed first-run/test-retry counts without inventing a pass total.

The final aggregate reran Rust tests but does not expose a combined test-case count through xtask's successful summary. The exact focused architecture and WP2 selector counts above are the observed counts.

## Cleanup ledger and retained exceptions

- WP2 removed the obsolete root supply-chain classifier shortcut and stale current-layout root markers. WP3 removed redundant workspace declarations from members while retaining consumer-specific manifest details.
- No extra workspace, nested manifest, duplicate lock, compatibility fallback, active rust/ path consumer, unused helper, suppression, or test weakening was introduced.
- Historical source references and deliberate negative legacy-path tests remain documented in WP2-correction.md. They are revision-bound evidence, fixtures, or guards rather than current consumers.
- The main integration worktree has no source changes beyond the two implementation commits before this evidence-only change. The isolated WP2/WP3 test worktree has generated untracked TS-RS files; an attempt to remove those temporary generated files was rejected by execution policy after path verification. They remain under %TEMP%, are not staged, and do not exist in the pushed tree.

## Pending evidence and authority

Hosted CI for the eventual pushed evidence HEAD is not awaited. Final hosted Windows, package/NSIS, updater bridge, and qualification gates remain required because previous hosted results are bound to an older source SHA. No production signing, physical-input qualification, release, deployment, merge, or WP4 work was performed.
