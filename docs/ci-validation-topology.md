# CI validation topology

This document records the accepted validation topology. The Rust-native
`cargo xtask ci classify` command is the source of lane selection. Unknown,
malformed, unavailable, ambiguous, or empty Git diffs fail closed to full
validation. It writes the lane values and `classification_reason` to stdout and
appends them to `GITHUB_OUTPUT` using a delimiter-safe multiline record for the
human-readable reason.

## Triggers

- `.github/workflows/ci.yml` runs for pushes to `main`, pull requests, and
  explicit `workflow_dispatch` requests. It also runs full validation every
  Sunday at 03:17 UTC.
- A website-affecting `main` push runs the `site` job, packages the validated
  `apps/website/dist` directory as the Pages artifact, and then runs the automatic
  Pages deployment after the required status gate succeeds. Website package
  manifest and lockfile changes stay in the site lane.
- `.github/workflows/pages.yml` is the manual Pages recovery workflow. It is
  `workflow_dispatch`-only and retains its self-contained checkout, install,
  build, verification, and deployment path.
- Pull request runs share a workflow-and-PR concurrency group and may cancel an
  older run for the same PR. Manual runs have a unique run-ID group, and only
  PR runs are cancelable.

## Classifier outputs

The `changes` job projects these classifier outputs:

- `rust_required`: Rust workspace validation is required.
- `desktop_required`: the Desktop web/browser lane is required.
- `desktop_e2e_required`: Desktop development-server E2E validation is
  required.
- `desktop_native_required`: Windows native Desktop/Tauri validation is
  required.
- `package_required`: package and Tauri qualification is required.
- `updater_required`: updater bridge/contract/E2E validation is required.
- `release_required`: the Windows release-contract lane is required.
- `signing_proof_required`: the independent unsigned-before-sign test-key proof
  is required.
- `malware_proof_required`: the hosted ClamAV and real NSIS positive-control
  proof is required.
- `supply_chain_required`: advisory and dependency-policy checks are required.
- `site_required`: website validation and Pages artifact packaging are
  required.
- `classification_reason`: the human-readable classifier explanation.

The `changes` job installs the pinned Rust toolchain, caches the xtask build,
and has a 15-minute budget for the first cold build. Pull requests explicitly
use `--diff-mode pr`: the classifier verifies that the checked-out `HEAD` is
the event head, computes `git merge-base` for event base/head, and diffs from
that merge base to the head. This avoids treating commits added to `main` after
the feature branch split as PR changes. Pushes explicitly use
`--diff-mode push` and diff `before..head`. For a shallow PR checkout,
bounded retries fetch the event base branch and the repository's
`refs/pull/<number>/head` ref; the classifier verifies that ref still names the
event head. It does not ask the base repository to fetch a fork head by its raw
object ID. Missing, mismatched, or inaccessible refs, source SHA mismatches,
absent merge bases, malformed paths, failed diffs, and empty diffs request full
validation with an explanation. `workflow_dispatch` and the weekly schedule
request full validation. Edits to the Rust classifier, its CLI dispatcher, or
CI control plane also select full validation.

`static_required` is a derived CI projection, not a classifier output. It is
true for Rust, Desktop, package, updater, or release work, and for the
existing `static-only` classification. `desktop_required` is the final
Desktop web/browser meaning; native validation uses the separate
`desktop_native_required` output. `desktop_e2e_required` remains the
development-server E2E selector.

The Linux `static` job installs Go 1.25.0 and actionlint v1.7.12 at pinned
versions, then validates every `.github/workflows/*.yml` file before running
the Rust static and security contracts. The Rust release-runner contract
checks that these pins and the actionlint invocation remain in CI.

Unknown paths, classifier failures, and explicit full validation set every
lane to true. The new proof lanes remain independent jobs and never consume the
candidate artifact as a substitute for either proof.

The aggregate status remains exactly `Sky Auto Player — required CI gate`.
It requires every selected validation lane to succeed and permits an unselected
lane only when its classifier output requires that lane to be skipped. A
successful workflow with a missing required status is not equivalent to this
gate. The status job also requires each selected proof job to succeed and each
unselected proof job to report `skipped`.

## Representative path matrix

