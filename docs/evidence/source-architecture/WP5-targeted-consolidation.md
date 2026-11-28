# WP5 Targeted Consolidation Evidence

**Disposition:** WP5A and WP5B are complete for leader review. This is source-level evidence, not merge or release approval.

## Source identity and commits

- Authorized base: `3ebce623b197b5be342a206b5c84bcf6422f12af` on `refactor/source-architecture`.
- WP5A desktop control-plane organization: `c7bdb83755113f6940d9c3dd06aad94bead9bae4`.
- WP5B engineering tooling organization: `2c22d56a4e13a37bb8d760de0f49cde1e4c950e4`.
- Documentation-only correction to the WP4 owner map and LOC table: `eef1024493ee23b5ca420d5b981edb354ea4b9ef`.
- Final canonical validation and clean build source: `eef1024493ee23b5ca420d5b981edb354ea4b9ef`, tree `7bd9b9ea79352a3706dc649ce04e9a4fe75351c5`. Its product sources are the WP5 implementation; the only subsequent change from `2c22d56` is the WP4 evidence correction.
- No additional implementation branch was created. WP5A and WP5B are separate cohesive commits.
- The selector inventory, focused suites and runnable CLI self-test were exercised on WP5 implementation commit `2c22d56a4e13a37bb8d760de0f49cde1e4c950e4`, tree `dbeeca723ac30b1b080375d4132de6d6f3bea120`. The only later change before canonical validation was the WP4 evidence correction.

## Ownership map and preserved boundaries

| Area | Before | Physical owner after WP5 | Retained responsibility |
| --- | --- | --- | --- |
| Playback preparation | `native_runtime/playback/session.rs` | `playback/session/preparation.rs` | Session lifecycle, service state, worker, monitor, dispatch, shutdown and retirement remain in `session.rs`. |
| Schedule and capacity | `session.rs` | `playback/session/prepare_capacity.rs` | Compile and capacity helpers keep their original arithmetic, bounds and cancellation checkpoints. |
| Playback tests | `session.rs` | `session/preparation_cancellation_tests.rs`, `session/owner_identity_poll_consumer_tests.rs` | Named test modules and selector paths remain stable. |
| Native updater | `native_update.rs` | `native_update/state.rs`, `native_update/progress.rs`, `native_update/tests.rs` | Check/install flow, trust roots, verification, transitions, persistence and pre-exit ordering remain in `native_update.rs`. |
| Release tooling | `release_v4.rs` | `release_v4/selftest.rs`, `release_v4/tests.rs` | Production transaction, real transport, authorization, signing/build/qualification and recovery ordering remain in `release_v4.rs` and its existing production modules. `self_test` remains compiled into the runnable xtask. |
| Updater fixture | `updater_fixture.rs` | `updater_fixture/http.rs`, `updater_fixture/tests.rs` | Qualification, bridge build, signing, candidate checks and cleanup remain in `updater_fixture.rs`. |
| CI classifier | `ci_classifier.rs` | `ci_classifier/tests.rs` | Classification matrix, Git semantics, fail-closed behavior and workflow projection remain unchanged in `ci_classifier.rs`. |

No playback lifecycle, synchronization, updater trust, release transaction, CI classification, native dispatch, frontend store or engine hot-path redesign was made. Public/package identities, IPC, runtime behavior, features, dependency graph, profiles, compiler pin and safety boundaries remain unchanged.

## Test selectors and release CLI scenarios

The accepted WP4 all-features inventory had 1,514 unique selectors. The final WP5 inventory has 1,521: **seven added source-owner guard tests and zero removed selectors**. The comparison used `cargo test --workspace --all-features --locked -- --list`; no benchmark selector was reported. Existing playback cancellation/owner-identity, updater state/progress, release, updater-fixture and classifier test selectors retain their named module paths.

| Suite | Stable selector namespace |
| --- | --- |
| Playback cancellation and owner poll consumer | `native_runtime::playback::session::preparation_cancellation_tests::*`; `native_runtime::playback::session::owner_identity_poll_consumer_tests::*` |
| Updater state and progress | `native_update::tests::*` |
| Release Rust tests and runnable self-test | `release_v4::tests::*`; `release_v4::selftest` remains a normal compiled module |
| Updater qualification fixture | `updater_fixture::tests::*` |
| CI classifier | `ci_classifier::tests::*` |

The seven new negative/source-owner regression selectors are:

- `checks::ci_contracts::tests::classifier_source_contract_reads_implementation_and_test_owners_separately`
- `checks::ci_contracts::tests::updater_bridge_source_contract_reads_implementation_and_test_owners_separately`
- `checks::release_pipeline::tests::metadata_recovery_guard_reads_the_moved_release_test_owner`
- `checks::release_pipeline::tests::release_selftest_guard_reads_the_runnable_owner_and_rejects_a_missing_scenario`
- `checks::release_pipeline::tests::updater_fixture_guard_reads_production_http_and_test_owners_separately`
- `checks::trust::tests::fixture_new_root_guard_reads_runtime_and_test_owners_separately`
- `checks::trust::tests::updater_http_guard_reads_the_extracted_http_implementation`

