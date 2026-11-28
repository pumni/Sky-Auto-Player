# TR0 baseline, diagnosis and tooling repair

**Handoff status: BLOCKED for leader review.** The two dispatched repairs are
implemented and verified. The clean Rust and desktop groups pass. The canonical
`cargo xtask check all --json-summary` run stops at the supply-chain executable
version guard because `cargo audit --version` reports a different executable
name than the guard expects. The actual pinned audit and vet commands pass. No
guard, dependency, runner, test, or policy change was made to work around it.

## Source identity and scope

- Branch: `refactor/source-architecture`; no new branch or PR was created.
- Local checkout initially had clean HEAD `bffefa92e33b5cf9a3fcb914682bc5e407e970f7`.
  After fetch, the remote was exactly the authorized start
  `46c2d0ddca299dc0f5f972bc0b721eee7af78c94`; it was fast-forwarded without
  overwriting local changes. Start tree: `a8023dfd2500a46bbfe82df786458f26bdbef86e`;
  parent: `5bab33faefef97d15e0625d98b1e0eadd209966e`. The start-to-research-baseline
  diff was limited to the three planning documents.
- Clean implementation and locally tested source commit:
  `21616185de8516b75da1a2de39b4a9e7ea435c2d`, tree
  `d8cffee00f1b28dd8572e98d5850e5139f036635`, parent `46c2d0dd...`.
  The subsequent evidence completion commit contains documentation/inventory
  updates only; the handoff comment gives the exact pushed HEAD.
- Implementation changes are limited to `.gitattributes` and
  `.cargo/audit.toml`. The remaining commit content is this evidence and the
  machine-readable inventories in `docs/evidence/test-verification/inventory/`.

## Host and pinned tools

Windows 11 build 26300; AMD Ryzen 5 5500U, 6 physical/12 logical cores, about
16.1 GB RAM. Repository pins resolved to Rust/Cargo 1.99.1, rustfmt 1.10.0,
Clippy 0.1.99, Node 24.21.0 and Bun 1.4.4. Security tools were cargo-audit
0.22.2 and cargo-vet 0.10.2. Browser tooling was Playwright 1.64.1 with
Chromium 156.0.8078.4. Rust commands below use
`node scripts/toolchain-pin.mjs exec --` (abbreviated **P**).

The root checkout had `core.autocrlf=input`; desktop and website Bun dependencies
and website `dist`, `test-results`, and `playwright-report` directories existed
before TR0. Comparator measurements used a detached local worktree at
`bffefa92e33b5cf9a3fcb914682bc5e407e970f7`; focused benchmark runs did not
compete with browser jobs.

## Test and feature inventory

The full selector lists, target metadata, feature-only selectors, test-related
source/support paths, and counts are recorded in the inventory files. Counts
come from pinned list commands; source-path scans are supplementary.

| Inventory | Fresh count |
| --- | ---: |
| Workspace Rust selectors, all features | 1,523 across 8 crates |
| Existing ignored Rust selectors in that listing | 11 |
| Desktop shell, default features + `tauri-test` | 345; 4 ignored |
| Desktop shell, no default features + `tauri-test` | 353; 4 ignored |
| Desktop shell, all features | 346; 4 ignored |
| `ts-rs` export selectors | 89 per desktop config; 356 rows across four listings |
| Rust target metadata | 28 targets; 7 active doctest targets and 0 doctest selectors |
| Desktop Vitest list | 485 selectors |
| Desktop development Playwright list | 175 selectors |
| Desktop production-bundle Playwright list | 1 selector |
| Website Playwright list | 115 selectors |
| Website Bun unit list | 10 selectors |
| Frontend/browser selectors total | 786 |
| Tracked test-related Rust and JS/TS source/support files | 247 Rust + 83 JS/TS = 330 |

The 23 ignored count in `inventory-counts.json` is the 11 workspace ignores
plus the same four desktop ignores repeated in each of three feature listings.
Ignored tests were listed, not executed. The ordinary workspace test command
does not request ignored tests. Desktop feature-only selectors are enumerated
in [`desktop-feature-selector-differences.tsv`](inventory/desktop-feature-selector-differences.tsv):
8 no-default-only IPC selectors, 3 production metadata selectors shared by
default/no-default and absent from all-features, and 4 all-features fixture
update selectors. The plain default desktop-shell test configuration without
`tauri-test` is unsupported under the repository's existing WP6 guidance and
was not run.

The authorized-start Rust inventory commands were:

- `P cargo metadata --locked --format-version 1`
- `P cargo test --workspace --all-features --locked -- --list`
- `P cargo test -p sky_desktop_shell --lib --features tauri-test --locked -- --list`
- `P cargo test -p sky_desktop_shell --lib --no-default-features --features tauri-test --locked -- --list`
- `P cargo test -p sky_desktop_shell --lib --all-features --locked -- --list`

