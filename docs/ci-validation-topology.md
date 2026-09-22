# CI validation topology

This document records the accepted validation topology. The classifier is the
source of lane selection; unknown or malformed classification is always
fail-closed to full validation.

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