The runnable `cargo xtask release-v4 self-test` remains outside `cfg(test)` and passed all 34 scenarios. The inventory in `release_v4/selftest.rs` is: `mock_pre_token_latest_guard_stable_and_beta`; `mock_pre_token_latest_guard_rejects_invalid_state_and_identity`; `mock_pre_token_read_only_transport_rejects_mutations`; `mock_pre_token_latest_guard_job_authorization`; `mock_publish_succeeds`; `mock_draft_readback_failures_never_patch`; `mock_non_monotonic_candidate_never_mutates`; `mock_draft_post_timeout_reconciles_exact_marker`; `mock_upload_timeout_reconciles_exact_asset`; `mock_complete_transaction_and_raw_metadata`; `mock_metadata_promotion_is_idempotent_after_committed_timeout`; `mock_metadata_promotion_rejects_changed_latest`; `mock_publication_identity_and_asset_inventory_fail_closed`; `mock_pr_merge_sha_does_not_break_release_or_owned_cleanup`; `mock_conflicting_draft_is_not_deleted`; `mock_digest_mismatch_cleans_owned_draft`; `mock_partial_upload_cleans_owned_draft`; `mock_cleanup_failure_preserves_original_error`; `mock_patch_timeout_reconciles_exact_release_id`; `mock_unknown_publication_never_cleans_up`; `mock_latest_policy_failure_is_post_publication`; `metadata_token_probe_rejects_wrong_repository_state`; `mock_stale_evidence_identity_fails_closed`; `mock_manifest_path_traversal_fails_closed`; `mock_release_doctor_detects_metadata_endpoint_divergence`; `doctor_state_input_rejects_malformed_and_oversized_files`; `retry_policy_retries_only_transient_responses`; `marker_parser_rejects_duplicates_and_multiple_markers`; `mutation_adapter_is_at_most_once`; `raw_metadata_retry_converges_after_cache_lag`; `raw_metadata_retry_fails_with_bounded_diagnostics`; `runtime_identity_fails_closed_without_exact_workflow_context`; `workflow_run_identity_fails_closed`; `timestamp_normalization_is_utc_and_strict`.

## Focused and canonical validation

Commands ran from repository root unless noted. Rust invocations used the repository toolchain pin and locked dependencies. `TS_RS_EXPORT_DIR` and test target directories were external to the checkout.

| Command / selector | Result |
| --- | --- |
| `node scripts/toolchain-pin.mjs exec -- cargo fmt --all -- --check` | Exit 0. |
| `node scripts/toolchain-pin.mjs exec -- cargo test --workspace --all-features --locked -- --list` | Exit 0; 1,521 unique selectors, 0 removed versus accepted WP4. |
| `node scripts/toolchain-pin.mjs exec -- cargo test -p sky_desktop_shell --lib --features tauri-test --locked` | Exit 0; 341 passed, 0 failed, 4 ignored. |
| `node scripts/toolchain-pin.mjs exec -- cargo test -p sky_xtask --bin xtask --locked release_v4::tests` | Exit 0; 16 passed, 0 failed, 2 intentionally ignored. |
| `node scripts/toolchain-pin.mjs exec -- cargo test -p sky_xtask --bin xtask --locked updater_fixture::tests` | Exit 0; 19 passed, 0 failed, 1 intentionally ignored. |
| `node scripts/toolchain-pin.mjs exec -- cargo test -p sky_xtask --bin xtask --locked ci_classifier::tests` | Exit 0; 24 passed, 0 failed, 0 ignored. |
| Source-guard filters `checks::release_pipeline::tests`, `checks::ci_contracts::tests`, `checks::trust::tests`, `checks::recovery_auth::tests` | Exit 0; respectively 9, 15, 4 and 2 passed; includes moved-owner negative cases. |
| `node scripts/toolchain-pin.mjs exec -- cargo xtask release-v4 self-test` | Exit 0 in 5.33s; all 34 runnable scenarios passed. |
| Four locked Windows Cargo tree projections | Commands used the desktop manifest and `x86_64-pc-windows-msvc`: `-e normal,build`; `-e normal,build,features`; `--no-default-features --features desktop-runtime,packaged-assets,tauri-update-fixture -e normal,build,features`; and `--no-default-features --features tauri-test -e normal,build,features`. All exited 0 with 1,018 production normal/build lines; 2,977 default feature lines; 2,977 updater-fixture feature lines; and 2,793 qualification feature lines. These counts match the accepted WP4 projections. The accepted normalized fingerprints are production normal/build `c23a486c1034067cc8b5fb304e4b23dbcbc6c36bdb14cc81e82b7aa2af562e68`, default/updater fixture `a16c58dd14c862141a187981fa88dbc9227066cf4cc94d22b05beb1aaaba23fa`, and qualification `beeaece1c43c3341aaf777b04febc57131db8e85dab93be71800e3e552fc9d5a`. Cargo manifests, root workspace policy, `Cargo.lock`, compiler pin and Cargo config have no diff from WP4, so no effective dependency or feature edge changed. |
| `node scripts/toolchain-pin.mjs exec -- cargo xtask check all` | Exit 0 in 18m36s on the clean committed source. Rust, strict Clippy, static/security, cargo-vet, Bun checks, desktop E2E, feature checks and bindings passed. The aggregate does not report a combined Rust test count or per-case E2E count. |