| Changed path | Intended major lanes |
| --- | --- |
| `README.md` or general `docs/**` | No specialized lane; aggregate status only |
| `docs/releases/**` | Static validation only; no release contract |
| `apps/website/**`, `.github/actions/site-validate/**`, `pages.yml` | Site validation and exact Pages artifact packaging, including website dependency changes |
| `.bun-version`, Rust setup action, shared toolchain resolver | Full validation across all lanes |
| `apps/desktop/src/**`, React styles/tests on a PR | Desktop web/browser and E2E; no Windows jobs by default, except the Windows integration paths below |
| `apps/desktop/src/bridge/tauriBridge.ts`, `apps/desktop/src/bridge/generated/**`, `apps/desktop/src/platform/runtime.ts`, `apps/desktop/src/platform/windowControls.ts`, `apps/desktop/src/app/BootstrapGate.tsx`, `apps/desktop/src/app/usePackagedSmokeTest.ts` | Desktop web/browser and E2E plus Windows native and NSIS package qualification |
| Any `apps/desktop/**` change on a `main` push | Desktop web/browser and E2E plus Windows candidate/package qualification |
| `apps/desktop/package.json`, `bun.lock` | Desktop web/browser, Windows candidate/package, updater and dependency checks |
| `apps/desktop/src-tauri/**` production source/config/hooks/resources | Windows native and package qualification; updater lane for updater identity/behavior changes |
| NSIS hooks, installer format or packaged payload | Windows package qualification plus signing and malware proofs |
| Shipping `rust/crates/**` source | Rust/native and Windows package qualification; manifest changes also run dependency checks |
| Signing adapter, updater trust, Authenticode scripts/contracts | Signing proof plus related Rust, updater, package, and release lanes |
| Malware scanner, policy, bootstrap, or policy tests | Malware proof and release/security contracts, without an unnecessary Windows NSIS candidate |
| `.github/workflows/release-v4.yml` | Release contract plus independent signing and malware proofs |
| `ci.yml`, classifier, Rust toolchain pin, unknown path | Full validation across all lanes |
| Retired root `desktop/**` or `site/**` | Unknown path; fail-closed full validation |
| Release executable or policy script | Release contract and its selected Windows/security lanes |

Multi-path changes use the union of their lane requirements. A release note
does not suppress a lane required by another changed path.

## Build-once boundaries

- The current Tauri candidate is built once and its exact bytes are consumed
  by packaged and updater qualification.
- The updater bridge is built once and consumed by updater E2E.
- The Desktop web production bundle is built once per `desktop_web` job, then
  previewed for bundle smoke testing.
- The site `dist` is built and tested once in the CI `site` job, packaged
  directly from that directory, and deployed as the exact validated Pages
  artifact.

