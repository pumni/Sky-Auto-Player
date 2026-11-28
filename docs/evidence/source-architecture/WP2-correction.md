# WP2 Correction Evidence

**Disposition:** The correction checkpoint was satisfied on the authorized integration branch before WP3 began. This records source evidence; it is not leader acceptance.

## Source identity

- Authorized base: c9d782c1006e372f9eefada4ac5567bb0faae45b.
- WP2 correction commit: 912d2d054cb1feb152bdae3f57e0ea3c7f33ada3.
- WP2 correction tree: 7dd84e7e5ebea2f61bcbf18080e34bf7e3c3da58.
- Tested detached checkout: exact correction commit above; the checkout had no rust/ directory. Cargo outputs were directed to a separate temporary target directory.
- WP3 proceeded only after the required clean-checkout test and relevant desktop tests passed, per issue #689 comment 7231793680 and implementation plan §14.

## Four review findings

| Finding | Correction and evidence |
| --- | --- |
| R01: root identity regression | Replaced the assertion that rust/ exists with a canonical repository-root identity check and root Cargo.toml, crates/, xtask/Cargo.toml, and desktop manifest markers. The debug repository-root-not-CWD invariant remains. On the clean detached checkout, the exact filtered desktop test passed 1/1; the full desktop library suite passed 349, with 4 intentionally ignored. |
| R02: supply-chain policy coverage | Removed the root supply-chain special case that selected only supply_chain_required. Root supply-chain changes now follow the existing conservative unknown-path route and select all LANES. The focused classifier suite passed 24/24, including a policy-only config.toml diff, old-layout deletion/root-layout addition, and conservative build_support classification. The required gate name and projection were unchanged. |
| R03: architecture advisory delta | Compared normalized warning identities from the exact ad8ca09 baseline, WP2 correction 912d2d0, and final WP3 source 1f70882. The 69-warning baseline gains only xtask_module_lines for xtask/src/checks/rust_toolchain.rs at its existing 860-line advisory threshold. No threshold or allowlist changed. WP3 adds/removes zero warning identities. |
| R04: legacy consumer re-sweep | Re-audited active code, tests, workflows, scripts, manifests, fixtures, documentation, generated bindings, and static/dynamic path construction across unchanged files. Corrected every confirmed current-layout consumer and reconciled root supply-chain ownership and watch/classification behavior. No additional active legacy-layout consumer remained. |

## Clean-checkout and validation record

Checkpoint worktree: %TEMP%/sky-source-architecture-wp2-correction-clean-912d2d05 at the WP2 correction SHA. Before running tests, git status --porcelain was empty and Test-Path rust returned false. CARGO_TARGET_DIR was set outside that worktree at %TEMP%/sky-source-architecture-wp2-correction-target-912d2d05.

- From %TEMP%, node <worktree>/scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path <worktree>/Cargo.toml -p sky_desktop_shell --lib --no-default-features --features tauri-test debug_install_root_is_repository_root_not_cwd — exit 0; 1 passed, 0 failed, 0 ignored; 198.7 seconds including cold compilation.
- Same command without the test-name filter — exit 0; 349 passed, 0 failed, 4 ignored; 22.62 seconds total, including 15.28 seconds compilation and 6.04 seconds test execution.
- Repository-root command: node scripts/toolchain-pin.mjs exec -- cargo test --locked --manifest-path Cargo.toml -p sky_xtask ci_classifier::tests -- --test-threads=1 — exit 0; 24 passed.
- Repository-root command: node scripts/toolchain-pin.mjs exec -- cargo xtask check static --skip-supply-chain — exit 0; architecture/security, Tauri feature/lock, CI/release, and toolchain checks passed; architecture reported 70 warnings.
- Repository-root command: node scripts/toolchain-pin.mjs exec -- cargo fmt --all -- --check — exit 0.
- git diff --check — exit 0.

## Warning-set comparison

The baseline log is bound to ad8ca09ae27893d49d07cb72e0fe15eee3c3ede1 (tree 3bba598741ca5ce8911c80e9245c49fa69fc0cf2). The WP2 correction log is bound to 912d2d054cb1feb152bdae3f57e0ea3c7f33ada3. The final WP3 log is bound to 1f70882a87851eeb935399b8fdc8c827d0d79941. For identity comparison only, historical rust/ prefixes were normalized to the root path.

| Comparison | Baseline warnings | Result |
| --- | ---: | --- |
| ad8ca09 to WP2 correction | 69 to 70 | Added exactly xtask_module_lines / xtask/src/checks/rust_toolchain.rs / threshold 860; removed none. |
| WP2 correction to WP3 final | 70 to 70 | Added none; removed none. |

The source remains cohesive; the leader explicitly accepted this sole advisory. No suppression was added and no limit changed.

## Consumer disposition and cleanup ledger

- Current workspace guidance now names the repository root in AGENTS.md and docs/architecture.md.
- storage_bootstrap.rs now asserts the canonical root and current root markers instead of requiring rust/.
- xtask/src/ci_classifier.rs uses the existing fail-closed path for root supply-chain policy changes; the explicit narrow supply-chain lane shortcut was removed.
- Current-versus-historical manifest guidance in docs/playback-verification.md is explicit. It does not autodetect or fall back to a nested legacy manifest.
- Retained rust/ references are revision-bound historical evidence or qualification fixtures, deliberate negative tests for rejecting nested toolchains/retired wiring, the classifier's old-layout rename/deletion fixture, and .gitignore protection for ignored owner output. They are not active current-layout consumers.
- Full re-sweep covered literal and quoted paths, join/path-component construction, CWD and parent-depth assumptions, include/source/config markers, platform conditionals, CI/package/updater scripts, and generated binding routes. No broad refactor or compatibility fallback was introduced.

## Risks and limits

The correction checkpoint does not authorize or claim physical-input qualification, production signing, release, deployment, or merge. Final aggregate and hosted status are recorded in WP3-manifest-consolidation.md; the final hosted package/updater proof is not transferred from an older SHA.

Some later WP3 tests generated untracked TypeScript outputs inside the disposable detached test worktree. They are not tracked or staged; the integration worktree remained clean. An attempted cleanup of those generated files was rejected by the execution policy, so they remain isolated under %TEMP% and are disclosed rather than hidden.
