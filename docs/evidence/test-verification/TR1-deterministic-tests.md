# TR1 deterministic test oracles and bounded dependency correction

## Result and source identity

TR1 implementation and required local verification are complete. This report records the tested source and remaining Leader decisions; it does not claim merge readiness.

- Authorized start: `b8a3affc0d37014d578d435fcd089528cf647b0c` on `refactor/source-architecture`.
- Clean canonical source tested: `9a5229a20bd6b9f8df272f2d3016cd4e05455486`, tree `b8dc533eadcb53aa780e3dc8e5248de2eef6be81`.
- The evidence files are added after that canonical run. The issue handoff records their final evidence-only commit, the exact pushed HEAD/tree, and the hosted CI query. No source or lockfile edits followed the canonical run.
- TR1 code/dependency commits: `83194505` (Astro/fast-uri lock update), `23a87752` (deterministic test oracles), `075da433` (session-scoped recovery polling), `3e0021d3` (Clippy type alias), `9a5229a2` (stable WASM runtime peer).
- Machine-readable canonical output: [TR1-canonical-summary.json](TR1-canonical-summary.json). Focused per-sample runtimes and the viewport repeat runs: [TR1-benchmark-samples.tsv](TR1-benchmark-samples.tsv).

The binding order was Issue #691 comment `7241012591`, plan §4 and SPEC D01–D03/A02–A03. The four scoped guides were added at `crates/sky_player/AGENTS.md`, `apps/desktop/tests/AGENTS.md`, `apps/desktop/src-tauri/AGENTS.md`, and `xtask/AGENTS.md`. The root guide, CI topology, test-runner, coverage policy, public/wire interfaces and production playback timing were not changed.

## Selector and oracle mapping

Test selector names remain stable. The following changes replace timing guesses with causal acknowledgements or bounded state observation; none of the tests were removed.

| Owner and selector | Old oracle | New oracle and invariant | Negative control |
| --- | --- | --- | --- |
| `sky_player::engine::tests::supervisor_retirement::signal_supervisor_expiry_prompt_without_locks_under_contention` | Fixed contention sleeps around a signal expected not to wait on the metrics lock. | An owned blocker signals after acquiring the contested mutex; the test invokes the signal on another thread and proves completion before release. The original `<50 ms` production-path bound is retained. Bounded receive and Drop cleanup release and join both threads on failure. | Injecting the real metrics-lock dependency fails with `signal completes while metrics mutex is held: Timeout` (exit 101, 2.01 s). |
| `...::active_worker_contested_reason_lock_preserves_supervisor_unwind_reason_after_finalization` | Fixed 20 ms delay before checking reason-guard drop. | The blocker confirms mutex ownership; a candidate drop runs on an owned thread and must finish while the lock is held. Explicit release/join and bounded watchdog protect failure cleanup. | Blocking the contested reason lock fails with `guard drop completes while metrics mutex is held: Timeout` (exit 101, 2.06 s). |
| `...::production_worker_focus_restore_resumes_after_grace_with_mock_sender`; `...::overlapping_manual_pause_and_focus_loss_resumes_cleanly`; related startup/manual-pause cases | 30–50 ms sleeps assumed the worker had observed focus or pause decisions. | `StartupOrderingHook` acknowledgements and bounded state polling prove the worker processed focus rejection/restoration while manual pause remained active. Focus-grace timing remains an actual timer contract. | Wrong-decision/duplicate-effect controls are in the recovery tests below; tests use mocked senders only. |
| `...::quit_concurrent_with_modifier_timeout_honors_quit`; `...::skip_concurrent_with_modifier_timeout_honors_skip` | A fixed 45 ms wait attempted to place terminal input after modifier timeout. | Test-only `StartupCheckpoint::ModifierTimeoutBoundary` pauses at the worker’s actual QPC deadline, then releases the real Quit/Skip race. No timeout policy or native scheduler was changed. | Existing terminal-result assertions fail if the losing terminal decision is substituted; all targeted controls passed at the real boundary. |
| `sky_desktop_shell::native_runtime::tests::playback_integration::terminal_release::{monitor_starts_post_join_recovery_after_stop_retirement_and_later_focus_return, post_join_recovery_observes_focus_return_after_terminal_publication_without_winevent, post_join_recovery_rejects_foreign_foreground_owner_drift_and_prejoin_start}` | Recovery was inferred from elapsed time/shared attempt counts, which could observe another parallel test. | A narrowly test-only poll hook reports the session ID, exact foreground decision and whether recovery was attempted. The receiver filters by its session. This preserves parallel test execution and verifies no-send, allowed recovery, owner drift and pre-join cases. | Wrong-decision and duplicate-effect mutants each fail the matching negative assertion (exit 101); no physical input is called. |
| `...::presentation_events::rapid_playback_sessions_queue_terminal_before_successor_starting` | Active status was treated as proof that terminal UI publication had arrived. | Status remains only the successor-admission fence. The successor starts before terminal-event waiting begins. A bounded final-publication wait then verifies each session’s lifecycle order and the existing cross-session terminal-before-successor-Starting order. | Missing terminal publication times out after 3.05 s with both sessions’ buffered Starting/Playing/Finished events but no terminal. Reversing queue order fails `first_terminal < second_starting` (exit 101). |
| Five viewport cases `player dock and compact pager fit {1649x927,1005x615,987x691,800x560,640x448}` | Existing geometry oracle was suspected after the historical 124 px CI report. | No F03 test or UI code change was warranted. The test uses the semantic `contentinfo` landmark named “Player controls”, reads related rectangles from one coherent browser snapshot, and retains the original 1 px alignment/height bounds. | Changing pager padding from `py-0.5` to `py-6` makes the 1649×927 case fail: expected pager height `<=38`, received `81`. Trace SHA-256 `E49DEB70C27D74915B85AA932A66055FB9A6439926790EF9C73C152954038BA9`; local ignored trace path `apps/desktop/test-results/tr1-f03-invalid-layout-20261128/player-dock-density-player-d8c97--compact-pager-fit-1649x927/trace.zip`. The CSS mutation was reverted. |
| Five ordinary-app forged privileged-event smoke cases in `apps/desktop/src/app/App.test.tsx` | Five 250 ms delays were used before asserting no bridge effects. | A listener-registration spy proves the ordinary app never registers the privileged event listener; existing effect assertions remain. The focused app suite passed 27/27. | The listener assertion directly rejects accidental privileged subscription. |

