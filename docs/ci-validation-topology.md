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
the Rust static and security contracts. It also runs deterministic offline
fixtures for the read-only CI metrics tool. The Rust release-runner contract
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
| `docs/releases/**` (`{tag}.md`) | Release assembly consumes the matching notes file; documentation-only CI classification |
| `apps/website/**`, `.github/actions/site-validate/**`, `pages.yml` | Site validation and exact Pages artifact packaging, including website dependency changes |
| `.bun-version`, Rust setup action, shared toolchain resolver | Full validation across all lanes |
| `apps/desktop/src/**`, React styles/tests on a PR | Desktop web/browser and E2E; no Windows jobs by default, except the Windows integration paths below |
| Windows integration paths:<br>`apps/desktop/src/bridge/tauriBridge.ts`<br>`apps/desktop/src/bridge/startupTelemetry.ts`<br>`apps/desktop/src/bridge/generated/**`<br>`apps/desktop/src/platform/runtime.ts`<br>`apps/desktop/src/platform/windowControls.ts`<br>`apps/desktop/src/platform/startupWindowReveal.ts`<br>`apps/desktop/src/platform/systemAppearance.ts`<br>`apps/desktop/src/platform/externalLinks.ts`<br>`apps/desktop/src/app/BootstrapGate.tsx`<br>`apps/desktop/src/app/usePackagedSmokeTest.ts`<br>`apps/desktop/src/main.tsx` | Desktop web/browser and E2E plus Windows native and NSIS package qualification |
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

The classifier keeps these Tauri entrypoint and adapter paths in an explicit
Windows integration owner set. A Rust regression test also scans production
`.ts`/`.tsx` files for direct runtime imports from `@tauri-apps/api/*` and
`@tauri-apps/plugin-*`, excluding test/spec files, declaration files, and
type-only imports, and asserts that each importer selects Windows native and
package qualification.
This scan is test-only; path classification does not inspect frontend source at
runtime. New Tauri runtime adapters must be added to the explicit owner set.

## Build-once boundaries

- The current Tauri candidate is built once and its exact bytes are consumed
  by packaged and updater qualification.
- The updater bridge is built once and consumed by updater E2E.
- The Desktop web production bundle is built once per `desktop_web` job, then
  previewed for bundle smoke testing.
- The site `dist` is built and tested once in the CI `site` job, packaged
  directly from that directory, and deployed as the exact validated Pages
  artifact.

