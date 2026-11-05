# WP-C Rust tooling audit and Issue #574 diagnostics

Date: 2026-11-05

WP-B source snapshot: `aa00b9d5ac9095e141698e199cfcdfdd14c86803`

Accepted WP-A source snapshot: `2e0e5d1613848e1c57fd474d1bfc985e09fc9438`

This record covers the Rust architecture allowlist ratchet, the tracked evidence
inventory and retention recommendation, a read-only investigation of Issue
#574, and the desktop Biome diagnostics. No playback/runtime source, timing,
session ownership, native test assertion, concurrency, release profile,
evidence asset, or ClamAV scanner was changed.

## Rust architecture allowlist

`cargo xtask check static` owns the architecture policy. The allowlist now uses
exact baselines for live metric exceptions, subjects for individual functions
and context structs, non-empty reason/review fields, normalized repository
paths, known rule names, and duplicate-entry rejection. A metric above its
recorded baseline fails. A metric exception that no longer represents a live
violation fails as stale. The 19 live `unsafe_boundary` entries remain intact.

Eight stale metric entries were removed: `planning.rs` (`regular_module_lines`),
the two Win32 facades (`input.rs`, `wait.rs`), `SessionShared` in `shared.rs`,
and four removed glob-import sites (`physical.rs`, `raw.rs`, `wait/spin.rs`,
`worker/orchestration.rs`). Active baselines are recorded in
`.config/rust_architecture_allowlist.json` and checked by
`rust/xtask/src/audits/architecture/allowlist.rs`.

The production glob check now counts root-level `use super::*;` imports and
ignores nested test-module imports and test-only `*_tests.rs` source files. It
finds one existing import in
`rust/crates/sky_player/src/engine/worker.rs`, covered by its existing exact
baseline of one, and one in `rust/crates/sky_player/src/engine/session.rs`,
which has no approved `production_glob_import` exception. Static therefore
fails at `session.rs` until the Leader decides whether to approve a targeted
baseline exception or require an explicit-import refactor. No exception or
runtime refactor was chosen here.

Focused architecture tests passed **16/16** after the module split. They cover
baseline acceptance, baseline growth, missing/duplicate exception rejection,
stale exceptions, top-level versus nested imports, unsafe-boundary negatives,
and module budgets. The initial focused run found `architecture.rs` at 1,734
lines; the allowlist implementation was split into a child module with a 350
line budget, leaving the existing 1,600 line limit unchanged.

## Issue #574 read-only reproduction

The test file has the same Git blob at WP-A and WP-B:

```text
desktop/src-tauri/src/native_runtime/tests/playback_integration/terminal_release.rs
blob 521e2737f338866603770748aaf7570717c54883
```

The reproduced failure is at line 946:

```text
assertion failed: active.done.load(Ordering::Acquire)
```

It occurs after the delayed cleanup notification and monitor join. The full
backtrace and raw command output are preserved in the workspace log. No root
cause is established. Runs used Windows 11 Home build 26300, an AMD Ryzen 5
5500U with 12 logical processors, Rust/Cargo 1.99.1, `RUST_TEST_THREADS=1`,
`RUST_BACKTRACE=full`, and the same Cargo target directory. Exact environment
records are in [`wp-a-environment.txt`](wp-a-environment.txt) and
[`wp-b-environment.txt`](wp-b-environment.txt).

| Source | Command/result | Diagnostic outcome |
| --- | --- | --- |
| WP-A | `cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked`; exit 101 | Reproduced #574: `sky_desktop_shell --lib` had 326 passed, 1 failed, 4 ignored; the failing assertion was `active.done` at line 946. |
| WP-A | `cargo test --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --all-features --locked`; exit 0 | 327 passed, 4 ignored. |
| WP-A | Three exact isolated runs with `--all-features --locked -- <test-name> --exact --nocapture`; exit 0 each | 3/3 passed in 0.13s, 0.10s, and 0.10s. |
| WP-B | `cargo test --manifest-path rust/Cargo.toml --workspace --all-features --locked`; exit 101 | Did not reach the #574 crate: `sky_xtask::release_v4::tests::real_curl_upload_fails_closed_on_http_errors_and_redirects` failed at `release_v4.rs:7451` (`left: None`, `right: Some(422)`). Its full log is retained. |
| WP-B | `cargo test --manifest-path rust/Cargo.toml -p sky_desktop_shell --lib --all-features --locked`; exit 0 | 327 passed, 4 ignored. |
| WP-B | Three exact isolated runs with `--all-features --locked -- <test-name> --exact --nocapture`; exit 0 each | 3/3 passed, each in 0.10s. |

