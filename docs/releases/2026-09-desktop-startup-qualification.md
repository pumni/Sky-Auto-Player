# Desktop Startup Qualification — Gate 4

Status: Gate 4 technical and remote qualification evidence recorded for #267.

The comparison uses the locked endpoints:

- baseline: `aa3e20104d11af27cecbfa56f30a41fa0bb1fcf6`
- final candidate: `bdcb7d1799f3b2212121d699abbb0a7eae3a9906`
- branch: `test/desktop-startup-qualification`

## Environment

| Item | Value |
| --- | --- |
| OS | Windows 11 Home, build 26200, x64 |
| CPU | AMD Ryzen 5 5500U, 12 logical processors |
| Memory | 16 GB |
| Storage | NVMe host volumes |
| Rust | 1.98.1 |
| Bun | 1.4.2 |
| Node | 22.13.1 |

The benchmark used packaged `dist` executables built with the same Tauri profile. It did not use
`tauri dev`. The updater key was an ephemeral local fixture outside the repository; Authenticode
remained `unsigned-zero-budget`.

## Methodology

The existing `desktop/scripts/benchmark-startup.mjs` harness was run at each endpoint with three
fixtures (`minimal`, `representative`, and `large`), one discarded warm-up, and ten measured
process restarts for both `coldish` and `warm` modes.

- cold-ish: fresh app-data root and manifest for every run; Windows filesystem/OS caches were not
  cleared;
- warm: one app-data root reused after warm-up; process restart remained part of every sample;
- large fixture: 40 directories × 32 imported sheets = 1,280 imported entries;
- raw machine-readable reports: [baseline JSON](2026-09-desktop-startup-baseline.json) and
  [final JSON](2026-09-desktop-startup-final.json).

The harness preserved the milestone meanings: `react.shell_ready`, cached catalog availability,
catalog readiness/reconciliation, and cold rebuild are reported separately. `shell` below is the
`process.entry → react.shell_ready` median; p95 is the same metric's p95.

The first final-candidate invocation was rejected during the first discarded warm-up
(`minimal` / `cold-ish`, run 0). Its trace was missing `settings.reload.start`,
`settings.reload.end`, and `react.catalog_ready`. No rejected run was added to either report's
measured samples. The parser, marker definitions, fixtures, and methodology were not changed;
the exact command was rerun unchanged, and the rerun completed all 60 measured samples.

## Before/after results

Times are milliseconds. The local shell deltas are retained for traceability, not as a claim of
shell-startup improvement; process-to-shell is effectively flat at the campaign level.

| Fixture | Mode | Baseline shell median / p95 | Final shell median / p95 | Delta |
| --- | --- | ---: | ---: | ---: |
| minimal | cold-ish | 744.71 / 817.23 | 807.67 / 933.53 | +8.5% |
| minimal | warm | 740.82 / 802.23 | 827.35 / 953.84 | +11.7% |
| representative | cold-ish | 757.52 / 817.93 | 755.79 / 1,058.22 | -0.2% |
| representative | warm | 754.77 / 810.80 | 828.80 / 1,094.28 | +9.8% |
| large | cold-ish | 869.97 / 981.35 | 806.02 / 931.30 | -7.4% |
| large | warm | 900.50 / 1,316.71 | 815.63 / 886.64 | -9.4% |

The final candidate's selected milestones are:

| Milestone | Mode | Median |
| --- | --- | ---: |
| React initialize → shell ready | warm | 117.00 ms |
| Fast bootstrap | warm | 0.68 ms |
| Cached catalog available | warm | 43.86 ms |
| React catalog ready | warm | 912.21 ms |
| Background reconciliation complete | warm | 1,069.72 ms |
| Cold/no-cache rebuild | cold-ish | 260.04 ms |

The warm cached catalog target (`≤100 ms`) and warm React/bootstrap targets are met. All shell
medians are below 1 second. Representative-fixture p95 is above 1 second and is retained here as
tail evidence; it is not hidden by redefining readiness. The large/warm p95, the primary expensive
catalog scenario, is 886.64 ms.

The accepted Gate 0 comparison reference for large/warm was approximately 810.0 ms median /
868.6 ms p95. Against that reference, the final 815.63 ms median / 886.64 ms p95 is flat with
normal run variance and slightly slower, so no shell-startup gain is claimed. The qualified gains
are the 0.68 ms bootstrap, 43.86 ms cached availability, 117 ms React initialize-to-shell
duration, and catalog I/O removed from shell TTI.

## Final architecture note

The qualified startup state machine is:

```text
process entry
  → Tauri setup
  → cheap event-hub subscription
  → native runtime/settings/bootstrap snapshot
  → React shell ready
  → cached catalog available when valid
  → background compose/index/reconcile
  → catalog ready and fully reconciled
```

