# WP2 — Root workspace cutover evidence

## Source identity and commits

- Authorized branch: `refactor/source-architecture`.
- Accepted WP2 base: `ad8ca09ae27893d49d07cb72e0fe15eee3c3ede1`.
- Final implementation source used for validation: `a993c5188f7b242c490bf3e0796b72154f0d8f8f` (tree `073dc7caee0cd33bec4d843ddb90156466c5f6dd`).
- The report is an evidence-only addition made after validation; it does not change the tested implementation tree. The pushed report commit is the handoff HEAD.
- Reviewable commit separation:
  - `57b0448299e3bc46556cd327b3d133279d4141c3` — `fix(build): lock Tauri Cargo subprocesses`.
  - `5bdbfbb328f56eda7932601be1502c66083b34d7` — `refactor(rust): move workspace to repository root`.
  - `a993c5188f7b242c490bf3e0796b72154f0d8f8f` — `fix(checks): register Tauri lock contract leaf`.

## Cutover and preserved semantics

The workspace manifest, lock, compiler pin, six library crates, xtask, build support, and cargo-vet supply-chain data now live at the repository root. The root workspace has resolver 3, eight members (`sky_app_core`, `sky_dispatch_core`, `sky_dispatch_win32`, `sky_native_adapters`, `sky_player`, `sky_windows_security`, `sky_xtask`, and `sky_desktop_shell`), and no `default-members`, matching the accepted baseline behavior. Package names and versions, dependency declarations, target predicates, feature declarations, and profile settings were preserved. The desktop shell remains under `apps/desktop/src-tauri`.

The only root workspace manifest path adjustment from baseline is the desktop member path. The checked-in Cargo.lock remains byte-identical with SHA-256 `a30234510d815f56e1ca49c9168e14e3189166b9c401216c10e288a1f145e3ca`. Release profile values remain `lto=false`, `codegen-units=16`, `opt-level=2`, `panic=unwind`, `strip=symbols`, and `overflow-checks=true`; the shipping `dist` profile still inherits release and sets `debug=false`, `debug-assertions=false`, `lto=thin`, `codegen-units=1`, and `opt-level=3`.

The pin remains Rust 1.99.1. Root command discovery, trusted pin verification, compiler-override rejection, target overrides, and output containment remain in place. The root xtask alias is locked. The three supply-chain files were moved byte-for-byte to Cargo Vet's root default location; their SHA-256 values remain:

| File | SHA-256 |
| --- | --- |
| `supply-chain/audits.toml` | `c77c98abfbedf2d8c0c9556376aefb5271c1189d30466205c9cee4e2b9bc8847c` |
| `supply-chain/config.toml` | `00199f28ce329e1fb9919f065f8187f41d44abe2312eaabe7c124ee38c1b7d7e` |
| `supply-chain/imports.lock` | `ff01c4f0f1a725196404edd25fbab0b4ad073de1264fa124b92e2344ec39eb7a` |

## Graph, feature, profile, and selector comparisons

Locked Cargo metadata on the final workspace resolved 544 packages and eight workspace members. The before/after metadata comparison normalized only the checkout-root path and had zero semantic differences in package identities, dependency resolution, member set, declared/resolved features, target kinds, or normal/build/dev/target dependency distinctions. The expected root, build, and target directory paths changed with the workspace location. Final raw metadata SHA-256 is `246b093f0cf5344908f99f9f47c570359026575684d2bf35ec7c61766e328155`.

Four normalized locked graph snapshots compared line-for-line equal against the accepted source (zero mismatches):

| Graph | Lines | Final snapshot SHA-256 |
| --- | ---: | --- |
| Production default normal/build dependencies | 1,018 | `728347fed0b40d6e313e7d736b52891b97eb5dd673930ae60f240db4be1cd69a` |
| Production default feature tree | 2,977 | `ad43ff6ab34fc74dabe1938f724af5f263c36b735014ff1a995f7219c27c8d7d` |
| Updater fixture feature tree | 2,977 | `ad43ff6ab34fc74dabe1938f724af5f263c36b735014ff1a995f7219c27c8d7d` |
| Qualification binding `tauri-test` feature tree | 2,793 | `b02946c2166c45b932e28fb54b56742fbb8c9480b3de0f2212cfac23308b5150` |

