# Wave 7 Optimization Report

Date recorded: 2026-09-02
Wave 6 baseline: `main@509db0a191c22b8d082945473fa7b58fa6864f41`

This report records the first measured Wave 7 experiment. The only adopted
candidate so far is CI critical-path parallelization. No product, realtime,
compiler-profile, Tauri-feature, or cache implementation was changed.

## Method and baseline

The baseline is the exact post-merge Wave 6 main qualification run:

| Field | Value |
| --- | --- |
| Workflow | `33554867111` |
| Runner images | `ubuntu-24.04`; `windows-2025-vs2026` |
| Rust / Bun | `1.98.0` / `1.4.0` |
| Baseline cache | packaged exact cache miss; restore-key hit |
| Static | 3m36s |
| Windows validation | 8m16s |
| Packaged job | 21m07s |
| Required gate | 8s |
| Workflow creation to completion | 22m07s |
| Artifact | 9,239,431-byte upload; 4,531,763-byte inner ZIP |

The baseline package-required path serialized restricted repository checks in
the packaged job before running `dist`. It included, under the restricted
PATH, `check static`, `check rust`, `check desktop`, `dist`, `verify-dist`,
packaged shell/GUI smoke, updater exact-artifact qualification, and upload.

All comparisons below are provisional. The baseline and candidate use the
same hosted Windows image family, but they are not a controlled ten-run
sample and the candidate validation cache was warm while its packaged cache
was a miss.

## Candidate A — parallel restricted validation and exact dist

### Hypothesis

Move the restricted source/test proof into the required Windows validation
job while keeping the restricted exact production build and package proof in
the packaged job. Package-sensitive changes must force `code_required=true`,
so the moved restricted validation cannot be skipped. This removes serialized
work without removing a gate.

### Implementation

Candidate commits:

| Commit | Purpose |
| --- | --- |
| `c3615ffc9ed7baf0e838a38eaa647c17fd311bc7` | initial Lane A topology candidate; rejected before adoption because its new classifier assertion exposed a Clippy failure and the step lacked explicit intermediate exit checks |
| `f79a955956dd15c213aea662dba6d8f2149f273b` | corrected candidate; Clippy-clean assertions and fail-fast restricted validation |

The final candidate changes only CI classification/workflow routing and the
Wave 7 baseline/report records. Product and realtime source are unchanged.

### Coverage equivalence

| Gate | Wave 6 baseline | Candidate A |
| --- | --- | --- |
| Security / architecture / retirement | static job and packaged restricted proof | static job and required restricted validation |
| Rust fmt, Clippy, workspace tests | packaged restricted `check rust`; Windows job also validated | required restricted `check rust` in Windows job; normal package build remains separate |
| Desktop native checks | packaged restricted `check desktop`; Windows validation | required restricted `check desktop` in Windows job |
| Python-unavailable source/test proof | packaged job | Windows validation job, with `python`, `python3`, `py`, and `uv` unavailable |
| Exact production build | packaged job, restricted PATH | packaged job, restricted PATH |
| Provenance observation | build and copied binaries | unchanged in packaged `dist` |
| `verify-dist` | packaged job | packaged job |
| Shell and GUI smoke | normal and restricted packaged smoke | unchanged, both timed packaged paths |
| Updater exact-artifact suite | packaged job | packaged job |
| Artifact upload / manifest | packaged job | packaged job |

The classifier now makes every package-sensitive path code-required. The
required gate therefore waits for the Windows validation job as well as the
packaged job. The package job no longer runs the three repository checks, but
it still performs the complete restricted production-build and artifact
qualification path.

### Candidate qualification

| Metric | Baseline | Candidate A (`f79a9559`) | Delta | Cache state |
| --- | ---: | ---: | ---: | --- |
| Workflow critical path | 22m07s | 10m59s | -11m08s (-50.3%) | baseline packaged miss; candidate validate hit / packaged miss |
| Static job | 3m36s | 3m37s | +1s | separate job |
| Windows validation job | 8m16s | 6m39s | -1m37s | candidate exact validation cache hit |
| Packaged job | 21m07s | 10m05s | -11m02s | candidate packaged exact cache miss |
| `dist` qualification phase | about 5m19s after restricted checks | 6m12s build/qualification phase | not directly comparable | candidate restricted package path |
| Artifact upload | 9,239,431 bytes | 9,239,259 bytes | -172 bytes | different commit/build |
| Inner ZIP | 4,531,763 bytes | 4,531,747 bytes | -16 bytes | different commit/build |

Candidate workflow and jobs:

| Job | ID | Duration | Result |
| --- | ---: | ---: | --- |
| Classify validation layers | `100031910173` | 32s | PASS |
| Static and security gates | `100032087798` | 3m37s | PASS |
| Windows compatibility and unit tests | `100032087781` | 6m39s | PASS |
| Packaged frozen unsigned app smoke | `100032087678` | 10m05s | PASS |
| Required CI gate | `100035127542` | 11s | PASS |
| Workflow | `33560612129` | 10m59s | PASS |