Frontend/browser listings used `bunx vitest list --reporter=json` and
`bunx playwright test --list` in `apps/desktop`; the production-bundle listing
used `PLAYWRIGHT_TEST_DIR=./tests/e2e-bundle`. The website listing used
`PLAYWRIGHT_BASE_URL=http://127.0.0.1:4173 bunx playwright test --list` in
`apps/website`; its runner owns the preview URL during actual functional tests.
The qualification-only `real-input-acceptance` binary was run with its feature
enabled: all 60
test-harness tests passed. Those selectors exercise offline parsing and evidence
oracles; they do not call the binary entrypoint's `run_windows` path or
`SendInput`. No physical input or ReceiveOnly qualification was performed.

The desktop baseline `bun run test` passed 553 tests across 47 files; its list
contained 485 selectors because parameterized tests expand during execution.
The website baseline `bun run test:unit` passed 10 tests with 93 expectations.
All test-source inventory rows are discoverable in
[`test-source-files.tsv`](inventory/test-source-files.tsv); support files are
included and are not counted as test suites.

## Synchronization, ownership and F01–F03

[`timing-candidates.tsv`](inventory/timing-candidates.tsv) has 2,281 lexical
timer, timeout, polling, checkpoint, and wall-clock candidates. Its function
column is a preceding-token hint and `lexical-context-unverified` is deliberate:
it is not a semantic call graph. The count is physical data rows (each row has
eight tab-separated fields); the source excerpts contain quote characters, so
PowerShell `Import-Csv` is not a valid row counter for this unquoted TSV. The
reviewed helper/caller cases are in
[`timing-helper-callers.tsv`](inventory/timing-helper-callers.tsv), and shared
environment, file, process, browser, OS-singleton, and cleanup ownership is in
[`resource-ownership.tsv`](inventory/resource-ownership.tsv).

- **F01:** `signal_supervisor_expiry_prompt_without_locks_under_contention`
  (`supervisor_retirement.rs:636`) and
  `active_worker_contested_reason_lock_preserves_supervisor_unwind_reason_after_finalization`
  (`:922`) start a lock blocker, wait a fixed 20 ms without a lock-acquired
  signal, then rely on a sub-50 ms assertion. The blocker may not hold the lock
  when measured, and the elapsed threshold is host-sensitive.
- **F02:** `production_worker_focus_restore_resumes_after_grace_with_mock_sender`
  (`startup_admission.rs:1190`) has explicit startup checkpoints and pause
  acknowledgements, then fixed 40 ms and 30 ms negative send windows. In
  `terminal_release.rs`, post-join recovery selectors at lines 965, 1019, and
  1203 inspect status/foreground state with fixed 80 ms or 150 ms no-send
  windows; some lack a recovery-cycle acknowledgement. These tests use mocks.
- **F03:** historical CI run 46104910120, job 140855631080, on
  `f89baf898ab0db98fc2c49c785958e0525014a45` failed the 1649x927 compact-player
  assertion at `player-dock-density.spec.ts:124`: the expected FPS/timing-control
  vertical delta was at most 1 px; the report recorded 124 px. No trace/report
  with actual rectangles, style, or DOM state was available. On the
  `bffefa92...` comparator, the focused case passed once (19.880 s outer,
  15.4 s Playwright); the five viewport cases passed 50/50 at workers=1 and
  50/50 at workers=2. The full 175-selector run passed 174 and skipped the
  explicit manual visual-evidence capture. The geometry assertion and product
  code were not changed.

No TR1 synchronization or browser-oracle changes were made. Candidate
parallelism and cleanup notes remain descriptive; no runner/parallelism policy
was selected.

## Static guards, xtask and CI map

[`xtask-guard-inventory.tsv`](inventory/xtask-guard-inventory.tsv) covers all
21 static leaves with invariant, implementation source, known negative
selector/fixture, qualitative cost/risk, and a TR0 disposition of retain until
leader review. Two entries have no dedicated negative mutation fixture found
in this survey; they say so explicitly. Guard disposition remains with the
Leader.

[`xtask-check-map.tsv`](inventory/xtask-check-map.tsv) records 31 group/leaf
entries, command arguments, cwd, features, output roots, environment, skip
semantics, and reuse. `check all` is sequential and fail-fast: static, Rust,
then desktop. `SKY_CHECK_SKIP_SUPPLY_CHAIN` skips only when its trimmed value is
exactly `1`; `SKY_DESKTOP_SKIP_BROWSER` skips when the variable exists, even if
empty. Neither was set in the local group runs. The normal Rust leaf sets
`CARGO_TARGET_DIR=target/check-all-rust-tests`, `RUST_TEST_THREADS=1`, and
`TS_RS_EXPORT_DIR=target/xtask-bindings`. Desktop-native uses a temporary
`TAURI_CONFIG`, compiles runtime and packaged-assets configurations, checks
all-features, then performs read-only binding export/drift against 93 files and
command names.