Test discovery at the accepted base found 1,502 entries across 30 harnesses, zero benches. Final discovery found 1,506 entries across the same 30 harnesses, zero benches. This is discovery, not a pass count. The name comparison reports two selectors renamed to reflect root-pin terminology and six added selectors (net +4); no prior selector was silently dropped. Added regression coverage includes all active locked Tauri builders, separator placement/duplicate rejection, root pin behavior, classifier old-path deletion plus root-path addition, and aggregate check registration. The final list log SHA-256 is `2d970a9dfaaf987d87ae4ba705a9a38f39a71032a7e8caa372236aaa11c91848`; the sorted name delta SHA-256 is `5a800d1dd3861382f2b2a365f8d6fdaf54fa7d50e794f1e1bff7c5842e91d64f`.

## Locked Cargo subprocess policy

The separate lock-mode correction covers all six active Tauri dist/package builders discovered: the desktop package script, both CI Tauri builds, `release_v4::build_unsigned`, the credential CLI candidate build, and the updater fixture bridge build. Each passes `--locked` to Cargo after the Tauri argument separator where applicable. The owning contract tests prove every active builder carries the flag and reject absent, misplaced, or duplicate placement. The contract is included in the static aggregate inventory; the follow-up test prevents a registered-but-unexecuted leaf.

Pinned Cargo fail-closed probes used isolated temporary manifests and targets, with no change to the repository lock:

| Fixture | Command | Exit | Observed result |
| --- | --- | ---: | --- |
| Stale lock missing the manifest dependency | `node scripts/toolchain-pin.mjs exec -- cargo build --locked --offline --manifest-path <fixture>/Cargo.toml` | 101 (expected) | Cargo refused to update the lock because `--locked` was passed. Log SHA-256 `ca199122d62ff592c5b320a48d5c791198cc91a02494a856914544568b297ec5`. |
| No lockfile | Same command with the missing-lock fixture | 101 (expected) | Cargo refused to create the lock because `--locked` was passed. Log SHA-256 `899c7e8a515d1b7cfbd799ea171f73d090b86b35e53e9140ad8c442ae0f189b4`. |

From `apps/desktop`, the pinned Tauri CLI command was `node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist`. Its verbose output records the actual child as `cargo build --locked --profile dist --bins --features tauri/custom-protocol`. It exited 0 in 304.98 seconds. The unbundled binary was 16,670,208 bytes, SHA-256 `d81d441e00c3b24a7fc206d554968b5e592a82716f843d0aaf6e46642092d4f2`; byte inspection confirmed the final implementation source SHA was embedded. No installer was bundled, signed, or executed.

## Local validation

The canonical aggregate, metadata/graph comparisons, package build, lock probes, formatting, final test discovery, and final targeted regressions below are bound to source `a993c5188f7b242c490bf3e0796b72154f0d8f8f` and tree `073dc7caee0cd33bec4d843ddb90156466c5f6dd`, except where a baseline identity is stated. The broader focused Rust suite ran after the workspace cutover commit `5bdbfbb328f56eda7932601be1502c66083b34d7`; the final aggregate then revalidated the complete final source. Commands ran on Windows using the pinned Rust 1.99.1 toolchain.

| Check | Working directory / command | Result |
| --- | --- | --- |
| Canonical aggregate | Repository root: `node scripts/toolchain-pin.mjs exec -- cargo xtask check all` | Exit 0; 845.3 s. Rust production/all-target Clippy and workspace all-feature tests, desktop checks/E2E, generated bindings, CI/release/security/supply-chain contracts all reported PASS. Aggregate log SHA-256 `f46ed97cddf7f6f8a5c3b6ee4db7e5776c4a84e91ddb48134d16a7c5cc81437d`. |
| Formatting | Repository root: `cargo fmt --all -- --check` | Exit 0. |
| Locked metadata | Repository root: `cargo metadata --locked --manifest-path Cargo.toml --format-version 1` | Exit 0; 544 packages, eight workspace members. |
| Toolchain pin tests | Repository root: `node --test scripts/toolchain-pin.test.mjs` | 6/6 passed. Root-pin unit selectors: 8/8 passed. |
| Focused Rust regressions | Root `sky_xtask` selectors after cutover commit `5bdbfbb`: Tauri lock contract (15), CI classifier (23), root pin (8), app boundaries (9), security (9), root discovery (1), bridge (5), updater fixture (19 passed, 1 existing ignored), CI contracts (16), release runner (8), release pipeline (7), generated bindings selector (1) | All executed selectors passed. After the final leaf-registration change, the CI/release/bindings selectors were repeated on `a993c518` for 32 passed; the new leaf-registration selector passed 1/1. The updater signer-pair unit remains intentionally ignored because the owning CI invokes it with pinned Bun desktop dependencies. Per-test runtimes are present in Cargo output; focused command wall durations were not separately recorded. |
| Root xtask discovery | From repository root, `crates/sky_app_core`, `apps/desktop/src-tauri`, and `xtask`: `node scripts/toolchain-pin.mjs exec -- cargo xtask --help` | Exit 0 from all four working directories. |
| Tauri package build | `apps/desktop`: `node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist` | Exit 0; exact child args, binary size, and SHA-256 recorded above. |
| Negative locked-mode probes | Repository root; isolated stale/missing-lock manifests with `--locked --offline` | Both exit 101 for the expected lock refusal; repository Cargo.lock unchanged. |

