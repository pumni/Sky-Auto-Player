# CI validation topology

This document records the accepted validation topology. The Rust-native
`cargo xtask ci classify` command is the source of lane selection; unknown or
malformed classification is always fail-closed to full validation. It writes
the nine lane values and `classification_reason` to stdout and appends them to
`GITHUB_OUTPUT` using a delimiter-safe multiline record for the human-readable
reason.

## Triggers

- `.github/workflows/ci.yml` runs for pushes to `main`, pull requests, and
  explicit `workflow_dispatch` requests.
- A website-affecting `main` push runs the `site` job, packages the validated
  `site/dist` directory as the Pages artifact, and then runs the automatic
  Pages deployment after the required status gate succeeds.
- `.github/workflows/pages.yml` is the manual Pages recovery workflow. It is
  `workflow_dispatch`-only and retains its self-contained checkout, install,
  build, verification, and deployment path.

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
- `supply_chain_required`: advisory and dependency-policy checks are required.
- `site_required`: website validation and Pages artifact packaging are
  required.
- `classification_reason`: the human-readable classifier explanation.

The `changes` job installs the pinned Rust toolchain, caches the xtask build,
and has a 15-minute budget for the first cold build. Pull request and push
classification use the event's explicit base/head commit identities, fetch
missing commits at shallow depth, and never depend on the checkout's current
branch or attached-HEAD state. `workflow_dispatch` requests full validation.
Edits to the Rust classifier and its CLI dispatcher also select full
validation, matching the previous classifier's self-change policy.

`static_required` is a derived CI projection, not a classifier output. It is
true for Rust, Desktop, package, updater, or release work, and for the
existing `static-only` classification. `desktop_required` is the final
Desktop web/browser meaning; native validation uses the separate
`desktop_native_required` output. `desktop_e2e_required` remains the
development-server E2E selector.

Unknown paths, classifier failures, and explicit full validation set every
lane to true. This preserves fail-closed behavior.

## Representative path matrix

| Changed path | Intended major lanes |
| --- | --- |
| `README.md` or general `docs/**` | No specialized lane; aggregate status only |
| `docs/releases/**` | Static validation only; no release contract |
| `desktop/src/**` | Desktop web/browser and Desktop E2E; no native lane |
| `desktop/tests/e2e/**` | Desktop web/browser and Desktop E2E; no native lane |
| `desktop/src-tauri/src/**` | Rust and Windows native Desktop validation |
| `desktop/src-tauri/tauri.conf.json` | Desktop web, native, package, and updater qualification |
| Cargo manifests, lockfiles, or toolchain configuration | Rust plus the affected package/updater/release/security lanes |
| `site/**` | Site validation and exact Pages artifact packaging |
| Release workflow or executable release script | Release contract and its required static/security lanes |
| Classifier or CI workflow | Fail-closed/full validation |
| Unknown path | Full validation across all lanes |

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

Site CI synchronizes the application version once before `check:ci` and
`build:ci`. Local `bun run check` and `bun run build` remain self-contained
through their existing pre-hooks, and manual Pages recovery remains
self-contained.

## Deliberate non-deduplication

Separate runner checkout, Rust setup, and Bun setup remain where they preserve
job isolation and parallel execution. The Windows validation dependency check
is shared within `validate` because both native validation lanes require the
same tool list; lane-specific repository checks remain separate.

Release-specific, updater, signing, SBOM, NSIS, browser, accessibility, and
artifact-integrity tests remain in their existing jobs. This topology does not
trade those checks for shared state.

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
artifact inventory and digest checks, PE inspection, and native calibration
build/stage/installed verification. The workflow still uses PowerShell where
it operates Windows tooling, runs a Tauri fixture, installs/uninstalls NSIS,
or restores fixture source bytes.

| PowerShell responsibility before Wave A | Rust owner after Wave A | Remaining consumer or adapter |
| --- | --- | --- |
| `ci_classify.ps1` and `test_ci_classify.ps1` | `rust/xtask/src/ci_classifier.rs` plus unit tests | `changes` job calls `cargo xtask ci classify`; event base/head SHAs and manual full dispatch are explicit. |
| `ci_validate_candidate.ps1` and its self-test | `ci_candidate.rs` and shared `ci_artifacts.rs`; schema v2 and its exact four-file inventory are preserved | Candidate producer, packaged CI, and updater fixture call `cargo xtask ci candidate create/validate`. |
| `ci_validate_bridge.ps1` and its self-test | `ci_bridge.rs` and `ci_artifacts.rs`; bridge schema v1 and its exact two-file inventory are preserved | `ci_build_updater_bridge.ps1` retains the Windows fixture build and source restore, then calls Rust create; updater consumers call Rust validate. |
| `build_native_calibration.ps1` | `calibration.rs` builds the locked x86_64 MSVC `dist` binary and stages it with rollback | Tauri `beforeBuildCommand` and Windows CI call `cargo xtask calibration build`. |
| `verify_installed_calibration.ps1` and its self-test | `calibration.rs` checks size, SHA-256, x64 PE machine, and bounded `--help`; evidence schema v1 is preserved | Packaged CI and production release orchestration call `cargo xtask calibration verify-installed`. |
| Candidate source identity parsing in `updater_candidate_source_identity.ps1` | `ci.rs` reads the manifest at the exact Git commit and validates SemVer and source identity | The PowerShell file remains a thin caller and keeps the updater fixture's byte-exact restore boundary. |

The migrated PowerShell implementations and their script self-tests are removed
after checking workflow, Tauri, release, updater fixture, and manual callers.
`scripts/capture_defender_evidence.ps1` remains as separate Windows evidence
tooling, and `scripts/scan_v4_defender_exact.ps1` keeps the release's exact
artifact scan. No Wave A command changes gameplay input behavior;
the calibration executable runs only for its bounded `--help` probe.