The website owns a committed snapshot of the latest verified public stable
release in `apps/website/src/data/product-release.json`. Its metadata parser requires
exact `MAJOR.MINOR.PATCH` with no prerelease or build metadata, plus the matching
`v`-prefixed tag; Astro embeds that snapshot without reading the
desktop Cargo manifest or calling Cargo, Tauri, or a release API. Site CI,
local checks/builds, and manual Pages recovery invoke their Bun/Astro commands
directly, without version synchronization pre-hooks. CI validates the
metadata, site checks, build, output, SEO, and functional contracts; manual
Pages recovery verifies the built output and SEO before deployment. Refresh
the snapshot manually after a stable release is publicly published; see
[`apps/website/README.md`](../apps/website/README.md#published-application-version).

## Deliberate non-deduplication

Separate runner checkout, Rust setup, and Bun setup remain where they preserve
job isolation and parallel execution. The Windows validation dependency check
is shared within `validate` because both native validation lanes require the
same tool list; lane-specific repository checks remain separate.

Release-specific, updater, signing, SBOM, NSIS, browser, accessibility, and
artifact-integrity tests remain in their existing jobs. This topology does not
trade those checks for shared state.

## Local command roots

- Run `cargo xtask check all` at the repository root for the canonical
  pre-review static, Rust, and desktop gate. `cargo xtask check desktop-native`
  runs the native Tauri feature checks and generated binding comparison.
- Run desktop Bun commands from `apps/desktop/`. `bun run dev` starts Vite;
  on Windows, `bun run tauri:dev` starts the native shell around Vite.
- Run website Bun commands from `apps/website/` with its own frozen lockfile.
  `bun run check`, `lint`, `format:check`, `build`, `verify:dist`,
  `verify:seo`, and `test:functional` are the standalone site gates.
- The packaging hook is configured for the `apps/desktop/` working directory:
  after `bun run build`, `cd ..\\..` returns to the repository root for
  `cargo xtask calibration build`. CI and local package qualification use that
  same relocated path.

## Safe classifier extension procedure

Future classifier changes must:

1. Add the path rule.
2. Add a positive fixture.
3. Add a negative or union fixture.
4. Preserve unknown-path fail-closed behavior.
5. Run classifier self-tests and the static contract.
6. Verify exact-head CI.

The fixture must demonstrate both the intended lane and the unchanged lanes
for representative multi-path and unknown-path inputs.

## Rust-owned CI, artifact, and calibration tools

Rust owns deterministic CI classification, candidate and bridge schemas,
artifact inventory and digest checks, PE inspection, native calibration, and
the isolated NSIS/updater fixture lifecycle. PowerShell remains for Windows
tool installation, Authenticode provider/tool invocation, and release
transaction orchestration where those are still active adapters.

| PowerShell responsibility before Wave A | Rust owner after Wave A | Remaining consumer or adapter |
| --- | --- | --- |
| `ci_classify.ps1` and `test_ci_classify.ps1` | `rust/xtask/src/ci_classifier.rs` plus unit tests | `changes` job calls `cargo xtask ci classify`; event base/head SHAs and manual full dispatch are explicit. |
| `ci_validate_candidate.ps1` and its self-test | `ci_candidate.rs` and shared `ci_artifacts.rs`; schema v2 and its exact four-file inventory are preserved | Candidate producer, packaged CI, and updater fixture call `cargo xtask ci candidate create/validate`. |
| `ci_validate_bridge.ps1` and its self-test | `ci_bridge.rs` and `ci_artifacts.rs`; bridge schema v1 and its exact two-file inventory are preserved | `cargo xtask ci bridge build-fixture` builds a detached source worktree, then creates and validates the bridge contract; updater consumers call Rust validate. |
| `build_native_calibration.ps1` | `calibration.rs` builds the locked x86_64 MSVC `dist` binary and stages it with rollback | Tauri `beforeBuildCommand` and Windows CI call `cargo xtask calibration build`. |
| `verify_installed_calibration.ps1` and its self-test | `calibration.rs` checks size, SHA-256, x64 PE machine, and bounded `--help`; evidence schema v1 is preserved | Packaged CI and production release orchestration call `cargo xtask calibration verify-installed`. |
| Candidate source identity and fixture-source preparation in the updater scripts | `ci.rs` owns exact-commit source identity; `updater_fixture.rs` owns detached historical bridge construction and source isolation | The canonical repository's Cargo manifest, lockfile, Tauri config, and catalog are never temporarily rewritten. |

Wave C adds `sky_windows_security::registry` and the `xtask` NSIS/updater
owners. `nsis_harness.rs` scopes product HKCU keys, install/AppData roots,
shortcuts, child processes, and cleanup to a unique runner-temp root;
`updater_fixture.rs` verifies exact candidate/bridge bytes, loopback HTTP,
rollback, N-to-N+1 migration, old-root rejection, and user-song preservation.
The Windows workflow jobs call these Rust commands directly. The migrated
PowerShell scripts and current remaining entrypoints are tracked in
[`powershell-dependency-ledger.md`](powershell-dependency-ledger.md).

The migrated CI and Wave C fixture scripts and their script self-tests are
retired after the workflow, release, documentation, and test consumers were
migrated. `scripts/capture_defender_evidence.ps1` remains as separate Windows
evidence tooling, and `scripts/scan_v4_defender_exact.ps1` keeps the release's
exact artifact scan. No migrated command changes gameplay input behavior;
calibration verification runs only its bounded `--help` probe.

## Wave C execution and teardown bounds

The `updater_bridge`, `updater_e2e`, and `packaged` jobs in
`.github/workflows/ci.yml` each have a 90-minute job timeout. The hosted
`.github/workflows/release-v4.yml` bounds its unsigned build at 120 minutes,
sign/qualification at 180 minutes, attestation at 45 minutes, publication at
60 minutes, and final verification at 20 minutes. Its preflight and metadata
jobs do not set `timeout-minutes` and therefore use GitHub Actions' documented
360-minute job default. The retired `rehearse-v4.yml` production rebuild no
longer exists.

The fixture owners also bound individual work: bridge dependency installation
has a 45-minute deadline and bridge compilation has a 75-minute deadline;
NSIS install and uninstall children have 300- and 60-second deadlines;
updater children have 180-second deadlines. These operation deadlines do not
control teardown. Owned Windows process trees are terminated with a 2-second
teardown budget, and inherited output pipes have a 500-millisecond drain
budget. Loopback request headers have a 10-second total read deadline; each
response's headers and body share a 120-second absolute write deadline. Each
socket operation is capped at 2 seconds, and shutdown interrupts an active
socket before joining the fixture server thread. Cleanup continues across all
registered stages and returns aggregated failures. The outer job timeout
remains the final bound if a runner or operating-system operation does not
respond.

The documented GitHub Actions job default is described in the
[workflow syntax reference](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#jobsjob_idtimeout-minutes).

## Wave E deterministic evidence tools

`rust/xtask/src/assembly_audit.rs` now owns the deterministic parsing and
assertions for the dispatch, prepared-normal, and modifier-guard optimized
assembly audits. The existing `scripts/audit_*_assembly.ps1` names remain as
thin command adapters because the retained RT evidence runner calls them.
Their Rust fixtures cover both accepted and rejected assembly shapes.

`cargo xtask calibration project-evidence` owns startup calibration JSON
projection bytes and SHA-256 sidecars. It normalizes CRLF and CR to LF, writes
UTF-8 without a BOM, and emits exactly one trailing LF. The startup export
PowerShell entrypoint still validates the 120-run manifests and assembles the
sanitized paired-run report; its projection helper delegates to Rust. The
existing self-test checks that adapter against LF/CRLF fixtures and Git
checkout behavior with `core.autocrlf=true`.

Wave E did not change the native acceptance runner, WinForms ReceiveOnly sink,
startup qualification orchestration, Defender evidence capture, or GitHub
evidence publisher. It ran no physical input acceptance. The full startup
report builder remains PowerShell until a complete golden export fixture can
prove output parity.