- Event ownership: `AppState` owns the shared hub. Subscription does not construct the native
  runtime, and pre-runtime subscribers receive ordered lifecycle events after runtime creation.
- Settings ownership: `BootstrapDto.settings` is authoritative for initial frontend state.
  External file edits become authoritative only at explicit refresh boundaries such as settings
  reads, update handoff, and mutations.
- Cache source of truth: the library manifest and filesystem remain authoritative. The versioned
  cache is an optimization, guarded by static/dynamic source fingerprints, canonical-path-derived
  identities, bounded validation, and atomic replacement.
- Reconciliation: hydrate and full compose are coherent replacements. Background workers carry an
  epoch claim; stale workers discard work before apply/persist and cannot overwrite a newer import
  or reload.
- Frontend boundary: Settings, Diagnostics, Calibration, and Update are deferred; browser bundle
  tests prove they are not fetched before their feature opens. Public `isTauri()` is confined to
  the platform bridge boundary.

## Qualification matrix

| Scenario | Evidence | Result |
| --- | --- | --- |
| First run / no settings / no cache | Final cold-ish benchmark | PASS |
| Normal warm restart | Final warm benchmark with reused app-data | PASS |
| Empty library | Minimal fixture | PASS |
| Built-in catalog | Installed manifest verification: 96 active songs | PASS |
| User songs directory | Installed fresh-app-data self-test adds and composes one user sheet | PASS |
| Imported recursive folder | Representative and large fixtures; source counters validated | PASS |
| Imported file path | Native import command and catalog regression coverage | PASS |
| Unavailable imported folder | `unavailable_import_preserves_cached_entries_and_manifest_intent` | PASS |
| Corrupt/truncated cache | `corrupt_cache_falls_back_to_cold_rebuild` | PASS |
| Schema/source staleness | Source-fingerprint and cache-schema regression matrix | PASS |
| Same-size + same-mtime mutation | `same_size_same_mtime_content_mutation_still_matches_clean_compose` | PASS |
| Settings patch/restart semantics | `native_bootstrap_uses_explicit_install_root_and_returns_plain_dto` and update-check regression | PASS |
| Calibration recommendation update | `calibration_recommendation_never_changes_the_persisted_user_margin` | PASS |
| Playback prepare/start/stop | Packaged native safe self-test and desktop regression suite | PASS |
| Shutdown during reconcile | Epoch fencing/shutdown regression and packaged lifecycle smoke | PASS |
| NSIS install/launch/uninstall | Local exact-candidate installer qualification and CI #1026 packaged qualification | PASS |
| Updater signing/key rotation contract | Candidate contract, updater signature, rotation self-test, and CI #1026 updater fixture | PASS |
| Second-instance Windows behavior | Local installed-app probe; first instance remained alive | PASS |
| Browser/mock and lazy surfaces | Desktop browser E2E and bundle network assertions | PASS |

The locally qualified NSIS candidate was `Sky Auto Player_4.0.1_x64-setup.exe`:

- installer SHA-256: `b2e583e8b3f67f6ec843c7adb1a266e175dafb8e98d534f33eb95e2d5f4bc212`;
- updater signature SHA-256: `39f065447114395d804b79847184b1f91b49881e64fd528cf6cbb7d504108f56`;
- candidate contract source SHA: `bdcb7d1799f3b2212121d699abbb0a7eae3a9906`.

## Verification record

The following completed successfully on the final integration tree:

- `cargo xtask check all`;
- 160 desktop unit tests and 47 browser E2E tests;
- production bundle E2E with deferred-chunk network assertions;
- exact NSIS candidate contract, unsigned Authenticode evidence, SPDX generation/verification, and
  `cargo xtask verify-tauri-bundle`;
- installed built-in catalog verification, native shell self-test, GUI smoke, second-instance
  probe, and uninstall;
- updater key-rotation fixture.

The qualification source head `903d2a509420b5a63041f3f6fcd98937d406f040` was green in the
manual full [CI #1026](https://github.com/pumni/Sky-Auto-Player/actions/runs/34996084259), run ID
`34996084259`. The full remote run executed and passed the Tauri candidate, packaged NSIS,
updater bridge/fixture, key rotation, release contract, supply-chain, Windows/native, desktop
browser, and required CI gates. The post-qualification branch revision is a docs-only follow-up
correcting milestone labels and verification wording; it does not change production code, raw JSON,
fixtures, parser, or benchmark methodology, so CI #1026 remains accepted without rerun. The local
package evidence above remains supplementary; remote
qualification is the merge gate, and production signing policy remains unchanged.