Site CI and local builds invoke Bun/Astro directly. The website embeds stable
download and release history destinations without reading desktop version
metadata or calling a release API. CI validates the build, output, SEO, and
functional contracts. Manual Pages recovery verifies the built output and
SEO before deployment. See the [website release policy](../apps/website/README.md#release-destinations).

## Deliberate non-deduplication

Separate runner checkout, Rust setup, and Bun setup remain where they preserve
job isolation and parallel execution. The Windows validation dependency check
is shared within `validate` because both native validation lanes require the
same tool list; lane-specific repository checks remain separate.

Release-specific, updater, signing, SBOM, NSIS, browser, accessibility, and
artifact-integrity tests remain in their existing jobs. This topology does not
trade those checks for shared state.

## Local impact inspection

`cargo xtask ci classify --working-tree [--base <ref>]` is a read-only preview of PR-mode
lanes for staged, unstaged, and untracked non-ignored paths relative to HEAD, or from
the local merge-base of an explicit base ref.
It uses the existing classifier, treats renames as deletion plus addition, and
fails closed on Git errors, missing base commits, or malformed paths. This is an
advisory impact hint, not a replacement for the canonical merge gate or a
behavioral risk assessment.

`cargo xtask check <group> --json-summary` emits an `[xtask-summary]` JSON record
with monotonic `duration_ms`, observed leaf and group outcomes, bounded failure reasons, not-run steps, explicit supply-chain/browser
skips and catalog reuse. The summary reports start and finish source observations (`head`, `tree`, and `status`),
along with `overall_status` (`"clean"`, `"dirty"`, `"changed-head-or-tree"`, or `"unknown"`); it records working tree cleanliness at start and finish without
asserting continuous immutability throughout execution. Non-clean inputs are explicitly identified with an `unqualified-` prefix on `source_identity` rather than
asserting clean HEAD evidence. Group success does not imply unexecuted children ran; ordinary failures still emit the summary while
preserving non-zero exit semantics.
Preserve the raw command log, exit code, and source identity separately. This summary is not merge authorization.

## Local command roots

- At the repository root, run `node scripts/toolchain-pin.mjs exec -- cargo xtask check all`
  for the canonical final static, Rust, and desktop gate. Prefix other root Cargo commands
  below with the same resolver (see [Rust toolchain policy](rust-toolchain-policy.md)). `cargo xtask check desktop-native`
  runs warning-denying native Tauri production lint, all-feature checking,
  and generated binding comparison. `cargo xtask check rust` also lints each
  first-party library separately without test features, in the `dist` profile,
  plus the standalone xtask binary before the all-feature Clippy/test pass.
- Run desktop Bun commands from `apps/desktop/`. `bun run dev` starts Vite;
  on Windows, `bun run tauri:dev` starts the native shell around Vite.
- Run website Bun commands from `apps/website/` with its own frozen lockfile.
  `bun run check`, `lint`, `format:check`, `build`, `verify:dist`,
  `verify:seo`, and `test:functional` are the standalone site gates.
- The Tauri packaging hook is configured for the `apps/desktop/` working
  directory and runs `bun run build`. Calibration is no longer a packaged
  executable or package-time build step; native wake diagnostics remain part of
  the production runtime.

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
artifact inventory and digest checks, PE inspection, and the isolated
NSIS/updater fixture lifecycle. PowerShell remains for Windows
tool installation, Authenticode provider/tool invocation, release transaction
orchestration, Defender evidence capture, and the exact Defender artifact scan.

| Rust owner | Current contract | Active caller or adapter |
| --- | --- | --- |
| `rust/xtask/src/ci_classifier.rs` and unit tests | Classifies event base/head SHAs and manual full dispatch; unknown or invalid changes fail closed to full validation. | The `changes` job calls `cargo xtask ci classify`. |
| `ci_candidate.rs` and shared `ci_artifacts.rs` | Candidate schema v3 binds only the installer, updater signature, and public key; the exact four-file inventory remains locked. | Candidate producer, packaged CI, and updater fixture call `cargo xtask ci candidate create/validate`. |
| `ci_bridge.rs` and `ci_artifacts.rs` | Bridge schema v1 preserves its exact two-file inventory and detached source worktree. | `cargo xtask ci bridge build-fixture` builds and validates the bridge; updater consumers call Rust validation. |
| `startup_calibration.rs` evidence projection | Canonicalizes historical startup wake-calibration evidence bytes and writes the SHA-256 sidecar. Production startup wake calibration has been removed in favor of the fixed `1000 us` spin threshold; this offline projection utility is retained solely for historical evidence reproducibility and its CI self-test. | Historical evidence export scripts and CI self-test call `cargo xtask startup-calibration project-evidence`. |
| `ci.rs` and `updater_fixture.rs` | Verify exact-commit source identity and prepare detached fixture sources without rewriting the canonical Cargo manifest, lockfile, Tauri config, or catalog. | Updater fixture callers use the Rust-owned source preparation and validation. |

`sky_windows_security::registry` and the `xtask` NSIS/updater owners enforce the
Windows fixture boundary. `nsis_harness.rs` scopes product HKCU keys,
install/AppData roots, shortcuts, child processes, and cleanup to a unique
runner-temp root. `updater_fixture.rs` verifies exact candidate/bridge bytes,
loopback HTTP, rollback, N-to-N+1 migration, old-root rejection, and user-song
preservation. Windows workflow jobs call these Rust commands directly.

## Hosted execution and teardown bounds

The `updater_bridge`, `updater_e2e`, and `packaged` jobs in
`.github/workflows/ci.yml` each have a 90-minute job timeout. The hosted
`.github/workflows/release-v4.yml` bounds its unsigned build at 120 minutes,
sign/qualification at 180 minutes, attestation at 45 minutes, publication at
60 minutes, and final verification at 20 minutes. Its preflight and metadata
jobs do not set `timeout-minutes` and therefore use GitHub Actions' documented
360-minute job default.

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

## Deterministic evidence tools

`rust/xtask/src/assembly_audit.rs` owns deterministic checks for the live
`sky_player::engine::worker::dispatch_loop::dispatch_candidate`,
`sky_player::engine::worker::admission::verify_final_packet_admission`, and the
Win32 deferred sender. Both `dispatch_candidate` and
`verify_final_packet_admission` carry `#[inline(never)]`. The `dispatch` audit
verifies unconditional CFG dominance (`entry -> verify_final_packet_admission -> deferred_sender -> on_send_completed_at -> commit_physical_packet`) with no
jump-table bypass waivers, audits the final admission gate body for fixed
modifier checks and fresh owner proof without physical sends, and checks the
post-gate suffix before `deferred_sender`. The `modifier-guard` audit checks
the fixed modifier order and the scoped Win32 send path, including one
`SendInput` between pre-call and completion QPC samples. It deliberately does
not apply copy/division thresholds to the whole worker function.

`prepared-normal` is retained only as a command-line compatibility alias for
the same live dispatch audit; it does not refer to the removed prepared-normal
scheduler. The `scripts/audit_*_assembly.ps1` files are thin adapters for these
current audits. Rust fixture tests cover accepted and rejected assembly
shapes, and the audit has also been run against actual Windows x64 `dist`
assembly output.

`cargo xtask startup-calibration project-evidence` owns startup wake-calibration JSON
projection bytes and SHA-256 sidecars. It normalizes CRLF and CR to LF, writes
UTF-8 without a BOM, and emits exactly one trailing LF. The startup export
PowerShell entrypoint still validates the 120-run manifests and assembles the
sanitized paired-run report; its projection helper delegates to Rust. The
existing self-test checks that adapter against LF/CRLF fixtures and Git
checkout behavior with `core.autocrlf=true`.

The complete startup report builder remains PowerShell while a golden export
fixture for Rust output parity is unavailable.
