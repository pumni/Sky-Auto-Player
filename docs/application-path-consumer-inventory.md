# Application path-consumer inventory

This inventory records the WP-B relocation from the WP-A baseline
`dc0c66153b1b1ba01f4fbc9621a8e77a9edeb92f`. The application trees moved as
tracked files; package names, Bun lockfiles, build outputs, runtime behavior,
and release identity remain unchanged.

## Build-time relative paths

| Consumer | Before | After | Resolution boundary |
| --- | --- | --- | --- |
| `rust/Cargo.toml` workspace member | `../desktop/src-tauri` | `../apps/desktop/src-tauri` | Relative to the Rust workspace root |
| `apps/desktop/src-tauri/Cargo.toml` workspace and local crate dependencies | `../../rust` and `../../rust/crates/*` | `../../../rust` and `../../../rust/crates/*` | Relative to the Tauri crate manifest |
| `apps/desktop/src-tauri/tauri.conf.json` Rust watch folders | `../../rust/*` | `../../../rust/*` | Relative to the Tauri configuration |
| `apps/desktop/src-tauri/tauri.conf.json` bundled song inputs | `../../songs/*`, `../../builtin-songs/manifest.json` | `../../../songs/*`, `../../../builtin-songs/manifest.json` | Relative to the Tauri configuration; bundle destinations are unchanged |
| `apps/desktop/src-tauri/tauri.conf.json` signing script | `../../scripts/sign_v4_authenticode.ps1` | `../../../scripts/sign_v4_authenticode.ps1` | Relative to the Tauri configuration |
| `apps/desktop/src-tauri/tauri.conf.json` calibration build command | `cd ..` | `cd ..\\..` | From the desktop app working directory to the repository root |
| `apps/desktop/src-tauri/build.rs` shared Git metadata module and rerun marker | `../../rust/build_support/git_metadata.rs` | `../../../rust/build_support/git_metadata.rs` | Relative to the build script |
| `apps/desktop/src-tauri/src/lib.rs` native test-song fixture | `../../songs/blue.json` | `../../../songs/blue.json` | Relative to the Cargo manifest directory |
| `apps/desktop/src-tauri/src/native_runtime/tests/playback_integration/start_prepare.rs` schedule contract fixture | Six parent traversals to `tests/fixtures/song_planning/schedule_contract_cases.json` | Seven parent traversals to the same fixture | Relative to the Rust source file |
| `storage_bootstrap.rs` repository-root app assertion | `desktop/` | `apps/desktop/` | Repository root returned by install-root discovery |
| Desktop repository-root discovery scripts | `../..` from `desktop/scripts` | `../../..` from `apps/desktop/scripts` | Repository root for profiling and native build-info checks |
| `apps/desktop/tests/e2e/sheet-compatibility.spec.ts` screenshot evidence output | `../docs/evidence/issue-567-wp-c` | `test-results/visual-evidence/issue-567-wp-c` | Screenshot captures are opt-in, transient and ignored by Git; historical evidence remains accessible at its recorded source revision |

The Tauri `frontendDist`, development URL, resource destination names, icon,
installer mode, updater key, product identity, and generated calibration
resource destination remain unchanged. The independent
`apps/desktop/bun.lock` and `apps/website/bun.lock` remain in place.

## Repository-root path consumers

The following consumers now resolve the relocated application paths from the
repository root:

- Rust workspace and binding output: `rust/Cargo.toml`, `.cargo/config.toml`.
- xtask path ownership, checks, credential, release, bundle, SBOM, updater,
  tooling, and audits: 30 source modules under `rust/xtask/src/`,
  including `checks.rs`, `credential_cli.rs`, `ci_classifier.rs`, `ci.rs`,
  `repo.rs`, `release_signing.rs`, `release_v4.rs`, `tauri_bundle.rs`,
  `sbom.rs`, `updater_fixture.rs`, and `updater_trust.rs`. Website CI and
  architecture test modules are split into focused files.
- WP-C `audits/app_boundaries.rs` locks the app-local manifests and Bun locks,
  rejects cross-app imports/build dependencies and duplicate retired roots,
  and resolves Tauri's package resources, signing script, watch folders, and
  build hook from their documented Windows working directories.
- CI and automation: `.github/actions/site-validate/action.yml`,
  `.github/workflows/ci.yml`, `pages.yml`, `hosted-v4-signing-proof.yml`,
  `release-v4.yml`, `.github/dependabot.yml`, and `.github/CODEOWNERS`.
- Root scripts and policy data: `scripts/toolchain-pin.mjs`, its tests,
  `scripts/run_rt_dispatch_evidence.ps1`, `.gitignore`,
  `.config/rust_architecture_allowlist.json`, `AGENTS.md`, `README.md`, and
  `CONTRIBUTING.md`.
- Maintained architecture, tooling, classifier, packaging, updater, release,
  and branding documentation, plus the current website source instructions.
- App-contained root consumers: the three desktop repository-root discovery
  scripts; the Tauri shared-metadata include, test-song and schedule-fixture
  paths, and repository-root assertion; the generated-binding and benchmark
  path notes; the SEO output path; and the English and Vietnamese source FAQs.

The path classifier maps `apps/website/**` to the website lane,
`apps/desktop/src/**` to desktop web and E2E, and
`apps/desktop/src-tauri/src/**` to Rust and Windows native validation. The
Cargo, Tauri config, lockfile, shared Bun pin, release, mixed-path, CI
self-change, classifier self-change, and unknown-path cases retain their
previous lane unions. Former root `desktop/**` and `site/**` paths are unknown
and select full fail-closed validation.

## Historical references deliberately unchanged

Revision-bound records retain their original paths so commands, source hashes,
and measurement evidence still identify the checked-out revision that produced
them. This includes `docs/adr/**`, `docs/evidence/**`, `docs/releases/**`, the
dated native playback and numeric compatibility records, the R3 and phase
reports, and the Wave A source snapshot in `docs/v4-hosted-signing-proof.md`.
The measured historical sections in `docs/maintenance-test-pipeline-report.md`
also retain their original commands and paths; its current validation summary
uses the relocated paths.

The Markdown integrity audit permits missing targets under the retired
`desktop/` and `site/` roots only for links originating in those revision-bound
history directories. Active documentation and unrelated broken local links
remain errors. The website contract, classifier, and this history-link rule
retain old-root literals only as negative or historical-reference cases.

The classifier keeps old-root examples only as fail-closed negative fixtures.
The website CI contract also retains an old artifact path as a negative case.
Neither fixture routes a real old-root path into an application lane.

## Counts and stale-path result

| Inventory measure | Baseline | Current |
| --- | ---: | ---: |
| Tracked desktop application files | 348 | 348 |
| Tracked website application files | 131 | 131 |
| Active repository-root path-consumer files outside the moved trees | 57 | 59 |
| App-contained path-consumer files, including root-discovery scripts | 12 | 13 |
| Total active path-consumer files | 69 | 72 |
| Active executable/config/current-contract consumers of the retired root paths | 65 | 0 |

The baseline counts scan the tracked WP-A tree for desktop and website path
consumers, excluding moved application files from the repository-root subtotal
and excluding revision-bound evidence. The current counts use the same scope.
The one additional current consumer file is `AGENTS.md`, which now documents
both application roots. A full hidden-file scan found no active old-root path
consumer outside the explicit negative fixtures and revision-bound records.