The focused release test suite initially exposed a parallel temp-directory collision: 15 passed, 1 failed and 2 were ignored because `real_curl_upload_fails_closed_on_http_errors_and_redirects` observed another test's payload. The test owner now uses a test-only process-wide atomic sequence in `test_unique_temp`; production naming and assertions are unchanged. After that correction, the full release test filter passed 16/2 ignored. An earlier desktop parallel run had one transient failure in `rapid_playback_sessions_queue_terminal_before_successor_starting` (348 passed, 1 failed, 4 ignored); the isolated selector passed, the full retry passed 349/4 ignored, and the exact-final canonical aggregate passed. No test, assertion, timeout, worker setting or safety gate was weakened.

## Source guards, dependencies and generated bindings

Release and updater source checks now read production owners and test/mock owners separately. `release_pipeline.rs` retains production authority/order checks on the production files; runnable self-test scenario markers are read from `release_v4/selftest.rs`; recovery markers from `release_v4/tests.rs`; fixture HTTP bounds and protocol markers from `updater_fixture/http.rs`; fixture test markers from `updater_fixture/tests.rs`; classifier implementation and tests are scanned separately. The seven added regression selectors above reject missing or misplaced owners/markers rather than allowing test text to satisfy a production contract.

The WP5 diff from accepted WP4 has no changes to any `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo/config.toml`, toolchain pin script or generated binding blob. Locked metadata therefore retains the accepted WP4 projection: 8 workspace members, 73 direct member edges and 544 resolved nodes (semantic SHA-256 `8359fcd5034a6e64c7d927dbadad1df06245ed476e2bd8bc1c0f83ae8fb43e6c`). The accepted four graph fingerprints remain applicable because their manifest/lock/feature inputs are byte-identical. All 93 checked-in files under `apps/desktop/src/bridge/generated` have identical path/blob entries at WP4 and WP5. Canonical binding validation passed.

## Warning delta and cleanup ledger

Architecture warnings changed from 68 at WP4 to 67 at WP5. The sole removed identity is `xtask_module_lines|xtask/src/ci_classifier.rs`, after extracting its tests. No warning identity was added; no threshold or suppression changed. Security findings remain zero.

- Removed inline preparation/capacity implementations and tests from `session.rs`; the extracted owners are the only implementations.
- Removed updater state/progress/tests from `native_update.rs`; it retains the updater service and authority flow.
- Removed runnable self-test/fixtures and Rust test bodies from `release_v4.rs`; CLI reachability is through the new production-compiled `selftest.rs`, and tests live only in `tests.rs`.
- Removed fixture HTTP and test bodies from `updater_fixture.rs`; loopback implementation and tests each have one owner.
- Removed classifier test bodies from `ci_classifier.rs`; classification code remains there.
- Updated active source guards to inspect actual implementations and preserved test owners. No test-only marker was added to production files, and no old duplicate, dead helper, stale active path or compatibility implementation was retained.
- WP4 evidence naming now lists adapter `catalog_tests.rs`, `tests.rs`, `settings_store_tests.rs` and desktop `lib_tests/ipc_tests.rs` / `update_smoke_tests.rs`. Its LOC counts distinguish physical from nonblank lines; the adapter extracted-owner nonblank total is corrected from 1,620 to 2,036. No WP4 product source was changed for this correction.

## Clean artifact provenance and limits

A detached clean worktree at `eef1024493ee23b5ca420d5b981edb354ea4b9ef` had 0 status entries and no legacy `rust/` directory. `bun install --frozen-lockfile` in `apps/desktop` installed 298 locked packages and left tracked status clean. The first build attempt before that install failed because the fresh worktree had no frontend packages; the build passed after the locked install.

From `apps/desktop`, the exact command `node ../../scripts/toolchain-pin.mjs exec -- bun run tauri build --ci --no-bundle --verbose -- --locked --profile dist` exited 0 in 10m00s. Verbose output confirmed the child `cargo build --locked --profile dist --bins --features tauri/custom-protocol`. Rust was `1.99.1 (58cf8f9c3 2026-10-13)`. The build-info freshness self-test reported a clean worktree and embedded commit `eef1024493ee23b5ca420d5b981edb354ea4b9ef`, matching HEAD. Unbundled executable: 16,670,208 bytes, SHA-256 `8b0f5dbd6a3fd358036aa2b1391f1cadd34d8ddb8626a53face0aa5b4fabcf456`. Post-build worktree status remained clean. This is local unbundled build evidence, not installer qualification.

Hosted checks at the pushed evidence HEAD are not awaited and must be read at handoff. Hosted package/updater proofs remain pending until reported by CI. No production signing, live credentials, release/deployment, merge, physical-input qualification or WP6 work was performed. Frontend store changes, engine hot-path changes, playback lifecycle redesign, updater trust/transaction redesign and CI classification redesign remain deferred under the approved exclusions; no material design decision is requested.