An earlier aggregate attempt on `5bdbfbb328f56eda7932601be1502c66083b34d7` exited 101 because the new lock-contract leaf was not registered in the static aggregate inventory. The leaf was registered in `a993c5188f7b242c490bf3e0796b72154f0d8f8f`, a regression test was added, focused validation passed, and the final-source canonical aggregate then passed. The earlier attempt is not represented as a pass.

The accepted WP1 aggregate recorded 69 architecture warnings/advisories. The final aggregate passes architecture checks with 70 warnings and security checks with zero baseline-covered findings. No warning threshold or allowlist was relaxed. WP1's raw per-warning list is not available in the local evidence bundle, so the single-count difference is not yet classified as a path-normalized finding or a new diagnostic. This remains an explicit leader-review item; no claim is made that the 69-to-70 delta is explained.

## Package and hosted proofs not completed

`makensis.exe` was not available on PATH or in the standard Program Files locations. The NSIS installer and synthetic updater bridge qualification therefore remain pending; the bridge path requires an installer and disposable signed bridge. The updater fixture unit suite passed as stated above, but it is not equivalent to the full bridge qualification. No production signing, release, deployment, or physical-input qualification was performed. The package proof is limited to the unbundled Rust/Tauri binary.

Hosted CI was not awaited. The post-push handoff in issue #689 records the status and run links for the exact pushed evidence HEAD. Required hosted product lanes remain acceptance requirements even if the source is accepted provisionally.

Sanitized raw execution logs and machine-readable comparison inputs remain in the task-local temporary evidence area and are not committed: the raw logs include machine paths and the graph/metadata files are generated bulk evidence. Their relevant semantic results and SHA-256 digests are recorded above. Those local files are not directly accessible to GitHub reviewers.

## Cleanup ledger and residuals

| Previous location / mechanism | Replacement and consumers | Disposition |
| --- | --- | --- |
| `rust/Cargo.toml`, `rust/Cargo.lock`, `rust/rust-toolchain.toml` | Root manifest, lock, pin; root metadata, pinned command, CI cache/setup, package and release commands | Moved; one live root workspace/lock/pin. No tracked `rust/` files remain. |
| `rust/crates/*` (six libraries) | `crates/*`; updated manifests, build paths, tests, bindings, scripts, CI, and live docs | Moved with package identities and source logic preserved. |
| `rust/xtask` and `rust/build_support` | `xtask` and root `build_support`; root discovery and all active tooling/release/updater callers updated | Moved; no dual-layout fallback. |
| `rust/supply-chain/*` | Root `supply-chain/*`; Cargo Vet, SBOM/audit, CI classifier, CODEOWNERS, gitattributes and checks | Moved byte-for-byte; root path routes the supply-chain/security lanes. |
| Active Rust path consumers | Root Cargo paths/targets and current package locations | Updated across CI/release/recovery, Tauri config/build support, updater fixtures, scripts, test selectors, and live docs. Classifier regression covers old-side deletion plus root-side addition and fails closed on ambiguous changes. |

Intentional historical references remain in revision-bound qualification evidence/scripts, immutable provenance links, negative tests that reject legacy nested pins, the old-side classifier fixture, and `.gitignore`'s `rust/target/` entry protecting ignored owner artifacts. They do not select a fallback workspace. Root Cargo build output created for validation was removed with pinned `cargo clean` (17.9 GiB). Two stale, task-created Git worktree registrations pointing to absent directories were pruned; only the authorized integration worktree remains. The disposable stale/missing-lock fixture files are outside the repository. PowerShell deletion was rejected by the environment policy even after resolving and constraining the paths to the task temp directory, so those temporary fixture files could not be removed; their logs are retained outside Git. No such files are tracked or staged.

At handoff, repository status is checked clean aside from this evidence commit, with no build output, binary, raw log, secret, or installer artifact staged. No dependency, feature, compiler, profile, runtime behavior, IPC, persistence, safety, or release authority change was made beyond the separately authorized inner `--locked` correction and root path cutover.
