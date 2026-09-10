# CI / Release Control-Plane Refactor — Codex Work Order

## Goal

Simplify the GitHub Actions control plane and path classification without weakening release integrity.

This PR is **control-plane only**. It must not implement the later "single current candidate build" artifact topology. That is a separate follow-up PR.

## Non-negotiable architecture

After this PR, active workflow files should be exactly:

1. `.github/workflows/ci.yml`
2. `.github/workflows/pages.yml`
3. `.github/workflows/release-v4.yml`
4. `.github/workflows/rehearse-v4.yml`

Delete:

- `.github/workflows/release.yml`
- `.github/workflows/site-ci.yml`
- `.github/workflows/rehearse-v4-production-topology.yml`

Rename / consolidate:

- `.github/workflows/rehearse-v4-draft.yml` -> `.github/workflows/rehearse-v4.yml`
- Keep the controlled draft rehearsal behavior as the sole permanent rehearsal workflow.
- Keep `scripts/test_v4_production_topology_rehearsal.ps1` as a local/diagnostic harness; do not expose it as a second permanent workflow.

Production and rehearsal must remain separate workflows. Do not create a reusable release workflow abstraction only to DRY YAML.

## Required gate invariant

Keep the required check name exactly:

`Sky Auto Player — required CI gate`

Do not create a second branch-protection-required status. The site validation job must feed this aggregate gate.

## Classifier redesign

Remove the Rust classifier crate from the Rust workspace and replace it with a small PowerShell classifier:

- `scripts/ci_classify.ps1`
- `scripts/test_ci_classify.ps1`

No external classifier action, no new mini-framework, no JSON schema layer unless strictly required by an existing repo contract.

### Public classifier outputs

Emit exactly these boolean outputs plus a human-readable reason:

- `rust_required`
- `desktop_required`
- `desktop_e2e_required`
- `package_required`
- `updater_required`
- `release_required`
- `supply_chain_required`
- `site_required`
- `classification_reason`

`static_required` is derived inside `ci.yml`; it is not a public classifier output.

Recommended derivation:

```text
static_required =
    rust_required
 || desktop_required
 || package_required
 || updater_required
 || release_required
```

### Event behavior

Pull request:

- classify `github.event.pull_request.base.sha .. github.event.pull_request.head.sha`
- PR cache mode remains read-only where currently applicable.

Push to `main`:

- classify `github.event.before .. github.sha`
- main remains the cache-population path where currently applicable.
- do **not** automatically force the full qualification matrix for every main push.

Manual `workflow_dispatch`:

- run full validation.

Fail closed:

- if base SHA is absent, all-zero, unavailable, cannot be fetched, or `git diff` fails, classify as full validation.
- never convert a classifier failure into "no changes".

## Canonical path/risk matrix

The matrix below is the implementation contract. Do not broaden heavy lanes without a concrete failure mode.

| Changed path | Rust | Desktop | Desktop E2E | Package | Updater | Release | Supply | Site |
|---|:---:|:---:|:---:|:---:|:---:|:---:|:---:|:---:|
| `README.md`, `CHANGELOG.md`, ordinary `docs/**` | | | | | | | | |
| `docs/releases/**` | | | | | | X | | |
| `site/**` | | | | | | | | X |
| `.github/actions/site-validate/**` | | | | | | | | X |
| `.github/workflows/pages.yml` | | | | | | | | X |
| `desktop/src/**` | | X | X | | | | | |
| `desktop/tests/**` | | X | X | | | | | |
| `desktop/scripts/**` | | X | X | | | | | |
| frontend config under `desktop/` | | X | X | | | | | |
| `desktop/package.json`, `desktop/bun.lock` | | X | X | X | X | | X | |
| `desktop/src-tauri/src/**` default | X | X | | | | | | |
| `desktop/src-tauri/src/main.rs` | X | X | | X | | | | |
| `desktop/src-tauri/src/lib.rs` | X | X | | X | | | | |
| `desktop/src-tauri/src/native_update.rs` | X | X | | | X | | | |
| `desktop/src-tauri/build.rs` | X | X | | X | X | | | |
| `desktop/src-tauri/tauri.conf.json` | | X | | X | X | | | |
| `desktop/src-tauri/capabilities/**` | | X | | X | | | | |
| `desktop/src-tauri/icons/**` | | | | X | | | | |
| `desktop/src-tauri/Cargo.toml` | X | X | | X | X | X | X | X |
| `rust/crates/**/src/**` | X | | | | | | | |
| `rust/crates/**/tests/**` | X | | | | | | | |
| `rust/crates/*/Cargo.toml` | X | | | | | | X | |
| `tests/**` | X | | | | | | | |
| `rust/xtask/**` default | X | | | | | | | |
| `rust/xtask/src/tauri_bundle.rs` | X | | | X | | | | |
| `rust/xtask/src/sbom.rs` | X | | | X | | | | |
| `rust/xtask/src/builtin_catalog.rs` | X | | | X | | | | |
| `rust/xtask/src/release_metadata.rs` | | | | | X | X | | |
| `rust/xtask/src/supply_chain.rs` | | | | | | | X | |
| `rust/Cargo.toml` | X | | | X | X | X | X | |
| `rust/Cargo.lock` | X | | | X | X | X | X | |
| `rust/rust-toolchain.toml` | X | X | | X | X | X | | |
| `.cargo/**` | X | X | | X | X | X | | |
| `.config/rust_architecture_allowlist.json` | static-only | | | | | | | |
| `.config/security_audit_baseline.json` | | | | | | | X | |
| `songs/**` | | | | X | | | | |
| `builtin-songs/**` | | | | X | | | | |
| updater scripts | | | | | X | | | |
| Authenticode/package scripts | | | | X | | | | |
| release scripts | | | | | | X | | |
| `scripts/promote_v4_metadata.ps1` | | | | | X | X | | |
| unknown `scripts/**` | X | X | | | | | | |
| `.github/workflows/release-v4.yml` | | | | | | X | | |
| `.github/workflows/rehearse-v4.yml` | | | | | | X | | |
| `.github/workflows/ci.yml` | X | X | X | X | X | X | X | X |
| classifier implementation/tests | X | X | X | X | X | X | X | X |
| unknown `.github/workflows/**` | X | X | X | X | X | X | X | X |
| unknown `.github/actions/**` | X | X | X | X | X | X | X | X |