The restricted validation log proves all four discovery commands were absent
and that `check static`, `check rust`, and `check desktop` completed with
PASS. The packaged log proves `dist`, `verify-dist`, both normal/restricted
shell and GUI smoke paths, updater qualification, and upload completed with
PASS. The candidate package's Rust-cache save reported a key reservation
collision during post-job cleanup; it did not affect qualification or artifact
contents and is not counted as a cache optimization.

### Candidate artifact inspection

The uploaded candidate artifact was downloaded from Actions and inspected,
not inferred from the generated summary:

| Field | Value |
| --- | --- |
| Artifact ID | `9821354646` |
| Upload size | 9,239,259 bytes |
| Upload digest | `sha256:b3998dcd0549b67590c9381ed755b76072b0fa0fce4f3d79b0645144a3fa8238` |
| Inner ZIP | `Sky-Auto-Player-v3.5.0.zip` |
| Inner ZIP size / SHA-256 | 4,531,747 bytes / `feda06896120cf6b00162c16f8c7d0dacd689a85a13d8c41e46c90eff4ce9edc` |
| MANIFEST SHA-256 | `626695d3f6a006cf68e2706febec075f1d5b8ace6a6a0f5ab224996fc2eaacc5` |
| Portable / managed entries | 102 / 101 |

The downloaded tree had no missing manifest entries, no unexpected managed
entries (the only un-managed file was `MANIFEST.json`), and zero file
hash/size mismatches. The ZIP entry set matched the release tree exactly.
The negative scan found zero Core/Python/runtime/test artifacts. Desktop and
calibration metadata observed from the copied binaries reported:

```text
repo_head                         = f79a955956dd15c213aea662dba6d8f2149f273b
desktop.native_build_commit       = f79a955956dd15c213aea662dba6d8f2149f273b
calibration.source_git_sha        = f79a955956dd15c213aea662dba6d8f2149f273b
calibration.native_build_id       = f79a955956dd15c213aea662dba6d8f2149f273b
MANIFEST.git_head                 = f79a955956dd15c213aea662dba6d8f2149f273b
MANIFEST.native_build_commit      = f79a955956dd15c213aea662dba6d8f2149f273b
PROVENANCE.repo_head              = f79a955956dd15c213aea662dba6d8f2149f273b
PROVENANCE.native_build_commit    = f79a955956dd15c213aea662dba6d8f2149f273b
native_source_fingerprint         = 6aa3f9d6f05f1c1778d3cdfba1e7cf2a5ba8867d7259501fefc9d42bd11c56d9
```

### Decision

`ADOPTED`, subject to final exact-HEAD qualification after this report is
committed. The observed reduction is large, coverage is preserved, and the
change is independently revertible in the CI/classifier files. No claim is
made that the engineering p50/p95 targets are met.

## Other candidates

These candidates were not changed before Lane A was measured. They remain
explicitly unattempted rather than being forced into the same experiment:

| Candidate | Decision | Reason |
| --- | --- | --- |
| Browser path-awareness | `NOT_ATTEMPTED` | audit and Lane A validation first; no evidence yet that browser work can be safely skipped for each path class |
| Static tooling cache | `NOT_ATTEMPTED` | no measured installation bottleneck or isolated A/B yet |
| Tauri feature pruning | `NOT_ATTEMPTED` | no feature graph A/B; preserve `wry`/`custom-protocol` and capability semantics |
| `sccache` | `NOT_ATTEMPTED` | no compile-dominant measurement or cache overhead study yet |
| `profile.dist` | `NOT_ATTEMPTED` | highest runtime risk; no optimized-profile RT comparison mechanism added |
| Test binary partitioning | `NOT_ATTEMPTED` | no compile-vs-execution bottleneck measurement |
| Binding modernization | `NOT_ATTEMPTED` | no material binding-generation cost identified |
| Frontend output reuse | `NOT_ATTEMPTED` | avoid stateful reuse without freshness/provenance evidence |

## Runtime and safety

Candidate A changes only required-job topology and classifier routing. It does
not alter the compiler profile, Tauri features, production binaries, player,
calibration, updater, or artifact layout. Existing Wave 6 authoritative
security, architecture, realtime/no-allocation, focus/release-all,
updater, provenance, and exact artifact suites remain green.

Therefore no new timing number is fabricated:

```text
No runtime build-semantics change; authoritative regression suite remains green.
```

## Metrics status and targets

```text
metrics_status = provisional
natural comparable main runs = 1 baseline run
authoritative p50/p95 = unavailable (fewer than 10 representative natural runs)
```

Engineering targets remain targets, not coverage-reduction permissions:

```text
ordinary PR p50 <= 5m
ordinary PR p95 <= 8m
main exact p50 <= 10m
```

The final exact PR workflow after this report commit will be the qualification
HEAD for human review. No merge, release tag, or follow-on optimization wave
is authorized by this report.