`FocusPauseHook`, `focus_pause_hook`, its config field and runtime call path were removed; a repository-wide `rg` found no remaining references. The former shared recovery attempt counter was removed in favor of session-scoped observations. The modifier timeout boundary is compiled only under test/test-support configuration. The browser E2E selector itself was unchanged.

### Timing review disposition

- **Fixed:** lock-acquisition sleeps in both F01 tests; focus/manual-pause sleeps that asserted worker processing; modifier-timeout placement sleeps; the F02 shared attempt-count oracle; the F15 status-as-publication assumption; ordinary-app 250 ms no-effect waits.
- **Bounded observable polling:** worker focus/pause counters, exact recovery poll decisions/session IDs, and final per-session publication buffers. Polls use a timeout only as a watchdog and always retain cleanup.
- **Retained real timing:** modifier backoff and hold deadlines, pause exclusion, focus grace, supervisor lease expiry, telemetry publication cadence/frozen snapshots, and the 80 ms direct-dispatch quiet interval that proves no periodic wake. Browser waits remain only for the actual 120 ms library debounce and 700 ms mock request. These tests still exercise elapsed time or QPC policy by design.
- **Reviewed and deferred:** other lexical timing hits outside the dispatched owners, and event-delivery/supervisor-lease cases whose sleeps exercise real expiry or publication behavior. The 2,281-row TR0 lexical inventory was discovery data, not a mandate to rewrite every timer.
- Frontend fake-clock cleanup and store/bridge disposal were checked; each affected suite restores clocks and disposes state on its existing teardown path.

## Repeats, benchmark samples and first failures

The focused Rust samples ran ten times at authorized start `b8a3aff...` and ten times at tested source `9a5229a...`; table runtimes are test-harness seconds rounded by libtest, not command wall time. Each sample and its pass/fail count is in the TSV.

| Group | Start median (range) | TR1 median (range) | Samples and result |
| --- | ---: | ---: | --- |
| F01 signal | 0.02 s (0.02–0.02) | 0.00 s (0.00–0.00) | 10 each, 1/1 pass |
| F01 guard drop | 0.08 s (0.08–0.08) | 0.06 s (0.06–0.06) | 10 each, 1/1 pass |
| Startup focus restore | 0.23 s (0.23–0.23) | 0.19 s (0.19–0.19) | 10 each, 1/1 pass |
| Modifier timeout Quit/Skip pair | 0.10 s (0.09–0.10) | 0.19 s (0.19–0.19) | 10 each, 2/2 pass; TR1 now reaches the real timeout checkpoint |
| F02 recovery group | 0.465 s (0.46–0.47) | 0.69 s (0.68–0.70) | 10 each, 3/3 pass; additional time is acknowledged processing, not an artificial suite lock |
| F15 rapid presentation | 0.05 s (0.05–0.05) | 0.05 s (0.05–0.05) | 10 each, 1/1 pass |

