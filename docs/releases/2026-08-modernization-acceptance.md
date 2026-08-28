# Dependency, Toolchain, Packaging & Release Modernization Acceptance

Date: 2026-08-28

## A. Baseline and final state

- Requested baseline: `cf6fb3de97f5e3290c46c7bd7532c90733bbfe9b`.
- Final implementation tip before this report-only commit: `d1afa3d1c16a8adb2dbc4b39ef6fa593a1955cc9`.
- Branch: `main`.
- `origin/main` was fetched before finalization and remains at the requested baseline; no newer
  remote changes were overwritten. The local branch is ahead and has not been pushed.
- The nine local commits after the baseline remain purpose-separated by workstream.

Environment used for qualification:

| Component | Qualified value |
| --- | --- |
| OS | Windows 11 build `26200` |
| Python | `3.14.7+freethreaded` |
| uv | `0.12.7` |
| Rust | `1.98.0` |
| PyO3 | `0.29.2` |
| PyInstaller | `6.22.2` |
| Maturin | `1.15.0` |
| Bun | `1.4.0` |
| Astro | `7.2.9` |
| TypeScript | `6.0.3` |

CPU model capture was unavailable because WMI access was denied on this host.

## B. P0 CI investigation

The old GitHub CI run `33180962015` failed only at the partial transport fault matrix. The failure
was reproducible as a terminal snapshot consistency race, not as a SendInput behavior problem.

`SnapshotBuffer` uses two reader-pinned slots. During worker finalization, both slots can be pinned
briefly, causing the forced terminal metrics publication to return `false`. The old code ignored
that result and published terminal lifecycle state afterward, so a terminal snapshot could expose
`outcome="error"` while `sendinput_partial_events` was still zero.

Fix in `rust/crates/sky_player_rs/src/engine/telemetry/metrics.rs` and
`rust/crates/sky_player_rs/src/engine/worker/cleanup.rs`:

- terminal metrics publication now has an explicit success contract;
- it yields and retries only the bounded telemetry snapshot publication until a slot is available;
- lifecycle terminal publication therefore follows a self-consistent terminal counter snapshot;
- no SendInput retry, transport retry, timing change, or precision-path allocation was added.

Evidence:

- exact P0 node: `100/100` pass, `0` failures;
- full equivalent non-slow suite: `946 passed, 14 skipped, 1 xfailed`;
- full equivalent suite: `946 passed, 14 skipped, 1 xfailed`;
- Rust workspace and zero-allocation gates pass.

The canonical `scripts/check.py tests` command still encounters `PermissionError: [WinError 5]`
when pytest removes the pre-existing locked root `.pytest_tmp` directory. This is a host filesystem
condition; the same tests pass with an isolated `.tmp` basetemp.

## C. Changes committed locally

| Commit | Scope | Result and evidence |
| --- | --- | --- |
| `cf1a82e` | deterministic partial-fault terminal counters | P0 race fixed; 100-run stability pass |
| `2ec516a` | exact Python and uv pinning | `.python-version` is `3.14.7+freethreaded`; `uv.toml` requires `==0.12.7`; frozen sync pass |
| `819adb3` | Maturin, pytest-benchmark, attestation action | Maturin `1.15.0`, benchmark `5.3.0`, attestation `v4.2.2` pinned by full SHA; static/tests/wheel gates pass |
| `4d3d444` | PyInstaller source provenance | PyInstaller `6.22.2` source tag is checked against upstream commit `19f42e7f13d56cd880a4ced8bb3594875e5227c6` |
| `de487d5` | updater ZIP backend | `zip 8.6.0` with `zlib-rs`; real release archive qualification and updater security matrix pass |
| `4182509` | site patch/minor dependencies | `@astrojs/check 0.9.10`, axe `4.13.0`, Playwright `1.62.1`, typescript-eslint `8.68.0`, ESLint `10.9.1`, eslint-plugin-astro `3.1.0`; site suite pass |
| `77b6402` | Bun toolchain pin | `site/package.json` now pins `bun@1.4.0`; Bun 1.4 full site suite pass |
| `40e4f74` | Dependabot grouping | runtime, tooling, packaging, site regular/major, and action security boundaries are separated |
| `d1afa3d` | Astro minor update | Astro `7.2.9` isolated and main candidate qualification pass |

Python runtime packages `packaging 26.3` and `textual 8.2.8`, and PyInstaller tooling `6.22.2` /
hooks `2026.7`, were already present on the requested baseline or had already been qualified; no
duplicate update was introduced.

## D. Rejected or deferred changes

- TypeScript 7.0.2 was tried in an isolated worktree. `astro check` rejects it because TypeScript 7
  no longer exposes the programmatic compiler API currently required by Astro language server.
  TypeScript remains `6.0.3`; do not retry until the Astro toolchain supports TS7.
- An earlier `cold_path()` experiment was reverted because it produced no measurable dispatch win.
- No Rust dispatch, SendInput, QPC, wait, spin, calibration, or packet semantics were changed.
- No Tokio, new concurrency primitive, retry path, custom allocator, unsafe optimization, or global
  overflow-check change was introduced.