The package-wide and isolated passes alongside the WP-A workspace failure are
consistent with a broader-run interaction or an intermittent failure; they do
not identify the cause. The handoff's earlier WP-B `check all` failure was
reported at the same assertion, but its temporary RTK log disappeared before
it could be copied into this record. The captured WP-B workspace rerun above
stopped at the independent Curl test, so it is not a fresh full-workspace
result for #574.

The first WP-A build attempt did not start tests because its detached worktree
lacked the Git-ignored `native_calibration.exe` required by Tauri build setup.
The retry used a copy of the existing local file, verified at SHA-256
`13AAB87A7EEE3F2BB98D2F6993266320CC3B0FD5A48854807469FECB1CBF868B`; the
worktree remained clean. The initial WP-B crate command without
`--all-features` and three malformed isolated invocations also did not run the
test; their complete logs are retained and are excluded from the pass counts.

No timeout, scheduling, runtime, or test change was made. Any fix requires a
separate Leader approval.

## Biome inventory

From `desktop`, `bun run biome lint . --reporter=json` exited 0 with 177
unchanged files, 0 errors, 17 warnings, and 5 infos. The raw reporter output is
[`biome-lint.json`](biome-lint.json); the path/rule/severity/line and
actionability inventory is [`biome-inventory.csv`](biome-inventory.csv).
No diagnostic was suppressed or changed.

## Evidence inventory and retention recommendation

[`evidence-inventory-wp-b.csv`](evidence-inventory-wp-b.csv) snapshots all 210
tracked files under `docs/evidence` at WP-B, including byte size, SHA-256, and
text-reference paths. The reference scan covered 214 tracked text files in
`docs`, `.github`, `scripts`, and `.config`. It found 143 evidence files with a
matching unique basename (or full repository path when names repeat). This
tracked-source scan cannot establish whether an external consumer uses an
unreferenced file. The inventory includes 134 PNGs (7,830,167 bytes) and 25
gzip traces (6,497,672 bytes); the complete snapshot is 15,370,988 bytes.

Recommendation: retain all existing evidence in place. The total is about
15.4 MB, and every gzip trace matched a trace manifest. The scan found 47 PNGs
and 16 Markdown files without a matching reference in the scanned source set;
these are review candidates only, not proof that they are obsolete. If archive
cost later becomes material, first assign an evidence owner, verify all
consumers, preserve original hashes and a durable manifest/pointer, and test
restore before changing storage. No existing file was deleted, moved,
compressed, or rewritten.

## PowerShell caller audit

The source-bound caller audit is recorded in
[`../../powershell-dependency-ledger.md`](../../powershell-dependency-ledger.md).
Five standalone test scripts have explicit CI invocations; key rotation and
production-signing tests also have runbook references. The credential-broker
self-test has no tracked CI/runbook caller found and remains marked
**INVESTIGATE**. The evidence-contract script belongs to the documented manual
RT lane. The build-provenance script is read as a source fixture by the Rust
Tauri metadata check; that is not an invocation. No script was removed or
reclassified as retired.

## Verification artifacts

All command logs are retained in this directory, including failures. Key
artifacts:

- [`wp-c-architecture-unit-tests-final-run.log`](wp-c-architecture-unit-tests-final-run.log): 16/16 focused tests passed after the test-only path correction.
- [`wp-a-full-workspace-test.log`](wp-a-full-workspace-test.log): full-workspace reproduction of #574.
- [`wp-b-full-workspace-test.log`](wp-b-full-workspace-test.log): unrelated Curl regression failure before the #574 crate.
- [`wp-a-desktop-shell-lib-all-features-with-resource.log`](wp-a-desktop-shell-lib-all-features-with-resource.log) and [`wp-b-desktop-shell-lib-all-features.log`](wp-b-desktop-shell-lib-all-features.log): package-level passes.
- [`wp-c-static-gate-final-worktree.log`](wp-c-static-gate-final-worktree.log): final static result; docs integrity passes, then architecture stops at the unapproved session glob.
- [`wp-c-check-all-final-worktree.log`](wp-c-check-all-final-worktree.log): final aggregate gate stops at that same static error before Rust and desktop groups.
- Earlier failed attempts remain alongside the final logs, including the initial module-budget and test-source-classification findings.

`cargo fmt --manifest-path rust/Cargo.toml --all -- --check` passed. The final
focused architecture test command passed 16/16. `cargo xtask check static`
exited 1 with one architecture error for `engine/session.rs`; its unsafe
inventory still reports 19 production boundaries. `cargo xtask check all`
also exited 1 at static, so its Rust and desktop groups did not run. The first
static attempts, format check, and over-budget architecture unit run are
retained as well; later runs correct their documentation-link, test-path, and
module-budget findings.

## Decision request

**DECISION REQUEST — production glob in `engine/session.rs`.** The improved
check correctly exposes the existing `use super::*;` at line 7 with no
`production_glob_import` baseline. Please decide whether to authorize an exact baseline exception or
require an explicit-import change. Until then, `cargo xtask check static` and
the aggregate gate are expected to stop at this architecture error. No live
runtime behavior was changed to resolve it.

## Post-review amendment — L-005 correction

This amendment follows the source review of original WP-C checkpoint
`45e202b07d54c0f98ee42c732147d8725d9fa933`. That checkpoint intentionally
failed the static and aggregate gates because `session.rs` had no approved
glob-import exception. The historical failure logs and the original findings
above remain unchanged; this correction checkpoint records the approved
exception and its fresh qualification evidence separately.