With `--json-summary`, `CheckSession` reports group and leaf status, overall
duration, and start/end HEAD/tree/cleanliness. It does not report per-leaf
durations. `process::run` captures and suppresses successful child output, so
the raw group log and summary preserve exit/status evidence, not every passing
test line. The process helpers and their bounded child/pipe ownership tests
remain unchanged.

[`ci-lanes.tsv`](inventory/ci-lanes.tsv) maps the ten classifier outputs;
[`ci-jobs.tsv`](inventory/ci-jobs.tsv) maps all 16 workflow jobs, needs,
conditions, runners, effective permissions, and source/artifact bindings. The
required aggregate is `Sky Auto Player — required CI gate`; selected lanes must
succeed and unselected lanes must be skipped. Candidate schema v3 binds the
event-head source SHA and exactly four files (installer, updater signature,
public key, `candidate.json`). Bridge schema v1 binds source SHA and exactly two
files. Artifact names include the source SHA and consumers download from the
current workflow run. The signing proof's temporary workspace key includes
both `GITHUB_RUN_ID` and `GITHUB_RUN_ATTEMPT`. Job permissions remain scoped;
the site validator uploads the exact `dist` artifact and Pages deployment is a
separate qualifying `main`-push path.

## F11/F12 baseline and repairs

**F11 baseline:** `P cargo fmt --all -- --check` exited 1 in 4.757 s on the
authorized start. rustfmt reported incorrect newline style in
`crates/sky_native_adapters/src/lib.rs`; `git ls-files --eol` showed index LF
and a mixed worktree file. Added `*.rs text eol=lf` to `.gitattributes`.

On a fresh local clone of the clean implementation commit with
`core.autocrlf=true`, all 332 tracked `.rs` files reported worktree LF, none
reported mixed/CRLF, the Rust attribute reported `text=set, eol=lf`, and pinned
`P cargo fmt --all -- --check` exited 0 in 6.788 s. The fresh clone remained
clean. The historical
`docs/evidence/issue-661-m2/dispatch-lifecycle-report.json` exception still
checks out CRLF. All 165 protected Git blobs in
[`protected-content-blobs.tsv`](inventory/protected-content-blobs.tsv) matched
the authorized-start blob IDs. Eight raw fresh-worktree hashes differed only
by `core.autocrlf=true` CRLF conversion (`Cargo.lock`, both Bun lockfiles,
`docs/ui/current/manifest.json`, and four song-planning JSON fixtures); their
LF-normalized hashes matched the baseline and the clone stayed clean. No
unrelated attributes or lock/fixture source files were changed.

**F12 baseline:** `P cargo audit --file Cargo.lock` exited 1 in 251 ms because
the cargo-audit 0.22.2 parser required `[output].quiet`. Added `quiet = false`
and preserved `fetch = true`, `stale = false`, `ignore = []`, and
`show_tree = true`. `P cargo audit --file Cargo.lock` then exited 0 in 7.534 s,
loading 1,571 advisories and scanning 544 locked dependencies. It reported
seven allowed warnings, with no configured ignores: unmaintained `chrono`
(RUSTSEC-2026-0543), `proc-macro-error` (RUSTSEC-2024-0370), `zbus_names`
(RUSTSEC-2026-0371), `zvariant` (RUSTSEC-2026-0370), `zvariant_derive`
(RUSTSEC-2026-0372); unsound `glib` (RUSTSEC-2024-0429) and `rustix`
(RUSTSEC-2026-0486). `P cargo vet --manifest-path Cargo.toml --locked` exited
0 in 1.808 s: 1 partially audited and 535 exempted.

## Local verification and first failures

The first canonical `P cargo xtask check all --json-summary` attempt was
preserved and not retried. It exited 1 after 9.797 s at `supply-chain`:
`cargo audit --version` produced `cargo-audit-audit 0.22.2`, while
`xtask/src/supply_chain.rs` expects `cargo-audit 0.22.2`. Direct
`cargo-audit --version` reports the expected `cargo-audit 0.22.2`, and the
actual full-lock audit plus vet above pass. The first six static leaves passed;
the supply-chain leaf failed; the remaining 14 static leaves and Rust/desktop
groups were `not-run` in that canonical invocation. Its summary marked the
pre-commit working tree unqualified/dirty. This executable-name contract is
outside the two authorized repairs; no shim, skip, or source guard change was
used.

The remaining groups were then run on the clean implementation commit:

| Command | Result | Xtask duration | Notes |
| --- | --- | ---: | --- |
| `P cargo xtask check rust --json-summary` | exit 0; clean source identity `21616185...`; all 5 leaves passed | 352.390 s | Pinned fmt; six production-library and xtask no-default dist Clippy checks; workspace all-target/all-features Clippy; workspace all-features tests |
| `P cargo xtask check desktop --json-summary` | exit 0; clean source identity `21616185...`; all 4 leaves passed | 431.496 s | Frozen install, desktop check, unskipped 175-selector E2E (174 pass and one explicit visual-capture skip), native runtime/package clippy, all-features check, binding drift |
| `P cargo fmt --all -- --check` | exit 0 | 2.659 s | Implementation checkout after F11 |
| qualification-only `cargo test -p sky_player --bin rt-native-acceptance --features real-input-acceptance --locked -- --test-threads=1` via P | exit 0; 60 passed, 0 failed/ignored | 15.661 s outer | Test harness only; no physical-input entrypoint |

Website CI steps ran sequentially in `apps/website`: frozen install 0.462 s;
`bun run test:unit` 0.340 s (10 passed/93 expectations); `bun run check`
18.081 s (51 files, 0 errors, 0 warnings, 1 hint); lint 3.481 s; format check
2.671 s; build 5.187 s; dist verification 0.128 s (18 routes/17 static
files); SEO verification 0.116 s (0 errors/warnings); `CI=true bun run
test:functional` 30.532 s (115 passed, Playwright 26.7 s). The pinned Chromium
was already installed on Windows; the Linux-only `playwright install --with-deps`
step was not run.

Xtask's `process::run` suppresses successful child output. The Rust group log
therefore proves all five leaves and the final command exit status, while the
workspace selector inventory provides the 1,523 selectors and 11 ignored
selectors; an exact executed-test total was not emitted by the runner.

An initial PowerShell wrapper attempt invoked bare Bun and recorded Bun help for
the website steps. Those logs were retained as `website-*-invalid-no-args.log`
and are not counted as verification. The corrected commands above have separate
`website-*-after-fix.log` files.

The F11 first-failure log, F12 first-failure log, corrected logs, list output,
benchmark samples and comparator runs are retained outside the Git tree under
the executor's TEMP directory `sky-auto-player-tr0-20261128-46c2d0dd`. The
committed summary and machine inventory are the shareable evidence; the raw log
directory name is stable for local follow-up and avoids embedding a user path.
Key captures are `fmt-before.log`, `audit-before.log`,
`cargo-xtask-check-all-after-fix.log`,
`cargo-xtask-check-rust-after-fix.log`,
`cargo-xtask-check-desktop-after-fix.log`, `audit-after-fix.log`,
`vet-after-fix.log`, `fresh-autocrlf-fmt.log`, and `bffefa-*.log`.

## Comparator measurements

The machine-readable samples are in
[`benchmark-samples.tsv`](inventory/benchmark-samples.tsv). On the original
successful `bffefa92...` comparator, five `sky_player` engine-lib samples passed
370 tests: outer median 30.058 s (warm median 30.048 s; range 30.001–30.199 s;
the cold first run was 60.980 s). The desktop all-features shell samples were
342 pass/4 ignored at 259.916 s and 56.427 s, then 341 pass/1 failed/4 ignored
at 25.783 s. That first failure was
`rapid_playback_sessions_queue_terminal_before_successor_starting`, which
expected a session `Finished` event after observing inactive service status;
the event publication lacked an acknowledgement. The sequence stopped on that
first failure. Separate default/no-default `tauri-test` runs passed 341/349
with four ignored each.

Five timed xtask suite samples passed (357 unit + 2 integration tests): median
53.409 s, range 52.547–55.537 s. The first raw sample passed but its elapsed
time was not retained. Five binding drift samples all matched the 93 tracked
files and command names: overall median 8.935 s; warm median 8.020 s; range
6.848–13.388 s (cold first sample 45.316 s). Full comparator E2E took 618.728 s.
No arbitrary speedup threshold was applied.

## CI status, limitations and cleanup ledger

Hosted CI was not awaited. The Issue #691 handoff records the workflow status
for the exact pushed HEAD; queued/running status is not reported as a pass.

Remaining limits are the canonical supply-chain version-name mismatch, absent
historical F03 trace/DOM measurements, lexical (not semantic) timing
classification, and lack of per-leaf xtask durations. The static guard,
security, release, dependency and feature policies were not changed. No TR1 or
other package was started; the next dispatch and any decision on the version
guard belong to the Leader.

The comparator worktree and fresh autocrlf clone were temporary task-owned
checkouts and were removed after verification. Raw logs remain in TEMP. Existing
website output directories were not deleted. Generated/ignored Cargo targets
were retained because their prior ownership was not established. Binding files
remain byte-identical; no production credentials, real input, release,
deployment, force push, merge, or extra remote branch was used.