Viewport-only repeat: five viewports × 10, workers=1 and workers=2. TR0 comparator `bffefa92e33b5cf9a3fcb914682bc5e407e970f7` passed 50/50 in 3.4 min and 1.9 min respectively; final source passed 50/50 in 1.9 min and 1.1 min. Playwright reports rounded wall times. The additional full-file run also passed 80/80 at workers=1 (3.2 min) and workers=2 (1.9 min). No speed target or tolerance change is claimed.

The first full desktop feature matrix after introducing F02 acknowledgements failed once in each configuration: a process-global attempts atomic observed unrelated parallel recovery tests (`left 2, right 1`). The assertion was corrected to filter acknowledgements by session ID; no tests were serialized. The rerun passed all three matrices:

| Configuration | Result | Test harness | Command wall |
| --- | --- | ---: | ---: |
| Default | 341 passed, 0 failed, 4 ignored | 4.94 s | 16.451 s |
| `--no-default-features --features tauri-test` | 349 passed, 0 failed, 4 ignored | 4.91 s | 25.245 s |
| All features | 342 passed, 0 failed, 4 ignored | 4.83 s | 24.348 s |

Other preserved first verification failures and corrections:

- Initial website `astro check` failed because the lock had floated `@emnapi/runtime@2.0.0-beta.2`; the bridge threw `TypeError: this.bridge.deleteEnv is not a function`. Restoring the already-valid stable `1.11.3` peer resolution made `astro check` and the final full website sequence pass. No package manifest or override was added.
- Initial pinned Clippy found `clippy::type_complexity` in the new recovery hook. A scoped test-only type alias fixed it; the complete pinned Clippy gate then passed.
- The first format check after that edit identified formatter-only layout changes; `cargo fmt` applied them and the final canonical format check passed.
- The first canonical all-gate run passed but correctly marked its unstaged-source identity dirty/unqualified. After committing the two one-file corrections, the exact same canonical command passed on the clean source SHA/tree listed above. The clean run took 554,905 ms; its complete summary records static, Rust and desktop groups as passed and merge readiness as not assessed.

Sanitized negative-control excerpts:

```text
F01 signal: signal completes while metrics mutex is held: Timeout
F01 drop: guard drop completes while metrics mutex is held: Timeout
F02 wrong decision: assertion failed: !wait_for_recovery_poll(..., RecoveryForegroundPoll::NotForeground)
F02 duplicate effect: assertion failed: !wait_for_recovery_poll(..., RecoveryForegroundPoll::ForegroundMatch)
F15 missing terminal: timed out waiting for final lifecycle publication; both session IDs had Starting/Playing/Finished buffered, no terminal publication
F15 reordered queue: assertion failed: first_terminal < second_starting
F03 wrong layout: Expected: <=38; Received: 81
```

All deliberate negative controls exited nonzero as expected; their temporary mutations were reverted. The F03 trace and detailed error context remain local/ignored to avoid committing a bulk trace. No negative control used actual gameplay input.

## Verification

Pinned tools: Node `v24.21.0`, Bun `1.4.4`, Rust/rustfmt/Clippy `1.99.1`, cargo-audit `0.22.2` (the subcommand emits `cargo-audit-audit 0.22.2`), cargo-vet `0.10.2`.