- ZIP producer reproducibility was not silently “fixed”. Two same-host builds using the existing
  `Compress-Archive` pipeline produced different bytes, so producer reproducibility remains a
  separate investigation.
- Pytest 9, SHA2 0.11, and unrelated major ecosystem migrations were not bundled into this work.

## E. Packaging and release acceptance

Passed at the final implementation tip:

- exact free-threaded native wheel tag: `cp314t-win_amd64`;
- `build_info()` reports `free_threaded=True`, PyO3 `0.29.2`, Rust `1.98.0`, schema `4`, Win32
  backend enabled, and the expected source commit;
- GIL remains disabled after native import;
- source-built PyInstaller bootloader path and allowlisted upstream commit verification;
- PyInstaller application build and textual/optimized/Rust smoke tests;
- release manifest generation and verification;
- packaged updater update plus injected rollback E2E;
- updater safety tests for hash mismatch, manifest mismatch, malicious paths, rollback, locked files,
  concurrent updater state, and user-state preservation.

The release ZIP reproducibility experiment found five differing entries, including generated
metadata and native binaries. The existing archive producer therefore remains an open maintenance
item, not a merged behavior change.

## F. Performance evidence

The ZIP experiment used a real v3.4.5 release tree with 235 files and 30,835,159 input bytes,
10 warmups and 30 measured iterations per run.

| Metric | `zip 2.4.2` | `zip 8.6.0 + zlib-rs` | Delta |
| --- | ---: | ---: | ---: |
| across-run median | 1,097,057 us | 1,015,414 us | -7.4% |
| across-run p95 | 1,436,732 us | 1,292,528 us | -10.0% |
| across-run max | 1,956,166 us | 1,444,442 us | -26.2% |
| measured total | 36,295,267 us | 31,329,758 us | -13.7% |

The complete raw matrix is in
`docs/perf-baselines/2026-08-updater-zip-qualification.md`. The candidate had one tail outlier,
so these numbers are qualification evidence, not a guaranteed tail bound.

Bun comparison used five runs each on the same site source and lockfile:

| Metric | Bun 1.3.14 | Bun 1.4.0 | Delta |
| --- | ---: | ---: | ---: |
| clean install | 6,023 ms | 5,263 ms | -12.6% |
| warm install | 104 ms | 74 ms | -29.1% |
| `bun run check` median | 11,975 ms | 11,955 ms | -0.2% |
| `bun run build` median | 3,985 ms | 3,913 ms | -1.8% |

The small check/build differences are treated as noise; the decision to pin Bun 1.4 is
reproducibility/current-support, not a runtime performance claim. Both Bun versions passed the
58-test functional/accessibility/navigation suite.

## G. Risk assessment

| Area | Risk | Assessment |
| --- | --- | --- |
| Terminal telemetry fix | LOW | Scope is outside the precision boundary; terminal snapshot consistency is directly tested. |
| Python/uv/tooling pins | LOW | Frozen sync, static checks, full tests, and native wheel metadata pass. |
| ZIP 8 migration | MEDIUM | Major dependency/API and decompression backend change; archive, security, rollback, and user-state tests pass. |
| Site/Astro/Bun updates | LOW | Frozen install, check, lint, format, build, dist/SEO, and 58 E2E pass. |
| ZIP byte reproducibility | MEDIUM | Current `Compress-Archive` output is not deterministic; root cause remains unqualified. |
| Final GitHub CI | MEDIUM | Required final commit run is pending because local commits were not pushed. |
| Real production SendInput qualification | HIGH/OPEN | This program did not run the interactive desktop production sink matrix; no timing retune is recommended. |

## H. Final recommendations

### MERGE AFTER CI FIX

- `cf1a82e` P0 terminal counter fix.
- Exact Python/uv pins and allowed tooling/attestation updates.
- Source bootloader provenance check.
- `zip 8.6.0 + zlib-rs` updater migration.
- Site patch/minor updates, Astro `7.2.9`, Bun `1.4.0`, and Dependabot grouping.

All have local evidence, but the required GitHub CI gate must be run on the final pushed commits.

### KEEP CURRENT

- TypeScript 6 until Astro language-server TS7 support exists.
- Pytest 8.4.2, SHA2 0.10, Rust dispatch architecture, and current timing constants.
- Existing `Compress-Archive` producer until a reproducibility fix is small and qualified.

### INVESTIGATE SEPARATELY

- Root cause and minimal fix for byte-identical release ZIPs.
- Interactive Windows production `SendInput` qualification with project-owned sink and scheduler/
  syscall timestamp separation.
- TypeScript 7 after Astro and typescript-eslint publish compatible releases.

### DO NOT DO

Do not combine this maintenance work with a packaging architecture rewrite, public ZIP format change,
new async/concurrency subsystem, Python runtime migration to 3.15, or Rust real-time dispatch
refactor.