L-005 at
[issue comment 6662903829](https://github.com/pumni/Sky-Auto-Player/issues/595#issuecomment-6662903829),
with the formal
[WP-C review](https://github.com/pumni/Sky-Auto-Player/pull/596#pullrequestreview-5788094099),
approved exactly this `production_glob_import` exception:

```json
{
  "path": "rust/crates/sky_player/src/engine/session.rs",
  "rule": "production_glob_import",
  "reason": "pre-existing engine session import surface; preserve qualified session lifecycle while explicit dependency refactor awaits a separately reviewed architecture change",
  "review_trigger": "review before adding session dependencies or changing session ownership; remove when explicit imports are qualified",
  "baseline": 1
}
```

It is the only new allowlist entry. The `regular_module_lines` baseline for
`session.rs` remains exactly 1908; `session.rs` and runtime behavior were not
changed. All 19 Win32 `unsafe_boundary` entries and other exceptions remain as
reviewed.

The former test that expected the current repository architecture audit to
fail is now a positive full-policy test. It checks the exact exception fields
and requires `architecture(&root)` to pass. A separate targeted negative test
requires failure when the exception is missing or keyed to another path, when
the live glob count grows to two, and when the exception is stale after the
violation is removed. `wp-c-l005-architecture-tests-final.log` records the
focused test command and complete output: 17 passed, 0 failed, exit 0.

Fresh final static and aggregate gate command outputs, including their exact
exit codes and any failure diagnostics, are recorded without truncation in
[`wp-c-l005-static-final.log`](wp-c-l005-static-final.log) and
[`wp-c-l005-check-all-final.log`](wp-c-l005-check-all-final.log). The final
format check and whitespace check are recorded in
[`wp-c-l005-format-check-final.log`](wp-c-l005-format-check-final.log) and
[`wp-c-l005-diff-check.log`](wp-c-l005-diff-check.log). These uniquely named
L-005 artifacts do not replace any original WP-C log.

The first static invocation in the new log exited 1 because its check-all log
link target had not yet been created. After creating that evidence target, the
final static invocation passed with exit 0. The final `cargo xtask check all`
run passed its embedded static gate and formatting check, then exited 1 at
Clippy before Rust or desktop test groups ran. Clippy reported
`too_many_arguments` (10/7) for the pre-existing WP-C helper
`architecture_metric_record` in `rust/xtask/src/audits/architecture/allowlist.rs:202`;
the command's exit code was 1 (nested `cargo clippy` exit code 101). The full
raw output is retained in `wp-c-l005-check-all-final.log`. It did not reach the
native #574 or Curl tests, so their historical logs and diagnoses remain the
only evidence for those failures.

**DECISION REQUEST:** Making the aggregate gate pass requires changing the
Clippy-reported WP-C helper or its lint policy, which is outside L-005's
approved exact waiver, focused policy tests, and report correction. Please
decide whether that helper cleanup is separately authorized; no such change
was made in this checkpoint.

## Post-review amendment — L-006 Clippy helper correction

This amendment resolves the Clippy-helper decision request above under L-006.

L-006 at
[issue comment 6663468784](https://github.com/pumni/Sky-Auto-Player/issues/595#issuecomment-6663468784)
and the corresponding
[formal review](https://github.com/pumni/Sky-Auto-Player/pull/596#pullrequestreview-5788438633)
authorized a local typed descriptor refactor for the architecture ratchet.
The implementation source checkpoint is
`c7f390eef809475df152004baec8cc6b31e07992`, based on the reviewed L-005
checkpoint `5974657d8f44ad12a0a9c373e6635777460c1d40`.

`architecture_metric_record` now takes an immutable, copyable
`ArchitectureMetric<'a>` containing `path`, `rule`, `subject`, `observed`,
`unit`, and `hard_limit`. The helper destructures that descriptor and retains
the existing key lookup, ratchet decisions, warning/error text, and stale
entry behavior. Production callsites and related tests use the descriptor;
no lint suppression or flag change was made. No allowlist entry, baseline,
runtime code, release policy, or security policy changed.

The first focused test run exposed the existing 1,600-line `architecture.rs`
budget: the converted calls temporarily brought the file to 1,612 lines. The
full raw failure is preserved in
[`wp-c-l006-architecture-tests.log`](wp-c-l006-architecture-tests.log). The
test fixtures now reuse the immutable descriptors for repeated metrics; the
budget was not raised. The final focused run passed 17/17.

| Final qualification | Result | Raw evidence |
| --- | --- | --- |
| `cargo fmt --manifest-path rust/Cargo.toml --all -- --check` | PASS, exit 0 | [`wp-c-l006-format-check-final.log`](wp-c-l006-format-check-final.log) |
| Focused architecture tests | PASS, 17 passed, exit 0 | [`wp-c-l006-architecture-tests-final.log`](wp-c-l006-architecture-tests-final.log) |
| Explicit workspace Clippy with all targets/features and `-D warnings` | PASS, exit 0 | [`wp-c-l006-workspace-clippy.log`](wp-c-l006-workspace-clippy.log) |
| `cargo xtask check static` | PASS, exit 0 | [`wp-c-l006-static-final.log`](wp-c-l006-static-final.log) |
| `cargo xtask check all` | PASS, exit 0 | [`wp-c-l006-check-all-final.log`](wp-c-l006-check-all-final.log) |
| `git diff --check` | PASS, exit 0 | [`wp-c-l006-diff-check.log`](wp-c-l006-diff-check.log) |

The aggregate run passed workspace Rust tests, desktop `bun run check` and
E2E, desktop cargo checks, and the `sky_desktop_shell` `tauri-test` library
tests. Issue #574's `active.done` failure and the Curl fixture failure did not
recur in this run. No native/runtime or Curl/release fixture changes were
made; prior diagnostic records remain intact. A static check run after this
report amendment is recorded in
[`wp-c-l006-report-static-check.log`](wp-c-l006-report-static-check.log).

## Post-review amendment — L-007 integration qualification and Curl fixture RCA

L-007 authorized investigating the GitHub-hosted failure in
[run 42094708972, job 127481938775](https://github.com/pumni/Sky-Auto-Player/actions/runs/42094708972/job/127481938775).
This work starts from the WP-C accepted source checkpoint
`eed18d7d9222b179baeb0e2a239d5871e9b4d94c`; WP-C architecture work was not
repeated.

The hosted assertion saw `left: None`, expected `Some(503)`, and the test
process exited 101. Its job log shows the five loopback requests through the
503 case, but not curl's version or captured child stdout/stderr. The hosted
runner and missing evidence are summarized in
[`wp-c-l007-hosted-job-excerpt.txt`](wp-c-l007-hosted-job-excerpt.txt). On this
Windows 11 machine, curl is 8.21.0 (Schannel), all proxy environment variables
were unset, and WinHTTP was direct; full sanitized environment details are in
[`wp-c-l007-environment.txt`](wp-c-l007-environment.txt).

Controlled serial runs on the unmodified fixture reproduced intermittent
failures: 2/10 in the initial batch (302 and 403), then 3/10 with temporary
test-only diagnostics (302, 401, and 403). Each had status `None`; the
sanitized stderr was `curl: (56) Recv failure: Connection was aborted`. The
fixture diagnostics showed zero request bytes and
`WouldBlock: A non-blocking socket operation could not be completed
immediately. (os error 10035)`, with no response write error. The listener was
made nonblocking so its accept loop could poll; the accepted stream was also
nonblocking on Windows. Its old `while let Ok(read(...))` treated `WouldBlock`
as end-of-request and sent the status response before it had read the upload
body. Curl then aborted the upload. This explains why the observed failing
status varied with the loop iteration and why the same test reported `None`
for hosted 503 and local 422. The raw pre-fix runs are preserved in
[`wp-c-l007-focused-repros.log`](wp-c-l007-focused-repros.log) and
[`wp-c-l007-focused-diagnostics.log`](wp-c-l007-focused-diagnostics.log).

The correction is confined to the test fixture in `rust/xtask/src/release_v4.rs`:
it explicitly makes each accepted socket blocking, requires the complete
expected upload body before sending a response, surfaces fixture read/write
and thread failures, and keeps the existing read timeout and request bound.
The production curl command, authorization, response parser, and release
policy are unchanged. Production still parses trimmed curl stdout as a `u16`,
filters status zero to `None`, and includes only sanitized stderr in
`HttpFailure`; the archived hosted log and production test helper do not expose
the raw child stdout. The regression test still checks each exact status
401/403/422/500/503/302 and the no-response fail-closed case.

| Qualification on corrected source | Result | Evidence |
| --- | --- | --- |
| Focused Curl fixture, 10 serial repetitions (`RUST_TEST_THREADS=1`) | PASS, 10/10, exit 0 | [`wp-c-l007-focused-fixed-repros.log`](wp-c-l007-focused-fixed-repros.log) |
| `release_v4::tests` | PASS, 16 passed, 2 ignored, exit 0 | [`wp-c-l007-release-tests.log`](wp-c-l007-release-tests.log) |
| Rust format check | PASS, exit 0 | [`wp-c-l007-format-check.log`](wp-c-l007-format-check.log) |
| Workspace Clippy (`--all-targets --all-features --locked -- -D warnings`) | PASS, exit 0 | [`wp-c-l007-workspace-clippy.log`](wp-c-l007-workspace-clippy.log) |
| `cargo xtask check static` | PASS, exit 0 | [`wp-c-l007-static.log`](wp-c-l007-static.log) |
| `cargo xtask check all` | PASS, exit 0 | [`wp-c-l007-check-all.log`](wp-c-l007-check-all.log) |

The full integration gate passed Rust workspace tests, Bun checks and E2E,
desktop Cargo checks, and Tauri library tests. No Curl-version pin or logging
change was made to CI; the hosted runner's curl version remains unrecorded.
There is no remaining architecture decision request for this test-only
correction. Final static contract and whitespace checks for this repository
state are recorded in [`wp-c-l007-final-static.log`](wp-c-l007-final-static.log)
and [`wp-c-l007-final-diff-check.log`](wp-c-l007-final-diff-check.log).