| Command, from repository root unless noted | Result |
| --- | --- |
| `node scripts/toolchain-pin.mjs exec -- cargo fmt --all -- --check` | Pass |
| `node scripts/toolchain-pin.mjs exec -- cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Pass after the type-alias correction |
| `node scripts/toolchain-pin.mjs exec -- cargo test -p sky_player --lib --locked` | 344 passed, 0 failed |
| Desktop native lib tests in default, no-default `tauri-test`, and all-feature configurations | All three pass; counts above; normal harness parallelism preserved |
| `node scripts/toolchain-pin.mjs exec -- cargo test -p sky_xtask --bin xtask cargo_audit_accepts_only_the_two_pinned_complete_banners -- --test-threads=1` | 1 passed; rejects wrong names/versions, extra text, empty and multiline banners |
| `node scripts/toolchain-pin.mjs exec -- cargo audit --file Cargo.lock`; `node scripts/toolchain-pin.mjs exec -- cargo vet --manifest-path Cargo.toml --locked`; `node scripts/toolchain-pin.mjs exec -- cargo xtask audit supply-chain` | All pass. Audit scanned 544 dependencies / 1,571 advisories with seven existing allowed warnings and no new suppression; vet reports 1 partially audited and 535 exempted. Canonical supply-chain route also passed. |
| `node scripts/toolchain-pin.mjs exec -- cargo xtask check all --json-summary` | Pass on clean `9a5229a...`, tree `b8dc533...`; 554,905 ms. Exact JSON is committed beside this report. |
| `bun run check` from `apps/desktop` | Pass: typecheck, Biome, ESLint, tooling 13 tests, Vitest 553/553, and build. Existing advisory chunk-size warning only. |
| Desktop full `bun run test:e2e` | 174 passed, one existing explicit manual-visual-capture skip, 0 failed; 221.968 s. |
| Website from `apps/website`: frozen install, `bun run test:unit`, `check`, `lint`, `format:check`, `build`, `verify:dist`, `verify:seo`, `test:functional` (`CI=true` for functional run) | All pass on final lock. Unit 10 tests/93 expectations; Astro check 51 files/0 errors/0 warnings/1 deprecation hint; distribution 18 routes/17 static files; SEO 0 errors/warnings; functional 115 passed. |

The desktop native full matrix regenerated four trailing-space changes in `apps/desktop/src/bridge/generated/BootstrapDto.ts`; they were restored after confirming that file was clean before generation. The later canonical clean run began and ended with clean status. No generated or test-result artifacts remain tracked.

## F13/F14 supply-chain result and decision request

F13 now accepts only the two complete trimmed pinned banners observed for cargo-audit 0.22.2: `cargo-audit 0.22.2` and `cargo-audit-audit 0.22.2`. No substring, wildcard, alternate version, extra/multiline output or fallback program is accepted. Existing audit/vet execution and warning semantics remain intact.

F14 was bounded to the authorized lockfile repairs: Astro `7.4.0 → 7.4.7` and fast-uri `3.1.8 → 3.1.9`. The package manifest and overrides are unchanged. `@emnapi/runtime` is restored to stable `1.11.3`. Website tests/check/lint/format/build/distribution/SEO/functional checks pass. The local machine has no `osv-scanner`; Docker exists but its Linux engine is unavailable, so no local OSV scan is claimed.

The website lock still contains `minimatch@10.2.6`. The [first advisory](https://github.com/advisories/GHSA-g3hv-75wg-m249) affects `<11.0.1`; a second current [minimatch cache advisory](https://github.com/isaacs/minimatch/security/advisories/GHSA-hqmv-qgp3-jx75) affects `11.0.3–11.0.7` and is fixed in `11.0.8`. Registry metadata queried on 2026-11-29 reports latest `minimatch@11.0.9`.

The installed transitive parents require minimatch `^10.2.4` (`@eslint/config-array@0.23.5`), `^10.2.5` (`eslint@10.12.0`), and `^10.2.2` (`@typescript-eslint/typescript-estree@8.71.2`). Current published candidates appear within the existing root manifest ranges: ESLint `10.16.0` is within `^10.12.0`, and `@typescript-eslint/parser@8.74.0` is within `^8.71.2`; their current dependency graph declares minimatch `^11.0.5` and `^11.0.1` respectively, with ESLint 10 accepted by the parser peer range. A fresh lock resolution can therefore reach the patched 11.0.8+ line without a minimatch override, but it would be an additional coordinated lint-toolchain update beyond the specifically authorized Astro/fast-uri changes. No such update was made. Candidate metadata: [eslint 10.16.0](https://www.npmjs.com/package/eslint/v/10.16.0), [typescript-eslint parser 8.74.0](https://www.npmjs.com/package/%40typescript-eslint/parser/v/8.74.0), [minimatch 11.0.9](https://www.npmjs.com/package/minimatch/v/11.0.9).

Decision requested: authorize a tested in-range ESLint/parser lock refresh to clear minimatch, or defer it to a later dispatch. Also decide whether the website Node engine range should be raised: Astro 7.4.7 requires `>=22.23.4`, while the website manifest currently includes Node `22.22.3` in `^22.22.3`; pinned Node `24.21.0` is supported. No engine policy change was made.

## Cleanup, limits and qualification

- Removed the obsolete focus-pause hook and global recovery attempt counter; migrated all callers to acknowledged observation. Repository-wide symbol search is empty.
- All mutated F01/F02/F03/F15 control code/styles were restored. The detached start-SHA benchmark worktree was removed after use; focused benchmark samples were retained in the committed TSV. Existing unrelated local worktrees were left untouched.
- Temp logs remain under the executor's local `%TEMP%\sky-auto-player-tr1-20261128-b8a3affc`; only the concise sample table, sanitized negative excerpts and canonical JSON are committed so review does not depend on TEMP access. The F03 trace is local and ignored, with its SHA-256 listed above.
- No SendInput/gameplay path, live receiver, physical-input/ReceiveOnly qualification, production credential, release, deployment or merge was invoked. Test recovery used mocks. This is source/local verification, not physical-input evidence.
- Hosted CI was not awaited. The final issue handoff records the workflow/status query immediately after the push; pending/absent checks are not reported as passed.