### Important interpretation rules

- "Changes shipped in the installer" does **not** automatically mean `package_required=true`.
- `package_required` is for packaging-specific failure modes: Tauri/NSIS config, bundle resources, startup/bootstrap, packaging/signing mechanics, bundle verification.
- Ordinary UI/business logic should be covered by frontend/native tests, not by install/uninstall smoke.
- `desktop/src/bridge/**` and `desktop/src/platform/**` are integration-sensitive but not package-sensitive by default.
- `native_runtime.rs` must **not** be made updater-sensitive solely because it contains shutdown phases; preserve current heavy-lane behavior for this PR. A cheaper update-safety contract can be designed later.
- `main.rs` and `lib.rs` remain package-sensitive because they are executable/bootstrap surfaces.
- `songs/**` and `builtin-songs/**` are package-sensitive because they are bundled Tauri resources.

## Desktop browser E2E decoupling

Today browser E2E is effectively tied to `package_required`. Break that coupling.

For this PR:

- `desktop_required` controls normal desktop verification.
- `desktop_e2e_required` controls Chromium install + browser E2E.
- A frontend-only PR such as `desktop/src/App.tsx` must still run desktop browser E2E but must not run NSIS qualification.

Do **not** move browser E2E to Ubuntu in this PR. Runner separation is a later latency PR. This PR only fixes classification/control flow.

## Site CI consolidation

Move the PR website validation into `ci.yml` as a conditional `site` job and include it in the aggregate required gate.

Delete `.github/workflows/site-ci.yml`.

After site validation becomes part of the protected aggregate gate, simplify `pages.yml` so the post-merge deploy path does not repeat the full lint/format/Playwright/accessibility suite. Keep only what is required to build, verify deployable output, upload, and deploy.

Avoid introducing another composite action solely to DRY a few deploy steps.

## Release workflow simplification

Production release remains isolated in `.github/workflows/release-v4.yml`.

Reduce production `workflow_dispatch` semantic inputs to **zero**.

Derive:

- `source_sha = github.sha`
- `version = desktop/src-tauri/Cargo.toml`
- `tag = v${version}`
- `channel = prerelease SemVer ? beta : stable`
- `release_notes_path = docs/releases/v${version}.md`

Remove manual `publication_date_utc` input.

For promoted updater metadata, use the actual GitHub Release `published_at` timestamp as `pub_date` rather than a pre-publication operator timestamp.

Preserve fail-closed validation that source/version/tag/channel/notes correspond to the exact workflow source.

Do not add a fake confirmation input such as "type RELEASE".

## Rehearsal workflow simplification

Keep a single permanent rehearsal workflow:

`.github/workflows/rehearse-v4.yml`

It must preserve the controlled draft rehearsal behavior:

- dedicated release-class runner/environment
- exact source binding
- build candidate
- create draft/tag
- re-download exact draft bytes
- qualify downloaded bytes
- exercise release attestation path as currently intended
- clean up rehearsal draft/tag
- verify external production state did not change

Do not add a `mode: topology|draft` input. The standalone production-topology workflow is retired; its PowerShell harness remains available locally/diagnostically.

## Release/security invariants that must not regress

Preserve:

- official Tauri updater signatures
- exact source SHA binding
- immutable production GitHub Release behavior
- installer and updater-signature digests
- SBOM and production provenance/attestations
- protected release metadata promotion
- dedicated production release runner/environment boundary
- GitHub Actions pinned by full commit SHA
- least-privilege job permissions
- production/rehearsal separation

Do not merge release and rehearsal authority into one workflow.

## Explicitly out of scope for this PR

Do not implement:

- one-build candidate producer / package+updater artifact fan-out
- prebuilt updater bridge
- updater old-root rejection redesign
- moving desktop browser E2E from Windows to Ubuntu
- broad cache redesign
- dist-profile weakening
- merge queue
- new reusable release workflow abstraction
- splitting `ci.yml` into many workflow files

Those belong to later PRs after this control-plane refactor is green and measured.

## Required classifier self-tests

At minimum, table-driven tests must assert:

| Example diff | Required result |
|---|---|
| `README.md` | all domains false |
| `docs/evidence/foo.png` | all domains false |
| `docs/releases/v4.0.2.md` | release only |
| `site/src/pages/index.astro` | site only |
| `desktop/src/App.tsx` | desktop + desktop-e2e; not package |
| `desktop/src/bridge/tauriBridge.ts` | desktop + desktop-e2e; not package |
| `desktop/src-tauri/src/commands.rs` | rust + desktop; not package |
| `desktop/src-tauri/src/native_runtime.rs` | rust + desktop; not updater/package |
| `desktop/src-tauri/src/native_update.rs` | rust + desktop + updater; not package |
| `desktop/src-tauri/src/main.rs` | rust + desktop + package |
| `desktop/src-tauri/src/lib.rs` | rust + desktop + package |
| `desktop/src-tauri/tauri.conf.json` | desktop + package + updater |
| `desktop/src-tauri/icons/icon.ico` | package only |
| `songs/foo.json` | package only |
| `builtin-songs/manifest.json` | package only |
| `rust/crates/sky_player/src/lib.rs` | rust only |
| `rust/crates/sky_player/Cargo.toml` | rust + supply |
| `rust/Cargo.lock` | rust + package + updater + release + supply |
| `scripts/ci_tauri_update_e2e_core.ps1` | updater only |
| `scripts/v4_release_pipeline.ps1` | release only |
| `scripts/promote_v4_metadata.ps1` | updater + release |
| `.github/workflows/pages.yml` | site only |
| `.github/workflows/release-v4.yml` | release only |
| `.github/workflows/ci.yml` | full matrix |
| classifier script/test | full matrix |
| invalid/unusable base SHA | full matrix |

## Acceptance criteria

1. Exactly four active workflow files remain as defined above.
2. `Sky Auto Player — required CI gate` keeps the same check name and gates every relevant conditional job, including site validation.
3. The Rust classifier crate and its workspace membership are removed.
4. Classifier startup requires no Rust toolchain installation/compilation.
5. PR path classification follows the canonical matrix above.
6. Main-push classification uses `before..sha`; it does not force full validation unless classification fails closed.
7. Manual CI dispatch still requests full validation.
8. Frontend-only desktop changes retain browser E2E coverage but do not trigger NSIS qualification.
9. `songs/**` and `builtin-songs/**` trigger package qualification.
10. Site validation is branch-gate-relevant through the existing aggregate check; Pages deploy no longer repeats the full site test suite.
11. Production release has zero semantic dispatch inputs and derives source/version/tag/channel/notes from the exact workflow source.
12. Metadata `pub_date` is derived from GitHub Release `published_at`.
13. A single controlled draft rehearsal workflow remains; production-topology remains only as a local diagnostic harness.
14. All full-SHA action pins and release/security invariants remain intact.
15. No implementation from the explicitly out-of-scope follow-up PRs is introduced.

## Validation expectations

Because this PR changes `.github/workflows/ci.yml` and the classifier itself, it must classify itself as **full validation**.

Before merge, verify at least:

- classifier self-tests
- static/security contracts
- Rust/native validation
- desktop validation + browser E2E
- package qualification
- updater fixture qualification
- release contract acceptance
- supply-chain checks
- site validation
- aggregate required gate

Also review the final workflow list and confirm no stale references to deleted/renamed workflow paths remain in static contracts, docs, attestation signer checks, or release tests.

## Follow-up sequence

After this PR is green and merged:

1. **Build once / qualify many**: introduce a minimal current-candidate producer and make package/updater lanes consume the exact same workflow artifact; preserve updater old-root rejection without rebuilding the candidate; remove ordinary-CI attestations if still present.
2. **Critical-path latency**: move browser/frontend lane to Ubuntu, split native/frontend validation, benchmark cache/tooling changes, and consider a prebuilt updater bridge only if measurements still justify it.

Do not pull either follow-up into this PR.
